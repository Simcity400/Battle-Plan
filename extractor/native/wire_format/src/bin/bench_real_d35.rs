//! Measure the wire format against the shipping compact stream on REAL d35
//! `air_variant_stats` dynamics.
//!
//! Input: a real `collector_stream.bin` from a d35-scheduled capture
//! (`runs/batch2/deepwar_t1`). The stream's own d35 DATA records are the shipping
//! format's cost for the domain: a 36-byte header plus a fixed 64-byte row per
//! *changed* variant. This tool replays those records into per-hour variant
//! states through `wire_format::d35::Accumulator` — the same fold
//! `world_fold::fold_air_variant_stats` performs — and re-encodes every hour with
//! this crate's grammar: a BASELINE at each block start, field-granular
//! stored-minimal DELTAs between.
//!
//! **d35 is incremental at the shipping layer, and that is the whole story of
//! the measurement.** The collector emits a row only when that variant's stats
//! changed since its own previous frame (`attach_log.rs:52773`-`:52777`), and the
//! fold never clears, never deletes and never re-baselines. So one record is
//! *not* one state: the hour's content lives in the accumulator, and a hour whose
//! payload carries no rows is a real hour whose state equals the previous one.
//! On the reference capture 145 of 146 records are bare 36-byte headers.
//!
//! Two layers therefore have to be kept apart, exactly as in d22:
//!
//! *Shipping layer*: an unbounded upsert run with no baseline of any kind. The
//! catalog is stated once, at the first hour that carries it, and never again.
//! *Wire layer*: a BASELINE at every `--block-hours` boundary and a
//! stored-minimal DELTA (§7.1) between. The block cadence is the container's
//! open question, not this grammar's, so the tool takes it on the command line
//! and the report carries both shapes — at 48 h the arm pays for four full
//! re-statements of a catalog the shipping arm stated once, and at
//! `--block-hours 0` (one baseline for the whole run) the two arms are compared
//! on grammar alone. Reporting only the first would measure re-baseline policy
//! and call it a grammar result.
//!
//! **Nothing here re-derives a contract it can call.** Row and payload decoding
//! go through `wire_format::d35`'s public boundary; H4CR framing, record
//! semantics and sequence continuity go through `compact_contract`'s own parser
//! and stateful validator. A harness that reimplements either drifts from the
//! recorder without anything failing.
//!
//! **The whole stream is validated before any artifact is written.** A stream
//! that is truncated, spliced, or corrupt would otherwise yield a smaller but
//! entirely plausible measurement. `scan_stream` therefore enforces, fail-closed:
//!
//! - every record header through `compact_contract::parse_compact_record_header`
//!   (magic, version, header length, payload bound);
//! - every record through `compact_contract::stream_validator::StatefulStreamValidator`
//!   (sequence continuity from the stream's first record, record-type and
//!   record-flag semantics, FRAME_END shape);
//! - the hour boundary rules of `recorder_host::stream_fold::BoundaryTracker`
//!   (`recorder_host/src/stream_fold.rs:277`) that do not require DOMAIN_PERF
//!   accounting: a DOMAIN_DATA record opens its game hour, a FRAME_END closes the
//!   open hour and only that hour, and closed hours strictly increase;
//! - the EOF rules of that tracker's `eof_check` (`:459`): no residual bytes, no
//!   hour left open, at least one FRAME_END, and the final record is a FRAME_END.
//!
//! That is a documented **subset** of the production boundary contract:
//! `recorder_host::stream_fold::verify_stream_logical_boundary` (`:537`) is the
//! full authority, and it additionally checks FRAME_END seal fields and
//! DOMAIN_PERF domain accounting. This crate cannot depend on `recorder_host`, so
//! the subset is enforced here and the full check remains the recorder's.
//!
//! **The one exclusion path, and where it sits.** `fold_air_variant_stats` has
//! exactly one return-without-folding path: `handle_compact_failure`
//! (`world_fold.rs:15149`-`:15155`). It has no delta-before-baseline path (it
//! never reads `COMPACT_STATUS_DELTA_DATA`), no partial-before-baseline path and
//! no rowless-partial path — a rowless payload is ordinary. The position matters:
//! the fold checks the 36-byte minimum and both scale words **before** the status
//! guard (`:15137`-`:15148`) and the exact payload length **after** it (`:15156`).
//! So this harness classifies with `d35::parse_header`, which runs only the
//! pre-guard checks, and calls `d35::Accumulator::fold_payload` — which runs the
//! length check — only for records that pass the guard. Using `split_payload` to
//! classify would refuse failure-status records the recorder accepts.
//!
//! **No other writer touches the accumulator.** `world_fold.rs` mutates
//! `self.air_variant_stats` in exactly two places: the empty `BTreeMap` at
//! construction (`:2447`) and the insert in the fold (`:15194`). `:8624` and
//! `:8648` read it for the variant projection. Nothing prunes it — unlike d26,
//! whose accumulator the recorder prunes from D20 deletion rows — so the key set
//! is monotone and a wire delta can never carry a tombstone. That is asserted
//! every hour below rather than assumed.
//!
//! Outputs, for the compression step (`compress_arms.py` beside the run):
//!   ship_d35.bin   — the d35 DATA records verbatim (32-byte H4CR header +
//!                    payload), concatenated in stream order
//!   wire_new.bin   — the wire-format bodies, concatenated in hour order
//!   *.offsets      — hour-boundary byte offsets, one per line, for exact
//!                    block-span slicing
//!
//! It also prints the real churn surface (changed rows per hour at stored
//! precision) and the wire arm's baseline/delta raw split.

use std::io::Write;
use std::ops::Range;

use compact_contract::campaign_identity::COMPACT_RECORD_TYPE_STREAM_HEADER;
use compact_contract::stream_validator::StatefulStreamValidator;
use compact_contract::{
    compact_status_failure_flags_after_expected_partial_tolerance, parse_compact_record_header,
    COMPACT_RECORD_HEADER_LEN, COMPACT_RECORD_TYPE_DOMAIN_DATA, COMPACT_RECORD_TYPE_FRAME_END,
};
use wire_format::body::{
    apply, decode_baseline, decode_delta, diff, encode_baseline, encode_delta, KeyedState,
};
use wire_format::d35;

const D35: u16 = 35;
const DEFAULT_BLOCK_HOURS: usize = 48;

/// One usable d35 hour: its game hour, the accumulator's content *after* folding
/// that hour's record, and the record's byte span in the source stream.
///
/// The state is a snapshot of the running accumulator, not the record's rows —
/// that is the difference an incremental shipping layer makes. It is built
/// during the scan, not later: every way a row can be rejected is a property of
/// the stream, so it belongs to the pass that either accepts the stream or
/// refuses it.
struct D35Hour {
    game_hour: i32,
    state: KeyedState,
    /// Rows this record actually carried — the shipping layer's own churn.
    payload_rows: usize,
    ship_span: Range<usize>,
}

struct StreamScan {
    hours: Vec<D35Hour>,
    /// d35 records whose payload status flags mark the domain-hour failed.
    failure_records: usize,
    /// Measured d35 hours that do not follow the previous measured one by one.
    non_contiguous: usize,
    /// Variants the collector saw but could not read, summed over the run. With
    /// no delete grammar these keep their previous values in the accumulator.
    invalid_variant_rows: u64,
}

/// Validate the whole stream against the canonical contract and fold the usable
/// d35 DATA records into per-hour states. Every rejection path is an `Err`,
/// never a silently shorter measurement and never a panic.
fn scan_stream(stream: &[u8], collection: &wire_format::Collection) -> Result<StreamScan, String> {
    // `StatefulStreamValidator` deliberately never sniffs a payload to pick a
    // protocol, so the harness picks one explicitly from the FIRST RECORD'S
    // FRAMING HEADER: an identity-bearing stream opens with the CID1 record at
    // `(sequence, domain, type) = (0, 0, 4)`
    // (`compact_contract/src/stream_validator.rs:104`), and anything else is
    // legacy, whose first ordinary record is sequence 1.
    let first = parse_compact_record_header(stream).map_err(|e| format!("first record: {e}"))?;
    let mut validator = if first.sequence == 0
        && first.domain_id == 0
        && first.record_type == COMPACT_RECORD_TYPE_STREAM_HEADER
    {
        StatefulStreamValidator::identity_v1()
    } else {
        StatefulStreamValidator::legacy()
    };

    let mut hours: Vec<D35Hour> = Vec::new();
    let mut failure_records = 0usize;
    let mut non_contiguous = 0usize;
    let mut invalid_variant_rows = 0u64;

    let mut open_hour: Option<i32> = None;
    let mut last_closed_hour: Option<i32> = None;
    let mut last_record_type: Option<u16> = None;
    // Hour of the last d35 DATA record of ANY status — the uniqueness rule.
    let mut last_d35_hour: Option<i32> = None;
    // Hour of the last d35 record that entered the measurement — the contiguity
    // report, which a skipped failure hour legitimately breaks.
    let mut last_measured_hour: Option<i32> = None;

    // The running accumulator. d35 is incremental, so this survives across
    // records and IS the domain's state; a per-record state would be wrong.
    let mut accumulator = d35::Accumulator::new();

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

        if header.domain_id == D35 && header.record_type == COMPACT_RECORD_TYPE_DOMAIN_DATA {
            // Only the checks the fold makes BEFORE its status guard: the
            // 36-byte minimum (`world_fold.rs:15137`) and both scale words being
            // non-zero (`:15146`-`:15148`). The exact length check comes after
            // the guard and so must not run on a record we may exclude.
            let d35_header = d35::parse_header(payload)
                .map_err(|e| format!("d35 payload at byte {pos}: {e}"))?;

            // One d35 DATA record per game hour, checked over EVERY d35 record
            // and BEFORE any status-based exclusion. Gating this on the record
            // being usable would let a success+failure pair in one hour through
            // the rule entirely.
            if let Some(previous) = last_d35_hour {
                if header.game_hour_raw <= previous {
                    return Err(format!(
                        "d35 DATA at byte {pos} carries hour {} after hour {previous}",
                        header.game_hour_raw
                    ));
                }
            }
            last_d35_hour = Some(header.game_hour_raw);

            // `handle_compact_failure` (`world_fold.rs:2843`, called at `:15149`)
            // marks the domain-hour unknown and RETAINS the accumulator, folding
            // nothing from this record. The predicate is `compact_contract`'s
            // own, not a copied bit mask, so it cannot drift from the recorder.
            if compact_status_failure_flags_after_expected_partial_tolerance(
                D35,
                d35_header.status_flags,
            ) != 0
            {
                failure_records += 1;
            } else {
                if let Some(previous) = last_measured_hour {
                    if header.game_hour_raw != previous + 1 {
                        non_contiguous += 1;
                    }
                }
                last_measured_hour = Some(header.game_hour_raw);

                // Fold exactly as `fold_air_variant_stats` does, through the
                // domain's own public boundary. This runs the exact-length check
                // and every per-row rejection, in the fold's order.
                let folded = accumulator
                    .fold_payload(collection, payload)
                    .map_err(|e| format!("d35 payload at byte {pos}: {e}"))?;
                invalid_variant_rows += folded.invalid_variant_rows as u64;

                hours.push(D35Hour {
                    game_hour: header.game_hour_raw,
                    // The accumulator's content, not the record's rows: d35's
                    // shipping layer is incremental.
                    state: accumulator.variants.clone(),
                    payload_rows: folded.row_count as usize,
                    ship_span: pos..record_end,
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
        return Err("stream carries no usable d35 DATA records".to_string());
    }

    Ok(StreamScan { hours, failure_records, non_contiguous, invalid_variant_rows })
}

fn main() {
    let mut args = std::env::args().skip(1);
    let usage = "usage: bench_real_d35 <collector_stream.bin> <out dir> [block_hours; 0 = one \
                 baseline for the whole run]";
    let input = args.next().unwrap_or_else(|| panic!("{usage}"));
    let out_dir = args.next().unwrap_or_else(|| panic!("{usage}"));
    let block_hours: usize = match args.next() {
        Some(v) => v.parse().unwrap_or_else(|_| panic!("{usage}")),
        None => DEFAULT_BLOCK_HOURS,
    };
    let stream = std::fs::read(&input).expect("read stream");
    let schema = d35::schema();
    let collection = &schema.collections[0];

    // ---- Pass 1: validate the WHOLE stream and fold its d35 DATA records. ---
    // No artifact is created before this succeeds, so a refused stream leaves no
    // partial measurement behind to be mistaken for a result.
    let scan = match scan_stream(&stream, collection) {
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
    let mut baseline_bytes = 0usize;
    let mut delta_bytes = 0usize;
    let mut empty_hours = 0usize;
    let mut changed_rows_total = 0usize;
    let mut shipping_rows_total = 0usize;
    let mut variants_total = 0usize;

    for hour in &scan.hours {
        // Shipping arm: the record's exact bytes, header included.
        ship.extend_from_slice(&stream[hour.ship_span.clone()]);
        ship_offsets.push(ship.len());
        shipping_rows_total += hour.payload_rows;

        let state: &KeyedState = &hour.state;
        variants_total += state.len();

        // New arm: BASELINE at each block start, stored-minimal DELTA between.
        // `block_hours == 0` means a single baseline for the whole run, which is
        // the shipping layer's own cadence and so the grammar-only comparison.
        let hour_start = wire.len();
        let block_start = block_hours != 0 && hours_seen % block_hours == 0;
        let mut emitted = true;
        if block_start || prior.is_none() {
            encode_baseline(&schema, std::slice::from_ref(state), &mut wire)
                .expect("baseline encodes");
            baselines += 1;
            baseline_bytes += wire.len() - hour_start;
        } else {
            let prev = prior.unwrap();
            let d = diff(collection, prev, state);
            // The theorem this domain's grammar guarantees: the fold has no
            // delete row and never clears, and nothing else writes the
            // accumulator, so a key cannot leave it. If this ever fires, either
            // the fold gained a delete grammar or the replay is wrong.
            assert!(
                d.tombstones.is_empty(),
                "game hour {}: an upsert-only accumulator produced a tombstone",
                hour.game_hour
            );
            if d.is_empty() {
                empty_hours += 1;
                emitted = false;
            } else {
                changed_rows_total += d.changed.len();
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
        // equal the fold's, which is precisely what "nothing changed" asserts —
        // and on this domain that is the overwhelming majority of hours.
        if emitted {
            let hour_bytes = &wire[hour_start..];
            if block_start || reader.is_none() {
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
        hours_seen += 1;
    }

    for (name, bytes) in [("ship_d35.bin", &ship), ("wire_new.bin", &wire)] {
        let mut f = std::fs::File::create(std::path::Path::new(&out_dir).join(name)).unwrap();
        f.write_all(bytes).unwrap();
    }
    for (name, offsets) in [
        ("ship_d35.offsets", &ship_offsets),
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
        "invalid variant rows {} (omitted by the collector; kept stale by the fold)",
        scan.invalid_variant_rows
    );
    println!(
        "variants avg         {:.1}",
        variants_total as f64 / hours_seen as f64
    );
    println!(
        "shipping churn       {shipping_rows_total} rows over {hours_seen} hours \
         ({:.2}/hour) — the shipping layer is incremental",
        shipping_rows_total as f64 / hours_seen as f64
    );
    if block_hours == 0 {
        println!("baselines            {baselines} (one for the whole run)");
    } else {
        println!("baselines            {baselines} (every {block_hours} h)");
    }
    println!("delta hours          {delta_hours}   empty hours {empty_hours}");
    println!(
        "real churn           {:.2} changed rows/hour, 0 tombstones/hour \
         (upsert-only: a tombstone is unreachable, asserted every hour)",
        changed_rows_total as f64 / delta_hours.max(1) as f64
    );
    println!("wire raw split       baseline {baseline_bytes} B + delta {delta_bytes} B");
    println!("ship_d35.bin         {} bytes", ship.len());
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
        d35::schema().collections[0].clone()
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

    /// One 64-byte d35 row, packed as the collector packs one
    /// (`attach_log.rs:70136`-`:70151`).
    fn d35_row(variant_id: u32, speed_cache: i32) -> Vec<u8> {
        let mut row = Vec::with_capacity(d35::ROW_LEN);
        row.extend_from_slice(&variant_id.to_le_bytes());
        row.extend_from_slice(&0u32.to_le_bytes()); // flags
        for token in [20_201u32, 17, 4] {
            row.extend_from_slice(&token.to_le_bytes());
        }
        row.extend_from_slice(&speed_cache.to_le_bytes());
        for stat in [350_000i32, 0, 0, 0, 80_000, 0, 0, 0, 1_950_000] {
            row.extend_from_slice(&stat.to_le_bytes());
        }
        row.extend_from_slice(&0u32.to_le_bytes()); // reserved
        assert_eq!(row.len(), d35::ROW_LEN);
        row
    }

    /// A d35 DATA payload: the 36-byte header then `rows`.
    fn d35_payload(status_flags: u32, rows: &[Vec<u8>]) -> Vec<u8> {
        d35_payload_scaled(status_flags, (100_000, 1_000), rows)
    }

    fn d35_payload_scaled(status_flags: u32, scales: (u32, u32), rows: &[Vec<u8>]) -> Vec<u8> {
        let mut p = Vec::new();
        p.extend_from_slice(&0u64.to_le_bytes()); // elapsed_wall_ms
        p.extend_from_slice(&(rows.len() as u32).to_le_bytes()); // row_count
        p.extend_from_slice(&(rows.len() as u32).to_le_bytes()); // changed_rows
        p.extend_from_slice(&status_flags.to_le_bytes());
        p.extend_from_slice(&204u32.to_le_bytes()); // observed_variant_rows
        p.extend_from_slice(&0u32.to_le_bytes()); // invalid_variant_rows
        p.extend_from_slice(&scales.0.to_le_bytes());
        p.extend_from_slice(&scales.1.to_le_bytes());
        for row in rows {
            p.extend_from_slice(row);
        }
        assert_eq!(p.len(), d35::HEADER_LEN + rows.len() * d35::ROW_LEN);
        p
    }

    /// Three closed hours: a stating hour, then two rowless ones — the shape the
    /// real capture has (1 record with rows, 145 bare headers).
    fn good_stream() -> Vec<u8> {
        let mut s = Vec::new();
        s.extend(record(
            1,
            100,
            D35,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &d35_payload(0, &[d35_row(7, 45_000_000), d35_row(9, 46_000_000)]),
        ));
        s.extend(frame_end(2, 100));
        s.extend(record(3, 101, D35, COMPACT_RECORD_TYPE_DOMAIN_DATA, &d35_payload(0, &[])));
        s.extend(frame_end(4, 101));
        s.extend(record(
            5,
            102,
            D35,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &d35_payload(0, &[d35_row(7, 47_000_000)]),
        ));
        s.extend(frame_end(6, 102));
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
    fn a_well_formed_stream_yields_its_d35_hours() {
        let scan = scan_stream(&good_stream(), &test_collection()).expect("accepted");
        assert_eq!(scan.hours.len(), 3);
        assert_eq!(scan.hours[0].game_hour, 100);
        assert_eq!(scan.failure_records, 0);
        assert_eq!(scan.non_contiguous, 0);
    }

    #[test]
    fn a_rowless_hour_carries_the_accumulator_forward_not_an_empty_state() {
        // The defining property of an incremental shipping layer, and the one a
        // harness copied from a full-replace domain would get catastrophically
        // wrong: hour 101 carries no rows and its state is still both variants.
        let scan = scan_stream(&good_stream(), &test_collection()).expect("accepted");
        assert_eq!(scan.hours[0].state.len(), 2);
        assert_eq!(scan.hours[0].payload_rows, 2);
        assert_eq!(scan.hours[1].state.len(), 2, "a bare header is not an empty hour");
        assert_eq!(scan.hours[1].payload_rows, 0);
        assert_eq!(scan.hours[2].state.len(), 2);
        // And the upsert landed: variant 7's speed changed at hour 102, variant
        // 9's did not, and neither key left.
        let c = test_collection();
        let idx = c
            .value_fields()
            .position(|f| f.field_id == d35::field::SPEED_CACHE)
            .expect("field");
        use wire_format::body::key;
        assert_eq!(
            scan.hours[1].state[&key(&[7])].values[idx],
            wire_format::Value::Continuous(450.0)
        );
        assert_eq!(
            scan.hours[2].state[&key(&[7])].values[idx],
            wire_format::Value::Continuous(470.0)
        );
        assert_eq!(
            scan.hours[2].state[&key(&[9])].values[idx],
            wire_format::Value::Continuous(460.0),
            "an unrestated variant keeps its value"
        );
    }

    #[test]
    fn a_stream_truncated_on_a_record_boundary_is_rejected() {
        let full = good_stream();
        // A cut immediately after a DATA record leaves that hour open and ends
        // the stream on a record that is not a FRAME_END.
        for records in [1usize, 3, 5] {
            let cut = boundary(&full, records);
            assert!(
                scan_stream(&full[..cut], &test_collection()).is_err(),
                "a stream cut after {records} record(s) leaves an hour open"
            );
        }
        // A cut exactly at a FRAME_END boundary IS a well-formed shorter stream.
        for (records, expected_hours) in [(2usize, 1usize), (4, 2), (6, 3)] {
            let cut = boundary(&full, records);
            let scan = scan_stream(&full[..cut], &test_collection())
                .expect("closed stream is well-formed");
            assert_eq!(scan.hours.len(), expected_hours, "cut after {records}");
        }
    }

    #[test]
    fn a_stream_ending_mid_record_is_rejected() {
        let full = good_stream();
        let boundaries: Vec<usize> = (0..=6).map(|n| boundary(&full, n)).collect();
        for cut in 1..full.len() {
            if boundaries.contains(&cut) {
                continue;
            }
            assert!(
                scan_stream(&full[..cut], &test_collection()).is_err(),
                "mid-record cut at {cut}"
            );
        }
        assert!(
            scan_stream(&[], &test_collection()).is_err(),
            "an empty stream is not a measurement"
        );
    }

    #[test]
    fn a_header_the_canonical_parser_rejects_is_rejected_here() {
        for records in 0..6 {
            let at = boundary(&good_stream(), records);
            for (offset, patch) in [
                (0usize, vec![b'X']),              // magic
                (4, 2u16.to_le_bytes().to_vec()),  // unsupported version
                (6, 33u16.to_le_bytes().to_vec()), // unsupported header length
            ] {
                let mut s = good_stream();
                s[at + offset..at + offset + patch.len()].copy_from_slice(&patch);
                assert!(
                    scan_stream(&s, &test_collection()).is_err(),
                    "record {records} patched at +{offset} must be refused"
                );
            }
        }
    }

    #[test]
    fn a_sequence_discontinuity_is_rejected() {
        for records in 0..6 {
            let at = boundary(&good_stream(), records);
            for wrong in [0u64, 9] {
                let mut s = good_stream();
                s[at + 8..at + 16].copy_from_slice(&wrong.to_le_bytes());
                assert!(
                    scan_stream(&s, &test_collection()).is_err(),
                    "record {records} sequence {wrong}"
                );
            }
        }
    }

    #[test]
    fn a_frame_end_that_does_not_close_its_open_hour_is_rejected() {
        let mut s = Vec::new();
        s.extend(record(1, 100, D35, COMPACT_RECORD_TYPE_DOMAIN_DATA, &d35_payload(0, &[])));
        s.extend(frame_end(2, 999));
        assert!(scan_stream(&s, &test_collection()).is_err());

        let mut s = Vec::new();
        s.extend(frame_end(1, 100));
        assert!(scan_stream(&s, &test_collection()).is_err());
    }

    #[test]
    fn non_monotonic_hours_are_rejected() {
        let mut s = Vec::new();
        s.extend(record(1, 101, D35, COMPACT_RECORD_TYPE_DOMAIN_DATA, &d35_payload(0, &[])));
        s.extend(frame_end(2, 101));
        s.extend(record(3, 100, D35, COMPACT_RECORD_TYPE_DOMAIN_DATA, &d35_payload(0, &[])));
        s.extend(frame_end(4, 100));
        assert!(scan_stream(&s, &test_collection()).is_err());
    }

    #[test]
    fn a_second_d35_record_in_one_hour_is_rejected() {
        // The rule runs over EVERY d35 DATA record and before status exclusion,
        // so all four success/failure orderings are refused — gating it on the
        // record being usable would let a success+failure pair through. On an
        // incremental domain this matters more than on a full-replace one: two
        // records in one hour would fold twice into one measurement step.
        const FAILED: u32 = 1; // COMPACT_STATUS_STATE_UNAVAILABLE
        for (first, second) in [(0u32, 0u32), (0, FAILED), (FAILED, 0), (FAILED, FAILED)] {
            let mut s = Vec::new();
            s.extend(record(
                1,
                100,
                D35,
                COMPACT_RECORD_TYPE_DOMAIN_DATA,
                &d35_payload(first, &[d35_row(7, 1_000_000)]),
            ));
            s.extend(record(
                2,
                100,
                D35,
                COMPACT_RECORD_TYPE_DOMAIN_DATA,
                &d35_payload(second, &[d35_row(7, 2_000_000)]),
            ));
            s.extend(frame_end(3, 100));
            assert!(
                scan_stream(&s, &test_collection()).is_err(),
                "two d35 records in hour 100 with statuses {first:#x}/{second:#x}"
            );
        }
    }

    #[test]
    fn a_failure_status_record_is_excluded_and_folds_nothing() {
        // Every failure bit in `COMPACT_STATUS_FAILURE_MASK`, one at a time: the
        // fold retains the accumulator and folds nothing, so the hour is not d35
        // data AND its rows must not reach the accumulator. On an incremental
        // domain the second half is the one that would corrupt every later hour
        // if it were wrong, so it is asserted on the surviving hour's state.
        for bit in [0u32, 1, 2, 3, 6, 7, 8, 9, 12] {
            let mut s = Vec::new();
            s.extend(record(
                1,
                100,
                D35,
                COMPACT_RECORD_TYPE_DOMAIN_DATA,
                &d35_payload(1 << bit, &[d35_row(7, 1_000_000), d35_row(8, 1_000_000)]),
            ));
            s.extend(frame_end(2, 100));
            s.extend(record(
                3,
                101,
                D35,
                COMPACT_RECORD_TYPE_DOMAIN_DATA,
                &d35_payload(0, &[d35_row(9, 2_000_000)]),
            ));
            s.extend(frame_end(4, 101));
            let scan =
                scan_stream(&s, &test_collection()).expect("the stream itself is well-formed");
            assert_eq!(scan.failure_records, 1, "bit {bit}");
            assert_eq!(scan.hours.len(), 1, "bit {bit}");
            assert_eq!(scan.hours[0].game_hour, 101, "bit {bit}");
            assert_eq!(
                scan.hours[0].state.len(),
                1,
                "bit {bit}: the excluded record's rows must not reach the accumulator"
            );
        }
        // A non-failure bit is NOT treated as failure. d35's fold never reads
        // DELTA_DATA at all, so a record carrying it folds exactly as one
        // without it — there is no baseline/delta distinction to make.
        let mut s = Vec::new();
        s.extend(record(
            1,
            100,
            D35,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &d35_payload(1 << 10, &[d35_row(7, 1_000_000)]),
        ));
        s.extend(frame_end(2, 100));
        let scan = scan_stream(&s, &test_collection()).expect("well-formed");
        assert_eq!(scan.failure_records, 0);
        assert_eq!(scan.hours.len(), 1);
        assert_eq!(scan.hours[0].state.len(), 1);
    }

    #[test]
    fn a_failure_record_is_classified_before_the_length_check_the_fold_defers() {
        // `fold_air_variant_stats` returns on a failure status at `:15149`
        // BEFORE the exact-length check at `:15156`, so a failure record whose
        // length disagrees with its `row_count` is one the recorder accepts. The
        // harness must classify it out, not refuse the stream over it.
        let mut bad = d35_payload(1, &[]);
        bad[8..12].copy_from_slice(&9u32.to_le_bytes()); // row_count lies
        let mut s = Vec::new();
        s.extend(record(1, 100, D35, COMPACT_RECORD_TYPE_DOMAIN_DATA, &bad));
        s.extend(frame_end(2, 100));
        s.extend(record(
            3,
            101,
            D35,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &d35_payload(0, &[d35_row(7, 1_000_000)]),
        ));
        s.extend(frame_end(4, 101));
        let scan = scan_stream(&s, &test_collection()).expect("the recorder accepts this stream");
        assert_eq!(scan.failure_records, 1);
        assert_eq!(scan.hours.len(), 1);

        // But a check the fold makes BEFORE the guard still applies to a failure
        // record: a zero scale word rejects the payload regardless of status.
        let mut s = Vec::new();
        s.extend(record(
            1,
            100,
            D35,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &d35_payload_scaled(1, (0, 1_000), &[]),
        ));
        s.extend(frame_end(2, 100));
        assert!(scan_stream(&s, &test_collection()).is_err());
    }

    #[test]
    fn a_stream_with_no_d35_records_is_refused_not_reported_as_zero() {
        let mut s = Vec::new();
        s.extend(record(1, 100, 20, COMPACT_RECORD_TYPE_DOMAIN_DATA, &[0u8; 8]));
        s.extend(frame_end(2, 100));
        assert!(scan_stream(&s, &test_collection()).is_err());
    }

    #[test]
    fn a_malformed_d35_payload_is_refused_not_skipped() {
        // A short or inconsistent payload on a SUCCESS record is a corrupt
        // stream, not an hour to drop quietly.
        let mut payload = d35_payload(0, &[d35_row(7, 1_000_000)]);
        payload.truncate(payload.len() - 1);
        let mut s = Vec::new();
        s.extend(record(1, 100, D35, COMPACT_RECORD_TYPE_DOMAIN_DATA, &payload));
        s.extend(frame_end(2, 100));
        assert!(scan_stream(&s, &test_collection()).is_err());

        // A row the FOLD rejects must make the SCAN fail, not be accepted and
        // then unwind while measuring. d35's row rejections are exactly two.
        let mut invalid: Vec<Vec<u8>> = Vec::new();
        invalid.push(d35_payload(0, &[d35_row(0, 1_000_000)])); // zero variant_id
        let mut reserved = d35_payload(0, &[d35_row(7, 1_000_000)]);
        let at = d35::HEADER_LEN + 60;
        reserved[at..at + 4].copy_from_slice(&1u32.to_le_bytes());
        invalid.push(reserved);
        // And both scale words, on a success record.
        invalid.push(d35_payload_scaled(0, (0, 1_000), &[d35_row(7, 1_000_000)]));
        invalid.push(d35_payload_scaled(0, (100_000, 0), &[d35_row(7, 1_000_000)]));
        for (i, payload) in invalid.into_iter().enumerate() {
            let mut s = Vec::new();
            s.extend(record(1, 100, D35, COMPACT_RECORD_TYPE_DOMAIN_DATA, &payload));
            s.extend(frame_end(2, 100));
            assert!(
                scan_stream(&s, &test_collection()).is_err(),
                "invalid payload {i} must make the scan fail"
            );
        }
    }

    #[test]
    fn a_repeated_variant_in_one_payload_is_folded_not_refused() {
        // The opposite of d63: `world_fold.rs:15176` inserts without checking,
        // so a repeat is last-writer-wins and the stream is fine. A harness that
        // copied d63's duplicate rejection would refuse a stream the recorder
        // reads.
        let mut s = Vec::new();
        s.extend(record(
            1,
            100,
            D35,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &d35_payload(0, &[d35_row(7, 1_000_000), d35_row(7, 2_000_000)]),
        ));
        s.extend(frame_end(2, 100));
        let scan = scan_stream(&s, &test_collection()).expect("a duplicate key is not corruption");
        assert_eq!(scan.hours[0].state.len(), 1);
        assert_eq!(scan.hours[0].payload_rows, 2);
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
