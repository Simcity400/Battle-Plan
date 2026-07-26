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
//! pulls in every capture lane), so the subset is enforced here and the full check
//! remains the recorder's.
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

use std::io::Write;
use std::ops::Range;

use compact_contract::campaign_identity::COMPACT_RECORD_TYPE_STREAM_HEADER;
use compact_contract::stream_validator::StatefulStreamValidator;
use compact_contract::{
    compact_status_failure_flags_after_expected_partial_tolerance, parse_compact_record_header,
    COMPACT_RECORD_HEADER_LEN, COMPACT_RECORD_TYPE_DOMAIN_DATA, COMPACT_RECORD_TYPE_FRAME_END,
    COMPACT_STATUS_DELTA_DATA, COMPACT_STATUS_FAILURE_MASK,
};
use wire_format::body::{
    apply, decode_baseline, decode_delta, diff, encode_baseline, encode_delta, KeyedState,
};
use wire_format::d36;

const D36: u16 = 36;
const BLOCK_HOURS: usize = 48;

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

/// Validate the whole stream against the canonical contract and fold its d36
/// DATA records into per-hour states. Every rejection path is an `Err`, never a
/// silently shorter measurement and never a panic.
fn scan_stream(stream: &[u8], schema: &wire_format::Schema) -> Result<StreamScan, String> {
    // The two constants `d36` names locally, against the contract crate that owns
    // them. A drift here would silently change which hours are measured.
    if d36::STATUS_DELTA_DATA != COMPACT_STATUS_DELTA_DATA {
        return Err("d36::STATUS_DELTA_DATA disagrees with compact_contract".to_string());
    }
    if d36::FAILURE_STATUS_MASK != COMPACT_STATUS_FAILURE_MASK {
        return Err("d36::FAILURE_STATUS_MASK disagrees with compact_contract".to_string());
    }

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

    let mut hours: Vec<D36Hour> = Vec::new();
    let mut failure_records = 0usize;
    let mut delta_before_baseline = 0usize;
    let mut non_contiguous = 0usize;
    let mut resets = 0usize;

    let mut open_hour: Option<i32> = None;
    let mut last_closed_hour: Option<i32> = None;
    let mut last_record_type: Option<u16> = None;
    // Hour of the last d36 DATA record of ANY status — the uniqueness rule.
    let mut last_d36_hour: Option<i32> = None;
    // Hour of the last d36 record that entered the measurement — the contiguity
    // report, which a skipped failure hour legitimately breaks.
    let mut last_measured_hour: Option<i32> = None;

    // The accumulator, carried across records exactly as the recorder carries it.
    let mut state = d36::empty_state();

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

        if header.domain_id == D36 && header.record_type == COMPACT_RECORD_TYPE_DOMAIN_DATA {
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

            // The fold's own predicate, not a copied bit mask: it applies d36's
            // lane-local mask (none) and expected-partial tolerance (none), so it
            // cannot drift from the recorder. `d36::State::fold_payload` reaches
            // the same verdict from its own constant; both are asserted to agree.
            let contract_failure = compact_status_failure_flags_after_expected_partial_tolerance(
                D36,
                d36::parse_header(payload)
                    .map_err(|e| format!("d36 payload at byte {pos}: {e}"))?
                    .status_flags,
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
                d36::FoldOutcome::Folded { reset, upserts, deletes } => {
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

    Ok(StreamScan {
        hours,
        failure_records,
        delta_before_baseline,
        non_contiguous,
        resets,
    })
}

fn main() {
    let mut args = std::env::args().skip(1);
    let input = args
        .next()
        .expect("usage: bench_real_d36 <collector_stream.bin> <out dir>");
    let out_dir = args
        .next()
        .expect("usage: bench_real_d36 <collector_stream.bin> <out dir>");
    let stream = std::fs::read(&input).expect("read stream");
    let schema = d36::schema();
    let collection = &schema.collections[0];

    // ---- Pass 1: validate the WHOLE stream and fold its d36 DATA records. ----
    // No artifact is created before this succeeds, so a refused stream leaves no
    // partial measurement behind to be mistaken for a result.
    let scan = match scan_stream(&stream, &schema) {
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
    println!("baselines            {baselines} (every {BLOCK_HOURS} h)");
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
        scan_stream(stream, &d36::schema())
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
        s.extend(record(1, 100, 20, COMPACT_RECORD_TYPE_DOMAIN_DATA, &[0u8; 8]));
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
            s.extend(record(1, 100, D36, COMPACT_RECORD_TYPE_DOMAIN_DATA, &payload));
            s.extend(frame_end(2, 100));
            assert!(scan(&s).is_err(), "malformed payload {i} must make the scan fail");
        }
    }

    #[test]
    fn an_extreme_amount_is_measured_not_refused() {
        // The fold range-checks `amount_raw` not at all, so a capture carrying an
        // enormous one is a stream the recorder folds and this harness must
        // measure. Regression for the round-1 review finding: an earlier mapper
        // refused magnitudes above 2^51, which made the whole stream unmeasurable.
        for amount in [
            (1i64 << 51) + 1,
            100_000_000_000_000_000, // the collector's own ceiling, 1e12 * 10^5
            i64::MAX,
            i64::MIN,
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
}
