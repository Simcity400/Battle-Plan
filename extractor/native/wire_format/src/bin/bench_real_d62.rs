//! Measure the wire format against the shipping compact stream on REAL d62
//! `hub_motorization_live` dynamics.
//!
//! Input: a real `collector_stream.bin` from a d62-scheduled capture
//! (`runs/batch1/deepwar_t1`). The stream's own d62 DATA records are the
//! shipping format's cost for the domain: a 32-byte header plus a fixed 32-byte
//! row per motorization map entry, full-replace every hour, exactly what the
//! recorder stores today. This tool replays those records into per-hour states —
//! the same fold `world_fold::fold_hub_motorization` performs — and re-encodes
//! every hour with this crate's grammar: a BASELINE at each 48-hour block start,
//! field-granular stored-minimal DELTAs between.
//!
//! **Nothing here re-derives a contract it can call.** Row and payload decoding
//! go through `wire_format::d62`'s public boundary; H4CR framing, record
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
//! check remains the recorder's. The subset covers every condition under which
//! this harness would otherwise report a number for a stream it should refuse.
//!
//! Outputs, for the compression step (`compress_arms.py` beside the run):
//!   ship_d62.bin   — the d62 DATA records verbatim (32-byte H4CR header +
//!                    payload), concatenated in stream order
//!   wire_new.bin   — the wire-format bodies, concatenated in hour order
//!   *.offsets      — hour-boundary byte offsets, one per line, for exact
//!                    block-span slicing
//!
//! It also prints the real churn surface (changed rows per hour at stored
//! precision).

use std::collections::BTreeMap;
use std::io::Write;
use std::ops::Range;

use compact_contract::campaign_identity::COMPACT_RECORD_TYPE_STREAM_HEADER;
use compact_contract::stream_validator::StatefulStreamValidator;
use compact_contract::{
    compact_status_failure_flags_after_expected_partial_tolerance,
    compact_status_unknown_flags_v1, parse_compact_record_header, COMPACT_RECORD_HEADER_LEN,
    COMPACT_RECORD_TYPE_DOMAIN_DATA, COMPACT_RECORD_TYPE_FRAME_END,
};
use wire_format::body::{
    apply, decode_baseline, decode_delta, diff, encode_baseline, encode_delta, KeyedState,
};
use wire_format::d62;

const D62: u16 = 62;
const BLOCK_HOURS: usize = 48;

/// One usable d62 hour: its game hour, the folded state of that hour, and its
/// byte span in the source stream (the shipping arm is those bytes verbatim,
/// header included).
///
/// The state is built during the scan, not later: every way a row can be
/// rejected — an out-of-range id, an illegal motorization level, a row missing a
/// verification witness, a duplicated key triple — is a property of the stream,
/// so it belongs to the pass that either accepts the stream or refuses it.
/// Deferring it would leave the harness able to accept a stream and then unwind
/// while measuring it.
struct D62Hour {
    game_hour: i32,
    state: KeyedState,
    ship_span: Range<usize>,
}

struct StreamScan {
    hours: Vec<D62Hour>,
    /// d62 records whose payload status flags mark the domain-hour failed.
    failure_records: usize,
    /// Measured d62 hours that do not follow the previous measured one by one.
    non_contiguous: usize,
}

/// Validate the whole stream against the canonical contract and fold the usable
/// d62 DATA records into per-hour states. Every rejection path is an `Err`,
/// never a silently shorter measurement and never a panic — see the module docs
/// for what is enforced and what the recorder's own validator additionally
/// checks.
fn scan_stream(stream: &[u8], collection: &wire_format::Collection) -> Result<StreamScan, String> {
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

    let mut hours: Vec<D62Hour> = Vec::new();
    let mut failure_records = 0usize;
    let mut non_contiguous = 0usize;

    let mut open_hour: Option<i32> = None;
    let mut last_closed_hour: Option<i32> = None;
    let mut last_record_type: Option<u16> = None;
    // Hour of the last d62 DATA record of ANY status — the uniqueness rule.
    let mut last_d62_hour: Option<i32> = None;
    // Hour of the last d62 record that entered the measurement — the
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

        if header.domain_id == D62 && header.record_type == COMPACT_RECORD_TYPE_DOMAIN_DATA {
            let (d62_header, body) = d62::split_payload(payload)
                .map_err(|e| format!("d62 payload at byte {pos}: {e}"))?;

            // One d62 DATA record per game hour, checked over EVERY d62 record
            // and BEFORE any status-based exclusion. d62 is full-replace, so a
            // second record for the same hour would become a second measurement
            // hour built from the same clock time — and gating this on the
            // record being usable would let a success/failure pair through the
            // rule entirely.
            if let Some(previous) = last_d62_hour {
                if header.game_hour_raw <= previous {
                    return Err(format!(
                        "d62 DATA at byte {pos} carries hour {} after hour {previous}",
                        header.game_hour_raw
                    ));
                }
            }
            last_d62_hour = Some(header.game_hour_raw);

            // VALIDITY BEFORE CLASSIFICATION. The failure predicate below reads
            // the status word to classify the v1 vocabulary; it is not a validity
            // predicate. `compact_contract/src/lib.rs:121`-`:123` says so in as
            // many words: "Callers handling untrusted status words must first
            // reject a non-zero `compact_status_unknown_flags_v1` result;
            // otherwise an unassigned bit has no defined classification." Bits
            // 14..=31 are unassigned (`status_flags_base.rs:21`-`:24`), and an
            // unassigned bit "is not evidence of a future informational status".
            //
            // Without this, a record with `status_flags = 1 << 14` reads as "not
            // a failure" — the failure helper masks with the *known* failure set
            // and returns zero for a bit it does not know — and the hour is
            // measured as healthy. The production lane rejects first
            // (`lanes/division_state_movement_live/domain20_compact.rs:419`);
            // this harness does the same rather than re-deriving a lenient rule.
            // No capture carries an unassigned bit, so no measured number moves.
            let unknown_status = compact_status_unknown_flags_v1(d62_header.status_flags);
            if unknown_status != 0 {
                return Err(format!(
                    "d62 DATA at byte {pos}: hour {} carries unassigned v1 status bits \
                     {unknown_status:#x}; an unassigned bit has no defined classification",
                    header.game_hour_raw
                ));
            }

            // **Per-row decoding runs BEFORE the status check, on every d62
            // record, because that is the order the fold runs it in.**
            // `fold_hub_motorization` calls `decode_payload_rows` first
            // (`world_fold.rs:15851`) and only reaches `handle_compact_failure`
            // (`:15862`) if that decode returned `Ok`. `decode_payload_rows`
            // applies `validate_row` (`lanes/hub_motorization_live/src/lib.rs:295`,
            // `:327`) to every row, so a malformed row in a FAILURE-status record
            // is still an error that marks the domain unknown and returns `Err`
            // (`:15854`-`:15860`) — the failure status never gets looked at.
            // Deferring this until after the status check would let this harness
            // accept a stream the fold refuses.
            let mut decoded: Vec<(wire_format::body::Key, wire_format::Record)> =
                Vec::with_capacity(d62_header.row_count as usize);
            for row in body.chunks_exact(d62::ROW_LEN) {
                let key =
                    d62::key_from_row(row).map_err(|e| format!("d62 row at byte {pos}: {e}"))?;
                let record = d62::record_from_row(collection, row, 0)
                    .map_err(|e| format!("d62 row at byte {pos}: {e}"))?;
                decoded.push((key, record));
            }

            // `world_fold::handle_compact_failure` (called at `world_fold.rs:15862`)
            // marks the domain-hour unknown and RETAINS the prior accumulator on
            // a failure status, folding nothing from this record. The predicate
            // is `compact_contract`'s own, not a copied bit mask: it already
            // applies d62's lane-local mask (none) and expected partial
            // tolerance (none), so it cannot drift from the recorder.
            if compact_status_failure_flags_after_expected_partial_tolerance(
                D62,
                d62_header.status_flags,
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

                // The rest of the fold, in the fold's own order: everything from
                // here down sits AFTER `handle_compact_failure` in
                // `fold_hub_motorization` (`:15869`-`:15909`) and so applies only
                // to a record the fold actually folds. A duplicated
                // `(owner, province, actor)` triple marks the domain unknown and
                // folds nothing (`:15881`-`:15897`) — never a last-writer-wins
                // overwrite — but in a failure-status record the fold has already
                // returned and never looks, which is why this is not hoisted.
                let mut rows = 0usize;
                let mut state: KeyedState = BTreeMap::new();
                for (key, record) in decoded {
                    rows += 1;
                    if state.insert(key, record).is_some() {
                        return Err(format!(
                            "d62 payload at byte {pos}: duplicate owner/province/actor row"
                        ));
                    }
                }
                // `world_fold.rs:15899`: the fold counts the rows it walked and
                // requires the header's total to match. `split_payload` checks
                // the same total against the declared row count; this closes the
                // third side of the triangle against the rows actually walked.
                if rows != d62_header.row_count as usize
                    || rows != d62_header.total_map_entries as usize
                {
                    return Err(format!("d62 payload at byte {pos}: row count mismatch"));
                }
                hours.push(D62Hour {
                    game_hour: header.game_hour_raw,
                    state,
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
        return Err("stream carries no usable d62 DATA records".to_string());
    }

    Ok(StreamScan { hours, failure_records, non_contiguous })
}

fn main() {
    let mut args = std::env::args().skip(1);
    let input = args
        .next()
        .expect("usage: bench_real_d62 <collector_stream.bin> <out dir>");
    let out_dir = args
        .next()
        .expect("usage: bench_real_d62 <collector_stream.bin> <out dir>");
    let stream = std::fs::read(&input).expect("read stream");
    let schema = d62::schema();
    let collection = &schema.collections[0];

    // ---- Pass 1: validate the WHOLE stream and fold its d62 DATA records. ---
    // No artifact is created before this succeeds, so a refused stream leaves no
    // partial measurement behind to be mistaken for a result. Everything that
    // can fail on stream content fails here, with a diagnosis and no unwind.
    let scan = match scan_stream(&stream, collection) {
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
    let mut empty_hours = 0usize;
    let mut changed_rows_total = 0usize;
    let mut tombstones_total = 0usize;
    let mut entries_total = 0usize;

    for hour in &scan.hours {
        // Shipping arm: the record's exact bytes, header included.
        ship.extend_from_slice(&stream[hour.ship_span.clone()]);
        ship_offsets.push(ship.len());

        // The hour's state, folded during the scan: d62 is full-replace, so the
        // hour's rows ARE the hour's whole state.
        let state: &KeyedState = &hour.state;
        entries_total += state.len();

        // New arm: BASELINE at each block start, stored-minimal DELTA between.
        let hour_start = wire.len();
        let block_start = hours_seen % BLOCK_HOURS == 0;
        let mut emitted = true;
        if block_start || prior.is_none() {
            encode_baseline(&schema, std::slice::from_ref(state), &mut wire)
                .expect("baseline encodes");
            baselines += 1;
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
        assert_eq!(replayed.len(), state.len(), "game hour {}: row count", hour.game_hour);
        let residual = diff(collection, replayed, state);
        assert!(
            residual.is_empty(),
            "game hour {}: reader state is not stored-equal to the fold",
            hour.game_hour
        );

        prior = Some(state);
        hours_seen += 1;
    }

    for (name, bytes) in [("ship_d62.bin", &ship), ("wire_new.bin", &wire)] {
        let mut f = std::fs::File::create(std::path::Path::new(&out_dir).join(name)).unwrap();
        f.write_all(bytes).unwrap();
    }
    for (name, offsets) in [
        ("ship_d62.offsets", &ship_offsets),
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
        "map entries avg      {:.1}",
        entries_total as f64 / hours_seen as f64
    );
    println!("baselines            {baselines} (every {BLOCK_HOURS} h)");
    println!("delta hours          {delta_hours}   empty hours {empty_hours}");
    println!(
        "real churn           {:.2} changed rows/hour, {:.3} tombstones/hour (stored precision)",
        changed_rows_total as f64 / delta_hours.max(1) as f64,
        tombstones_total as f64 / delta_hours.max(1) as f64
    );
    println!("ship_d62.bin         {} bytes", ship.len());
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
        d62::schema().collections[0].clone()
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

    /// One 32-byte d62 row, in the lane's row order.
    fn row(owner: u32, actor: u32, province: u32, level: u32) -> Vec<u8> {
        let mut out = Vec::with_capacity(d62::ROW_LEN);
        for word in [owner, actor, province, 5, 1, 3, level, d62::REQUIRED_FLAGS] {
            out.extend_from_slice(&word.to_le_bytes());
        }
        out
    }

    /// A d62 DATA payload: the 32-byte header then the rows.
    fn d62_payload(status_flags: u32, rows: &[Vec<u8>]) -> Vec<u8> {
        let mut payload = Vec::new();
        payload.extend_from_slice(&0u64.to_le_bytes()); // elapsed_wall_ms
        payload.extend_from_slice(&(rows.len() as u32).to_le_bytes()); // row_count
        payload.extend_from_slice(&status_flags.to_le_bytes());
        payload.extend_from_slice(&d62::SCHEMA_VERSION.to_le_bytes());
        payload.extend_from_slice(&1u32.to_le_bytes()); // total_settings_nodes
        payload.extend_from_slice(&(d62::ROW_LEN as u32).to_le_bytes());
        payload.extend_from_slice(&(rows.len() as u32).to_le_bytes()); // total_map_entries
        assert_eq!(payload.len(), d62::HEADER_LEN);
        for r in rows {
            payload.extend_from_slice(r);
        }
        payload
    }

    fn one_row_payload(status_flags: u32, level: u32) -> Vec<u8> {
        d62_payload(status_flags, &[row(16, 17, 13071, level)])
    }

    /// Two closed hours, each carrying one d62 DATA record.
    fn good_stream() -> Vec<u8> {
        let mut s = Vec::new();
        s.extend(record(1, 100, D62, COMPACT_RECORD_TYPE_DOMAIN_DATA, &one_row_payload(0, 1)));
        s.extend(frame_end(2, 100));
        s.extend(record(3, 101, D62, COMPACT_RECORD_TYPE_DOMAIN_DATA, &one_row_payload(0, 2)));
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
    fn a_well_formed_stream_yields_its_d62_hours() {
        let scan = scan_stream(&good_stream(), &test_collection()).expect("accepted");
        assert_eq!(scan.hours.len(), 2);
        assert_eq!(scan.hours[0].game_hour, 100);
        assert_eq!(scan.hours[1].game_hour, 101);
        assert_eq!(scan.failure_records, 0);
        assert_eq!(scan.non_contiguous, 0);
        assert_eq!(scan.hours[0].state.len(), 1);
        // The key is the fold's triple in the fold's order, not row order.
        assert_eq!(
            scan.hours[0].state.keys().next().unwrap(),
            &wire_format::body::key(&[16, 13071, 17])
        );
    }

    #[test]
    fn an_unassigned_status_bit_is_refused_not_classified() {
        // `compact_contract/src/lib.rs:121`-`:123`: the failure helper classifies
        // the v1 vocabulary and is not a validity predicate, so an unassigned bit
        // must be rejected BEFORE classification — otherwise it silently reads as
        // "not a failure" and the hour is measured as healthy.
        for bit in 14..32u32 {
            let status = 1u32 << bit;
            // Alone, combined with a known failure bit, and combined with a known
            // informational bit; with rows and without. The rejection must not
            // depend on any of it.
            for extra in [0u32, 1, 1 << 10] {
                for rows in [Vec::new(), vec![row(16, 17, 13071, 1)]] {
                    let mut s = Vec::new();
                    s.extend(record(
                        1,
                        100,
                        D62,
                        COMPACT_RECORD_TYPE_DOMAIN_DATA,
                        &d62_payload(status | extra, &rows),
                    ));
                    s.extend(frame_end(2, 100));
                    assert!(
                        scan_stream(&s, &test_collection()).is_err(),
                        "status bit {bit} (+{extra:#x}) with {} row(s) must be refused",
                        rows.len()
                    );
                }
            }
        }
        // And the whole known v1 mask, minus the failure bits, is still accepted:
        // the check must reject unassigned bits and nothing else.
        // `fold_hub_motorization` never reads an informational bit at all.
        for informational in [1u32 << 4, 1 << 5, 1 << 10, 1 << 11, 1 << 13] {
            let mut s = Vec::new();
            s.extend(record(
                1,
                100,
                D62,
                COMPACT_RECORD_TYPE_DOMAIN_DATA,
                &one_row_payload(informational, 1),
            ));
            s.extend(frame_end(2, 100));
            let scan = scan_stream(&s, &test_collection())
                .unwrap_or_else(|e| panic!("informational bit {informational:#x}: {e}"));
            assert_eq!(scan.hours.len(), 1, "bit {informational:#x}");
            assert_eq!(scan.failure_records, 0, "bit {informational:#x}");
        }
    }

    #[test]
    fn a_stream_truncated_on_a_record_boundary_is_rejected() {
        // A cut immediately after a d62 DATA record leaves that hour open and
        // ends the stream on a record that is not a FRAME_END. That would
        // otherwise produce a plausible shorter number; it must be refused.
        // Records 1 and 3 of `good_stream` are the DATA records.
        let full = good_stream();
        for records in [1usize, 3] {
            let cut = boundary(&full, records);
            assert!(
                scan_stream(&full[..cut], &test_collection()).is_err(),
                "a stream cut after {records} record(s) leaves an hour open"
            );
        }
        // A cut exactly at a FRAME_END boundary is a different thing: that IS a
        // well-formed shorter stream, and `eof_check` accepts it too. Refusing it
        // would be wrong. What matters is that it reports the hours it actually
        // closed, not the hours the untruncated file had.
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
        // Every cut that is not a record boundary, across the whole stream.
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
        // Version, magic and header length, at every record in the stream —
        // the canonical parser owns these and the harness must not read past it.
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
        // A swallowed or reordered record changes which hours the measurement
        // sees, so the sequence chain is load-bearing, not decoration.
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
        s.extend(record(1, 100, D62, COMPACT_RECORD_TYPE_DOMAIN_DATA, &one_row_payload(0, 1)));
        s.extend(frame_end(2, 999)); // closes an hour that is not open
        assert!(scan_stream(&s, &test_collection()).is_err());

        // A FRAME_END with nothing open at all.
        let mut s = Vec::new();
        s.extend(frame_end(1, 100));
        assert!(scan_stream(&s, &test_collection()).is_err());
    }

    #[test]
    fn non_monotonic_hours_are_rejected() {
        // A closed hour that repeats or goes backwards would fold two different
        // clock times into one measurement step.
        let mut s = Vec::new();
        s.extend(record(1, 101, D62, COMPACT_RECORD_TYPE_DOMAIN_DATA, &one_row_payload(0, 1)));
        s.extend(frame_end(2, 101));
        s.extend(record(3, 100, D62, COMPACT_RECORD_TYPE_DOMAIN_DATA, &one_row_payload(0, 2)));
        s.extend(frame_end(4, 100));
        assert!(scan_stream(&s, &test_collection()).is_err());
    }

    #[test]
    fn a_second_d62_record_in_one_hour_is_rejected() {
        // d62 is full-replace; two records in one hour would become two
        // measurement hours built from the same clock time. The rule is checked
        // over EVERY d62 DATA record and before status-based exclusion, so all
        // four success/failure orderings are refused — gating it on the record
        // being usable would let a success + failure pair through.
        const FAILED: u32 = 1; // COMPACT_STATUS_STATE_UNAVAILABLE
        for (first, second) in [(0u32, 0u32), (0, FAILED), (FAILED, 0), (FAILED, FAILED)] {
            let mut s = Vec::new();
            s.extend(record(
                1,
                100,
                D62,
                COMPACT_RECORD_TYPE_DOMAIN_DATA,
                &one_row_payload(first, 1),
            ));
            s.extend(record(
                2,
                100,
                D62,
                COMPACT_RECORD_TYPE_DOMAIN_DATA,
                &one_row_payload(second, 2),
            ));
            s.extend(frame_end(3, 100));
            assert!(
                scan_stream(&s, &test_collection()).is_err(),
                "two d62 records in hour 100 with statuses {first:#x}/{second:#x}"
            );
        }
    }

    #[test]
    fn a_failure_status_record_is_excluded_rather_than_measured() {
        // Every failure bit in `COMPACT_STATUS_FAILURE_MASK`, one at a time:
        // the fold retains the accumulator and folds nothing, so the hour is not
        // d62 data and must not enter either arm.
        for bit in [0u32, 1, 2, 3, 6, 7, 8, 9, 12] {
            let mut s = Vec::new();
            s.extend(record(
                1,
                100,
                D62,
                COMPACT_RECORD_TYPE_DOMAIN_DATA,
                &one_row_payload(1 << bit, 1),
            ));
            s.extend(frame_end(2, 100));
            s.extend(record(3, 101, D62, COMPACT_RECORD_TYPE_DOMAIN_DATA, &one_row_payload(0, 2)));
            s.extend(frame_end(4, 101));
            let scan =
                scan_stream(&s, &test_collection()).expect("the stream itself is well-formed");
            assert_eq!(scan.failure_records, 1, "bit {bit}");
            assert_eq!(scan.hours.len(), 1, "bit {bit}");
            assert_eq!(scan.hours[0].game_hour, 101, "bit {bit}");
        }
        // And a non-failure bit (DELTA_DATA, 1<<10) is NOT treated as failure.
        let mut s = Vec::new();
        s.extend(record(1, 100, D62, COMPACT_RECORD_TYPE_DOMAIN_DATA, &one_row_payload(1 << 10, 1)));
        s.extend(frame_end(2, 100));
        let scan = scan_stream(&s, &test_collection()).expect("well-formed");
        assert_eq!(scan.failure_records, 0);
        assert_eq!(scan.hours.len(), 1);
    }

    #[test]
    fn a_stream_with_no_d62_records_is_refused_not_reported_as_zero() {
        let mut s = Vec::new();
        s.extend(record(1, 100, 20, COMPACT_RECORD_TYPE_DOMAIN_DATA, &[0u8; 8]));
        s.extend(frame_end(2, 100));
        assert!(scan_stream(&s, &test_collection()).is_err());
    }

    #[test]
    fn a_malformed_d62_payload_is_refused_not_skipped() {
        // A short or inconsistent d62 payload is a corrupt stream, not an hour
        // to drop quietly.
        let mut payload = one_row_payload(0, 1);
        payload.truncate(payload.len() - 1);
        let mut s = Vec::new();
        s.extend(record(1, 100, D62, COMPACT_RECORD_TYPE_DOMAIN_DATA, &payload));
        s.extend(frame_end(2, 100));
        assert!(scan_stream(&s, &test_collection()).is_err());

        // A row the CODEC rejects must make the SCAN fail, not be accepted and
        // then unwind while measuring. The whole row-rejection space, not one
        // sampled case. (The status crossing lives in
        // `a_malformed_row_is_refused_whatever_the_records_status`.)
        let mut invalid: Vec<Vec<u8>> = row_rejecting_payloads(0);
        // A header declaring the wrong schema version or row stride.
        for (offset, value) in [(16usize, 3u32), (24, 16)] {
            let mut bad = d62_payload(0, &[row(16, 17, 13071, 1)]);
            bad[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
            invalid.push(bad);
        }
        // A header whose total_map_entries disagrees with its row count
        // (`world_fold.rs:15899`).
        let mut mismatch = d62_payload(0, &[row(16, 17, 13071, 1)]);
        mismatch[28..32].copy_from_slice(&7u32.to_le_bytes());
        invalid.push(mismatch);

        for (i, payload) in invalid.into_iter().enumerate() {
            let mut s = Vec::new();
            s.extend(record(1, 100, D62, COMPACT_RECORD_TYPE_DOMAIN_DATA, &payload));
            s.extend(frame_end(2, 100));
            assert!(
                scan_stream(&s, &test_collection()).is_err(),
                "invalid payload {i} must make the scan fail"
            );
        }

        // A key triple repeated within ONE frame marks the domain unknown and
        // folds nothing (`world_fold.rs:15881`-`:15897`) — it must be caught in
        // the scan, not asserted in the measuring pass. Two rows that differ in
        // a NON-key word still collide, which is the case a key-blind duplicate
        // check would miss.
        let mut dup_row = row(16, 17, 13071, 1);
        dup_row[16..20].copy_from_slice(&99u32.to_le_bytes()); // other record_index
        let payload = d62_payload(0, &[row(16, 17, 13071, 1), dup_row]);
        let mut s = Vec::new();
        s.extend(record(1, 100, D62, COMPACT_RECORD_TYPE_DOMAIN_DATA, &payload));
        s.extend(frame_end(2, 100));
        assert!(
            scan_stream(&s, &test_collection()).is_err(),
            "duplicate (owner, province, actor) in one frame"
        );

        // Two rows that differ ONLY in a key word are NOT duplicates.
        let payload = d62_payload(0, &[row(16, 17, 13071, 1), row(16, 18, 13071, 2)]);
        let mut s = Vec::new();
        s.extend(record(1, 100, D62, COMPACT_RECORD_TYPE_DOMAIN_DATA, &payload));
        s.extend(frame_end(2, 100));
        let scan = scan_stream(&s, &test_collection()).expect("two distinct actors");
        assert_eq!(scan.hours[0].state.len(), 2);
    }

    /// Every payload whose ROWS the codec rejects, paired with every status.
    fn row_rejecting_payloads(status_flags: u32) -> Vec<Vec<u8>> {
        let mut invalid: Vec<Vec<u8>> = vec![
            d62_payload(status_flags, &[row(0, 17, 13071, 1)]), // owner 0
            d62_payload(status_flags, &[row(16, 0, 13071, 1)]), // actor 0
            d62_payload(status_flags, &[row(16, 17, 0, 1)]),    // province 0
            d62_payload(
                status_flags,
                &[row(d62::MAX_COUNTRY_COUNT + 1, 17, 13071, 1)],
            ),
            d62_payload(
                status_flags,
                &[row(16, d62::MAX_COUNTRY_COUNT + 1, 13071, 1)],
            ),
            d62_payload(status_flags, &[row(16, 17, d62::MAX_PROVINCE_ID + 1, 1)]),
            d62_payload(status_flags, &[row(16, 17, 13071, 0)]), // level 0
            d62_payload(status_flags, &[row(16, 17, 13071, 3)]), // level 3
        ];
        // A zero state_id (`lib.rs:346`).
        let mut zero_state = d62_payload(status_flags, &[row(16, 17, 13071, 1)]);
        zero_state[d62::HEADER_LEN + 12..d62::HEADER_LEN + 16]
            .copy_from_slice(&0u32.to_le_bytes());
        invalid.push(zero_state);
        // A missing verification witness, an unknown flag bit, and no flags at
        // all (`lib.rs:358`-`:368`).
        for flags in [
            d62::REQUIRED_FLAGS & !d62::ROW_FLAG_ACTOR_VERIFIED,
            d62::REQUIRED_FLAGS | (1 << 20),
            0,
        ] {
            let mut bad = d62_payload(status_flags, &[row(16, 17, 13071, 1)]);
            bad[d62::HEADER_LEN + 28..d62::HEADER_LEN + 32]
                .copy_from_slice(&flags.to_le_bytes());
            invalid.push(bad);
        }
        invalid
    }

    #[test]
    fn a_malformed_row_is_refused_whatever_the_records_status() {
        // The reviewer's input, generalized. `fold_hub_motorization` calls
        // `decode_payload_rows` FIRST (`world_fold.rs:15851`) and only reaches
        // `handle_compact_failure` (`:15862`) if that decode succeeded, so a row
        // `validate_row` rejects is an error even in a failure-status record —
        // the status is never examined. A harness that classified the status
        // first would accept a stream the fold refuses. Swept over the whole
        // row-rejection space, crossed with a clean status, the reviewer's
        // `status_flags = 1`, and another failure bit.
        for status in [0u32, 1, 1 << 3] {
            for (i, payload) in row_rejecting_payloads(status).into_iter().enumerate() {
                let mut s = Vec::new();
                s.extend(record(1, 100, D62, COMPACT_RECORD_TYPE_DOMAIN_DATA, &payload));
                s.extend(frame_end(2, 100));
                // A second, wholly valid hour: without it a bug could be masked
                // by the "no usable d62 records" refusal rather than caught.
                s.extend(record(
                    3,
                    101,
                    D62,
                    COMPACT_RECORD_TYPE_DOMAIN_DATA,
                    &one_row_payload(0, 2),
                ));
                s.extend(frame_end(4, 101));
                assert!(
                    scan_stream(&s, &test_collection()).is_err(),
                    "payload {i} with status {status:#x} must make the scan fail"
                );
            }
        }
    }

    #[test]
    fn a_duplicate_key_in_a_failure_status_record_is_excluded_not_refused() {
        // The other direction of the same ordering rule, and the reason the
        // duplicate check is NOT hoisted alongside the row decode: the fold's
        // duplicate detection lives at `world_fold.rs:15881`, AFTER
        // `handle_compact_failure` has already returned `Ok` for a failed
        // record. Imposing a check the fold does not perform would make this
        // harness refuse a stream the recorder accepts.
        let dup = d62_payload(1, &[row(16, 17, 13071, 1), row(16, 17, 13071, 2)]);
        let mut s = Vec::new();
        s.extend(record(1, 100, D62, COMPACT_RECORD_TYPE_DOMAIN_DATA, &dup));
        s.extend(frame_end(2, 100));
        s.extend(record(3, 101, D62, COMPACT_RECORD_TYPE_DOMAIN_DATA, &one_row_payload(0, 2)));
        s.extend(frame_end(4, 101));
        let scan = scan_stream(&s, &test_collection()).expect("the fold accepts this stream");
        assert_eq!(scan.failure_records, 1);
        assert_eq!(scan.hours.len(), 1, "only the clean hour is measured");
        assert_eq!(scan.hours[0].game_hour, 101);

        // ... and with a clean status the very same payload IS refused.
        let dup = d62_payload(0, &[row(16, 17, 13071, 1), row(16, 17, 13071, 2)]);
        let mut s = Vec::new();
        s.extend(record(1, 100, D62, COMPACT_RECORD_TYPE_DOMAIN_DATA, &dup));
        s.extend(frame_end(2, 100));
        assert!(scan_stream(&s, &test_collection()).is_err());
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
