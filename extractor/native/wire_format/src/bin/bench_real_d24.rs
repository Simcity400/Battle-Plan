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
//!   (`recorder_host/src/stream_fold.rs:277`) that do not require DOMAIN_PERF
//!   accounting: a DOMAIN_DATA record opens its game hour, a FRAME_END closes
//!   the open hour and only that hour, and closed hours strictly increase;
//! - the EOF rules of that tracker's `eof_check` (`:459`): no residual bytes, no
//!   hour left open, at least one FRAME_END, and the final record is a FRAME_END.
//!
//! That is a documented **subset** of the production boundary contract:
//! `recorder_host::stream_fold::verify_stream_logical_boundary` (`:537`) is the
//! full authority, and it additionally checks FRAME_END seal fields and
//! DOMAIN_PERF domain accounting. This crate cannot depend on `recorder_host`
//! (it pulls in every capture lane), so the subset is enforced here and the full
//! check remains the recorder's.
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

use std::io::Write;
use std::ops::Range;

use compact_contract::campaign_identity::COMPACT_RECORD_TYPE_STREAM_HEADER;
use compact_contract::stream_validator::StatefulStreamValidator;
use compact_contract::{
    compact_status_failure_flags_after_expected_partial_tolerance, expected_partial_tolerance,
    lane_local_status_mask, parse_compact_record_header, COMPACT_RECORD_HEADER_LEN,
    COMPACT_RECORD_TYPE_DOMAIN_DATA, COMPACT_RECORD_TYPE_FRAME_END, COMPACT_STATUS_FAILURE_MASK,
};
use wire_format::body::{
    apply, decode_baseline, decode_delta, diff, encode_baseline, encode_delta, KeyedDelta,
    KeyedState,
};
use wire_format::d24::{self, FoldOutcome, WeatherState};
use wire_format::Schema;

const D24: u16 = 24;
const BLOCK_HOURS: usize = 48;

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
/// which is what `d24::FAILURE_STATUS_MASK` (the library's copy, kept because
/// `wire_format`'s library half must not depend on `compact_contract`) has to
/// equal. Asserting the three facts proves the two predicates agree for every
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

/// Validate the whole stream against the canonical contract and fold its usable
/// d24 DATA records, keeping only the byte spans the measurement needs. Every
/// rejection path is an `Err`, never a silently shorter measurement and never a
/// panic — see the module docs for what is enforced and what the recorder's own
/// validator additionally checks.
fn scan_stream(stream: &[u8], schema: &Schema) -> Result<StreamScan, String> {
    // `StatefulStreamValidator` deliberately never sniffs a payload to pick a
    // protocol, so the harness picks one explicitly from the FIRST RECORD'S
    // FRAMING HEADER: an identity-bearing stream opens with the CID1 record at
    // `(sequence, domain, type) = (0, 0, 4)` (`compact_contract/src/stream_validator.rs:104`),
    // and anything else is legacy, whose first ordinary record is sequence 1.
    // A wrong guess does not degrade to a lenient read — the validator rejects
    // the very first record.
    let first = parse_compact_record_header(stream).map_err(|e| format!("first record: {e}"))?;
    let mut validator = if first.sequence == 0
        && first.domain_id == 0
        && first.record_type == COMPACT_RECORD_TYPE_STREAM_HEADER
    {
        StatefulStreamValidator::identity_v1()
    } else {
        StatefulStreamValidator::legacy()
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
    // Hour of the last d24 DATA record of ANY status — the uniqueness rule.
    let mut last_d24_hour: Option<i32> = None;
    // Hour of the last d24 record that entered the measurement — the
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
            COMPACT_RECORD_TYPE_DOMAIN_DATA => match open_hour {
                Some(open) if open != header.game_hour_raw => {
                    return Err(format!(
                        "DOMAIN_DATA at byte {pos} carries hour {} while hour {open} is open",
                        header.game_hour_raw
                    ))
                }
                Some(_) => {}
                None => {
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
            }
            _ => {}
        }
        last_record_type = Some(header.record_type);

        if header.domain_id == D24 && header.record_type == COMPACT_RECORD_TYPE_DOMAIN_DATA {
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

            // `world_fold::handle_compact_failure` (`world_fold.rs:2843`, called
            // at `:16969`) marks the domain-hour unknown and RETAINS the prior
            // accumulator on a failure status, folding nothing from this record.
            // The predicate is `compact_contract`'s own, not a copied bit mask.
            let failed = compact_status_failure_flags_after_expected_partial_tolerance(
                D24,
                d24::parse_header(payload)
                    .map_err(|e| format!("d24 payload at byte {pos}: {e}"))?
                    .status_flags,
            ) != 0;

            // Fold exactly as `fold_weather` does — including every way a row
            // can be rejected, which is a property of the stream and so belongs
            // to the pass that either accepts the stream or refuses it.
            match state
                .fold_payload(schema, payload, 0)
                .map_err(|e| format!("d24 payload at byte {pos}: {e}"))?
            {
                FoldOutcome::FailureRetained => {
                    assert!(failed, "the library and contract failure predicates disagree");
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

    let mut args = std::env::args().skip(1);
    let input = args
        .next()
        .expect("usage: bench_real_d24 <collector_stream.bin> <out dir>");
    let out_dir = args
        .next()
        .expect("usage: bench_real_d24 <collector_stream.bin> <out dir>");
    let stream = std::fs::read(&input).expect("read stream");
    let schema = d24::schema();

    // ---- Pass 1: validate the WHOLE stream and fold its d24 DATA records. ---
    // No artifact is created before this succeeds, so a refused stream leaves no
    // partial measurement behind to be mistaken for a result. Everything that
    // can fail on stream content fails here, with a diagnosis and no unwind.
    let scan = match scan_stream(&stream, &schema) {
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
        let block_start = hours_seen % BLOCK_HOURS == 0;
        let mut emitted = true;
        if block_start || !reader_primed {
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
        if block_start {
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
            if block_start || !reader_primed {
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
    println!("baselines            {baselines} (every {BLOCK_HOURS} h)");
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
        s.extend(record(1, 100, D24, COMPACT_RECORD_TYPE_DOMAIN_DATA, &baseline_payload(0, 10)));
        s.extend(frame_end(2, 100));
        s.extend(record(3, 101, D24, COMPACT_RECORD_TYPE_DOMAIN_DATA, &delta_payload(0, 4, 11)));
        s.extend(frame_end(4, 101));
        s
    }

    fn scan(stream: &[u8]) -> Result<StreamScan, String> {
        scan_stream(stream, &d24::schema())
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
        assert_eq!(s.final_regions, d24::REGION_ROWS, "the delta deleted nothing");
    }

    #[test]
    fn a_delta_before_any_baseline_is_excluded_rather_than_measured() {
        // `world_fold.rs:16972`: the fold discards it, so it is not weather data
        // and must not enter either arm — but it does not make the stream bad.
        let mut s = Vec::new();
        s.extend(record(1, 100, D24, COMPACT_RECORD_TYPE_DOMAIN_DATA, &delta_payload(0, 4, 10)));
        s.extend(frame_end(2, 100));
        s.extend(record(3, 101, D24, COMPACT_RECORD_TYPE_DOMAIN_DATA, &baseline_payload(0, 11)));
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
        s.extend(record(1, 100, D24, COMPACT_RECORD_TYPE_DOMAIN_DATA, &baseline_payload(0, 10)));
        s.extend(frame_end(2, 999)); // closes an hour that is not open
        assert!(scan(&s).is_err());

        let mut s = Vec::new();
        s.extend(frame_end(1, 100));
        assert!(scan(&s).is_err(), "a FRAME_END with nothing open");
    }

    #[test]
    fn non_monotonic_hours_are_rejected() {
        let mut s = Vec::new();
        s.extend(record(1, 101, D24, COMPACT_RECORD_TYPE_DOMAIN_DATA, &baseline_payload(0, 10)));
        s.extend(frame_end(2, 101));
        s.extend(record(3, 100, D24, COMPACT_RECORD_TYPE_DOMAIN_DATA, &delta_payload(0, 4, 11)));
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
        s.extend(record(1, 100, D24, COMPACT_RECORD_TYPE_DOMAIN_DATA, &baseline_payload(0, 10)));
        s.extend(frame_end(2, 100));
        s.extend(record(3, 101, D24, COMPACT_RECORD_TYPE_DOMAIN_DATA, &delta_payload(0, 4, 11)));
        s.extend(frame_end(4, 101));
        let scanned = scan(&s).expect("well-formed");
        assert_eq!(scanned.failure_records, 0);
        assert_eq!(scanned.hours.len(), 2);
    }

    #[test]
    fn a_stream_with_no_d24_records_is_refused_not_reported_as_zero() {
        let mut s = Vec::new();
        s.extend(record(1, 100, 20, COMPACT_RECORD_TYPE_DOMAIN_DATA, &[0u8; 8]));
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
            assert!(scan(&s).is_err(), "invalid payload {i} must make the scan fail");
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
}
