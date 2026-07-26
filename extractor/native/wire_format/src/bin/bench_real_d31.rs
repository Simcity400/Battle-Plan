//! Measure the wire format against the shipping compact stream on REAL d31
//! `air_region_activity` dynamics.
//!
//! Input: a real `collector_stream.bin` from a d31-scheduled capture
//! (`runs/batch2/deepwar_t1`). The stream's own d31 DATA records are the shipping
//! format's cost for the domain: a 36-byte header plus a fixed 32-byte row per
//! *changed* `(region, owner, mission)` bucket, sparse after the first frame,
//! exactly what the recorder stores today. This tool replays those records
//! through `wire_format::d31::Accumulator` — the same fold
//! `world_fold::fold_air_region_activity` performs — into per-hour states, and
//! re-encodes every hour with this crate's grammar: a BASELINE at each 48-hour
//! block start, field-granular stored-minimal DELTAs between.
//!
//! **d31 is incremental, so one record is not one state.** The shipping arm
//! carries a few changed rows per hour and the wire arm's delta carries the
//! diff of two reconstructed states; the two therefore measure the same thing —
//! the cost of transporting one hour of change — but only the wire arm ever
//! re-sends a whole state. That asymmetry is what the baseline/delta split in the
//! output is for: the block ratio measures re-baseline policy, the delta bytes
//! measure the grammar.
//!
//! **Nothing here re-derives a contract it can call.** Header, row and frame
//! decoding go through `wire_format::d31`'s public boundary
//! (`peek_header`/`split_payload`/`parse_row`/`record_from_row`/`Accumulator`);
//! H4CR framing, record semantics and sequence continuity go through
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
//!   (`recorder_host/src/stream_fold.rs:277`) that do not require DOMAIN_PERF
//!   accounting: a DOMAIN_DATA record opens its game hour, a FRAME_END closes the
//!   open hour and only that hour, and closed hours strictly increase;
//! - the EOF rules of that tracker's `eof_check` (`:459`): no residual bytes, no
//!   hour left open, at least one FRAME_END, and the final record is a FRAME_END.
//!
//! That is a documented **subset** of the production boundary contract:
//! `recorder_host::stream_fold::verify_stream_logical_boundary` (`:537`) is the
//! full authority, and it additionally checks FRAME_END seal fields and
//! DOMAIN_PERF domain accounting. This crate cannot depend on `recorder_host` (it
//! pulls in every capture lane), so the subset is enforced here and the full
//! check remains the recorder's. The subset covers every condition under which
//! this harness would otherwise report a number for a stream it should refuse.
//!
//! **The two paths on which `fold_air_region_activity` returns without folding**
//! are excluded, at the position the fold puts them
//! (`world_fold.rs:14715`-`:14728`):
//!
//! 1. **failure status** — after the header-structural checks and before the
//!    exact-length check, so `d31::peek_header` runs on EVERY d31 record and
//!    `d31::split_payload` only on the ones that are folded. The predicate is
//!    `compact_contract`'s own
//!    `compact_status_failure_flags_after_expected_partial_tolerance`, never a
//!    hand-copied bit mask: d31 has no expected-partial tolerance, so the whole
//!    of `COMPACT_STATUS_FAILURE_MASK` (`COMPACT_STATUS_PARTIAL_DATA` included)
//!    counts.
//! 2. **delta before baseline** — owned by `d31::Accumulator::fold_payload`,
//!    which holds the `baseline_seen` state the rule consults.
//!
//! Neither drops an hour silently: both are counted and printed, and the hour
//! enters neither arm, because the recorder has no d31 data for it.
//!
//! Hour uniqueness is enforced across ALL d31 DATA records and BEFORE the status
//! exclusion — a success + failure pair in one hour would otherwise slip through
//! the rule entirely.
//!
//! Outputs, for the compression step (`compress_arms.py` beside the run):
//!   ship_d31.bin   — the d31 DATA records verbatim (32-byte H4CR header +
//!                    payload), concatenated in stream order
//!   wire_new.bin   — the wire-format bodies, concatenated in hour order
//!   *.offsets      — hour-boundary byte offsets, one per line, for exact
//!                    block-span slicing
//!
//! It also prints the real churn surface (changed rows and tombstones per hour at
//! stored precision) beside the shipping stream's own changed-row counts.

use std::io::Write;
use std::ops::Range;

use compact_contract::campaign_identity::COMPACT_RECORD_TYPE_STREAM_HEADER;
use compact_contract::stream_validator::StatefulStreamValidator;
use compact_contract::{
    compact_status_failure_flags_after_expected_partial_tolerance, parse_compact_record_header,
    COMPACT_RECORD_HEADER_LEN, COMPACT_RECORD_TYPE_DOMAIN_DATA, COMPACT_RECORD_TYPE_FRAME_END,
    COMPACT_STATUS_DELTA_DATA,
};
use wire_format::body::{
    apply, decode_baseline, decode_delta, diff, encode_baseline, encode_delta, KeyedState,
};
use wire_format::d31;

const D31: u16 = 31;
const BLOCK_HOURS: usize = 48;

/// One usable d31 hour: its game hour, the accumulated state after folding that
/// hour's record, and the record's byte span in the source stream (the shipping
/// arm is those bytes verbatim, header included).
///
/// The state is built during the scan, not later: every way a row can be rejected
/// — an unknown flag bit, a non-zero reserved word, a payload whose length
/// disagrees with its row count — is a property of the stream, so it belongs to
/// the pass that either accepts the stream or refuses it. Deferring it would leave
/// the harness able to accept a stream and then unwind while measuring it.
struct D31Hour {
    game_hour: i32,
    state: KeyedState,
    ship_span: Range<usize>,
    /// The frame's own `changed_rows`, for the shipping-side churn report.
    shipped_rows: u32,
    shipped_deletes: u32,
}

struct StreamScan {
    hours: Vec<D31Hour>,
    /// d31 records whose payload status flags mark the domain-hour failed.
    failure_records: usize,
    /// d31 delta records that arrived before any baseline (`world_fold.rs:14722`).
    delta_before_baseline: usize,
    /// Measured d31 hours that do not follow the previous measured one by one.
    non_contiguous: usize,
}

/// Validate the whole stream against the canonical contract and fold the usable
/// d31 DATA records into per-hour states. Every rejection path is an `Err`, never
/// a silently shorter measurement and never a panic — see the module docs for
/// what is enforced and what the recorder's own validator additionally checks.
fn scan_stream(stream: &[u8], collection: &wire_format::Collection) -> Result<StreamScan, String> {
    // `StatefulStreamValidator` deliberately never sniffs a payload to pick a
    // protocol, so the harness picks one explicitly from the FIRST RECORD'S
    // FRAMING HEADER: an identity-bearing stream opens with the CID1 record at
    // `(sequence, domain, type) = (0, 0, 4)`
    // (`compact_contract/src/stream_validator.rs:104`), and anything else is
    // legacy, whose first ordinary record is sequence 1. A wrong guess does not
    // degrade to a lenient read — the validator rejects the very first record.
    let first = parse_compact_record_header(stream).map_err(|e| format!("first record: {e}"))?;
    let mut validator = if first.sequence == 0
        && first.domain_id == 0
        && first.record_type == COMPACT_RECORD_TYPE_STREAM_HEADER
    {
        StatefulStreamValidator::identity_v1()
    } else {
        StatefulStreamValidator::legacy()
    };

    let mut hours: Vec<D31Hour> = Vec::new();
    let mut failure_records = 0usize;
    let mut delta_before_baseline = 0usize;
    let mut non_contiguous = 0usize;

    let mut open_hour: Option<i32> = None;
    let mut last_closed_hour: Option<i32> = None;
    let mut last_record_type: Option<u16> = None;
    // Hour of the last d31 DATA record of ANY status — the uniqueness rule.
    let mut last_d31_hour: Option<i32> = None;
    // Hour of the last d31 record that entered the measurement — the contiguity
    // report, which a skipped failure hour legitimately breaks.
    let mut last_measured_hour: Option<i32> = None;

    // The shipping fold's own accumulator, replayed through the domain module.
    let mut acc = d31::Accumulator::new();

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

        if header.domain_id == D31 && header.record_type == COMPACT_RECORD_TYPE_DOMAIN_DATA {
            // One d31 DATA record per game hour, checked over EVERY d31 record
            // and BEFORE any status-based exclusion. d31 is incremental, so a
            // second record for the same hour would fold twice into one clock
            // time — and gating this on the record being usable would let a
            // success/failure pair through the rule entirely.
            if let Some(previous) = last_d31_hour {
                if header.game_hour_raw <= previous {
                    return Err(format!(
                        "d31 DATA at byte {pos} carries hour {} after hour {previous}",
                        header.game_hour_raw
                    ));
                }
            }
            last_d31_hour = Some(header.game_hour_raw);

            // `world_fold.rs:14680`-`:14714`: the header-structural checks run on
            // every record, before either exclusion, and are hard errors.
            let payload_header = d31::peek_header(payload)
                .map_err(|e| format!("d31 payload at byte {pos}: {e}"))?;

            // `world_fold::handle_compact_failure` (`world_fold.rs:2843`, called
            // at `:14715`) marks the domain-hour unknown and RETAINS the prior
            // accumulator on a failure status, folding nothing from this record.
            // The predicate is `compact_contract`'s own, not a copied bit mask:
            // it already applies d31's lane-local mask (none) and expected
            // partial tolerance (none), so it cannot drift from the recorder.
            if compact_status_failure_flags_after_expected_partial_tolerance(
                D31,
                payload_header.status_flags,
            ) != 0
            {
                failure_records += 1;
                pos = record_end;
                continue;
            }

            let is_delta = payload_header.status_flags & COMPACT_STATUS_DELTA_DATA != 0;
            // Everything else — the exact-length check, the clear-or-sparse
            // decision, the row loop, the delete grammar — is the domain module's,
            // in the fold's own order.
            let outcome = acc
                .fold_payload(collection, payload, is_delta, 0)
                .map_err(|e| format!("d31 payload at byte {pos}: {e}"))?;
            if outcome == d31::FoldOutcome::DeltaBeforeBaseline {
                delta_before_baseline += 1;
                pos = record_end;
                continue;
            }

            if let Some(previous) = last_measured_hour {
                if header.game_hour_raw != previous + 1 {
                    non_contiguous += 1;
                }
            }
            last_measured_hour = Some(header.game_hour_raw);

            hours.push(D31Hour {
                game_hour: header.game_hour_raw,
                state: acc.rows.clone(),
                ship_span: pos..record_end,
                shipped_rows: payload_header.row_count,
                shipped_deletes: payload_header.deleted_rows,
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
        return Err("stream carries no usable d31 DATA records".to_string());
    }

    Ok(StreamScan {
        hours,
        failure_records,
        delta_before_baseline,
        non_contiguous,
    })
}

fn main() {
    let mut args = std::env::args().skip(1);
    let input = args
        .next()
        .expect("usage: bench_real_d31 <collector_stream.bin> <out dir>");
    let out_dir = args
        .next()
        .expect("usage: bench_real_d31 <collector_stream.bin> <out dir>");
    let stream = std::fs::read(&input).expect("read stream");
    let schema = d31::schema();
    let collection = &schema.collections[0];

    // ---- Pass 1: validate the WHOLE stream and fold its d31 DATA records. ----
    // No artifact is created before this succeeds, so a refused stream leaves no
    // partial measurement behind to be mistaken for a result. Everything that can
    // fail on stream content fails here, with a diagnosis and no unwind.
    let scan = match scan_stream(&stream, collection) {
        Ok(scan) => scan,
        Err(reason) => {
            eprintln!("{input}: {reason}");
            std::process::exit(1);
        }
    };
    std::fs::create_dir_all(&out_dir).expect("out dir");

    // ---- Pass 2: encode both arms. Nothing here reads stream content again, so
    // every assertion below is about the codec, not about the input. -----------

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
    let mut tombstones_total = 0usize;
    let mut buckets_total = 0usize;
    let mut shipped_rows_total = 0usize;
    let mut shipped_deletes_total = 0usize;
    let mut shipped_empty_frames = 0usize;

    for hour in &scan.hours {
        // Shipping arm: the record's exact bytes, header included.
        ship.extend_from_slice(&stream[hour.ship_span.clone()]);
        ship_offsets.push(ship.len());
        shipped_rows_total += hour.shipped_rows as usize;
        shipped_deletes_total += hour.shipped_deletes as usize;
        if hour.shipped_rows == 0 {
            shipped_empty_frames += 1;
        }

        // The hour's state, accumulated during the scan through the domain
        // module's own fold: d31 is sparse-after-first, so the hour's rows are
        // NOT the hour's state.
        let state: &KeyedState = &hour.state;
        buckets_total += state.len();

        // New arm: BASELINE at each block start, stored-minimal DELTA between.
        let hour_start = wire.len();
        let block_start = hours_seen % BLOCK_HOURS == 0;
        let mut emitted = true;
        if block_start || prior.is_none() {
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

    for (name, bytes) in [("ship_d31.bin", &ship), ("wire_new.bin", &wire)] {
        let mut f = std::fs::File::create(std::path::Path::new(&out_dir).join(name)).unwrap();
        f.write_all(bytes).unwrap();
    }
    for (name, offsets) in [
        ("ship_d31.offsets", &ship_offsets),
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
        "delta-before-baseline {} (excluded from both arms)",
        scan.delta_before_baseline
    );
    println!(
        "buckets avg          {:.1}",
        buckets_total as f64 / hours_seen as f64
    );
    println!("baselines            {baselines} (every {BLOCK_HOURS} h)");
    println!("delta hours          {delta_hours}   empty hours {empty_hours}");
    println!(
        "real churn           {:.2} changed rows/hour, {:.3} tombstones/hour (stored precision)",
        changed_rows_total as f64 / delta_hours.max(1) as f64,
        tombstones_total as f64 / delta_hours.max(1) as f64
    );
    println!(
        "shipping churn       {:.2} serialized rows/hour, {:.3} deletes/hour, \
         {shipped_empty_frames} zero-row frames",
        shipped_rows_total as f64 / hours_seen.max(1) as f64,
        shipped_deletes_total as f64 / hours_seen.max(1) as f64
    );
    println!("ship_d31.bin         {} bytes", ship.len());
    println!(
        "wire_new.bin         {} bytes ({:.1}%)",
        wire.len(),
        100.0 * wire.len() as f64 / ship.len() as f64
    );
    println!(
        "  of which baseline  {baseline_bytes} bytes ({:.1}%)   delta {delta_bytes} bytes ({:.1}%)",
        100.0 * baseline_bytes as f64 / wire.len().max(1) as f64,
        100.0 * delta_bytes as f64 / wire.len().max(1) as f64
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use compact_contract::{COMPACT_DOMAIN_FRAME, COMPACT_FRAME_END_PAYLOAD_LEN};

    fn test_collection() -> wire_format::Collection {
        d31::schema().collections[0].clone()
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

    /// One 32-byte d31 row, packed as the collector packs one
    /// (`attach_log.rs:69847`-`:69858`).
    fn row(region: u32, owner: u32, mask: u32, flags: u32, wings: u32, planes: u32, eff: u32) -> Vec<u8> {
        let mut r = Vec::new();
        for w in [region, owner, mask, flags, wings, planes, eff, 0] {
            r.extend_from_slice(&w.to_le_bytes());
        }
        r
    }

    /// A d31 DATA payload with the collector's own header arithmetic.
    fn d31_payload(status_flags: u32, rows: &[Vec<u8>], observed_bucket_rows: u32) -> Vec<u8> {
        let deleted = rows
            .iter()
            .filter(|r| u32::from_le_bytes(r[12..16].try_into().unwrap()) & d31::ROW_FLAG_DELETED != 0)
            .count() as u32;
        let mut p = Vec::new();
        p.extend_from_slice(&0u64.to_le_bytes()); // elapsed_wall_ms
        p.extend_from_slice(&(rows.len() as u32).to_le_bytes()); // row_count
        p.extend_from_slice(&(rows.len() as u32).to_le_bytes()); // changed_rows
        p.extend_from_slice(&status_flags.to_le_bytes());
        p.extend_from_slice(&0u32.to_le_bytes()); // observed_air_wing_rows
        p.extend_from_slice(&observed_bucket_rows.to_le_bytes());
        p.extend_from_slice(&deleted.to_le_bytes());
        p.extend_from_slice(&0u32.to_le_bytes()); // reserved0
        for r in rows {
            p.extend_from_slice(r);
        }
        p
    }

    /// Two closed hours: a baseline then a delta, the shape the real stream has.
    fn good_stream() -> Vec<u8> {
        let mut s = Vec::new();
        s.extend(record(
            1,
            100,
            D31,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &d31_payload(0, &[row(37, 12, 512, 0, 4, 76, 61)], 1),
        ));
        s.extend(frame_end(2, 100));
        s.extend(record(
            3,
            101,
            D31,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &d31_payload(
                COMPACT_STATUS_DELTA_DATA,
                &[row(37, 12, 512, 0, 5, 80, 70)],
                1,
            ),
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
    fn a_well_formed_stream_yields_its_d31_hours() {
        let scan = scan_stream(&good_stream(), &test_collection()).expect("accepted");
        assert_eq!(scan.hours.len(), 2);
        assert_eq!(scan.hours[0].game_hour, 100);
        assert_eq!(scan.hours[1].game_hour, 101);
        assert_eq!(scan.failure_records, 0);
        assert_eq!(scan.delta_before_baseline, 0);
        assert_eq!(scan.non_contiguous, 0);
        assert_eq!(scan.hours[0].state.len(), 1);
        // The delta UPDATED the one bucket rather than replacing the state.
        assert_eq!(scan.hours[1].state.len(), 1);
        assert_ne!(scan.hours[0].state, scan.hours[1].state);
    }

    #[test]
    fn the_scan_accumulates_rather_than_snapshotting() {
        // The property that separates d31 from a full-replace domain: an hour's
        // state is the fold's retained map, not the hour's rows. A delta that
        // mentions one bucket must leave the other one standing.
        let mut s = Vec::new();
        s.extend(record(
            1,
            100,
            D31,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &d31_payload(
                0,
                &[row(37, 12, 512, 0, 4, 76, 61), row(9, 3, 1, 0, 2, 30, 0)],
                2,
            ),
        ));
        s.extend(frame_end(2, 100));
        s.extend(record(
            3,
            101,
            D31,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &d31_payload(COMPACT_STATUS_DELTA_DATA, &[row(9, 3, 1, 0, 3, 44, 40)], 2),
        ));
        s.extend(frame_end(4, 101));
        // An empty delta frame: 23 of the capture's 146 records are this shape.
        s.extend(record(
            5,
            102,
            D31,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &d31_payload(COMPACT_STATUS_DELTA_DATA, &[], 2),
        ));
        s.extend(frame_end(6, 102));
        // A delete row removes its key and only its key.
        s.extend(record(
            7,
            103,
            D31,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &d31_payload(
                COMPACT_STATUS_DELTA_DATA,
                &[row(37, 12, 512, d31::ROW_FLAG_DELETED, 0, 0, 0)],
                1,
            ),
        ));
        s.extend(frame_end(8, 103));

        let scan = scan_stream(&s, &test_collection()).expect("accepted");
        assert_eq!(scan.hours.len(), 4);
        assert_eq!(scan.hours[0].state.len(), 2);
        assert_eq!(scan.hours[1].state.len(), 2, "a delta does not clear");
        assert_eq!(scan.hours[2].state, scan.hours[1].state, "an empty delta is a no-op");
        assert_eq!(scan.hours[3].state.len(), 1, "the delete removed one bucket");
    }

    #[test]
    fn a_non_delta_record_after_the_first_is_sparse_not_a_replace() {
        // `replace_or_sparse_after_first` (`world_fold.rs:2927`): only the FIRST
        // record clears. A harness that treated every non-delta record as a
        // snapshot would silently drop every bucket the later frame omits.
        let mut s = Vec::new();
        s.extend(record(
            1,
            100,
            D31,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &d31_payload(
                0,
                &[row(37, 12, 512, 0, 4, 76, 61), row(9, 3, 1, 0, 2, 30, 0)],
                2,
            ),
        ));
        s.extend(frame_end(2, 100));
        s.extend(record(
            3,
            101,
            D31,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &d31_payload(0, &[row(9, 3, 1, 0, 3, 44, 40)], 2),
        ));
        s.extend(frame_end(4, 101));
        let scan = scan_stream(&s, &test_collection()).expect("accepted");
        assert_eq!(scan.hours[1].state.len(), 2);
    }

    #[test]
    fn a_stream_truncated_on_a_record_boundary_is_rejected() {
        // A cut immediately after a d31 DATA record leaves that hour open and ends
        // the stream on a record that is not a FRAME_END.
        let full = good_stream();
        for records in [1usize, 3] {
            let cut = boundary(&full, records);
            assert!(
                scan_stream(&full[..cut], &test_collection()).is_err(),
                "a stream cut after {records} record(s) leaves an hour open"
            );
        }
        // A cut exactly at a FRAME_END boundary IS a well-formed shorter stream,
        // and `eof_check` accepts it too. What matters is that it reports the
        // hours it actually closed, not the hours the untruncated file had.
        for (records, expected_hours) in [(2usize, 1usize), (4, 2)] {
            let cut = boundary(&full, records);
            let scan = scan_stream(&full[..cut], &test_collection())
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
                    scan_stream(&s, &test_collection()).is_err(),
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
                    scan_stream(&s, &test_collection()).is_err(),
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
            D31,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &d31_payload(0, &[row(37, 12, 512, 0, 4, 76, 61)], 1),
        ));
        s.extend(frame_end(2, 999)); // closes an hour that is not open
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
            D31,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &d31_payload(0, &[row(37, 12, 512, 0, 4, 76, 61)], 1),
        ));
        s.extend(frame_end(2, 101));
        s.extend(record(
            3,
            100,
            D31,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &d31_payload(COMPACT_STATUS_DELTA_DATA, &[row(37, 12, 512, 0, 5, 80, 70)], 1),
        ));
        s.extend(frame_end(4, 100));
        assert!(scan_stream(&s, &test_collection()).is_err());
    }

    #[test]
    fn a_second_d31_record_in_one_hour_is_rejected() {
        // Two records in one hour would fold twice into one clock time. The rule
        // is checked over EVERY d31 DATA record and before status-based exclusion,
        // so all four success/failure orderings are refused.
        const FAILED: u32 = 1; // COMPACT_STATUS_STATE_UNAVAILABLE
        for (first, second) in [(0u32, 0u32), (0, FAILED), (FAILED, 0), (FAILED, FAILED)] {
            let mut s = Vec::new();
            s.extend(record(
                1,
                100,
                D31,
                COMPACT_RECORD_TYPE_DOMAIN_DATA,
                &d31_payload(first, &[row(37, 12, 512, 0, 4, 76, 61)], 1),
            ));
            s.extend(record(
                2,
                100,
                D31,
                COMPACT_RECORD_TYPE_DOMAIN_DATA,
                &d31_payload(second, &[row(37, 12, 512, 0, 5, 80, 70)], 1),
            ));
            s.extend(frame_end(3, 100));
            assert!(
                scan_stream(&s, &test_collection()).is_err(),
                "two d31 records in hour 100 with statuses {first:#x}/{second:#x}"
            );
        }
    }

    #[test]
    fn a_failure_status_record_is_excluded_rather_than_measured() {
        // Every failure bit in `COMPACT_STATUS_FAILURE_MASK`, one at a time: the
        // fold retains the accumulator and folds nothing, so the hour is not d31
        // data and must not enter either arm. d31 has NO expected-partial
        // tolerance, so `PARTIAL_DATA` (bit 9) is in the list.
        for bit in [0u32, 1, 2, 3, 6, 7, 8, 9, 12] {
            let mut s = Vec::new();
            s.extend(record(
                1,
                100,
                D31,
                COMPACT_RECORD_TYPE_DOMAIN_DATA,
                &d31_payload(1 << bit, &[row(37, 12, 512, 0, 4, 76, 61)], 1),
            ));
            s.extend(frame_end(2, 100));
            s.extend(record(
                3,
                101,
                D31,
                COMPACT_RECORD_TYPE_DOMAIN_DATA,
                &d31_payload(0, &[row(9, 3, 1, 0, 2, 30, 0)], 1),
            ));
            s.extend(frame_end(4, 101));
            let scan =
                scan_stream(&s, &test_collection()).expect("the stream itself is well-formed");
            assert_eq!(scan.failure_records, 1, "bit {bit}");
            assert_eq!(scan.hours.len(), 1, "bit {bit}");
            assert_eq!(scan.hours[0].game_hour, 101, "bit {bit}");
            // And the excluded record folded NOTHING: hour 101's baseline is the
            // only state, carrying one bucket, the one hour 101 serialized.
            assert_eq!(scan.hours[0].state.len(), 1, "bit {bit}");
            assert!(
                scan.hours[0]
                    .state
                    .contains_key(&wire_format::body::key(&[9, 3, 1])),
                "bit {bit}: the failure record's row must not be folded"
            );
        }
        // And a non-failure bit (DELTA_DATA) is NOT treated as failure — though as
        // the first record it becomes delta-before-baseline instead.
        let mut s = Vec::new();
        s.extend(record(
            1,
            100,
            D31,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &d31_payload(COMPACT_STATUS_DELTA_DATA, &[row(37, 12, 512, 0, 4, 76, 61)], 1),
        ));
        s.extend(frame_end(2, 100));
        s.extend(record(
            3,
            101,
            D31,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &d31_payload(0, &[row(9, 3, 1, 0, 2, 30, 0)], 1),
        ));
        s.extend(frame_end(4, 101));
        let scan = scan_stream(&s, &test_collection()).expect("well-formed");
        assert_eq!(scan.failure_records, 0);
        assert_eq!(scan.delta_before_baseline, 1);
        assert_eq!(scan.hours.len(), 1);
        assert_eq!(scan.hours[0].game_hour, 101);
    }

    #[test]
    fn a_failure_record_still_has_its_header_checked() {
        // `world_fold.rs:14680`-`:14714` runs BEFORE `handle_compact_failure` at
        // `:14715`, so a failure-status record with a malformed header is a hard
        // error, not a quietly skipped hour.
        let mut payload = d31_payload(1, &[row(37, 12, 512, 0, 4, 76, 61)], 1);
        payload[32..36].copy_from_slice(&1u32.to_le_bytes()); // reserved0
        let mut s = Vec::new();
        s.extend(record(1, 100, D31, COMPACT_RECORD_TYPE_DOMAIN_DATA, &payload));
        s.extend(frame_end(2, 100));
        assert!(scan_stream(&s, &test_collection()).is_err());

        // But the exact-length check runs AFTER the exclusion (`:14729`), so a
        // failure-status record whose length disagrees with its row_count is
        // accepted by the fold — and must be accepted here.
        let mut payload = d31_payload(1, &[row(37, 12, 512, 0, 4, 76, 61)], 1);
        payload.pop();
        let mut s = Vec::new();
        s.extend(record(1, 100, D31, COMPACT_RECORD_TYPE_DOMAIN_DATA, &payload));
        s.extend(frame_end(2, 100));
        s.extend(record(
            3,
            101,
            D31,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &d31_payload(0, &[row(9, 3, 1, 0, 2, 30, 0)], 1),
        ));
        s.extend(frame_end(4, 101));
        let scan = scan_stream(&s, &test_collection())
            .expect("the fold returns Ok before it checks the length");
        assert_eq!(scan.failure_records, 1);
        assert_eq!(scan.hours.len(), 1);
    }

    #[test]
    fn a_stream_with_no_d31_records_is_refused_not_reported_as_zero() {
        let mut s = Vec::new();
        s.extend(record(1, 100, 20, COMPACT_RECORD_TYPE_DOMAIN_DATA, &[0u8; 8]));
        s.extend(frame_end(2, 100));
        assert!(scan_stream(&s, &test_collection()).is_err());
    }

    #[test]
    fn a_malformed_d31_payload_is_refused_not_skipped() {
        // A short or inconsistent d31 payload is a corrupt stream, not an hour to
        // drop quietly. The whole rejection space, not one sample.
        let mut invalid: Vec<Vec<u8>> = Vec::new();
        // Truncated payload (`world_fold.rs:14731`).
        let mut short = d31_payload(0, &[row(37, 12, 512, 0, 4, 76, 61)], 1);
        short.truncate(short.len() - 1);
        invalid.push(short);
        // Shorter than the header (`:14680`).
        invalid.push(vec![0u8; d31::HEADER_LEN - 1]);
        // changed_rows > row_count (`:14691`).
        let mut bad = d31_payload(0, &[row(37, 12, 512, 0, 4, 76, 61)], 1);
        bad[12..16].copy_from_slice(&2u32.to_le_bytes());
        invalid.push(bad);
        // deleted_rows > row_count (`:14698`).
        let mut bad = d31_payload(0, &[row(37, 12, 512, 0, 4, 76, 61)], 1);
        bad[28..32].copy_from_slice(&2u32.to_le_bytes());
        invalid.push(bad);
        // row_count > observed_bucket_rows + deleted_rows (`:14705`).
        invalid.push(d31_payload(0, &[row(37, 12, 512, 0, 4, 76, 61)], 0));
        // Non-zero header reserved (`:14712`).
        let mut bad = d31_payload(0, &[row(37, 12, 512, 0, 4, 76, 61)], 1);
        bad[32..36].copy_from_slice(&1u32.to_le_bytes());
        invalid.push(bad);
        // An unknown row flag bit (`:14748`).
        invalid.push(d31_payload(0, &[row(37, 12, 512, 1 << 5, 0, 0, 0)], 1));
        // A non-zero row reserved word (`:14752`).
        let mut bad = d31_payload(0, &[row(37, 12, 512, 0, 4, 76, 61)], 1);
        let at = d31::HEADER_LEN + 28;
        bad[at..at + 4].copy_from_slice(&1u32.to_le_bytes());
        invalid.push(bad);

        for (i, payload) in invalid.into_iter().enumerate() {
            let mut s = Vec::new();
            s.extend(record(1, 100, D31, COMPACT_RECORD_TYPE_DOMAIN_DATA, &payload));
            s.extend(frame_end(2, 100));
            assert!(
                scan_stream(&s, &test_collection()).is_err(),
                "invalid payload {i} must make the scan fail"
            );
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
            let _ = scan_stream(&bytes, &test_collection());
        }
        // And corruption of a genuinely well-formed stream, byte by byte.
        let good = good_stream();
        for i in 0..good.len() {
            let mut s = good.clone();
            s[i] ^= 0xFF;
            let _ = scan_stream(&s, &test_collection());
        }
    }
}
