//! Measure the wire format against the shipping compact stream on REAL d29
//! `ships_live` dynamics.
//!
//! Input: a real `collector_stream.bin` from a d29-scheduled capture
//! (`runs/batch1/deepwar_t1`). The stream's own d29 DATA records are the shipping
//! format's cost for the domain: a 40-byte header plus, per serialized ship, 40
//! fixed bytes and a u16-length-prefixed `definition` string — one full baseline
//! followed by sparse per-ship updates and tombstones, exactly what the recorder
//! stores today. This tool replays those records into the accumulator
//! `world_fold::fold_ships` builds and re-encodes every hour with this crate's
//! grammar: a BASELINE at each 48-hour block start, field-granular
//! stored-minimal DELTAs between.
//!
//! **d29 is shipping-incremental, so one record is not one state.** `fold_ships`
//! clears only on its first record and upserts thereafter
//! (`replace_or_sparse_after_first`, `world_fold.rs:14550`), and a
//! `ROW_FLAG_DELETED_LOW_BIT` row removes a key (`:14555`). The accumulator is
//! therefore rebuilt by folding the whole record series, through
//! `wire_format::d29`'s public `State`/`fold_payload` boundary — the same code
//! the unit tests exercise. Deletes become wire **tombstones**, and nothing here
//! constructs one by hand: `body::diff` derives them from the two states, which
//! is what makes them minimal, and `body::encode_delta` refuses a tombstone for a
//! key the reader's reconstructed prior does not hold.
//!
//! **Two passes, because the states do not fit.** Pass 1 validates the whole
//! stream, folds every d29 record in full (so every way a row can be rejected is
//! exercised by the pass that accepts or refuses the stream) and retains only
//! record spans. Pass 2 re-folds through the identical code path while encoding,
//! holding at most three states at once. Up to ~1.75k ships × 903 hours does not
//! fit as per-hour snapshots.
//!
//! **The symbol table is built in pass 1, before any record is mapped.** §5
//! requires symbol ids to be file-global and forbids in-stream interning — "it
//! must be fully read before any seek" — so the `definition` vocabulary is
//! collected over the whole validated stream first and frozen into a
//! `d29::Symbols` that pass 2 only reads. See `d29`'s module docs for what §13
//! will replace.
//!
//! **Nothing here re-derives a contract it can call.** Row and payload decoding
//! go through `wire_format::d29`'s public boundary; H4CR framing, record
//! semantics and sequence continuity go through `compact_contract`'s own parser
//! and stateful validator; failure classification and the DELTA_DATA bit go
//! through `compact_contract`'s own predicates and constants rather than a copied
//! bit mask. A harness that reimplements any of them drifts from the recorder
//! without anything failing.
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
//!   hour left open, at least one FRAME_END, and the final record is a FRAME_END;
//! - and every rule `fold_ships` applies to a d29 payload, by folding each one
//!   during the scan.
//!
//! That is a documented **subset** of the production boundary contract:
//! `recorder_host::stream_fold::verify_stream_logical_boundary` (`:537`) is the
//! full authority, and it additionally checks FRAME_END seal fields and
//! DOMAIN_PERF domain accounting. This crate cannot depend on `recorder_host` (it
//! pulls in every capture lane), so the subset is enforced here and the full
//! check remains the recorder's.
//!
//! **Which hours are measured.** `fold_ships` has exactly two paths on which it
//! folds nothing and the retained accumulator stands, both taken *after* the
//! header is fully validated and *before* any row is walked
//! (`world_fold.rs:14508`-`:14513`, in this order):
//!
//! 1. any failure status, by `compact_contract`'s own tolerance-aware predicate
//!    (`handle_compact_failure`, `:2843`) — d29 has no expected-partial
//!    tolerance, so no partial is ever tolerated for it;
//! 2. a delta before any baseline (`reject_delta_before_baseline`, `:2875`).
//!
//! d29 has no `reject_expected_partial_before_baseline` call and no rowless-
//! partial path, so those two are the whole list. The exclusion's **position** is
//! the fold's: the header of every d29 record is validated whatever its status
//! (a malformed header is a rejected frame, not an excluded hour), and rows are
//! never walked for an excluded record. Hour uniqueness is enforced over EVERY
//! d29 DATA record, of every status, BEFORE either exclusion, or a
//! success+failure pair in one hour slips through. A stream cut exactly at a
//! FRAME_END boundary is a well-formed shorter stream and is accepted as such.
//!
//! **A rowless d29 record is measured, not excluded.** `fold_ships` folds it:
//! with `row_count == 0` the walk does nothing but `replace_or_sparse_after_first`
//! still runs, so a rowless *first* record would clear the accumulator. It is an
//! hour in which nothing changed, and the wire arm emits nothing for it. 14 of
//! the measured capture's 903 records are rowless.
//!
//! Outputs, for the compression step (`compress_arms.py` beside the run):
//!   ship_d29.bin   — the d29 DATA records verbatim (32-byte H4CR header +
//!                    payload), concatenated in stream order
//!   wire_new.bin   — the wire-format bodies, concatenated in hour order
//!   *.offsets      — hour-boundary byte offsets, one per line, for exact
//!                    block-span slicing
//!
//! It also prints the real churn surface (changed rows and tombstones per hour at
//! stored precision) and the baseline/delta split of the wire arm.

use std::collections::BTreeSet;
use std::io::Write;
use std::ops::Range;

use compact_contract::campaign_identity::COMPACT_RECORD_TYPE_STREAM_HEADER;
use compact_contract::stream_validator::StatefulStreamValidator;
use compact_contract::{
    compact_status_failure_flags_after_expected_partial_tolerance,
    compact_status_unknown_flags_v1, parse_compact_record_header, COMPACT_RECORD_HEADER_LEN,
    COMPACT_RECORD_TYPE_DOMAIN_DATA, COMPACT_RECORD_TYPE_FRAME_END, COMPACT_STATUS_DELTA_DATA,
};
use wire_format::body::{
    apply, decode_baseline, decode_delta, diff, encode_baseline, encode_delta, KeyedState,
};
use wire_format::d29::{self, Symbols};
// `Schema` is used by the test module only; the binary path takes it from `d29`.

const D29: u16 = 29;
const BLOCK_HOURS: usize = 48;

/// One usable d29 hour: its game hour, its byte span in the source stream (the
/// shipping arm is those bytes verbatim, header included), the payload span pass
/// 2 re-folds from, and the `sparse` decision that payload was folded under.
///
/// The state is deliberately NOT retained: see the module docs.
struct D29Hour {
    game_hour: i32,
    ship_span: Range<usize>,
    payload_span: Range<usize>,
    /// `replace_or_sparse_after_first`'s answer for this record
    /// (`world_fold.rs:2927`). Recorded rather than recomputed, so pass 2 folds
    /// under exactly the decision pass 1 validated.
    sparse: bool,
}

#[derive(Default)]
struct ScanCounts {
    /// d29 records whose payload status marks the domain-hour failed.
    failure_records: usize,
    /// Delta records that arrived before any baseline.
    delta_before_baseline: usize,
    /// Measured hours that do not follow the previous measured one by one.
    non_contiguous: usize,
    /// Measured records carrying no rows at all.
    rowless_records: usize,
    /// Rows the fold applied, across every measured record.
    rows_folded: usize,
    /// Tombstone rows the fold applied.
    delete_rows: usize,
}

struct StreamScan {
    hours: Vec<D29Hour>,
    counts: ScanCounts,
    /// The `definition` vocabulary of the whole stream, frozen before any record
    /// is mapped (§5).
    symbols: Symbols,
}

/// Validate the whole stream against the canonical contract and fold every
/// usable d29 DATA record. Every rejection path is an `Err`, never a silently
/// shorter measurement and never a panic — see the module docs for what is
/// enforced and what the recorder's own validator additionally checks.
fn scan_stream(stream: &[u8]) -> Result<StreamScan, String> {
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

    let mut hours: Vec<D29Hour> = Vec::new();
    let mut counts = ScanCounts::default();
    let mut vocabulary: BTreeSet<String> = BTreeSet::new();

    let mut open_hour: Option<i32> = None;
    let mut last_closed_hour: Option<i32> = None;
    let mut last_record_type: Option<u16> = None;
    // Hour of the last d29 DATA record of ANY status — the uniqueness rule.
    let mut last_d29_hour: Option<i32> = None;
    // Hour of the last d29 record that entered the measurement — the contiguity
    // report, which a skipped failure hour legitimately breaks.
    let mut last_measured_hour: Option<i32> = None;
    // `WorldFold::delta_baselines_seen` for domain 29 (`world_fold.rs:2927`).
    let mut baseline_seen = false;
    // Pass 1 cannot resolve symbols — the table is what this pass is building,
    // and §5 forbids interning one as the stream is read. So it applies the
    // fold's accumulation rules to a **key-only mirror** of the accumulator:
    // the same walk, the same per-row checks, the same clear/upsert/remove
    // rules, with no symbol dependency. The only failure pass 2 can then have
    // that pass 1 did not is an unresolvable symbol, and that is impossible by
    // construction — every live row's definition enters the vocabulary right
    // here, through the same `parse_row`.
    // `the_symbol_table_covers_every_definition_in_the_stream` tests that claim.
    let mut mirror: BTreeSet<u32> = BTreeSet::new();

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

        if header.domain_id == D29 && header.record_type == COMPACT_RECORD_TYPE_DOMAIN_DATA {
            // The header, first and unconditionally: `fold_ships` validates it
            // before it looks at the status word (`world_fold.rs:14475`-`:14507`),
            // so a malformed header is a rejected frame however the status reads.
            // Rows are NOT walked here — the fold walks them only after both
            // exclusions (`:14515`).
            let d29_header = d29::peek_header(payload)
                .map_err(|e| format!("d29 payload at byte {pos}: {e}"))?;

            // One d29 DATA record per game hour, checked over EVERY d29 record
            // and BEFORE any status-based exclusion: gating this on the record
            // being usable would let a success/failure pair through the rule
            // entirely.
            if let Some(previous) = last_d29_hour {
                if header.game_hour_raw <= previous {
                    return Err(format!(
                        "d29 DATA at byte {pos} carries hour {} after hour {previous}",
                        header.game_hour_raw
                    ));
                }
            }
            last_d29_hour = Some(header.game_hour_raw);

            // VALIDITY BEFORE CLASSIFICATION. Both exclusions below read the
            // status word to classify the v1 vocabulary; neither is a validity
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
            let unknown_status = compact_status_unknown_flags_v1(d29_header.status_flags);
            if unknown_status != 0 {
                return Err(format!(
                    "d29 DATA at byte {pos}: hour {} carries unassigned v1 status bits \
                     {unknown_status:#x}; an unassigned bit has no defined classification",
                    header.game_hour_raw
                ));
            }

            // Exclusion 1: `handle_compact_failure` (`:14508`). The predicate is
            // `compact_contract`'s own, not a copied bit mask: it already applies
            // d29's lane-local mask and expected-partial tolerance (d29 has
            // none), so it cannot drift from the recorder.
            if compact_status_failure_flags_after_expected_partial_tolerance(
                D29,
                d29_header.status_flags,
            ) != 0
            {
                counts.failure_records += 1;
                pos = record_end;
                continue;
            }
            // Exclusion 2: `reject_delta_before_baseline` (`:14511`).
            let delta = (d29_header.status_flags & COMPACT_STATUS_DELTA_DATA) != 0;
            if delta && !baseline_seen {
                counts.delta_before_baseline += 1;
                pos = record_end;
                continue;
            }

            // `replace_or_sparse_after_first` (`:2927`), side effect included.
            let sparse = if delta || baseline_seen {
                true
            } else {
                baseline_seen = true;
                false
            };

            if let Some(previous) = last_measured_hour {
                if header.game_hour_raw != previous + 1 {
                    counts.non_contiguous += 1;
                }
            }
            last_measured_hour = Some(header.game_hour_raw);

            // Fold exactly as `fold_ships` does, on a key-only mirror: the walk,
            // every per-row check and the clear/upsert/remove rules, without a
            // symbol table that does not exist yet. Pass 2 re-folds the same
            // spans through `d29::fold_payload` and must not fail where this did
            // not; the shared `split_payload`/`parse_row` is what guarantees it.
            let (walk_header, rows) = d29::split_payload(payload)
                .map_err(|e| format!("d29 payload at byte {pos}: {e}"))?;
            let mut parsed_rows = Vec::with_capacity(rows.len());
            for row in rows {
                parsed_rows.push(
                    d29::parse_row(row).map_err(|e| format!("d29 row at byte {pos}: {e}"))?,
                );
            }
            if walk_header.row_count == 0 {
                counts.rowless_records += 1;
            }
            if !sparse {
                mirror.clear();
            }
            for parsed in &parsed_rows {
                counts.rows_folded += 1;
                if parsed.is_deleted() {
                    counts.delete_rows += 1;
                    mirror.remove(&parsed.instance_id);
                } else {
                    mirror.insert(parsed.instance_id);
                    if let Some(definition) = parsed.definition.as_deref() {
                        if !vocabulary.contains(definition) {
                            vocabulary.insert(definition.to_string());
                        }
                    }
                }
            }

            hours.push(D29Hour {
                game_hour: header.game_hour_raw,
                ship_span: pos..record_end,
                payload_span: payload_start..record_end,
                sparse,
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
        return Err("stream carries no usable d29 DATA records".to_string());
    }

    // §5: the table is complete before any record is mapped, and never grows
    // afterwards. Proving pass 2 cannot need a symbol pass 1 did not see is what
    // makes "no in-stream interning" a property of this harness rather than a
    // hope: pass 2 walks exactly these spans with exactly this parse.
    let symbols = Symbols::from_vocabulary(vocabulary);

    Ok(StreamScan { hours, counts, symbols })
}

fn main() {
    let mut args = std::env::args().skip(1);
    let input = args
        .next()
        .expect("usage: bench_real_d29 <collector_stream.bin> <out dir>");
    let out_dir = args
        .next()
        .expect("usage: bench_real_d29 <collector_stream.bin> <out dir>");
    let stream = std::fs::read(&input).expect("read stream");
    let schema = d29::schema();
    let collection = &schema.collections[0];

    // ---- Pass 1: validate the WHOLE stream and fold its d29 DATA records. ---
    // No artifact is created before this succeeds, so a refused stream leaves no
    // partial measurement behind to be mistaken for a result.
    let scan = match scan_stream(&stream) {
        Ok(scan) => scan,
        Err(reason) => {
            eprintln!("{input}: {reason}");
            std::process::exit(1);
        }
    };
    std::fs::create_dir_all(&out_dir).expect("out dir");

    // ---- Pass 2: re-fold and encode both arms. ------------------------------

    let mut ship = Vec::new();
    let mut ship_offsets = vec![0usize];
    let mut wire = Vec::new();
    let mut wire_offsets = vec![0usize];

    let mut acc = d29::State::new();
    let mut reader: Option<KeyedState> = None;
    let mut hours_seen = 0usize;
    let mut baselines = 0usize;
    let mut baseline_bytes = 0usize;
    let mut delta_bytes = 0usize;
    let mut empty_hours = 0usize;
    let mut changed_rows_total = 0usize;
    let mut tombstones_total = 0usize;
    let mut ships_total = 0usize;

    for hour in &scan.hours {
        // Shipping arm: the record's exact bytes, header included.
        ship.extend_from_slice(&stream[hour.ship_span.clone()]);
        ship_offsets.push(ship.len());

        // The hour's state: the prior accumulator with this record folded in,
        // through the SAME public boundary the unit tests exercise.
        let prior: KeyedState = acc.ships().clone();
        d29::fold_payload(
            &schema,
            &mut acc,
            &stream[hour.payload_span.clone()],
            hour.sparse,
            0,
            &scan.symbols,
        )
        .unwrap_or_else(|e| panic!("game hour {}: pass 2 fold failed: {e}", hour.game_hour));
        let state: &KeyedState = acc.ships();
        ships_total += state.len();

        // New arm: BASELINE at each block start, stored-minimal DELTA between.
        let hour_start = wire.len();
        let block_start = hours_seen % BLOCK_HOURS == 0;
        let mut emitted = true;
        if block_start {
            encode_baseline(&schema, std::slice::from_ref(state), &mut wire)
                .expect("baseline encodes");
            baselines += 1;
            baseline_bytes += wire.len() - hour_start;
        } else {
            let d = diff(collection, &prior, state);
            if d.is_empty() {
                empty_hours += 1;
                emitted = false;
            } else {
                changed_rows_total += d.changed.len();
                tombstones_total += d.tombstones.len();
                encode_delta(&schema, std::slice::from_ref(&prior), &[Some(d)], &mut wire)
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
        // equal the fold's, which is precisely what "nothing changed" asserts.
        if emitted {
            let hour_bytes = &wire[hour_start..];
            if block_start {
                let (states, consumed) =
                    decode_baseline(&schema, hour_bytes, 0).expect("baseline decodes");
                assert_eq!(consumed, hour_bytes.len(), "baseline consumed exactly");
                reader = Some(states.into_iter().next().unwrap());
            } else {
                let reader_prior = reader.take().expect("a delta follows a baseline");
                let (deltas, consumed) =
                    decode_delta(&schema, std::slice::from_ref(&reader_prior), hour_bytes, 0)
                        .expect("delta decodes");
                assert_eq!(consumed, hour_bytes.len(), "delta consumed exactly");
                let mut next = reader_prior;
                apply(&mut next, deltas[0].as_ref().unwrap());
                reader = Some(next);
            }
        }
        let replayed = reader.as_ref().expect("the first hour is a baseline");
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

        hours_seen += 1;
    }

    for (name, bytes) in [("ship_d29.bin", &ship), ("wire_new.bin", &wire)] {
        let mut f = std::fs::File::create(std::path::Path::new(&out_dir).join(name)).unwrap();
        f.write_all(bytes).unwrap();
    }
    for (name, offsets) in [
        ("ship_d29.offsets", &ship_offsets),
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
        scan.counts.non_contiguous
    );
    println!(
        "excluded hours       {} failure-status, {} delta-before-baseline",
        scan.counts.failure_records, scan.counts.delta_before_baseline
    );
    println!(
        "shipping rows        {} folded ({} delete rows), {} rowless records",
        scan.counts.rows_folded, scan.counts.delete_rows, scan.counts.rowless_records
    );
    println!(
        "definition symbols   {} (§5 vocabulary, frozen before mapping)",
        scan.symbols.len()
    );
    println!(
        "ships avg            {:.1}",
        ships_total as f64 / hours_seen as f64
    );
    println!("baselines            {baselines} (every {BLOCK_HOURS} h)");
    println!("delta hours          {delta_hours}   empty hours {empty_hours}");
    println!(
        "real churn           {:.2} changed rows/hour, {:.3} tombstones/hour (stored precision)",
        changed_rows_total as f64 / delta_hours.max(1) as f64,
        tombstones_total as f64 / delta_hours.max(1) as f64
    );
    println!("ship_d29.bin         {} bytes", ship.len());
    println!(
        "wire_new.bin         {} bytes ({:.1}%)",
        wire.len(),
        100.0 * wire.len() as f64 / ship.len() as f64
    );
    println!(
        "wire raw split       {baseline_bytes} baseline + {delta_bytes} delta \
         ({:.1}% / {:.1}%)",
        100.0 * baseline_bytes as f64 / wire.len().max(1) as f64,
        100.0 * delta_bytes as f64 / wire.len().max(1) as f64
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use compact_contract::{COMPACT_DOMAIN_FRAME, COMPACT_FRAME_END_PAYLOAD_LEN};

    fn test_schema() -> wire_format::Schema {
        d29::schema()
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

    fn ship_row(instance_id: u32, strength_raw: u32, definition: &str) -> Vec<u8> {
        let flags = d29::FLAG_TASK_FORCE | d29::FLAG_DEFINITION | d29::FLAG_OWNER;
        let mut row = Vec::new();
        for word in [
            instance_id,
            flags,
            10,
            3,
            strength_raw,
            0,
            0,
            0,
            1,
            0,
        ] {
            row.extend_from_slice(&word.to_le_bytes());
        }
        row.extend_from_slice(&(definition.len() as u16).to_le_bytes());
        row.extend_from_slice(definition.as_bytes());
        row
    }

    fn delete_row(instance_id: u32) -> Vec<u8> {
        let mut row = Vec::new();
        for word in [instance_id, d29::FLAG_DELETED, 0, 0, 0, 0, 0, 0, 0, 0] {
            row.extend_from_slice(&word.to_le_bytes());
        }
        row.extend_from_slice(&0u16.to_le_bytes());
        row
    }

    fn d29_payload(status_flags: u32, rows: &[Vec<u8>], deleted_rows: u32) -> Vec<u8> {
        let row_count = rows.len() as u32;
        let mut out = Vec::new();
        out.extend_from_slice(&0u64.to_le_bytes()); // elapsed_wall_ms
        out.extend_from_slice(&row_count.to_le_bytes());
        out.extend_from_slice(&row_count.to_le_bytes()); // changed_rows
        out.extend_from_slice(&status_flags.to_le_bytes());
        out.extend_from_slice(&row_count.to_le_bytes()); // observed_ship_rows
        out.extend_from_slice(&1u32.to_le_bytes()); // observed_task_force_rows
        out.extend_from_slice(&deleted_rows.to_le_bytes());
        out.extend_from_slice(&d29::FIXED_SCALE_RAW.to_le_bytes());
        out.extend_from_slice(&0u32.to_le_bytes()); // reserved0
        for row in rows {
            out.extend_from_slice(row);
        }
        out
    }

    /// Two closed hours, each carrying one d29 DATA record.
    fn good_stream() -> Vec<u8> {
        let mut s = Vec::new();
        s.extend(record(
            1,
            100,
            D29,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &d29_payload(0, &[ship_row(1, 100_000, "destroyer")], 0),
        ));
        s.extend(frame_end(2, 100));
        s.extend(record(
            3,
            101,
            D29,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &d29_payload(0, &[ship_row(1, 200_000, "destroyer")], 0),
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
    fn a_well_formed_stream_yields_its_d29_hours() {
        let scan = scan_stream(&good_stream()).expect("accepted");
        assert_eq!(scan.hours.len(), 2);
        assert_eq!(scan.hours[0].game_hour, 100);
        assert_eq!(scan.hours[1].game_hour, 101);
        assert_eq!(scan.counts.failure_records, 0);
        assert_eq!(scan.counts.non_contiguous, 0);
        // The first record replaces, the second is sparse
        // (`replace_or_sparse_after_first`).
        assert!(!scan.hours[0].sparse);
        assert!(scan.hours[1].sparse);
        // The vocabulary is complete and frozen.
        assert_eq!(scan.symbols.len(), 1);
        assert!(scan.symbols.id("destroyer").is_some());
    }

    #[test]
    fn an_unassigned_status_bit_is_refused_not_classified() {
        // `compact_contract/src/lib.rs:121`-`:123`: the failure helpers classify
        // the v1 vocabulary and are not validity predicates, so an unassigned bit
        // must be rejected BEFORE classification — otherwise it silently reads as
        // "not a failure", passes the delta-before-baseline exclusion, and the
        // hour is measured as healthy.
        for bit in 14..32u32 {
            let status = 1u32 << bit;
            // Alone, combined with a known failure bit, and combined with the
            // informational DELTA_DATA bit; with rows and without. The rejection
            // must not depend on any of it, nor on whether a baseline was seen.
            for extra in [0u32, 1, COMPACT_STATUS_DELTA_DATA] {
                for rows in [Vec::new(), vec![ship_row(1, 100_000, "destroyer")]] {
                    let mut first = Vec::new();
                    first.extend(record(
                        1,
                        100,
                        D29,
                        COMPACT_RECORD_TYPE_DOMAIN_DATA,
                        &d29_payload(status | extra, &rows, 0),
                    ));
                    first.extend(frame_end(2, 100));
                    assert!(
                        scan_stream(&first).is_err(),
                        "status bit {bit} (+{extra:#x}) with {} row(s) as the first record must \
                         be refused",
                        rows.len()
                    );
                    let mut after = Vec::new();
                    after.extend(record(
                        1,
                        100,
                        D29,
                        COMPACT_RECORD_TYPE_DOMAIN_DATA,
                        &d29_payload(0, &[ship_row(1, 100_000, "destroyer")], 0),
                    ));
                    after.extend(frame_end(2, 100));
                    after.extend(record(
                        3,
                        101,
                        D29,
                        COMPACT_RECORD_TYPE_DOMAIN_DATA,
                        &d29_payload(status | extra, &rows, 0),
                    ));
                    after.extend(frame_end(4, 101));
                    assert!(
                        scan_stream(&after).is_err(),
                        "status bit {bit} (+{extra:#x}) with {} row(s) after a baseline must be \
                         refused",
                        rows.len()
                    );
                }
            }
        }
        // And the whole known v1 mask, minus the failure bits, is still accepted:
        // the check must reject unassigned bits and nothing else. DELTA_DATA is
        // the one d29 itself reads, and after a baseline it is a sparse update.
        for informational in [1u32 << 4, 1 << 5, COMPACT_STATUS_DELTA_DATA, 1 << 11, 1 << 13] {
            let mut s = Vec::new();
            s.extend(record(
                1,
                100,
                D29,
                COMPACT_RECORD_TYPE_DOMAIN_DATA,
                &d29_payload(0, &[ship_row(1, 100_000, "destroyer")], 0),
            ));
            s.extend(frame_end(2, 100));
            s.extend(record(
                3,
                101,
                D29,
                COMPACT_RECORD_TYPE_DOMAIN_DATA,
                &d29_payload(informational, &[ship_row(1, 200_000, "destroyer")], 0),
            ));
            s.extend(frame_end(4, 101));
            let scan = scan_stream(&s)
                .unwrap_or_else(|e| panic!("informational bit {informational:#x}: {e}"));
            assert_eq!(scan.hours.len(), 2, "bit {informational:#x}");
            assert_eq!(scan.counts.failure_records, 0, "bit {informational:#x}");
            assert_eq!(scan.counts.delta_before_baseline, 0, "bit {informational:#x}");
        }
    }

    #[test]
    fn the_symbol_table_covers_every_definition_in_the_stream() {
        // §5: fully read before any mapping. A symbol that appears only in a late
        // record must still be in the table pass 2 uses.
        let mut s = Vec::new();
        s.extend(record(
            1,
            100,
            D29,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &d29_payload(0, &[ship_row(1, 1, "destroyer")], 0),
        ));
        s.extend(frame_end(2, 100));
        s.extend(record(
            3,
            101,
            D29,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &d29_payload(0, &[ship_row(2, 1, "carrier")], 0),
        ));
        s.extend(frame_end(4, 101));
        let scan = scan_stream(&s).expect("accepted");
        assert_eq!(scan.symbols.len(), 2);
        for symbol in ["destroyer", "carrier"] {
            assert!(scan.symbols.id(symbol).is_some(), "{symbol}");
        }
        // And every row of every measured hour maps against it.
        let schema = test_schema();
        let mut acc = d29::State::new();
        for hour in &scan.hours {
            d29::fold_payload(
                &schema,
                &mut acc,
                &s[hour.payload_span.clone()],
                hour.sparse,
                0,
                &scan.symbols,
            )
            .expect("pass 2 folds against the frozen table");
        }
        assert_eq!(acc.len(), 2);
    }

    #[test]
    fn a_stream_truncated_on_a_record_boundary_is_rejected() {
        let full = good_stream();
        for records in [1usize, 3] {
            let cut = boundary(&full, records);
            assert!(
                scan_stream(&full[..cut]).is_err(),
                "a stream cut after {records} record(s) leaves an hour open"
            );
        }
        // A cut exactly at a FRAME_END boundary IS a well-formed shorter stream,
        // and `eof_check` accepts it. What matters is that it reports the hours it
        // actually closed.
        for (records, expected_hours) in [(2usize, 1usize), (4, 2)] {
            let cut = boundary(&full, records);
            let scan = scan_stream(&full[..cut])
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
                scan_stream(&full[..cut]).is_err(),
                "mid-record cut at {cut}"
            );
        }
        assert!(
            scan_stream(&[]).is_err(),
            "an empty stream is not a measurement"
        );
    }

    #[test]
    fn a_header_the_canonical_parser_rejects_is_rejected_here() {
        for records in 0..4 {
            let at = boundary(&good_stream(), records);
            for (offset, patch) in [
                (0usize, vec![b'X']),               // magic
                (4, 2u16.to_le_bytes().to_vec()),   // unsupported version
                (6, 33u16.to_le_bytes().to_vec()),  // unsupported header length
            ] {
                let mut s = good_stream();
                s[at + offset..at + offset + patch.len()].copy_from_slice(&patch);
                assert!(
                    scan_stream(&s).is_err(),
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
                    scan_stream(&s).is_err(),
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
            D29,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &d29_payload(0, &[ship_row(1, 1, "destroyer")], 0),
        ));
        s.extend(frame_end(2, 999));
        assert!(scan_stream(&s).is_err());

        let mut s = Vec::new();
        s.extend(frame_end(1, 100));
        assert!(scan_stream(&s).is_err());
    }

    #[test]
    fn non_monotonic_hours_are_rejected() {
        let mut s = Vec::new();
        s.extend(record(
            1,
            101,
            D29,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &d29_payload(0, &[ship_row(1, 1, "destroyer")], 0),
        ));
        s.extend(frame_end(2, 101));
        s.extend(record(
            3,
            100,
            D29,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &d29_payload(0, &[ship_row(1, 2, "destroyer")], 0),
        ));
        s.extend(frame_end(4, 100));
        assert!(scan_stream(&s).is_err());
    }

    #[test]
    fn a_second_d29_record_in_one_hour_is_rejected() {
        // Two records in one hour would become two measurement hours built from
        // the same clock time. The rule is checked over EVERY d29 DATA record and
        // BEFORE status exclusion, so all four success/failure orderings are
        // refused — gating it on the record being usable would let a
        // success + failure pair through.
        const FAILED: u32 = 1; // COMPACT_STATUS_STATE_UNAVAILABLE
        for (first, second) in [(0u32, 0u32), (0, FAILED), (FAILED, 0), (FAILED, FAILED)] {
            let mut s = Vec::new();
            s.extend(record(
                1,
                100,
                D29,
                COMPACT_RECORD_TYPE_DOMAIN_DATA,
                &d29_payload(first, &[ship_row(1, 1, "destroyer")], 0),
            ));
            s.extend(record(
                2,
                100,
                D29,
                COMPACT_RECORD_TYPE_DOMAIN_DATA,
                &d29_payload(second, &[ship_row(1, 2, "destroyer")], 0),
            ));
            s.extend(frame_end(3, 100));
            assert!(
                scan_stream(&s).is_err(),
                "two d29 records in hour 100 with statuses {first:#x}/{second:#x}"
            );
        }
    }

    #[test]
    fn a_failure_status_record_is_excluded_rather_than_measured() {
        // Every failure bit in `COMPACT_STATUS_FAILURE_MASK`, one at a time: the
        // fold retains the accumulator and folds nothing, so the hour is not d29
        // data and must not enter either arm.
        for bit in [0u32, 1, 2, 3, 6, 7, 8, 9, 12] {
            let mut s = Vec::new();
            s.extend(record(
                1,
                100,
                D29,
                COMPACT_RECORD_TYPE_DOMAIN_DATA,
                &d29_payload(1 << bit, &[ship_row(1, 1, "destroyer")], 0),
            ));
            s.extend(frame_end(2, 100));
            s.extend(record(
                3,
                101,
                D29,
                COMPACT_RECORD_TYPE_DOMAIN_DATA,
                &d29_payload(0, &[ship_row(1, 2, "destroyer")], 0),
            ));
            s.extend(frame_end(4, 101));
            let scan = scan_stream(&s).expect("the stream itself is well-formed");
            assert_eq!(scan.counts.failure_records, 1, "bit {bit}");
            assert_eq!(scan.hours.len(), 1, "bit {bit}");
            assert_eq!(scan.hours[0].game_hour, 101, "bit {bit}");
            // The excluded record's rows were never folded, so its definition is
            // not in the vocabulary if it appears nowhere else.
            assert!(!scan.hours[0].sparse, "the surviving hour is the baseline");
        }
        // And DELTA_DATA (1<<10) is NOT a failure.
        let mut s = Vec::new();
        s.extend(record(
            1,
            100,
            D29,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &d29_payload(0, &[ship_row(1, 1, "destroyer")], 0),
        ));
        s.extend(frame_end(2, 100));
        s.extend(record(
            3,
            101,
            D29,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &d29_payload(COMPACT_STATUS_DELTA_DATA, &[ship_row(2, 1, "carrier")], 0),
        ));
        s.extend(frame_end(4, 101));
        let scan = scan_stream(&s).expect("well-formed");
        assert_eq!(scan.counts.failure_records, 0);
        assert_eq!(scan.hours.len(), 2);
        assert!(scan.hours[1].sparse);
    }

    #[test]
    fn a_failure_record_does_not_walk_its_rows() {
        // The exclusion's POSITION: `fold_ships` checks status BEFORE the row
        // walk (`world_fold.rs:14508` vs `:14515`), so a failure record with rows
        // the fold would reject is still an excluded hour, not a refused stream.
        // Reversing the order here would refuse a stream the recorder accepts.
        let mut bad_row = ship_row(1, 1, "destroyer");
        bad_row[4..8].copy_from_slice(&(1u32 << 31).to_le_bytes()); // unknown flag
        let mut s = Vec::new();
        s.extend(record(
            1,
            100,
            D29,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &d29_payload(1, &[bad_row], 0),
        ));
        s.extend(frame_end(2, 100));
        s.extend(record(
            3,
            101,
            D29,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &d29_payload(0, &[ship_row(1, 2, "destroyer")], 0),
        ));
        s.extend(frame_end(4, 101));
        let scan = scan_stream(&s).expect("the failure record is skipped, not read");
        assert_eq!(scan.counts.failure_records, 1);
        assert_eq!(scan.hours.len(), 1);

        // But a failure record with a malformed HEADER is still a refused stream:
        // the fold validates the header before it reads the status word.
        let mut bad_header = d29_payload(1, &[ship_row(1, 1, "destroyer")], 0);
        bad_header[32..36].copy_from_slice(&32_768u32.to_le_bytes()); // wrong scale
        let mut s = Vec::new();
        s.extend(record(
            1,
            100,
            D29,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &bad_header,
        ));
        s.extend(frame_end(2, 100));
        assert!(scan_stream(&s).is_err());
    }

    #[test]
    fn a_delta_before_any_baseline_is_excluded() {
        // `reject_delta_before_baseline` (`world_fold.rs:14511`): the fold marks
        // the domain-hour unknown and folds nothing, so the hour is not data.
        let mut s = Vec::new();
        s.extend(record(
            1,
            100,
            D29,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &d29_payload(COMPACT_STATUS_DELTA_DATA, &[ship_row(1, 1, "destroyer")], 0),
        ));
        s.extend(frame_end(2, 100));
        s.extend(record(
            3,
            101,
            D29,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &d29_payload(0, &[ship_row(2, 1, "carrier")], 0),
        ));
        s.extend(frame_end(4, 101));
        let scan = scan_stream(&s).expect("well-formed");
        assert_eq!(scan.counts.delta_before_baseline, 1);
        assert_eq!(scan.hours.len(), 1);
        assert_eq!(scan.hours[0].game_hour, 101);
        assert!(
            !scan.hours[0].sparse,
            "the first record that actually folds is the one that clears"
        );
    }

    #[test]
    fn tombstones_and_rowless_records_are_counted_and_folded() {
        let mut s = Vec::new();
        s.extend(record(
            1,
            100,
            D29,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &d29_payload(0, &[ship_row(1, 1, "destroyer"), ship_row(2, 1, "carrier")], 0),
        ));
        s.extend(frame_end(2, 100));
        s.extend(record(
            3,
            101,
            D29,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &d29_payload(0, &[delete_row(2)], 1),
        ));
        s.extend(frame_end(4, 101));
        s.extend(record(
            5,
            102,
            D29,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &d29_payload(0, &[], 0),
        ));
        s.extend(frame_end(6, 102));
        let scan = scan_stream(&s).expect("well-formed");
        assert_eq!(scan.hours.len(), 3);
        assert_eq!(scan.counts.delete_rows, 1);
        assert_eq!(scan.counts.rowless_records, 1);
        assert_eq!(scan.counts.rows_folded, 3);

        // And the fold produces the states the differ turns into one tombstone.
        let schema = test_schema();
        let collection = &schema.collections[0];
        let mut acc = d29::State::new();
        let mut sizes = Vec::new();
        let mut tombstones = 0usize;
        for hour in &scan.hours {
            let prior = acc.ships().clone();
            d29::fold_payload(
                &schema,
                &mut acc,
                &s[hour.payload_span.clone()],
                hour.sparse,
                0,
                &scan.symbols,
            )
            .expect("folds");
            tombstones += diff(collection, &prior, acc.ships()).tombstones.len();
            sizes.push(acc.len());
        }
        assert_eq!(sizes, vec![2, 1, 1]);
        assert_eq!(tombstones, 1);
    }

    #[test]
    fn a_stream_with_no_d29_records_is_refused_not_reported_as_zero() {
        let mut s = Vec::new();
        s.extend(record(1, 100, 20, COMPACT_RECORD_TYPE_DOMAIN_DATA, &[0u8; 8]));
        s.extend(frame_end(2, 100));
        assert!(scan_stream(&s).is_err());
    }

    #[test]
    fn a_malformed_d29_payload_is_refused_not_skipped() {
        // A short or inconsistent payload is a corrupt stream, not an hour to
        // drop quietly.
        let mut payload = d29_payload(0, &[ship_row(1, 1, "destroyer")], 0);
        payload.truncate(payload.len() - 1);
        let mut s = Vec::new();
        s.extend(record(1, 100, D29, COMPACT_RECORD_TYPE_DOMAIN_DATA, &payload));
        s.extend(frame_end(2, 100));
        assert!(scan_stream(&s).is_err());

        // A row the FOLD rejects must make the SCAN fail, not be accepted and
        // then unwind while measuring. The whole row-rejection space, not one
        // sampled case.
        let mut invalid: Vec<Vec<u8>> = Vec::new();
        // an unknown flag bit (`world_fold.rs:14519`)
        let mut unknown_flag = ship_row(1, 1, "destroyer");
        unknown_flag[4..8].copy_from_slice(&(1u32 << 20).to_le_bytes());
        invalid.push(d29_payload(0, &[unknown_flag], 0));
        // a nonzero row reserved word (`:14533`)
        let mut reserved = ship_row(1, 1, "destroyer");
        reserved[36..40].copy_from_slice(&1u32.to_le_bytes());
        invalid.push(d29_payload(0, &[reserved], 0));
        // the definition flag with an empty string (`:14539`)
        invalid.push(d29_payload(0, &[ship_row(1, 1, "")], 0));
        // a definition string without the flag (`:14542`)
        let mut no_flag = ship_row(1, 1, "destroyer");
        let flags = d29::FLAG_TASK_FORCE | d29::FLAG_OWNER;
        no_flag[4..8].copy_from_slice(&flags.to_le_bytes());
        invalid.push(d29_payload(0, &[no_flag], 0));
        // trailing bytes (`:14547`)
        let mut trailing = d29_payload(0, &[ship_row(1, 1, "destroyer")], 0);
        trailing.push(0);
        invalid.push(trailing);
        // a wrong scale_raw (`:14504`)
        let mut scale = d29_payload(0, &[ship_row(1, 1, "destroyer")], 0);
        scale[32..36].copy_from_slice(&1u32.to_le_bytes());
        invalid.push(scale);
        for (i, payload) in invalid.into_iter().enumerate() {
            let mut s = Vec::new();
            s.extend(record(1, 100, D29, COMPACT_RECORD_TYPE_DOMAIN_DATA, &payload));
            s.extend(frame_end(2, 100));
            assert!(
                scan_stream(&s).is_err(),
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
            if len >= 4 && state & 1 == 0 {
                bytes[..4].copy_from_slice(b"H4CR");
            }
            let _ = scan_stream(&bytes);
        }
        // And corruption of a genuinely well-formed stream, byte by byte.
        let good = good_stream();
        for i in 0..good.len() {
            let mut s = good.clone();
            s[i] ^= 0xFF;
            let _ = scan_stream(&s);
        }
    }
}
