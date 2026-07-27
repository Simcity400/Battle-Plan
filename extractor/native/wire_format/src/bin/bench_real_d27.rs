//! Measure the wire format against the shipping compact stream on REAL d27
//! `army_orders` dynamics.
//!
//! Input: a real `collector_stream.bin` from a d27-scheduled capture
//! (`runs/batch2/deepwar_t1`). The stream's own d27 DATA records are the
//! shipping format's cost for the domain: a 40-byte header, a fixed 48-byte
//! order row per serialized order and a fixed 8-byte target row per covered
//! state, one full first frame followed by sparse upserts, exactly what the
//! recorder stores today. This tool replays those records into the per-hour
//! accumulator `world_fold::fold_army_orders` builds and re-encodes every hour
//! with this crate's grammar: a BASELINE at each 48-hour block start,
//! field-granular stored-minimal DELTAs between.
//!
//! **Two collections, and the second one carries an ordinal.** d27's payload
//! addresses an order's target states as an index range into a shared target
//! array; those indices are transport, so the harness flattens them into
//! `(owner_kind, owner_id, generation, state_id)` keyed rows
//! (`wire_format::d27`). The fold stores the targets as an ordered `Vec` whose
//! order the producer refuses to sort away
//! (`collector/src/attach_log.rs:19349`-`:19351`), so each child row also
//! carries its dense position within its order's range — otherwise the keyed
//! set would reproduce the set and silently drop the sequence. An order that
//! loses a target therefore produces a **tombstone** in the target collection,
//! and a deleted order row produces tombstones in both. Nothing constructs a
//! tombstone by hand: `body::diff` derives them from the two states, which is
//! what makes them minimal, and `body::encode_delta` refuses a tombstone for a
//! key the reader's reconstructed prior does not hold.
//!
//! **Nothing here re-derives a contract it can call.** Payload, row and frame
//! decoding go through `wire_format::d27`'s public boundary — including the
//! frame-level fold, `d27::fold_payload`, because one d27 record is not one
//! state; H4CR framing, record semantics and sequence continuity go through
//! `compact_contract`'s own parser and stateful validator; the failure and
//! expected-partial classification goes through `compact_contract`'s status
//! predicates rather than a copied bit mask. A harness that reimplements any of
//! them drifts from the recorder without anything failing.
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
//!   hour left open, at least one FRAME_END, and the final record is a FRAME_END;
//! - and every rule `fold_army_orders` applies to a d27 payload, by folding each
//!   one during the scan.
//!
//! That is a documented **subset** of the production boundary contract:
//! `recorder_host::stream_fold::verify_stream_logical_boundary` (`:537`) is the
//! full authority, and it additionally checks FRAME_END seal fields and
//! DOMAIN_PERF domain accounting. This crate cannot depend on `recorder_host`
//! (it pulls in every capture lane), so the subset is enforced here and the full
//! check remains the recorder's.
//!
//! **Which hours are measured.** `fold_army_orders` has four paths on which a
//! d27 record folds nothing and the retained accumulator stands, and their
//! ORDER and POSITION are the fold's, not a copy of another domain's:
//!
//! 1. any failure status, by `compact_contract`'s own tolerance-aware predicate
//!    (`handle_compact_failure`, `world_fold.rs:2863`, called at `:14171`) —
//!    d27 checks this FIRST, unlike d26;
//! 2. a tolerated-partial record before any complete baseline
//!    (`reject_expected_partial_before_baseline`, `:2924`, called at `:14174`);
//! 3. a delta before any baseline (`reject_delta_before_baseline`, `:2895`,
//!    called at `:14181`);
//! 4. **d27's own withhold** (`:14301`-`:14310`): a tolerated-partial record —
//!    d27 is one of exactly four domains with an expected-partial tolerance
//!    (`compact_contract/src/lib.rs:141`) — marks the domain-hour unknown and
//!    returns `Ok` **without folding**, because the omitted order identities are
//!    not serialized. And it sits **after** the order-row and target-row loops
//!    and before the accumulator write, so the recorder still rejects a
//!    malformed tolerated-partial frame. The harness reproduces that position
//!    exactly: it runs the full `d27::fold_payload` validation into a throwaway
//!    accumulator, so a corrupt withheld frame is still detected — it becomes a
//!    dropped record (the fifth outcome below) rather than a withhold, because
//!    that is what the recorder does with it — and then discards the result and
//!    excludes the hour.
//!
//! Those hours are excluded from BOTH arms — they are not d27 data. Hour
//! uniqueness is enforced over EVERY d27 DATA record, of every status, BEFORE
//! any of those exclusions, or a success+failure pair in one hour slips through.
//! A stream cut exactly at a FRAME_END boundary is a well-formed shorter stream
//! and is accepted as such.
//!
//! **A fifth outcome, and this capture needs it: a record the fold `Err`s on.**
//! The recipe's default is that a fold `Err` invalidates the run. That is not
//! what the recorder does here. Domain 27 is one of exactly four domains with a
//! **dedicated** fold-error arm (`world_fold.rs:4997`-`:5014`, matching
//! `DIVISION_EQUIPMENT | ARMY_ORDER | OOB_COMMAND | ARMY_BATTLEPLANS_DEPTH`),
//! and it does not reach the catch-all `note` arm at `:5016`. That arm calls
//! `mark_fold_domain_unknown(domain_id, "malformed_payload", "... accumulator
//! retained")`, which — per `:2835`-`:2845` — inserts the domain into
//! `fold_domains_seen_in_stream` and `fold_domains_unknown_this_hour`, records
//! the problem and notes it, and touches neither the accumulator nor
//! `delta_baselines_seen`. `mark_domain` is never called, so the hour's snapshot
//! publishes `battle_plan_orders: null` (`:7918`-`:7923`) and carries no d27
//! state at all. That is the *same* treatment the other three fold-nothing paths
//! get — "unknown this hour, accumulator retained" — which is why the hour
//! belongs in the exclusion set rather than in either arm. Mirroring the
//! recorder therefore means counting the record as
//! dropped and excluding its hour from both arms, not refusing the stream; a
//! stream rejection would produce no measurement of a fold the recorder does
//! run. **`deepwar_t1` exercises this**: when that capture was recorded the d27
//! emitter handed generation 0 to the first order it ever sampled —
//! `compact_army_order_delta_rows` read `ArmyOrderTracker::next_generation`
//! straight from its `Default` zero — while `fold_army_orders` rejects
//! `generation == 0` (`world_fold.rs:14226`), so the capture's **first d27
//! record — its only full 794-row baseline —** is unfoldable. Exactly one row in
//! the whole capture carries generation 0. The collector now normalises that
//! counter with `.max(1)` the way the D21 land-combat and D49/D50 naval trackers
//! already did, so no new capture can carry such a row; `deepwar_t1` still does,
//! and the count and every reason are printed, never absorbed.
//!
//! **`fold_army_orders` is the only writer of the d27 accumulator.**
//! `self.army_orders` is touched at exactly four places in `world_fold.rs` — its
//! declaration (`:1696`), its initialisation (`:2394`) and the
//! clear/remove/insert inside this fold (`:14313`, `:14322`, `:14326`). The one
//! other mention (`:7927`) is a read-only projection. (`army_orders_group_manager`
//! is d54's field, not this accumulator.) So there is no cross-domain mutation
//! for this harness to model, prove inert, or assert inert per capture.
//!
//! Outputs, for the compression step (`compress_arms.py` beside the run):
//!   ship_d27.bin   — the d27 DATA records verbatim (32-byte H4CR header +
//!                    payload), concatenated in stream order
//!   wire_new.bin   — the wire-format bodies, concatenated in hour order
//!   *.offsets      — hour-boundary byte offsets, one per line, for exact
//!                    block-span slicing
//!
//! It also prints the real churn surface twice — cadence-independent over
//! measured-hour transitions, and along the delta path — because one
//! denominator cannot answer both questions.

use std::io::Write;
use std::ops::Range;

use compact_contract::campaign_identity::COMPACT_RECORD_TYPE_STREAM_HEADER;
use compact_contract::stream_validator::StatefulStreamValidator;
use compact_contract::{
    compact_status_failure_flags_after_expected_partial_tolerance,
    compact_status_unknown_flags_v1, expected_partial_tolerance_applies,
    parse_compact_record_header, COMPACT_RECORD_HEADER_LEN, COMPACT_RECORD_TYPE_DOMAIN_DATA,
    COMPACT_RECORD_TYPE_FRAME_END, COMPACT_STATUS_DELTA_DATA,
};
use wire_format::body::{
    apply, decode_baseline, decode_delta, diff, encode_baseline, encode_delta, KeyedDelta,
    KeyedState,
};
use wire_format::d27;
use wire_format::Schema;

const D27: u16 = 27;
const BLOCK_HOURS: usize = 48;
const COLLECTIONS: usize = 2;

/// One usable d27 hour: its game hour, its byte span in the source stream (the
/// shipping arm is those bytes verbatim, header included), the payload span the
/// encode pass re-folds from, and whether that fold replaces or upserts.
///
/// The state is deliberately NOT retained here: the scan still folds every hour
/// in full, so every way a row can be rejected is exercised in the pass that
/// either accepts the stream or refuses it; the encode pass then re-folds
/// through the identical code path, which cannot fail where the scan did not.
struct D27Hour {
    game_hour: i32,
    ship_span: Range<usize>,
    payload_span: Range<usize>,
    /// `d27::frame_is_sparse` for this frame, decided in stream order.
    sparse: bool,
}

#[derive(Default)]
struct ScanCounts {
    /// d27 records whose payload status marks the domain-hour failed.
    failure_records: usize,
    /// Tolerated-partial records that arrived before a complete baseline.
    partial_before_baseline: usize,
    /// Delta records that arrived before a complete baseline.
    delta_before_baseline: usize,
    /// Tolerated-partial records the fold validates and then withholds
    /// (`world_fold.rs:14301`) — d27's own fold-nothing path.
    withheld_partial: usize,
    /// Records `fold_army_orders` returns `Err` for. **Not a stream rejection**:
    /// domain 27 has a dedicated fold-error arm (`world_fold.rs:4997`-`:5014`)
    /// that marks the domain-hour unknown with `"malformed_payload"` and
    /// "accumulator retained", leaving the accumulator, `delta_baselines_seen`
    /// and the domain's freshness flag all untouched — so the hour publishes no
    /// d27 state and belongs in neither arm. Counted and printed loudly rather
    /// than absorbed; see `reject_reasons`.
    rejected_records: usize,
    /// `(game_hour, reason)` for each rejected record, so the number above is
    /// never a bare count.
    reject_reasons: Vec<(i32, String)>,
    /// Measured hours that do not follow the previous measured one by one.
    non_contiguous: usize,
    /// Measured hours whose frame replaced the accumulator rather than upserting.
    replace_frames: usize,
}

struct StreamScan {
    hours: Vec<D27Hour>,
    counts: ScanCounts,
}

/// Validate the whole stream against the canonical contract and fold every
/// usable d27 DATA record. Every rejection path is an `Err`, never a silently
/// shorter measurement and never a panic.
fn scan_stream(stream: &[u8], schema: &Schema) -> Result<StreamScan, String> {
    let orders = &schema.collections[0];
    let targets = &schema.collections[1];

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

    let mut hours: Vec<D27Hour> = Vec::new();
    let mut counts = ScanCounts::default();

    let mut open_hour: Option<i32> = None;
    let mut last_closed_hour: Option<i32> = None;
    let mut last_record_type: Option<u16> = None;
    // Hour of the last d27 DATA record of ANY status — the uniqueness rule.
    let mut last_d27_hour: Option<i32> = None;
    // Hour of the last d27 record that entered the measurement — the contiguity
    // report, which a skipped excluded hour legitimately breaks.
    let mut last_measured_hour: Option<i32> = None;
    // `delta_baselines_seen` for d27 (`world_fold.rs:2957`, reached at `:14311`).
    let mut baseline_seen = false;
    let mut acc = d27::State::new();

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

        if header.domain_id == D27 && header.record_type == COMPACT_RECORD_TYPE_DOMAIN_DATA {
            // The header words the fold's own classification reads, through
            // d27's boundary rather than a second copy of the header layout.
            // `peek_header` applies only the 40-byte minimum, which is exactly
            // the fold's own call order: `need(payload, HEADER_LEN)` comes
            // before the status guards and the payload-length check comes after
            // them (`world_fold.rs:14162`-`:14192`), so a failure-status record
            // with a body length the fold never reaches is still classified.
            //
            // A payload too short even for the header is the fold's own
            // `need(payload, ARMY_ORDER_HEADER_LEN)` failing at `:14162`, which
            // is a fold `Err` — the recorder notes it and continues — so it is
            // a rejected record here too, not a refused stream. The hour
            // uniqueness rule below still sees it, because it is a rule about
            // the measurement and not about the fold.
            let short_header = d27::peek_header(payload).err();

            // One d27 DATA record per game hour, checked over EVERY d27 record
            // and BEFORE any status-based exclusion. A second record for the
            // same hour would become a second measurement hour built from the
            // same clock time — and gating this on the record being usable would
            // let a success/failure pair through the rule entirely.
            if let Some(previous) = last_d27_hour {
                if header.game_hour_raw <= previous {
                    return Err(format!(
                        "d27 DATA at byte {pos} carries hour {} after hour {previous}",
                        header.game_hour_raw
                    ));
                }
            }
            last_d27_hour = Some(header.game_hour_raw);

            if let Some(reason) = short_header {
                counts.rejected_records += 1;
                counts.reject_reasons.push((header.game_hour_raw, reason));
                pos = record_end;
                continue;
            }
            let d27_header = d27::peek_header(payload).expect("checked above");

            // VALIDITY BEFORE CLASSIFICATION. Everything below that reads the
            // status word — the expected-partial tolerance, the DELTA_DATA read,
            // the failure predicate — classifies the v1 vocabulary; none of it is
            // a validity predicate. `compact_contract/src/lib.rs:121`-`:123` says
            // so in as many words: "Callers handling untrusted status words must
            // first reject a non-zero `compact_status_unknown_flags_v1` result;
            // otherwise an unassigned bit has no defined classification." Bits
            // 14..=31 are unassigned (`status_flags_base.rs:21`-`:24`), and an
            // unassigned bit "is not evidence of a future informational status".
            //
            // Without this, a record with `status_flags = 1 << 14` reads as "not
            // a failure" — the failure helper masks with the *known* failure set
            // and returns zero for a bit it does not know — and the hour is
            // measured as healthy. No capture carries an unassigned bit, so no
            // measured number moves.
            let unknown_status = compact_status_unknown_flags_v1(d27_header.status_flags);
            if unknown_status != 0 {
                return Err(format!(
                    "d27 DATA at byte {pos}: hour {} carries unassigned v1 status bits \
                     {unknown_status:#x}; an unassigned bit has no defined classification",
                    header.game_hour_raw
                ));
            }

            // The fold's classification, from `compact_contract`'s predicates —
            // never a hand-copied mask literal, so it cannot drift from the
            // recorder's own tolerance table.
            let tolerance_applies =
                expected_partial_tolerance_applies(D27, d27_header.status_flags).is_some();
            let delta = d27_header.status_flags & COMPACT_STATUS_DELTA_DATA != 0;

            // The fold's three early fold-nothing guards, in the fold's own
            // order (`world_fold.rs:14171`-`:14183`): failure FIRST.
            if compact_status_failure_flags_after_expected_partial_tolerance(
                D27,
                d27_header.status_flags,
            ) != 0
            {
                counts.failure_records += 1;
            } else if tolerance_applies && !baseline_seen {
                counts.partial_before_baseline += 1;
            } else if delta && !baseline_seen {
                counts.delta_before_baseline += 1;
            } else if tolerance_applies {
                // d27's withhold (`:14301`). It sits AFTER the order-row and
                // target-row loops, so the frame is still fully validated — into
                // a throwaway accumulator, since its effect is discarded. It
                // also does not reach `replace_or_sparse_after_first`, so it
                // never marks a baseline seen.
                let mut discarded = d27::State::new();
                match d27::fold_payload(&mut discarded, payload, true, orders, targets) {
                    Ok(_) => counts.withheld_partial += 1,
                    // The withhold sits after the row loops, so a malformed
                    // tolerated-partial frame errors in the fold — and the
                    // recorder notes that error and continues, exactly as for
                    // any other d27 fold error. Either way the hour is not d27
                    // data; the two outcomes are counted apart because they are
                    // different facts about the capture.
                    Err(reason) => {
                        counts.rejected_records += 1;
                        counts.reject_reasons.push((header.game_hour_raw, reason));
                    }
                }
            } else {
                let sparse = d27::frame_is_sparse(delta, baseline_seen);
                // The accumulator is untouched on `Err` — `fold_army_orders`
                // builds its whole row picture before its first `clear`
                // (`world_fold.rs:14312`) and `fold_payload` mirrors that — and
                // so is `delta_baselines_seen`, because
                // `replace_or_sparse_after_first` sits at `:14311`, *after* the
                // row loops. So a rejected record leaves the next frame facing
                // exactly the state it would have faced anyway.
                match d27::fold_payload(&mut acc, payload, sparse, orders, targets) {
                    Err(reason) => {
                        counts.rejected_records += 1;
                        counts.reject_reasons.push((header.game_hour_raw, reason));
                    }
                    Ok(_) => {
                        if let Some(previous) = last_measured_hour {
                            if header.game_hour_raw != previous + 1 {
                                counts.non_contiguous += 1;
                            }
                        }
                        last_measured_hour = Some(header.game_hour_raw);
                        if !sparse {
                            counts.replace_frames += 1;
                            // `replace_or_sparse_after_first` inserts into
                            // `delta_baselines_seen` exactly when it returned
                            // `false`, and it is only reached on a frame the
                            // fold accepts.
                            baseline_seen = true;
                        }
                        hours.push(D27Hour {
                            game_hour: header.game_hour_raw,
                            ship_span: pos..record_end,
                            payload_span: payload_start..record_end,
                            sparse,
                        });
                    }
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
        return Err("stream carries no usable d27 DATA records".to_string());
    }

    Ok(StreamScan { hours, counts })
}

fn main() {
    let mut args = std::env::args().skip(1);
    let input = args
        .next()
        .expect("usage: bench_real_d27 <collector_stream.bin> <out dir>");
    let out_dir = args
        .next()
        .expect("usage: bench_real_d27 <collector_stream.bin> <out dir>");
    let stream = std::fs::read(&input).expect("read stream");
    let schema = d27::schema();

    // ---- Pass 1: validate the WHOLE stream and fold its d27 DATA records. ---
    // No artifact is created before this succeeds, so a refused stream leaves no
    // partial measurement behind to be mistaken for a result.
    let scan = match scan_stream(&stream, &schema) {
        Ok(scan) => scan,
        Err(reason) => {
            eprintln!("{input}: {reason}");
            std::process::exit(1);
        }
    };

    // ---- Pass 2: re-fold the accepted hours and encode both arms. -----------
    // The fold runs through the identical `d27::fold_payload` the scan proved
    // every hour against, so nothing here can fail on stream content; every
    // assertion below is about the codec.
    let mut ship = Vec::new();
    let mut ship_offsets = vec![0usize];
    let mut wire = Vec::new();
    let mut wire_offsets = vec![0usize];

    let mut acc = d27::State::new();
    let mut prior: Option<Vec<KeyedState>> = None;
    let mut reader: Option<Vec<KeyedState>> = None;
    let mut hours_seen = 0usize;
    let mut baselines = 0usize;
    let mut empty_hours = 0usize;
    let mut delta_hours_with_change = 0usize;
    // Delta-path churn: over the hours that actually emitted a delta.
    let mut changed_rows = [0usize; COLLECTIONS];
    let mut tombstones = [0usize; COLLECTIONS];
    // Cadence-independent churn: over every measured-hour transition, taken
    // BEFORE the block decision, so a change landing on a block boundary still
    // counts. `transitions` is `hours_seen - 1`, and the two denominators are
    // different numbers on purpose.
    let mut transitions = 0usize;
    let mut ci_changed_rows = [0usize; COLLECTIONS];
    let mut ci_tombstones = [0usize; COLLECTIONS];
    let mut ci_changed_hours = 0usize;
    let mut rows_total = [0usize; COLLECTIONS];
    let mut baseline_bytes = 0usize;
    let mut delta_bytes = 0usize;
    // Coverage: how much of the target-state path this capture exercises.
    let mut hours_with_targets = 0usize;
    let mut nonascending_target_runs = 0usize;

    for hour in &scan.hours {
        // Shipping arm: the record's exact bytes, header included.
        ship.extend_from_slice(&stream[hour.ship_span.clone()]);
        ship_offsets.push(ship.len());

        let payload = &stream[hour.payload_span.clone()];
        if let Err(reason) = d27::fold_payload(
            &mut acc,
            payload,
            hour.sparse,
            &schema.collections[0],
            &schema.collections[1],
        ) {
            eprintln!("{input}: game hour {} re-fold: {reason}", hour.game_hour);
            std::process::exit(1);
        }
        // The ordinal's coverage question: is the fold's stored order ever
        // something the keyed set's own key order would not reproduce? Counted
        // per order-hour, and reported rather than relied on.
        let mut any_targets = false;
        for entry in acc.orders.values() {
            if !entry.targets.is_empty() {
                any_targets = true;
            }
            if entry.targets.windows(2).any(|w| w[0] > w[1]) {
                nonascending_target_runs += 1;
            }
        }
        if any_targets {
            hours_with_targets += 1;
        }

        let states = acc.keyed_states();
        for i in 0..COLLECTIONS {
            rows_total[i] += states[i].len();
        }

        // Cadence-independent churn, decided before the block shape is.
        if let Some(previous) = prior.as_ref() {
            transitions += 1;
            let mut moved = false;
            for (i, collection) in schema.collections.iter().enumerate() {
                let d = diff(collection, &previous[i], &states[i]);
                ci_changed_rows[i] += d.changed.len();
                ci_tombstones[i] += d.tombstones.len();
                if !d.is_empty() {
                    moved = true;
                }
            }
            if moved {
                ci_changed_hours += 1;
            }
        }

        // New arm: BASELINE at each block start, stored-minimal DELTA between.
        let hour_start = wire.len();
        let block_start = hours_seen % BLOCK_HOURS == 0;
        let mut emitted = true;
        if block_start || prior.is_none() {
            encode_baseline(&schema, &states, &mut wire).expect("baseline encodes");
            baselines += 1;
            baseline_bytes += wire.len() - hour_start;
        } else {
            let previous = prior.as_ref().expect("prior exists");
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
                empty_hours += 1;
                emitted = false;
            } else {
                delta_hours_with_change += 1;
                for (i, delta) in deltas.iter().enumerate() {
                    if let Some(d) = delta {
                        changed_rows[i] += d.changed.len();
                        tombstones[i] += d.tombstones.len();
                    }
                }
                encode_delta(&schema, previous, &deltas, &mut wire).expect("delta encodes");
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
            if block_start || reader.is_none() {
                let (decoded, consumed) =
                    decode_baseline(&schema, hour_bytes, 0).expect("baseline decodes");
                assert_eq!(consumed, hour_bytes.len(), "baseline consumed exactly");
                reader = Some(decoded);
            } else {
                let mut reader_state = reader.take().expect("reader exists");
                let (deltas, consumed) =
                    decode_delta(&schema, &reader_state, hour_bytes, 0).expect("delta decodes");
                assert_eq!(consumed, hour_bytes.len(), "delta consumed exactly");
                for (i, delta) in deltas.iter().enumerate() {
                    if let Some(d) = delta {
                        apply(&mut reader_state[i], d);
                    }
                }
                reader = Some(reader_state);
            }
        }
        let replayed = reader.as_ref().expect("reader exists");
        for (i, collection) in schema.collections.iter().enumerate() {
            assert_eq!(
                replayed[i].len(),
                states[i].len(),
                "game hour {}: collection {} row count",
                hour.game_hour,
                collection.collection_id
            );
            assert!(
                diff(collection, &replayed[i], &states[i]).is_empty(),
                "game hour {}: collection {} reader state is not stored-equal to the fold",
                hour.game_hour,
                collection.collection_id
            );
        }

        prior = Some(states);
        hours_seen += 1;
    }

    std::fs::create_dir_all(&out_dir).expect("out dir");
    for (name, bytes) in [("ship_d27.bin", &ship), ("wire_new.bin", &wire)] {
        let mut f = std::fs::File::create(std::path::Path::new(&out_dir).join(name)).unwrap();
        f.write_all(bytes).unwrap();
    }
    for (name, offsets) in [
        ("ship_d27.offsets", &ship_offsets),
        ("wire_new.offsets", &wire_offsets),
    ] {
        let mut f = std::fs::File::create(std::path::Path::new(&out_dir).join(name)).unwrap();
        for o in offsets {
            writeln!(f, "{o}").unwrap();
        }
    }

    let c = &scan.counts;
    println!(
        "hours                {hours_seen} ({} non-contiguous steps)",
        c.non_contiguous
    );
    println!(
        "excluded hours       {} failure-status, {} partial-before-baseline, {} delta-before-baseline, {} withheld-partial",
        c.failure_records, c.partial_before_baseline, c.delta_before_baseline, c.withheld_partial
    );
    println!(
        "rejected records     {} (fold Err; the recorder marks the domain-hour \
         unknown and retains the accumulator, world_fold.rs:4997 — neither arm)",
        c.rejected_records
    );
    for (hour, reason) in &c.reject_reasons {
        println!("  hour {hour}: {reason}");
    }
    println!(
        "replace frames       {} (the fold clears only on the first non-delta frame)",
        c.replace_frames
    );
    println!(
        "rows avg             {:.1} orders, {:.1} target states",
        rows_total[0] as f64 / hours_seen as f64,
        rows_total[1] as f64 / hours_seen as f64
    );
    println!("baselines            {baselines} (every {BLOCK_HOURS} h)");
    println!("delta hours          {delta_hours_with_change}   empty hours {empty_hours}");

    // Churn, twice. The denominators differ on purpose: dividing an idle
    // domain's churn by emitted-delta hours only makes it look busier.
    let ci = transitions.max(1) as f64;
    println!(
        "churn cadence-indep  {transitions} measured-hour transitions ({ci_changed_hours} with any change)"
    );
    println!(
        "  orders             {:.2} changed rows/transition, {:.3} tombstones/transition",
        ci_changed_rows[0] as f64 / ci,
        ci_tombstones[0] as f64 / ci
    );
    println!(
        "  target states      {:.2} changed rows/transition, {:.3} tombstones/transition",
        ci_changed_rows[1] as f64 / ci,
        ci_tombstones[1] as f64 / ci
    );
    let denominator = delta_hours_with_change.max(1) as f64;
    println!("churn delta-path     over {delta_hours_with_change} emitted-delta hours");
    println!(
        "  orders             {:.2} changed rows/hour, {:.3} tombstones/hour (stored precision)",
        changed_rows[0] as f64 / denominator,
        tombstones[0] as f64 / denominator
    );
    println!(
        "  target states      {:.2} changed rows/hour, {:.3} tombstones/hour (stored precision)",
        changed_rows[1] as f64 / denominator,
        tombstones[1] as f64 / denominator
    );
    println!(
        "target coverage      {hours_with_targets}/{hours_seen} hours hold any target state; \
         {nonascending_target_runs} order-hours whose stored order is not ascending by state id"
    );
    println!("ship_d27.bin         {} bytes", ship.len());
    println!(
        "wire_new.bin         {} bytes ({:.1}%)  = {} baseline + {} delta",
        wire.len(),
        100.0 * wire.len() as f64 / ship.len() as f64,
        baseline_bytes,
        delta_bytes
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use compact_contract::{
        COMPACT_DOMAIN_FRAME, COMPACT_FRAME_END_PAYLOAD_LEN, COMPACT_STATUS_PARTIAL_DATA,
    };

    fn schema() -> Schema {
        d27::schema()
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

    /// A one-order d27 DATA payload: 40-byte header, one 48-byte area-defence
    /// row with `state_ids.len()` target rows behind it.
    fn d27_payload(status_flags: u32, owner_id: u32, state_ids: &[u32]) -> Vec<u8> {
        let mut p = Vec::new();
        p.extend_from_slice(&0u64.to_le_bytes()); // elapsed_wall_ms
        p.extend_from_slice(&1u32.to_le_bytes()); // row_count
        p.extend_from_slice(&1u32.to_le_bytes()); // changed_rows
        p.extend_from_slice(&status_flags.to_le_bytes());
        p.extend_from_slice(&1u32.to_le_bytes()); // root_rows
        p.extend_from_slice(&1u32.to_le_bytes()); // observed_army_rows
        p.extend_from_slice(&1u32.to_le_bytes()); // observed_order_rows
        p.extend_from_slice(&(state_ids.len() as u32).to_le_bytes()); // target_state_count
        p.extend_from_slice(&0u32.to_le_bytes()); // deleted_rows
        assert_eq!(p.len(), d27::HEADER_LEN);
        p.extend_from_slice(&owner_id.to_le_bytes()); // +0  owner_id
        p.extend_from_slice(&1u64.to_le_bytes()); // +4  generation
        p.extend_from_slice(
            &(d27::FLAG_TARGET_STATES | d27::FLAG_TARGET_STATE_COUNT).to_le_bytes(),
        ); // +12 flags
        p.extend_from_slice(&0u32.to_le_bytes()); // +16 slot_index
        p.extend_from_slice(&5i32.to_le_bytes()); // +20 order_type (AREA_DEFENSE)
        p.extend_from_slice(&0u32.to_le_bytes()); // +24 target_first
        p.extend_from_slice(&(state_ids.len() as u32).to_le_bytes()); // +28 target_count
        p.extend_from_slice(&(state_ids.len() as u32).to_le_bytes()); // +32 declared count
        p.extend_from_slice(&d27::TARGET_STATUS_EXACT.to_le_bytes()); // +36 status
        p.extend_from_slice(&d27::OWNER_KIND_ARMY.to_le_bytes()); // +40 owner_kind
        p.extend_from_slice(&0u32.to_le_bytes()); // +44 reserved
        assert_eq!(p.len(), d27::HEADER_LEN + d27::ROW_LEN);
        for id in state_ids {
            p.extend_from_slice(&id.to_le_bytes());
            p.extend_from_slice(&0u32.to_le_bytes());
        }
        p
    }

    /// Two closed hours, each carrying one d27 DATA record.
    fn good_stream() -> Vec<u8> {
        let mut s = Vec::new();
        s.extend(record(
            1,
            100,
            D27,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &d27_payload(0, 7, &[900, 431]),
        ));
        s.extend(frame_end(2, 100));
        s.extend(record(
            3,
            101,
            D27,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &d27_payload(COMPACT_STATUS_DELTA_DATA, 7, &[900, 12]),
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
    fn a_well_formed_stream_yields_its_d27_hours() {
        let scan = scan_stream(&good_stream(), &schema()).expect("accepted");
        assert_eq!(scan.hours.len(), 2);
        assert_eq!(scan.hours[0].game_hour, 100);
        assert_eq!(scan.hours[1].game_hour, 101);
        assert!(!scan.hours[0].sparse, "the first non-delta frame replaces");
        assert!(scan.hours[1].sparse, "everything after it upserts");
        assert_eq!(scan.counts.replace_frames, 1);
        assert_eq!(scan.counts.failure_records, 0);
        assert_eq!(scan.counts.withheld_partial, 0);
        assert_eq!(scan.counts.non_contiguous, 0);
    }

    #[test]
    fn an_unassigned_status_bit_is_refused_not_classified() {
        // `compact_contract/src/lib.rs:121`-`:123`: the failure helper and the
        // expected-partial tolerance classify the v1 vocabulary and are not
        // validity predicates, so an unassigned bit must be rejected BEFORE
        // classification — otherwise it silently reads as "not a failure", falls
        // through the three fold-nothing guards, and the hour is measured as
        // healthy.
        for bit in 14..32u32 {
            let status = 1u32 << bit;
            for extra in [0u32, 1, COMPACT_STATUS_DELTA_DATA, COMPACT_STATUS_PARTIAL_DATA] {
                let mut first = Vec::new();
                first.extend(record(
                    1,
                    100,
                    D27,
                    COMPACT_RECORD_TYPE_DOMAIN_DATA,
                    &d27_payload(status | extra, 7, &[900]),
                ));
                first.extend(frame_end(2, 100));
                assert!(
                    scan_stream(&first, &schema()).is_err(),
                    "status bit {bit} (+{extra:#x}) as the first record must be refused"
                );
                let mut after = Vec::new();
                after.extend(record(
                    1,
                    100,
                    D27,
                    COMPACT_RECORD_TYPE_DOMAIN_DATA,
                    &d27_payload(0, 7, &[900]),
                ));
                after.extend(frame_end(2, 100));
                after.extend(record(
                    3,
                    101,
                    D27,
                    COMPACT_RECORD_TYPE_DOMAIN_DATA,
                    &d27_payload(status | extra, 7, &[431]),
                ));
                after.extend(frame_end(4, 101));
                assert!(
                    scan_stream(&after, &schema()).is_err(),
                    "status bit {bit} (+{extra:#x}) after a baseline must be refused"
                );
            }
        }
        // And the assigned informational bits are still accepted: the check must
        // reject unassigned bits and nothing else.
        for informational in [1u32 << 4, 1 << 5, COMPACT_STATUS_DELTA_DATA, 1 << 11, 1 << 13] {
            let mut s = Vec::new();
            s.extend(record(
                1,
                100,
                D27,
                COMPACT_RECORD_TYPE_DOMAIN_DATA,
                &d27_payload(0, 7, &[900]),
            ));
            s.extend(frame_end(2, 100));
            s.extend(record(
                3,
                101,
                D27,
                COMPACT_RECORD_TYPE_DOMAIN_DATA,
                &d27_payload(informational, 7, &[431]),
            ));
            s.extend(frame_end(4, 101));
            let scan = scan_stream(&s, &schema())
                .unwrap_or_else(|e| panic!("informational bit {informational:#x}: {e}"));
            assert_eq!(scan.hours.len(), 2, "bit {informational:#x}");
            assert_eq!(scan.counts.failure_records, 0, "bit {informational:#x}");
            assert_eq!(scan.counts.withheld_partial, 0, "bit {informational:#x}");
        }
    }

    #[test]
    fn a_second_non_delta_frame_still_upserts() {
        // `replace_or_sparse_after_first` clears ONLY on the first non-delta
        // frame (`world_fold.rs:2954`). d27's real capture carries 99 status-0
        // records and only the first of them replaces; a harness that cleared on
        // every one would drop the whole accumulator 98 times.
        let mut s = Vec::new();
        s.extend(record(
            1,
            100,
            D27,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &d27_payload(0, 7, &[900]),
        ));
        s.extend(frame_end(2, 100));
        s.extend(record(
            3,
            101,
            D27,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &d27_payload(0, 8, &[431]),
        ));
        s.extend(frame_end(4, 101));
        let scan = scan_stream(&s, &schema()).expect("accepted");
        assert_eq!(scan.counts.replace_frames, 1);
        assert!(!scan.hours[0].sparse);
        assert!(scan.hours[1].sparse);
    }

    #[test]
    fn a_stream_truncated_on_a_record_boundary_is_rejected() {
        let full = good_stream();
        for records in [1usize, 3] {
            let cut = boundary(&full, records);
            assert!(
                scan_stream(&full[..cut], &schema()).is_err(),
                "a stream cut after {records} record(s) leaves an hour open"
            );
        }
        // A cut exactly at a FRAME_END boundary is a well-formed shorter stream
        // and `eof_check` accepts it; refusing it would be wrong.
        for (records, expected_hours) in [(2usize, 1usize), (4, 2)] {
            let cut = boundary(&full, records);
            let scan = scan_stream(&full[..cut], &schema()).expect("closed stream is well-formed");
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
                scan_stream(&full[..cut], &schema()).is_err(),
                "mid-record cut at {cut}"
            );
        }
        assert!(
            scan_stream(&[], &schema()).is_err(),
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
                    scan_stream(&s, &schema()).is_err(),
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
                    scan_stream(&s, &schema()).is_err(),
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
            D27,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &d27_payload(0, 7, &[900]),
        ));
        s.extend(frame_end(2, 999));
        assert!(scan_stream(&s, &schema()).is_err());

        let mut s = Vec::new();
        s.extend(frame_end(1, 100));
        assert!(scan_stream(&s, &schema()).is_err());
    }

    #[test]
    fn non_monotonic_hours_are_rejected() {
        let mut s = Vec::new();
        s.extend(record(
            1,
            101,
            D27,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &d27_payload(0, 7, &[900]),
        ));
        s.extend(frame_end(2, 101));
        s.extend(record(
            3,
            100,
            D27,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &d27_payload(0, 7, &[431]),
        ));
        s.extend(frame_end(4, 100));
        assert!(scan_stream(&s, &schema()).is_err());
    }

    #[test]
    fn a_second_d27_record_in_one_hour_is_rejected() {
        // The rule is checked over EVERY d27 DATA record and before status-based
        // exclusion, so all four success/failure orderings are refused.
        const FAILED: u32 = 1; // COMPACT_STATUS_STATE_UNAVAILABLE
        for (first, second) in [(0u32, 0u32), (0, FAILED), (FAILED, 0), (FAILED, FAILED)] {
            let mut s = Vec::new();
            s.extend(record(
                1,
                100,
                D27,
                COMPACT_RECORD_TYPE_DOMAIN_DATA,
                &d27_payload(first, 7, &[900]),
            ));
            s.extend(record(
                2,
                100,
                D27,
                COMPACT_RECORD_TYPE_DOMAIN_DATA,
                &d27_payload(second, 7, &[431]),
            ));
            s.extend(frame_end(3, 100));
            assert!(
                scan_stream(&s, &schema()).is_err(),
                "two d27 records in hour 100 with statuses {first:#x}/{second:#x}"
            );
        }
    }

    #[test]
    fn a_failure_status_record_is_excluded_rather_than_measured() {
        // Every failure bit in `COMPACT_STATUS_FAILURE_MASK` except bit 9,
        // `PARTIAL_DATA`, which is the one d27 tolerates and which reaches the
        // withhold path instead (covered by its own test). One bit at a time:
        // the fold retains the accumulator and folds nothing, so the hour is not
        // d27 data and must not enter either arm.
        for bit in [0u32, 1, 2, 3, 6, 7, 8, 12] {
            let mut s = Vec::new();
            s.extend(record(
                1,
                100,
                D27,
                COMPACT_RECORD_TYPE_DOMAIN_DATA,
                &d27_payload(1 << bit, 7, &[900]),
            ));
            s.extend(frame_end(2, 100));
            s.extend(record(
                3,
                101,
                D27,
                COMPACT_RECORD_TYPE_DOMAIN_DATA,
                &d27_payload(0, 7, &[431]),
            ));
            s.extend(frame_end(4, 101));
            let scan = scan_stream(&s, &schema()).expect("the stream itself is well-formed");
            assert_eq!(scan.counts.failure_records, 1, "bit {bit}");
            assert_eq!(scan.hours.len(), 1, "bit {bit}");
            assert_eq!(scan.hours[0].game_hour, 101, "bit {bit}");
        }
        // And DELTA_DATA is not a failure bit.
        let scan = scan_stream(&good_stream(), &schema()).expect("well-formed");
        assert_eq!(scan.counts.failure_records, 0);
        assert_eq!(scan.hours.len(), 2);
    }

    #[test]
    fn a_tolerated_partial_is_withheld_not_measured() {
        // d27's own fold-nothing path (`world_fold.rs:14301`): a PARTIAL_DATA
        // record with no other failure bit is validated and then withheld, so
        // its hour enters neither arm and its rows never reach the accumulator.
        let mut s = Vec::new();
        s.extend(record(
            1,
            100,
            D27,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &d27_payload(0, 7, &[900]),
        ));
        s.extend(frame_end(2, 100));
        s.extend(record(
            3,
            101,
            D27,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &d27_payload(COMPACT_STATUS_PARTIAL_DATA, 9, &[555]),
        ));
        s.extend(frame_end(4, 101));
        let scan = scan_stream(&s, &schema()).expect("well-formed");
        assert_eq!(scan.counts.withheld_partial, 1);
        assert_eq!(scan.counts.failure_records, 0);
        assert_eq!(scan.hours.len(), 1, "only hour 100 is d27 data");
        assert_eq!(scan.hours[0].game_hour, 100);

        // Before any baseline it is the OTHER exclusion, and it must not be
        // counted as a withhold.
        let mut s = Vec::new();
        s.extend(record(
            1,
            100,
            D27,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &d27_payload(COMPACT_STATUS_PARTIAL_DATA, 7, &[900]),
        ));
        s.extend(frame_end(2, 100));
        s.extend(record(
            3,
            101,
            D27,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &d27_payload(0, 7, &[431]),
        ));
        s.extend(frame_end(4, 101));
        let scan = scan_stream(&s, &schema()).expect("well-formed");
        assert_eq!(scan.counts.partial_before_baseline, 1);
        assert_eq!(scan.counts.withheld_partial, 0);
        assert_eq!(scan.hours.len(), 1);

        // A malformed tolerated-partial frame is still validated: the withhold
        // sits AFTER the fold's row loop, so the fold `Err`s rather than
        // withholding — and the record is counted as rejected, not as a
        // withhold, because those are different facts.
        let mut bad = d27_payload(COMPACT_STATUS_PARTIAL_DATA, 9, &[555]);
        let row = d27::HEADER_LEN;
        bad[row..row + 4].copy_from_slice(&0u32.to_le_bytes()); // owner_id 0
        let mut s = Vec::new();
        s.extend(record(
            1,
            100,
            D27,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &d27_payload(0, 7, &[900]),
        ));
        s.extend(frame_end(2, 100));
        s.extend(record(3, 101, D27, COMPACT_RECORD_TYPE_DOMAIN_DATA, &bad));
        s.extend(frame_end(4, 101));
        let scan = scan_stream(&s, &schema()).expect("the stream itself is well-formed");
        assert_eq!(scan.counts.rejected_records, 1);
        assert_eq!(scan.counts.withheld_partial, 0);
        assert_eq!(scan.hours.len(), 1);
    }

    #[test]
    fn a_delta_before_any_baseline_is_excluded() {
        let mut s = Vec::new();
        s.extend(record(
            1,
            100,
            D27,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &d27_payload(COMPACT_STATUS_DELTA_DATA, 7, &[900]),
        ));
        s.extend(frame_end(2, 100));
        s.extend(record(
            3,
            101,
            D27,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &d27_payload(0, 7, &[431]),
        ));
        s.extend(frame_end(4, 101));
        let scan = scan_stream(&s, &schema()).expect("well-formed");
        assert_eq!(scan.counts.delta_before_baseline, 1);
        assert_eq!(scan.hours.len(), 1);
        assert_eq!(scan.hours[0].game_hour, 101);
        assert!(!scan.hours[0].sparse, "hour 101 is still the first baseline");
    }

    #[test]
    fn a_stream_with_no_d27_records_is_refused_not_reported_as_zero() {
        let mut s = Vec::new();
        s.extend(record(1, 100, 20, COMPACT_RECORD_TYPE_DOMAIN_DATA, &[0u8; 8]));
        s.extend(frame_end(2, 100));
        assert!(scan_stream(&s, &schema()).is_err());
    }

    #[test]
    fn a_malformed_d27_payload_is_dropped_like_the_recorder_drops_it() {
        // Every case below is put in hour 101 behind a good hour 100, because a
        // stream that yields no usable hour at all is refused outright and that
        // would hide which rule fired.
        let prefix = {
            let mut s = Vec::new();
            s.extend(record(
                1,
                100,
                D27,
                COMPACT_RECORD_TYPE_DOMAIN_DATA,
                &d27_payload(0, 7, &[900]),
            ));
            s.extend(frame_end(2, 100));
            s
        };
        let dropped = |bad: &[u8]| -> ScanCounts {
            let mut s = prefix.clone();
            s.extend(record(3, 101, D27, COMPACT_RECORD_TYPE_DOMAIN_DATA, bad));
            s.extend(frame_end(4, 101));
            let scan = scan_stream(&s, &schema()).expect("the stream itself is well-formed");
            assert_eq!(scan.hours.len(), 1, "only the good hour is measured");
            assert_eq!(scan.hours[0].game_hour, 100);
            scan.counts
        };

        // A payload shorter than the whole body, and one shorter than even the
        // header: the fold's `need` at `:14162` and its length check at `:14185`
        // are both fold errors.
        let mut payload = d27_payload(0, 7, &[900]);
        payload.truncate(payload.len() - 1);
        assert_eq!(dropped(&payload).rejected_records, 1);
        assert_eq!(dropped(&[0u8; 8]).rejected_records, 1);

        // A row the FOLD rejects must be counted and dropped, never measured and
        // never silently skipped. The whole row-rejection space, not one
        // case: zero owner_id, zero generation, an out-of-set owner_kind, an
        // unknown flag bit, a non-zero reserved word, an out-of-vocabulary
        // status code, and a declared count without its gate.
        let row = d27::HEADER_LEN;
        let mut broken: Vec<Vec<u8>> = Vec::new();
        for (offset, word) in [
            (0usize, 0u32),                        // owner_id
            (12, d27::KNOWN_FLAGS | (1 << 20)),    // unknown flag bit
            (36, d27::TARGET_STATUS_CARDINALITY),  // status out of vocabulary
            (40, 2),                               // owner_kind out of set
            (44, 1),                               // reserved
        ] {
            let mut p = d27_payload(0, 7, &[900]);
            p[row + offset..row + offset + 4].copy_from_slice(&word.to_le_bytes());
            broken.push(p);
        }
        // A zero generation (a u64 at +4).
        let mut p = d27_payload(0, 7, &[900]);
        p[row + 4..row + 12].copy_from_slice(&0u64.to_le_bytes());
        broken.push(p);
        // A declared count with its gate cleared but the word left non-zero.
        let mut p = d27_payload(0, 7, &[900]);
        p[row + 12..row + 16].copy_from_slice(&d27::FLAG_TARGET_STATES.to_le_bytes());
        broken.push(p);
        // A zero state id.
        let mut p = d27_payload(0, 7, &[900]);
        let target = row + d27::ROW_LEN;
        p[target..target + 4].copy_from_slice(&0u32.to_le_bytes());
        broken.push(p);
        // A non-zero target reserved word.
        let mut p = d27_payload(0, 7, &[900]);
        p[target + 4..target + 8].copy_from_slice(&1u32.to_le_bytes());
        broken.push(p);
        // An unowned target child: the header claims two targets, the row owns
        // one.
        let mut p = d27_payload(0, 7, &[900, 431]);
        p[row + 28..row + 32].copy_from_slice(&1u32.to_le_bytes());
        broken.push(p);
        // A duplicated state id within one row.
        broken.push(d27_payload(0, 7, &[900, 900]));
        // A target range declared without the TARGET_STATES flag.
        let mut p = d27_payload(0, 7, &[900]);
        p[row + 12..row + 16].copy_from_slice(&d27::FLAG_TARGET_STATE_COUNT.to_le_bytes());
        broken.push(p);
        for (i, payload) in broken.into_iter().enumerate() {
            assert_eq!(
                dropped(&payload).rejected_records,
                1,
                "invalid payload {i} must be counted as a dropped record"
            );
        }

        // An identity repeated within ONE frame is a rejected frame
        // (`world_fold.rs:14237`), never a last-writer-wins overwrite.
        let mut p = d27_payload(0, 7, &[900]);
        let row_bytes: Vec<u8> = p[row..row + d27::ROW_LEN].to_vec();
        let target_bytes: Vec<u8> = p[row + d27::ROW_LEN..].to_vec();
        p.truncate(row);
        p.extend_from_slice(&row_bytes);
        p.extend_from_slice(&row_bytes);
        p.extend_from_slice(&target_bytes);
        p[8..12].copy_from_slice(&2u32.to_le_bytes()); // row_count
        p[12..16].copy_from_slice(&2u32.to_le_bytes()); // changed_rows
        assert_eq!(
            dropped(&p).rejected_records,
            1,
            "duplicate identity in one frame"
        );
    }

    /// The generation-zero case `deepwar_t1` actually contains, pinned as a
    /// fold-side regression: `fold_army_orders` rejects `generation == 0`
    /// (`world_fold.rs:14226`), so the capture's first and only full baseline is
    /// unfoldable. The collector no longer emits such a row —
    /// `compact_army_order_delta_rows` normalises its counter with `.max(1)` —
    /// but captures recorded before that fix still carry one, so the harness must
    /// keep accounting for it. The recorder notes and continues, and so does this harness —
    /// and the delta records that follow then hit
    /// `reject_delta_before_baseline`, which is how a real capture reaches that
    /// exclusion at all.
    #[test]
    fn a_generation_zero_row_is_dropped_and_the_deltas_behind_it_are_excluded() {
        let mut baseline = d27_payload(0, 7, &[900]);
        let row = d27::HEADER_LEN;
        baseline[row + 4..row + 12].copy_from_slice(&0u64.to_le_bytes());
        let mut s = Vec::new();
        s.extend(record(1, 100, D27, COMPACT_RECORD_TYPE_DOMAIN_DATA, &baseline));
        s.extend(frame_end(2, 100));
        s.extend(record(
            3,
            101,
            D27,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &d27_payload(COMPACT_STATUS_DELTA_DATA, 7, &[431]),
        ));
        s.extend(frame_end(4, 101));
        s.extend(record(
            5,
            102,
            D27,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &d27_payload(0, 7, &[555]),
        ));
        s.extend(frame_end(6, 102));
        let scan = scan_stream(&s, &schema()).expect("well-formed");
        assert_eq!(scan.counts.rejected_records, 1);
        assert_eq!(scan.counts.reject_reasons[0].0, 100);
        assert!(
            scan.counts.reject_reasons[0].1.contains("generation=0"),
            "{}",
            scan.counts.reject_reasons[0].1
        );
        assert_eq!(
            scan.counts.delta_before_baseline, 1,
            "the rejected baseline never set delta_baselines_seen"
        );
        assert_eq!(scan.hours.len(), 1, "only hour 102 is d27 data");
        assert_eq!(scan.hours[0].game_hour, 102);
        assert!(
            !scan.hours[0].sparse,
            "hour 102 becomes the first accepted baseline"
        );
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
            let _ = scan_stream(&bytes, &schema());
        }
        let good = good_stream();
        for i in 0..good.len() {
            let mut s = good.clone();
            s[i] ^= 0xFF;
            let _ = scan_stream(&s, &schema());
        }
    }
}
