//! Measure the wire format against the shipping compact stream on REAL d24
//! `weather_live` dynamics.
//!
//! Input: a real `collector_stream.bin` from a d24-scheduled capture
//! (`runs/batch1/deepwar_t1`). The stream's own d24 DATA records are the
//! shipping format's cost for the domain: a 48-byte header, then a fixed 32-byte
//! row per emitted strategic region, then a fixed 32-byte row per emitted
//! province — a full snapshot at the campaign's first weather hour and a sparse
//! delta every hour after, exactly what the recorder stores today. This tool
//! replays those records into the per-hour reconstructed weather state — the
//! same accumulator `world_fold::fold_weather` maintains — and re-encodes every
//! hour with this crate's grammar: a BASELINE at each 48-hour block start,
//! field-granular stored-minimal DELTAs between.
//!
//! **d24 is baseline + delta, not full replace.** An hour's state is not an
//! hour's rows: the shipping record is itself already a sparse patch, so both
//! arms are compared over the same reconstructed state and the wire arm's
//! saving is field granularity and quantization, not row-level deduplication.
//! The accumulator is `wire_format::d24::WeatherState`, shared with the tests,
//! so this harness cannot fold differently from what the schema was tested
//! against.
//!
//! **Nothing here re-derives a contract it can call.** Row and payload decoding
//! go through `wire_format::d24`'s public boundary; H4CR framing, record
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
//!   (`recorder_host/src/stream_fold.rs:277`), its DOMAIN_PERF hour binding
//!   (`:328`-`:365`), and the DATA arm's pending-PERF rule (`:378`-`:386`);
//! - the FRAME_END seal fields (`:46`-`:89`) and the seal's domain-count
//!   cross-check against observed non-probe PERF domains (`:411`-`:419`);
//! - the EOF rules of that tracker's `eof_check` (`:459`): no residual bytes, no
//!   hour left open, at least one FRAME_END, and the final record is a FRAME_END;
//! - d24's status-independent payload authority,
//!   `decode_compact_weather_data_payload`
//!   (`recorder_host/src/domain_data_authority.rs:16296`-`:16677`). It checks
//!   length, header relationships and every region/province row on EXCLUDED
//!   records too (while deliberately relaxing only `observed_region_rows` and
//!   full-snapshot `region_count` when failed), so [`check_authority_payload`]
//!   runs before status exclusion.
//!
//! That is a documented **subset** of the production boundary contract:
//! `recorder_host::stream_fold::verify_stream_logical_boundary` (`:537`) is the
//! full authority. The remaining crate-boundary gap is malformed payload data
//! for another domain: this one-domain `wire_format` harness can replay d24's
//! authority but cannot depend on all of `recorder_host`. The measured capture
//! was recorder-validated, so its other domains have already passed that check.
//!
//! These seal, PERF, pending-PERF, payload-authority and stated-identity checks
//! are the five-check sweep. The capture is legacy and recorder-valid, so the
//! default remains legacy and no accepted record or output byte changes.
//!
//! Outputs, for the compression step (`scripts/compress_arms.py`):
//!   ship_d24.bin   — the d24 DATA records verbatim (32-byte H4CR header +
//!                    payload), concatenated in stream order
//!   wire_new.bin   — the wire-format bodies, concatenated in hour order
//!   *.offsets      — hour-boundary byte offsets, one per line, for exact
//!                    block-span slicing
//!
//! It also prints the real churn surface (changed rows per hour at stored
//! precision).
//!
//! **Only d24 DATA payloads are validated.** Production runs
//! `validate_domain_data_payload_at` on the DATA payload of EVERY domain
//! (`stream_fold.rs:593`); this harness cannot, because validating another
//! domain's payload means owning that domain's authority, and `wire_format`
//! must not depend on `recorder_host`. Every DATA record of any other domain
//! therefore passes with its H4CR header, sequence continuity, hour-boundary,
//! PERF-binding and FRAME_END rules checked but its **payload bytes unread**.
//! The concrete hazard — a d24 record relabelled to another domain id, or
//! dropped outright, silently shrinking the measurement — is closed by an
//! explicit completeness check in `scan_stream`: every closed game hour must
//! carry exactly one d24 DATA record, of any status. That is a per-capture
//! assertion, not a grammar rule; a capture where d24 is not scheduled every
//! hour fails loudly there and a human adjusts.

use std::collections::BTreeSet;
use std::io::Write;
use std::ops::Range;

use compact_contract::stream_validator::{StatefulStreamValidator, StreamIdentityMode};
use compact_contract::{
    compact_status_failure_flags_after_expected_partial_tolerance, compact_status_unknown_flags_v1,
    expected_partial_tolerance, lane_local_status_mask, parse_compact_record_header,
    COMPACT_FRAME_END_PAYLOAD_LEN, COMPACT_RECORD_HEADER_LEN, COMPACT_RECORD_TYPE_DOMAIN_DATA,
    COMPACT_RECORD_TYPE_DOMAIN_PERF, COMPACT_RECORD_TYPE_FRAME_END, COMPACT_STATUS_FAILURE_MASK,
};
use wire_format::body::{
    apply, decode_baseline, decode_delta, diff, encode_baseline, encode_delta, KeyedDelta,
    KeyedState,
};
use wire_format::d24::{self, FoldOutcome, WeatherState};
use wire_format::Schema;

const D24: u16 = 24;
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
// Re-exported, not re-declared: a local `(1 << 14) - 1` became wrong the moment
// the contract assigned bit 14 to the measured-empty badge.
const FRAME_END_KNOWN_STATUS_MASK: u32 = compact_contract::COMPACT_STATUS_V1_KNOWN_MASK;
const PROBE_DOMAIN_ID: u16 = 250;

/// One usable d24 hour: its game hour and the byte span of its DATA record in
/// the source stream (the shipping arm is those bytes verbatim, header
/// included).
///
/// Unlike d63's harness this carries no folded state. d24's state is the
/// *accumulated* weather map — 305 regions plus up to 65,536 provinces — so
/// holding one snapshot per hour would hold the whole campaign's weather in
/// memory at once. The state is therefore re-folded in the encoding pass from
/// the same records, through the same `WeatherState`.
struct D24Hour {
    game_hour: i32,
    ship_span: Range<usize>,
}

struct StreamScan {
    hours: Vec<D24Hour>,
    /// d24 records whose payload status flags mark the domain-hour failed.
    failure_records: usize,
    /// d24 delta records that arrived before any baseline; the fold discards
    /// them (`world_fold.rs:16972`), so they are not weather data.
    delta_before_baseline: usize,
    /// Measured d24 hours that do not follow the previous measured one by one.
    non_contiguous: usize,
    /// Reconstructed row counts at the last measured hour, for the report.
    final_regions: usize,
    final_provinces: usize,
    baseline_records: usize,
}

/// d24 carries no lane-local status bits and no expected-partial tolerance, so
/// `compact_contract`'s failure predicate reduces exactly to the global mask —
/// which is what `d24::FAILURE_STATUS_MASK` (the local copy retained because d24
/// is not admitted to the production-codec dependency surface) has to equal.
/// Asserting the three facts proves the two predicates agree for every
/// status word, rather than for the words this capture happens to contain.
fn assert_failure_predicates_agree() {
    assert_eq!(
        lane_local_status_mask(D24),
        0,
        "d24 gained a lane-local status bit; d24::FAILURE_STATUS_MASK must be revisited"
    );
    assert!(
        expected_partial_tolerance(D24).is_none(),
        "d24 gained an expected-partial tolerance; d24::FAILURE_STATUS_MASK must be revisited"
    );
    assert_eq!(
        d24::FAILURE_STATUS_MASK,
        COMPACT_STATUS_FAILURE_MASK,
        "the library's failure mask has drifted from compact_contract's"
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

fn u32_at(bytes: &[u8], offset: usize) -> u32 {
    u32::from_le_bytes(
        bytes[offset..offset + 4]
            .try_into()
            .expect("bounded row word"),
    )
}

/// Replay `decode_compact_weather_data_payload`
/// (`recorder_host/src/domain_data_authority.rs:16296`-`:16677`) on every d24
/// record. Unlike `d24::split_payload`, this preserves production's two
/// failure-only relaxations instead of making excluded records stricter.
fn check_authority_payload(
    schema: &Schema,
    header: &d24::PayloadHeader,
    payload: &[u8],
    at_byte: usize,
) -> Result<(), String> {
    let invalid = |reason: String| Err(format!("d24 authority at byte {at_byte}: {reason}"));
    let region_count = header.region_count as usize;
    let province_count = header.province_count as usize;
    let region_bytes = region_count
        .checked_mul(d24::REGION_ROW_LEN)
        .ok_or_else(|| format!("d24 authority at byte {at_byte}: region count overflows"))?;
    let province_bytes = province_count
        .checked_mul(d24::PROVINCE_ROW_LEN)
        .ok_or_else(|| format!("d24 authority at byte {at_byte}: province count overflows"))?;
    let expected = d24::HEADER_LEN
        .checked_add(region_bytes)
        .and_then(|len| len.checked_add(province_bytes))
        .ok_or_else(|| format!("d24 authority at byte {at_byte}: payload length overflows"))?;
    if payload.len() != expected {
        return invalid(format!("payload length {} != {expected}", payload.len()));
    }

    let failed =
        compact_status_failure_flags_after_expected_partial_tolerance(D24, header.status_flags)
            != 0;
    let delta = header.is_delta();
    let observed_regions = header.observed_region_rows as usize;
    let observed_provinces = header.observed_province_rows as usize;
    let emitted_rows = region_count.saturating_add(province_count);
    if !failed && observed_regions != d24::REGION_ROWS {
        return invalid("observed_region_rows is not 305".to_string());
    }
    if observed_provinces != 0
        && !(d24::MIN_PROVINCE_ROWS..=d24::MAX_PROVINCE_ROWS).contains(&observed_provinces)
    {
        return invalid("observed_province_rows is outside its allowed range".to_string());
    }
    if !failed && !delta && region_count != d24::REGION_ROWS {
        return invalid("full snapshot region_count is not 305".to_string());
    }
    if !delta && observed_provinces != 0 && province_count != observed_provinces {
        return invalid("full snapshot province_count differs from observed count".to_string());
    }
    if region_count > observed_regions {
        return invalid("region_count exceeds observed_region_rows".to_string());
    }
    if observed_provinces == 0 && province_count != 0 {
        return invalid("province rows emitted with zero observed_province_rows".to_string());
    }
    if province_count > observed_provinces {
        return invalid("province_count exceeds observed_province_rows".to_string());
    }
    if header.changed_rows as usize > emitted_rows {
        return invalid("changed_rows exceeds emitted rows".to_string());
    }
    if delta && header.changed_rows as usize != emitted_rows {
        return invalid("delta changed_rows does not equal emitted rows".to_string());
    }
    if header.scale_raw != d24::FIXED_SCALE_RAW {
        return invalid("scale_raw differs from the fixed scale".to_string());
    }
    if header.active_region_count as usize > region_count {
        return invalid("active_region_count exceeds region_count".to_string());
    }
    if header.reserved0 != 0 || header.reserved1 != 0 {
        return invalid("header reserved fields are nonzero".to_string());
    }

    let region_collection = schema
        .collections
        .iter()
        .find(|collection| collection.collection_id == d24::REGIONS)
        .ok_or_else(|| "d24 schema has no regions collection".to_string())?;
    let province_collection = schema
        .collections
        .iter()
        .find(|collection| collection.collection_id == d24::PROVINCES)
        .ok_or_else(|| "d24 schema has no provinces collection".to_string())?;
    let region_end = d24::HEADER_LEN + region_bytes;
    let region_rows = &payload[d24::HEADER_LEN..region_end];
    let province_rows = &payload[region_end..];

    let mut seen_regions = BTreeSet::new();
    let mut computed_active_regions = 0usize;
    for (row_index, row) in region_rows.chunks_exact(d24::REGION_ROW_LEN).enumerate() {
        d24::record_from_region_row(region_collection, row, 0)
            .map_err(|e| format!("d24 authority region row {row_index} at byte {at_byte}: {e}"))?;
        let region_id = d24::region_id_of_row(row).expect("row mapper checked the id");
        if !seen_regions.insert(region_id) {
            return invalid(format!(
                "region row {row_index} duplicates region_id {region_id}"
            ));
        }
        if u32_at(row, 12) != 0 {
            computed_active_regions += 1;
        }
    }
    if computed_active_regions != header.active_region_count as usize {
        return invalid("active_region_count differs from the row-derived count".to_string());
    }

    let mut seen_provinces = BTreeSet::new();
    for (row_index, row) in province_rows
        .chunks_exact(d24::PROVINCE_ROW_LEN)
        .enumerate()
    {
        let province_id = d24::province_id_of_row(row)
            .map_err(|e| format!("d24 authority province row {row_index}: {e}"))?;
        d24::check_province_id(province_id, header.observed_province_rows).map_err(|e| {
            format!("d24 authority province row {row_index} at byte {at_byte}: {e}")
        })?;
        if !seen_provinces.insert(province_id) {
            return invalid(format!(
                "province row {row_index} duplicates province_id {province_id}"
            ));
        }
        let flags = u32_at(row, 4);
        if flags & d24::PROVINCE_STRUCT_VALIDATED == 0 {
            return invalid(format!(
                "province row {row_index} is missing the struct validation flag"
            ));
        }
        d24::record_from_province_row(province_collection, row, 0).map_err(|e| {
            format!("d24 authority province row {row_index} at byte {at_byte}: {e}")
        })?;
    }
    Ok(())
}

/// Validate the whole stream against the canonical contract and fold its usable
/// d24 DATA records, keeping only the byte spans the measurement needs. Every
/// rejection path is an `Err`, never a silently shorter measurement and never a
/// panic — see the module docs for what is enforced and what the recorder's own
/// validator additionally checks.
fn scan_stream(
    stream: &[u8],
    schema: &Schema,
    identity_mode: StreamIdentityMode,
) -> Result<StreamScan, String> {
    let mut validator = match identity_mode {
        StreamIdentityMode::IdentityV1 => StatefulStreamValidator::identity_v1(),
        StreamIdentityMode::Legacy => StatefulStreamValidator::legacy(),
    };

    let mut hours: Vec<D24Hour> = Vec::new();
    let mut failure_records = 0usize;
    let mut delta_before_baseline = 0usize;
    let mut non_contiguous = 0usize;
    let mut baseline_records = 0usize;
    let mut state = WeatherState::default();

    let mut open_hour: Option<i32> = None;
    let mut last_closed_hour: Option<i32> = None;
    let mut last_record_type: Option<u16> = None;
    let mut pending_perf_hour: Option<i32> = None;
    let mut non_probe_perf_domains: BTreeSet<u16> = BTreeSet::new();
    // Hour of the last d24 DATA record of ANY status — the uniqueness rule.
    let mut last_d24_hour: Option<i32> = None;
    // Hour of the last d24 record that entered the measurement — the
    // contiguity report, which a skipped failure hour legitimately breaks.
    let mut last_measured_hour: Option<i32> = None;

    // Closed game hours and d24 DATA records of ANY status — the
    // completeness rule at the end of the scan.
    let mut closed_hours = 0usize;
    let mut d24_records = 0usize;
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

        if header.domain_id == D24 && header.record_type == COMPACT_RECORD_TYPE_DOMAIN_DATA {
            d24_records += 1;
            // One d24 DATA record per game hour, checked over EVERY d24 record
            // and BEFORE any status-based exclusion. d24 accumulates, so a
            // second record for the same hour would fold twice into one clock
            // time — and gating this on the record being usable would let a
            // success/failure pair through the rule entirely.
            if let Some(previous) = last_d24_hour {
                if header.game_hour_raw <= previous {
                    return Err(format!(
                        "d24 DATA at byte {pos} carries hour {} after hour {previous}",
                        header.game_hour_raw
                    ));
                }
            }
            last_d24_hour = Some(header.game_hour_raw);

            let d24_header = d24::parse_header(payload)
                .map_err(|e| format!("d24 payload at byte {pos}: {e}"))?;

            // VALIDITY BEFORE CLASSIFICATION. Everything below that reads the
            // status word — the failure predicate here, the baseline/delta and
            // failure decisions inside `fold_payload` — classifies the v1
            // vocabulary; none of it is a validity predicate.
            // `compact_contract/src/lib.rs:121`-`:123` says so in as many words:
            // "Callers handling untrusted status words must first reject a
            // non-zero `compact_status_unknown_flags_v1` result; otherwise an
            // unassigned bit has no defined classification." Bits 15..=31 are
            // unassigned (`status_flags_base.rs:40`-`:44`), and an unassigned bit
            // "is not evidence of a future informational status".
            //
            // Without this, a record with `status_flags = 1 << 14` reads as "not
            // a failure" — the failure helper masks with the *known* failure set
            // and returns zero for a bit it does not know — and the hour is
            // measured as healthy. The production lane rejects first
            // (`lanes/division_state_movement_live/domain20_compact.rs:419`);
            // this harness does the same rather than re-deriving a lenient rule.
            // No capture carries an unassigned bit, so no measured number moves.
            let unknown_status = compact_status_unknown_flags_v1(d24_header.status_flags);
            if unknown_status != 0 {
                return Err(format!(
                    "d24 DATA at byte {pos}: hour {} carries unassigned v1 status bits \
                     {unknown_status:#x}; an unassigned bit has no defined classification",
                    header.game_hour_raw
                ));
            }

            check_authority_payload(schema, &d24_header, payload, pos)?;

            // `world_fold::handle_compact_failure` (`world_fold.rs:2843`, called
            // at `:16969`) marks the domain-hour unknown and RETAINS the prior
            // accumulator on a failure status, folding nothing from this record.
            // The predicate is `compact_contract`'s own, not a copied bit mask.
            let failed = compact_status_failure_flags_after_expected_partial_tolerance(
                D24,
                d24_header.status_flags,
            ) != 0;

            // Fold exactly as `fold_weather` does — including every way a row
            // can be rejected, which is a property of the stream and so belongs
            // to the pass that either accepts the stream or refuses it.
            match state
                .fold_payload(schema, payload, 0)
                .map_err(|e| format!("d24 payload at byte {pos}: {e}"))?
            {
                FoldOutcome::FailureRetained => {
                    assert!(
                        failed,
                        "the library and contract failure predicates disagree"
                    );
                    failure_records += 1;
                }
                FoldOutcome::DeltaBeforeBaseline => delta_before_baseline += 1,
                FoldOutcome::Folded { reset } => {
                    assert!(!failed, "a failed record must not fold");
                    if reset {
                        baseline_records += 1;
                    }
                    if let Some(previous) = last_measured_hour {
                        if header.game_hour_raw != previous + 1 {
                            non_contiguous += 1;
                        }
                    }
                    last_measured_hour = Some(header.game_hour_raw);
                    hours.push(D24Hour {
                        game_hour: header.game_hour_raw,
                        ship_span: pos..record_end,
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
        return Err("stream carries no usable d24 DATA records".to_string());
    }

    // COMPLETENESS: every closed game hour must carry exactly one d24 DATA
    // record, of any status. Production validates the payload of EVERY domain's
    // DATA record (`stream_fold.rs:593`); this harness cannot (module docs), so
    // a d24 record relabelled to another domain id — or lost outright — would
    // otherwise be silently ignored and the run would report a plausible
    // measurement over fewer hours. Per-hour uniqueness is already enforced
    // above, so equality of the two counts is exactly-one per closed hour. A
    // per-capture assertion, not a grammar rule: a capture that does not
    // schedule d24 every hour fails here loudly and a human adjusts, instead
    // of a number quietly shrinking.
    if d24_records != closed_hours {
        return Err(format!(
            "stream closes {closed_hours} game hour(s) but carries {d24_records} d24 DATA \
             record(s); every closed hour of a d24-scheduled capture must carry exactly one, \
             and this harness cannot validate the payloads of other domains to see where the \
             difference went"
        ));
    }

    Ok(StreamScan {
        hours,
        failure_records,
        delta_before_baseline,
        non_contiguous,
        final_regions: state.regions.len(),
        final_provinces: state.provinces.len(),
        baseline_records,
    })
}

fn main() {
    assert_failure_predicates_agree();

    const USAGE: &str = "usage: bench_real_d24 <collector_stream.bin> <out dir> \
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
    let schema = d24::schema();

    // ---- Pass 1: validate the WHOLE stream and fold its d24 DATA records. ---
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

    // ---- Pass 2: re-fold the usable records and encode both arms. Nothing here
    // reads a record pass 1 refused, so every assertion below is about the
    // codec, not about the input. -------------------------------------------

    let mut ship = Vec::new();
    let mut ship_offsets = vec![0usize];
    let mut wire = Vec::new();
    let mut wire_offsets = vec![0usize];

    let mut writer = WeatherState::default();
    // The reader's own reconstructed state, and therefore §7.1's prior: a
    // writer that diffed against its own state instead could emit a delta the
    // reader reconstructs differently and never notice.
    let mut reader: Vec<KeyedState> = vec![KeyedState::new(), KeyedState::new()];
    let mut reader_primed = false;

    let mut hours_seen = 0usize;
    let mut baselines = 0usize;
    // Baselines the §1.5 gap rule forced outside a scheduled block start.
    let mut gap_forced_baselines = 0usize;
    let mut previous_measured_hour: Option<i32> = None;
    let mut empty_hours = 0usize;
    let mut changed_rows_total = 0usize;
    let mut tombstones_total = 0usize;
    let mut regions_total = 0usize;
    let mut provinces_total = 0usize;
    // The block policy costs a full re-statement of the whole weather map every
    // `BLOCK_HOURS`, where the shipping stream states it once for the campaign.
    // Reporting the two parts separately is what separates the schema's cost
    // from the block policy's; a single total hides which one a number is about.
    let mut wire_baseline_bytes = 0usize;
    let mut wire_delta_bytes = 0usize;
    let mut ship_baseline_bytes = 0usize;
    let mut ship_delta_bytes = 0usize;

    for hour in &scan.hours {
        let record = &stream[hour.ship_span.clone()];
        let payload = &record[COMPACT_RECORD_HEADER_LEN..];
        let outcome = writer
            .fold_payload(&schema, payload, 0)
            .expect("pass 1 already accepted this payload");
        assert!(
            matches!(outcome, FoldOutcome::Folded { .. }),
            "pass 1 only kept records that fold"
        );

        // Shipping arm: the record's exact bytes, header included.
        ship.extend_from_slice(record);
        ship_offsets.push(ship.len());
        if matches!(outcome, FoldOutcome::Folded { reset: true }) {
            ship_baseline_bytes += record.len();
        } else {
            ship_delta_bytes += record.len();
        }

        regions_total += writer.regions.len();
        provinces_total += writer.provinces.len();
        let next: [&KeyedState; 2] = [&writer.regions, &writer.provinces];

        // New arm: BASELINE at each block start, stored-minimal DELTA between.
        let hour_start = wire.len();
        let baseline_here = opens_baseline(hours_seen, previous_measured_hour, hour.game_hour);
        if baseline_here && hours_seen % BLOCK_HOURS != 0 {
            gap_forced_baselines += 1;
        }
        let mut emitted = true;
        if baseline_here || !reader_primed {
            encode_baseline(
                &schema,
                &[writer.regions.clone(), writer.provinces.clone()],
                &mut wire,
            )
            .expect("baseline encodes");
            baselines += 1;
        } else {
            let deltas: Vec<Option<KeyedDelta>> = schema
                .collections
                .iter()
                .enumerate()
                .map(|(i, collection)| {
                    let d = diff(collection, &reader[i], next[i]);
                    if d.is_empty() {
                        None
                    } else {
                        Some(d)
                    }
                })
                .collect();
            if deltas.iter().all(Option::is_none) {
                empty_hours += 1;
                emitted = false;
            } else {
                for d in deltas.iter().flatten() {
                    changed_rows_total += d.changed.len();
                    tombstones_total += d.tombstones.len();
                }
                encode_delta(&schema, &reader, &deltas, &mut wire).expect("delta encodes");
            }
        }
        wire_offsets.push(wire.len());
        if baseline_here {
            wire_baseline_bytes += wire.len() - hour_start;
        } else {
            wire_delta_bytes += wire.len() - hour_start;
        }

        // Reader replay: decode the bytes just written against the reader's OWN
        // reconstructed prior, exactly as a conforming reader would, and require
        // the result to be stored-equal to this hour's folded state. This is
        // what makes the size claim a claim about decodable bytes. An hour that
        // emitted nothing is still checked: the reader's retained state must
        // already equal the fold's, which is precisely what "nothing changed"
        // asserts.
        if emitted {
            let hour_bytes = &wire[hour_start..];
            if baseline_here || !reader_primed {
                let (states, consumed) =
                    decode_baseline(&schema, hour_bytes, 0).expect("baseline decodes");
                assert_eq!(consumed, hour_bytes.len(), "baseline consumed exactly");
                reader = states;
                reader_primed = true;
            } else {
                let (deltas, consumed) =
                    decode_delta(&schema, &reader, hour_bytes, 0).expect("delta decodes");
                assert_eq!(consumed, hour_bytes.len(), "delta consumed exactly");
                for (i, delta) in deltas.iter().enumerate() {
                    if let Some(delta) = delta {
                        apply(&mut reader[i], delta);
                    }
                }
            }
        }
        for (i, collection) in schema.collections.iter().enumerate() {
            assert_eq!(
                reader[i].len(),
                next[i].len(),
                "game hour {}: collection {} row count",
                hour.game_hour,
                collection.collection_id
            );
            assert!(
                diff(collection, &reader[i], next[i]).is_empty(),
                "game hour {}: collection {} reader state is not stored-equal to the fold",
                hour.game_hour,
                collection.collection_id
            );
        }

        previous_measured_hour = Some(hour.game_hour);
        hours_seen += 1;
    }

    for (name, bytes) in [("ship_d24.bin", &ship), ("wire_new.bin", &wire)] {
        let mut f = std::fs::File::create(std::path::Path::new(&out_dir).join(name)).unwrap();
        f.write_all(bytes).unwrap();
    }
    for (name, offsets) in [
        ("ship_d24.offsets", &ship_offsets),
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
        "shipping snapshots   {} full, {} delta records",
        scan.baseline_records,
        hours_seen - scan.baseline_records
    );
    println!(
        "failure-status hours {} (excluded from both arms)",
        scan.failure_records
    );
    println!(
        "delta-before-baseline {} (folded nothing; excluded from both arms)",
        scan.delta_before_baseline
    );
    println!(
        "reconstructed rows   {} regions, {} provinces at the last measured hour",
        scan.final_regions, scan.final_provinces
    );
    println!(
        "state rows avg       {:.1} regions, {:.1} provinces",
        regions_total as f64 / hours_seen as f64,
        provinces_total as f64 / hours_seen as f64
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
    println!("ship_d24.bin         {} bytes", ship.len());
    println!(
        "wire_new.bin         {} bytes ({:.1}%)",
        wire.len(),
        100.0 * wire.len() as f64 / ship.len() as f64
    );
    // The two arms do NOT re-state the world equally often, and that dominates
    // the totals above: the shipping stream carries one full snapshot for the
    // whole campaign, while the block grammar restates it every 48 hours so a
    // reader can seek. Separating the parts is what makes the totals readable.
    println!(
        "  full-state bytes   ship {ship_baseline_bytes} in {} record(s), \
         wire {wire_baseline_bytes} in {baselines} baseline(s)",
        scan.baseline_records
    );
    println!(
        "  patch bytes        ship {ship_delta_bytes}, wire {wire_delta_bytes} ({:.1}%)",
        100.0 * wire_delta_bytes as f64 / ship_delta_bytes.max(1) as f64
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use compact_contract::COMPACT_DOMAIN_FRAME;

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

    fn region_row(region_id: u32, temperature_raw: i32) -> Vec<u8> {
        let mut row = Vec::new();
        row.extend_from_slice(&region_id.to_le_bytes());
        row.extend_from_slice(&0u32.to_le_bytes()); // flags: inactive, unnamed
        row.extend_from_slice(&temperature_raw.to_le_bytes());
        row.extend_from_slice(&[0u8; 16]); // phenomenon, next change, mods, intensity
        row.extend_from_slice(&0u32.to_le_bytes()); // reserved
        assert_eq!(row.len(), d24::REGION_ROW_LEN);
        row
    }

    /// The flag word the collector emits for a fully-read province.
    fn province_flags() -> u32 {
        d24::PROVINCE_TEMP_PRESENT
            | d24::PROVINCE_TEMP_CHANGE_PRESENT
            | d24::PROVINCE_WATER_PRESENT
            | d24::PROVINCE_SNOW_PRESENT
            | d24::PROVINCE_MUD_PRESENT
            | d24::PROVINCE_CUSTOM_MODIFIER_COUNT_PRESENT
            | d24::PROVINCE_STRUCT_VALIDATED
            | d24::PROVINCE_MAP_RECOLOR
            | d24::PROVINCE_DYNAMIC_LUT
    }

    fn province_row(province_id: u32, temperature_raw: i32) -> Vec<u8> {
        let mut row = Vec::new();
        row.extend_from_slice(&province_id.to_le_bytes());
        row.extend_from_slice(&province_flags().to_le_bytes());
        row.extend_from_slice(&temperature_raw.to_le_bytes());
        row.extend_from_slice(&1i32.to_le_bytes()); // temperature_change
        row.extend_from_slice(&2i32.to_le_bytes()); // water
        row.extend_from_slice(&3i32.to_le_bytes()); // snow
        row.extend_from_slice(&0u32.to_le_bytes()); // custom_modifier_count
        row.extend_from_slice(&0u32.to_le_bytes()); // reserved
        assert_eq!(row.len(), d24::PROVINCE_ROW_LEN);
        row
    }

    fn payload(
        status_flags: u32,
        observed_province_rows: u32,
        regions: &[Vec<u8>],
        provinces: &[Vec<u8>],
    ) -> Vec<u8> {
        let mut p = Vec::new();
        p.extend_from_slice(&0u64.to_le_bytes());
        p.extend_from_slice(&(regions.len() as u32).to_le_bytes());
        p.extend_from_slice(&((regions.len() + provinces.len()) as u32).to_le_bytes());
        p.extend_from_slice(&status_flags.to_le_bytes());
        p.extend_from_slice(&(provinces.len() as u32).to_le_bytes());
        p.extend_from_slice(&(d24::REGION_ROWS as u32).to_le_bytes());
        p.extend_from_slice(&observed_province_rows.to_le_bytes());
        p.extend_from_slice(&d24::FIXED_SCALE_RAW.to_le_bytes());
        p.extend_from_slice(&0u32.to_le_bytes()); // active_region_count
        p.extend_from_slice(&0u32.to_le_bytes()); // reserved0
        p.extend_from_slice(&0u32.to_le_bytes()); // reserved1
        assert_eq!(p.len(), d24::HEADER_LEN);
        for r in regions.iter().chain(provinces) {
            p.extend_from_slice(r);
        }
        p
    }

    /// A full snapshot: all 305 regions, no provinces (`observed_province_rows`
    /// may legally be zero, `world_fold.rs:16990`), which keeps the fixtures
    /// small without leaving the grammar.
    fn baseline_payload(status_flags: u32, temperature_raw: i32) -> Vec<u8> {
        let regions: Vec<Vec<u8>> = (0..d24::REGION_ROWS as u32)
            .map(|id| region_row(id, temperature_raw))
            .collect();
        payload(status_flags, 0, &regions, &[])
    }

    fn delta_payload(status_flags: u32, region_id: u32, temperature_raw: i32) -> Vec<u8> {
        payload(
            status_flags | d24::STATUS_DELTA_DATA,
            0,
            &[region_row(region_id, temperature_raw)],
            &[],
        )
    }

    /// Two closed hours: a full snapshot, then a one-row delta.
    fn good_stream() -> Vec<u8> {
        let mut s = Vec::new();
        s.extend(record(
            1,
            100,
            D24,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &baseline_payload(0, 10),
        ));
        s.extend(frame_end(2, 100));
        s.extend(record(
            3,
            101,
            D24,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &delta_payload(0, 4, 11),
        ));
        s.extend(frame_end(4, 101));
        s
    }

    fn scan(stream: &[u8]) -> Result<StreamScan, String> {
        scan_stream(stream, &d24::schema(), StreamIdentityMode::Legacy)
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
    fn the_library_and_contract_failure_predicates_agree() {
        assert_failure_predicates_agree();
    }

    #[test]
    fn a_well_formed_stream_yields_its_d24_hours() {
        let s = scan(&good_stream()).expect("accepted");
        assert_eq!(s.hours.len(), 2);
        assert_eq!(s.hours[0].game_hour, 100);
        assert_eq!(s.hours[1].game_hour, 101);
        assert_eq!(s.failure_records, 0);
        assert_eq!(s.delta_before_baseline, 0);
        assert_eq!(s.non_contiguous, 0);
        assert_eq!(s.baseline_records, 1, "one full snapshot");
        assert_eq!(
            s.final_regions,
            d24::REGION_ROWS,
            "the delta deleted nothing"
        );
    }

    #[test]
    fn an_unassigned_status_bit_is_refused_not_classified() {
        // `compact_contract/src/lib.rs:121`-`:123`: the failure helpers classify
        // the v1 vocabulary and are not validity predicates, so an unassigned bit
        // must be rejected BEFORE classification — otherwise it silently reads as
        // "not a failure", `fold_payload` gives it a baseline/delta meaning, and
        // the hour is measured as healthy.
        for bit in 15..32u32 {
            let status = 1u32 << bit;
            // Alone, combined with a known failure bit, and combined with the
            // informational DELTA_DATA bit; as a snapshot and as an incremental
            // record. The rejection must not depend on any of it.
            for extra in [0u32, 1, d24::STATUS_DELTA_DATA] {
                let mut s = Vec::new();
                s.extend(record(
                    1,
                    100,
                    D24,
                    COMPACT_RECORD_TYPE_DOMAIN_DATA,
                    &baseline_payload(status | extra, 10),
                ));
                s.extend(frame_end(2, 100));
                assert!(
                    scan(&s).is_err(),
                    "status bit {bit} (+{extra:#x}) on a snapshot must be refused"
                );
                // And on a record that follows a clean baseline.
                let mut after = Vec::new();
                after.extend(record(
                    1,
                    100,
                    D24,
                    COMPACT_RECORD_TYPE_DOMAIN_DATA,
                    &baseline_payload(0, 10),
                ));
                after.extend(frame_end(2, 100));
                after.extend(record(
                    3,
                    101,
                    D24,
                    COMPACT_RECORD_TYPE_DOMAIN_DATA,
                    &delta_payload(status | extra, 4, 11),
                ));
                after.extend(frame_end(4, 101));
                assert!(
                    scan(&after).is_err(),
                    "status bit {bit} (+{extra:#x}) on a delta must be refused"
                );
            }
        }
        // And the whole known v1 mask, minus the failure bits, is still accepted:
        // the check must reject unassigned bits and nothing else. DELTA_DATA is
        // the one d24 itself reads, and it is a baseline/delta selector, not a
        // failure.
        for informational in [1u32 << 4, 1 << 5, 1 << 11, 1 << 13] {
            let mut s = Vec::new();
            s.extend(record(
                1,
                100,
                D24,
                COMPACT_RECORD_TYPE_DOMAIN_DATA,
                &baseline_payload(informational, 10),
            ));
            s.extend(frame_end(2, 100));
            s.extend(record(
                3,
                101,
                D24,
                COMPACT_RECORD_TYPE_DOMAIN_DATA,
                &delta_payload(informational, 4, 11),
            ));
            s.extend(frame_end(4, 101));
            let result =
                scan(&s).unwrap_or_else(|e| panic!("informational bit {informational:#x}: {e}"));
            assert_eq!(result.hours.len(), 2, "bit {informational:#x}");
            assert_eq!(result.failure_records, 0, "bit {informational:#x}");
            assert_eq!(result.delta_before_baseline, 0, "bit {informational:#x}");
        }
    }

    #[test]
    fn a_delta_before_any_baseline_is_excluded_rather_than_measured() {
        // `world_fold.rs:16972`: the fold discards it, so it is not weather data
        // and must not enter either arm — but it does not make the stream bad.
        let mut s = Vec::new();
        s.extend(record(
            1,
            100,
            D24,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &delta_payload(0, 4, 10),
        ));
        s.extend(frame_end(2, 100));
        s.extend(record(
            3,
            101,
            D24,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &baseline_payload(0, 11),
        ));
        s.extend(frame_end(4, 101));
        let scanned = scan(&s).expect("the stream itself is well-formed");
        assert_eq!(scanned.delta_before_baseline, 1);
        assert_eq!(scanned.hours.len(), 1);
        assert_eq!(scanned.hours[0].game_hour, 101);
    }

    #[test]
    fn a_stream_truncated_on_a_record_boundary_is_rejected() {
        // A cut immediately after a d24 DATA record leaves that hour open and
        // ends the stream on a record that is not a FRAME_END.
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
        // Every cut that is not a record boundary. Strided so the sweep stays
        // cheap on a 305-region fixture; the boundaries themselves are covered
        // exhaustively by the test above.
        for cut in (1..full.len()).step_by(7) {
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
            D24,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &baseline_payload(0, 10),
        ));
        s.extend(frame_end(2, 999)); // closes an hour that is not open
        assert!(scan(&s).is_err());

        let mut s = Vec::new();
        s.extend(frame_end(1, 100));
        assert!(scan(&s).is_err(), "a FRAME_END with nothing open");
    }

    #[test]
    fn non_monotonic_hours_are_rejected() {
        let mut s = Vec::new();
        s.extend(record(
            1,
            101,
            D24,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &baseline_payload(0, 10),
        ));
        s.extend(frame_end(2, 101));
        s.extend(record(
            3,
            100,
            D24,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &delta_payload(0, 4, 11),
        ));
        s.extend(frame_end(4, 100));
        assert!(scan(&s).is_err());
    }

    #[test]
    fn a_second_d24_record_in_one_hour_is_rejected() {
        // d24 accumulates; two records in one hour would fold twice into one
        // clock time. The rule is checked over EVERY d24 DATA record and before
        // status-based exclusion, so all four success/failure orderings are
        // refused — gating it on the record being usable would let a success +
        // failure pair through.
        const FAILED: u32 = 1; // COMPACT_STATUS_STATE_UNAVAILABLE
        for (first, second) in [(0u32, 0u32), (0, FAILED), (FAILED, 0), (FAILED, FAILED)] {
            let mut s = Vec::new();
            s.extend(record(
                1,
                100,
                D24,
                COMPACT_RECORD_TYPE_DOMAIN_DATA,
                &baseline_payload(first, 10),
            ));
            s.extend(record(
                2,
                100,
                D24,
                COMPACT_RECORD_TYPE_DOMAIN_DATA,
                &baseline_payload(second, 11),
            ));
            s.extend(frame_end(3, 100));
            assert!(
                scan(&s).is_err(),
                "two d24 records in hour 100 with statuses {first:#x}/{second:#x}"
            );
        }
    }

    #[test]
    fn a_failure_status_record_is_excluded_rather_than_measured() {
        // Every failure bit in `COMPACT_STATUS_FAILURE_MASK`, one at a time: the
        // fold retains the accumulator and folds nothing, so the hour is not
        // d24 data and must not enter either arm.
        for bit in [0u32, 1, 2, 3, 6, 7, 8, 9, 12] {
            let mut s = Vec::new();
            s.extend(record(
                1,
                100,
                D24,
                COMPACT_RECORD_TYPE_DOMAIN_DATA,
                &baseline_payload(1 << bit, 10),
            ));
            s.extend(frame_end(2, 100));
            s.extend(record(
                3,
                101,
                D24,
                COMPACT_RECORD_TYPE_DOMAIN_DATA,
                &baseline_payload(0, 11),
            ));
            s.extend(frame_end(4, 101));
            let scanned = scan(&s).expect("the stream itself is well-formed");
            assert_eq!(scanned.failure_records, 1, "bit {bit}");
            assert_eq!(scanned.hours.len(), 1, "bit {bit}");
            assert_eq!(scanned.hours[0].game_hour, 101, "bit {bit}");
        }
        // And the DELTA_DATA bit (1<<10) is NOT treated as failure.
        let mut s = Vec::new();
        s.extend(record(
            1,
            100,
            D24,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &baseline_payload(0, 10),
        ));
        s.extend(frame_end(2, 100));
        s.extend(record(
            3,
            101,
            D24,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &delta_payload(0, 4, 11),
        ));
        s.extend(frame_end(4, 101));
        let scanned = scan(&s).expect("well-formed");
        assert_eq!(scanned.failure_records, 0);
        assert_eq!(scanned.hours.len(), 2);
    }

    #[test]
    fn a_stream_with_no_d24_records_is_refused_not_reported_as_zero() {
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
    fn a_malformed_d24_payload_is_refused_not_skipped() {
        // A short or inconsistent d24 payload is a corrupt stream, not an hour
        // to drop quietly. A row the FOLD rejects must make the SCAN fail, not
        // be accepted and then unwind while measuring — so the whole row- and
        // frame-rejection space is covered here.
        let mut invalid: Vec<Vec<u8>> = Vec::new();
        // Truncated payload.
        let mut short = baseline_payload(0, 10);
        short.truncate(short.len() - 1);
        invalid.push(short);
        // Shorter than the 48-byte header.
        invalid.push(vec![0u8; 20]);
        // A full snapshot that is not the whole 305-region set (`:16998`).
        invalid.push(payload(0, 0, &[region_row(0, 1)], &[]));
        // A wrong `scale_raw` (`:16984`).
        let mut wrong_scale = baseline_payload(0, 10);
        wrong_scale[32..36].copy_from_slice(&32_768u32.to_le_bytes());
        invalid.push(wrong_scale);
        // An out-of-range region id (`:17058`).
        let mut regions: Vec<Vec<u8>> = (0..d24::REGION_ROWS as u32)
            .map(|id| region_row(id, 10))
            .collect();
        regions[0] = region_row(305, 10);
        invalid.push(payload(0, 0, &regions, &[]));
        // A duplicated region id in one frame (`:17058`).
        let mut regions: Vec<Vec<u8>> = (0..d24::REGION_ROWS as u32)
            .map(|id| region_row(id, 10))
            .collect();
        regions[1] = region_row(0, 10);
        invalid.push(payload(0, 0, &regions, &[]));
        // An unknown region flag bit (`:17063`).
        let mut regions: Vec<Vec<u8>> = (0..d24::REGION_ROWS as u32)
            .map(|id| region_row(id, 10))
            .collect();
        regions[2][4..8].copy_from_slice(&(1u32 << 20).to_le_bytes());
        invalid.push(payload(0, 0, &regions, &[]));
        // A province id past `observed_province_rows` (`:17158`).
        let regions: Vec<Vec<u8>> = (0..d24::REGION_ROWS as u32)
            .map(|id| region_row(id, 10))
            .collect();
        invalid.push(payload(0, 10_000, &regions, &[province_row(10_000, 1)]));

        for (i, p) in invalid.into_iter().enumerate() {
            let mut s = Vec::new();
            s.extend(record(1, 100, D24, COMPACT_RECORD_TYPE_DOMAIN_DATA, &p));
            s.extend(frame_end(2, 100));
            assert!(
                scan(&s).is_err(),
                "invalid payload {i} must make the scan fail"
            );
        }
    }

    #[test]
    fn a_frame_end_that_does_not_seal_its_hour_refuses_the_stream() {
        let data = |sequence, hour| {
            record(
                sequence,
                hour,
                D24,
                COMPACT_RECORD_TYPE_DOMAIN_DATA,
                &baseline_payload(0, 10),
            )
        };
        let mut ok = Vec::new();
        ok.extend(data(1, 100));
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
        for seal in bad {
            let mut s = Vec::new();
            s.extend(data(1, 100));
            s.extend(seal);
            assert!(scan(&s).is_err());
        }
        for len in [0usize, 1, 16, 31, 33, 64] {
            let mut s = Vec::new();
            s.extend(data(1, 100));
            s.extend(record(
                2,
                100,
                COMPACT_DOMAIN_FRAME,
                COMPACT_RECORD_TYPE_FRAME_END,
                &vec![0u8; len],
            ));
            assert!(scan(&s).is_err(), "{len}-byte FRAME_END");
        }
        for informational in [1u32 << 4, 1 << 5, 1 << 10, 1 << 11, 1 << 13] {
            let mut s = Vec::new();
            s.extend(data(1, 100));
            s.extend(frame_end_sealed(2, 100, 0, 0, informational, 0));
            assert!(scan(&s).is_ok(), "informational status {informational:#x}");
        }
        let mut later = good_stream();
        let fourth = boundary(&later, 3);
        later[fourth + COMPACT_RECORD_HEADER_LEN + 28..fourth + COMPACT_RECORD_HEADER_LEN + 32]
            .copy_from_slice(&1u32.to_le_bytes());
        assert!(scan(&later).is_err());
    }

    #[test]
    fn perf_hour_binding_is_enforced() {
        let data = |sequence, hour| {
            record(
                sequence,
                hour,
                D24,
                COMPACT_RECORD_TYPE_DOMAIN_DATA,
                &baseline_payload(0, 10),
            )
        };
        let mut ok = Vec::new();
        ok.extend(perf(1, 100, 20));
        ok.extend(perf(2, 100, D24));
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

        for (emitted, scheduled) in [(1u32, 1u32), (3, 3), (2, 3), (3, 2)] {
            let mut s = Vec::new();
            s.extend(perf(1, 100, 20));
            s.extend(perf(2, 100, D24));
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
        assert_eq!(scan(&two_hours).expect("PERF state resets").hours.len(), 2);
    }

    #[test]
    fn a_data_record_must_open_the_hour_the_pending_perf_records_bound() {
        let data = |sequence, hour| {
            record(
                sequence,
                hour,
                D24,
                COMPACT_RECORD_TYPE_DOMAIN_DATA,
                &baseline_payload(0, 10),
            )
        };
        for (perf_hour, data_hour) in [(100i32, 101i32), (101, 100), (100, 200), (200, 100)] {
            let mut s = Vec::new();
            s.extend(perf(1, perf_hour, 20));
            s.extend(data(2, data_hour));
            s.extend(frame_end_sealed(3, data_hour, 1, 1, 0, 0));
            assert!(scan(&s).is_err(), "PERF {perf_hour}, DATA {data_hour}");
        }
        let mut agreeing = Vec::new();
        agreeing.extend(perf(1, 100, 20));
        agreeing.extend(data(2, 100));
        agreeing.extend(frame_end_sealed(3, 100, 1, 1, 0, 0));
        assert!(scan(&agreeing).is_ok());
    }

    #[test]
    fn an_excluded_record_is_still_checked_by_the_payload_authority() {
        const FAILED_DELTA: u32 = 1 | d24::STATUS_DELTA_DATA;
        let follow_with_baseline = |payload: Vec<u8>| {
            let mut s = Vec::new();
            s.extend(record(
                1,
                100,
                D24,
                COMPACT_RECORD_TYPE_DOMAIN_DATA,
                &payload,
            ));
            s.extend(frame_end(2, 100));
            s.extend(record(
                3,
                101,
                D24,
                COMPACT_RECORD_TYPE_DOMAIN_DATA,
                &baseline_payload(0, 10),
            ));
            s.extend(frame_end(4, 101));
            s
        };

        let mut bad = Vec::new();
        let mut long = delta_payload(FAILED_DELTA, 4, 10);
        long.push(0);
        bad.push(long);
        bad.push(payload(FAILED_DELTA, 1, &[region_row(4, 10)], &[]));
        let mut observed = delta_payload(FAILED_DELTA, 4, 10);
        observed[24..28].copy_from_slice(&0u32.to_le_bytes());
        bad.push(observed);
        let mut changed = delta_payload(FAILED_DELTA, 4, 10);
        changed[12..16].copy_from_slice(&0u32.to_le_bytes());
        bad.push(changed);
        let mut scale = delta_payload(FAILED_DELTA, 4, 10);
        scale[32..36].copy_from_slice(&1u32.to_le_bytes());
        bad.push(scale);
        let mut active = delta_payload(FAILED_DELTA, 4, 10);
        active[36..40].copy_from_slice(&1u32.to_le_bytes());
        bad.push(active);
        let mut reserved = delta_payload(FAILED_DELTA, 4, 10);
        reserved[40..44].copy_from_slice(&1u32.to_le_bytes());
        bad.push(reserved);

        let mut invalid_region_id = delta_payload(FAILED_DELTA, 4, 10);
        invalid_region_id[d24::HEADER_LEN..d24::HEADER_LEN + 4]
            .copy_from_slice(&305u32.to_le_bytes());
        bad.push(invalid_region_id);
        let mut region_flags = delta_payload(FAILED_DELTA, 4, 10);
        region_flags[d24::HEADER_LEN + 4..d24::HEADER_LEN + 8]
            .copy_from_slice(&(1u32 << 20).to_le_bytes());
        bad.push(region_flags);
        let mut phenomenon = delta_payload(FAILED_DELTA, 4, 10);
        phenomenon[d24::HEADER_LEN + 12..d24::HEADER_LEN + 16]
            .copy_from_slice(&(d24::MAX_RAW_ACTIVITY_CANDIDATE + 1).to_le_bytes());
        bad.push(phenomenon);
        let mut region_reserved = delta_payload(FAILED_DELTA, 4, 10);
        region_reserved[d24::HEADER_LEN + 28..d24::HEADER_LEN + 32]
            .copy_from_slice(&1u32.to_le_bytes());
        bad.push(region_reserved);
        let mut duplicate_region = payload(
            FAILED_DELTA,
            0,
            &[region_row(4, 10), region_row(4, 11)],
            &[],
        );
        duplicate_region[36..40].copy_from_slice(&0u32.to_le_bytes());
        bad.push(duplicate_region);

        let valid_province = province_row(1, 10);
        let province_payload = || payload(FAILED_DELTA, 10_000, &[], &[valid_province.clone()]);
        let mut province_id = province_payload();
        province_id[d24::HEADER_LEN..d24::HEADER_LEN + 4].copy_from_slice(&10_000u32.to_le_bytes());
        bad.push(province_id);
        let mut missing_struct = province_payload();
        let flags = province_flags() & !d24::PROVINCE_STRUCT_VALIDATED;
        missing_struct[d24::HEADER_LEN + 4..d24::HEADER_LEN + 8]
            .copy_from_slice(&flags.to_le_bytes());
        bad.push(missing_struct);
        let mut province_flags_bad = province_payload();
        province_flags_bad[d24::HEADER_LEN + 4..d24::HEADER_LEN + 8]
            .copy_from_slice(&(province_flags() | 1 << 20).to_le_bytes());
        bad.push(province_flags_bad);
        let mut province_reserved = province_payload();
        province_reserved[d24::HEADER_LEN + 28..d24::HEADER_LEN + 32]
            .copy_from_slice(&1u32.to_le_bytes());
        bad.push(province_reserved);

        for (index, payload) in bad.into_iter().enumerate() {
            assert!(
                scan(&follow_with_baseline(payload)).is_err(),
                "excluded malformed authority payload {index}"
            );
        }
        for bit in [0u32, 1, 2, 3, 6, 7, 8, 9, 12] {
            let mut payload = delta_payload((1 << bit) | d24::STATUS_DELTA_DATA, 4, 10);
            payload[d24::HEADER_LEN..d24::HEADER_LEN + 4].copy_from_slice(&305u32.to_le_bytes());
            assert!(
                scan(&follow_with_baseline(payload)).is_err(),
                "failure bit {bit}"
            );
        }

        let scanned = scan(&follow_with_baseline(delta_payload(FAILED_DELTA, 4, 10)))
            .expect("a well-formed excluded delta does not reach the fold-only baseline rule");
        assert_eq!(scanned.failure_records, 1);
        assert_eq!(scanned.delta_before_baseline, 0);
        assert_eq!(scanned.hours.len(), 1);
    }

    #[test]
    fn the_identity_protocol_is_stated_not_sniffed() {
        use compact_contract::campaign_identity::COMPACT_RECORD_TYPE_STREAM_HEADER;

        let schema = d24::schema();
        let legacy = good_stream();
        assert!(scan_stream(&legacy, &schema, StreamIdentityMode::IdentityV1).is_err());
        assert!(scan_stream(&legacy, &schema, StreamIdentityMode::Legacy).is_ok());
        let mut identity = record(
            0,
            0,
            0,
            COMPACT_RECORD_TYPE_STREAM_HEADER,
            b"CID1-not-a-real-identity-payload",
        );
        identity.extend(legacy);
        assert!(scan_stream(&identity, &schema, StreamIdentityMode::Legacy).is_err());
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
        // And corruption of a genuinely well-formed stream.
        let good = good_stream();
        for i in (0..good.len()).step_by(11) {
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
    fn an_hour_closed_without_a_d24_record_refuses_the_stream() {
        // The completeness bound on what this harness does NOT validate (module
        // docs): it cannot check other domains' payloads, so a d24 record
        // relabelled to another domain id — or dropped — must be caught by
        // counting, not silently measured as fewer hours.
        let mut s = good_stream();
        // Find the LAST d24 DATA record and relabel it to an unclaimed domain
        // id. The stream stays well formed — sequence, hour boundaries and
        // FRAME_END are untouched — so the completeness rule is the only rule
        // that can catch it.
        let mut pos = 0usize;
        let mut target = None;
        while pos < s.len() {
            let header = parse_compact_record_header(&s[pos..]).expect("well-formed");
            if header.domain_id == D24 && header.record_type == COMPACT_RECORD_TYPE_DOMAIN_DATA
            {
                target = Some(pos);
            }
            pos += COMPACT_RECORD_HEADER_LEN + header.payload_len;
        }
        let target = target.expect("good_stream carries a d24 DATA record");
        // domain_id sits at +20: magic(4) + version(2) + header_len(2) +
        // sequence(8) + game_hour(4).
        s[target + 20..target + 22].copy_from_slice(&99u16.to_le_bytes());
        let err = match scan(&s) {
            Err(e) => e,
            Ok(_) => panic!("an hour with no d24 record must refuse the stream"),
        };
        assert!(
            err.contains("game hour(s) but carries"),
            "the completeness check must be what rejects it: {err}"
        );
    }
}
