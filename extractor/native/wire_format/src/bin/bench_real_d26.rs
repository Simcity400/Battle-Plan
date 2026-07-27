//! Measure the wire format against the shipping compact stream on REAL d26
//! `division_equipment_live` dynamics.
//!
//! Input: a real `collector_stream.bin` from a d26-scheduled capture
//! (`runs/batch1/deepwar_t1`). The stream's own d26 DATA records are the
//! shipping format's cost for the domain: a 40-byte header, a fixed 24-byte root
//! row per serialized division and a fixed 16-byte entry row per equipment line,
//! one full baseline followed by deltas, exactly what the recorder stores today.
//! This tool replays those records into the per-hour accumulator
//! `world_fold::fold_division_equipment` builds and re-encodes every hour with
//! this crate's grammar: a BASELINE at each 48-hour block start, field-granular
//! stored-minimal DELTAs between.
//!
//! **Two collections, and the second one is where deletes live.** d26's payload
//! addresses a root's equipment as an index range into a shared entry array;
//! those indices are transport, so the harness flattens them into
//! `(owner_tag_db_id, div_id, variant_id)` keyed rows (`wire_format::d26`). A
//! root that loses a variant therefore produces a **tombstone** in the equipment
//! collection, and a `DELETED` root row produces tombstones in both. Nothing
//! constructs a tombstone by hand: `body::diff` derives them from the two states,
//! which is what makes them minimal, and `body::encode_delta` refuses a tombstone
//! for a key the reader's reconstructed prior does not hold.
//!
//! **Nothing here re-derives a contract it can call.** Row and payload decoding
//! go through `wire_format::d26`'s public boundary; H4CR framing, record
//! semantics and sequence continuity go through `compact_contract`'s own parser
//! and stateful validator; the frame-mode and failure classification go through
//! `compact_contract`'s status predicates rather than a copied bit mask. A
//! harness that reimplements any of them drifts from the recorder without
//! anything failing.
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
//! - and every rule `fold_division_equipment` applies to a d26 payload, by
//!   folding each one during the scan.
//!
//! That is a documented **subset** of the production boundary contract:
//! `recorder_host::stream_fold::verify_stream_logical_boundary` (`:537`) is the
//! full authority, and it additionally checks FRAME_END seal fields and
//! DOMAIN_PERF domain accounting. This crate cannot depend on `recorder_host`
//! (it pulls in every capture lane), so the subset is enforced here and the full
//! check remains the recorder's.
//!
//! **Which hours are measured.** The fold has four paths on which a d26 record
//! folds nothing and the retained accumulator stands
//! (`world_fold.rs:17301`-`:17335`, in this order):
//!
//! 1. a tolerated-partial record before any complete baseline
//!    (`reject_expected_partial_before_baseline`, `:2904`);
//! 2. a rowless tolerated-partial record (`:17311`);
//! 3. any other failure status, by `compact_contract`'s own tolerance-aware
//!    predicate (`handle_compact_failure`, `:2843`);
//! 4. a delta before any baseline (`reject_delta_before_baseline`, `:2875`).
//!
//! Those hours are excluded from BOTH arms — they are not d26 data. Hour
//! uniqueness is enforced over EVERY d26 DATA record, of every status, BEFORE
//! any of those exclusions, or a success+failure pair in one hour slips through.
//! A stream cut exactly at a FRAME_END boundary is a well-formed shorter stream
//! and is accepted as such.
//!
//! **`sparse_partial` hours ARE measured**, and that is a decision worth stating.
//! A tolerated-partial record carrying rows is the fold's third frame mode: it
//! applies its rows to the retained accumulator like a delta and then marks the
//! **domain-hour** unknown (`:17301`-`:17330`, `:17543`-`:17550`), because the
//! division identities it omitted are not serialized. It folds, so the recipe's
//! one exclusion rule — records that fold nothing — does not reach it, and its
//! rows are ordinary measured rows. Excluding the hour while still applying its
//! rows would also bias the comparison: the shipping arm banks the change in its
//! collector tracker and never pays for it again, while the wire arm would pay
//! for it in the next emitted delta. What the wire format genuinely cannot say
//! is "keys I did not look at may exist" — a domain-hour gap is §8 gap
//! normalisation, which has no implementation in this crate (`lib.rs`), and that
//! is reported as a gap rather than papered over with a per-field state. The
//! count is printed separately so the number stays interpretable. In
//! `runs/batch1/deepwar_t1` the count is zero, so the choice does not move this
//! measurement at all.
//!
//! **The D20 cross-domain prune is proved inert, not assumed away.** The
//! recorder also removes d26 keys from its accumulator when D20 reports a
//! division deleted in the same hour (`world_fold.rs:5074`-`:5099`), at hour
//! close and outside `fold_division_equipment`. A harness reading d26 records
//! alone cannot reproduce that, and if it fired the measured state series would
//! diverge from the recorder's. So the scan tracks D20's deleted `div_id`s per
//! hour and refuses the stream if any of them collides with a live d26 key at
//! the hour's close — an over-approximation of the recorder's condition, so it
//! cannot miss a firing. See `d20_deleted_div_ids`.
//!
//! Outputs, for the compression step (`compress_arms.py` beside the run):
//!   ship_d26.bin   — the d26 DATA records verbatim (32-byte H4CR header +
//!                    payload), concatenated in stream order
//!   wire_new.bin   — the wire-format bodies, concatenated in hour order
//!   *.offsets      — hour-boundary byte offsets, one per line, for exact
//!                    block-span slicing
//!
//! It also prints the real churn surface (changed rows and tombstones per hour
//! at stored precision, per collection).

use std::collections::BTreeMap;
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
use wire_format::d26::{self, FrameMode};
use wire_format::{Collection, Record, Schema};

const D26: u16 = 26;
const BLOCK_HOURS: usize = 48;

/// The fold's accumulator for one division root: its record in the `divisions`
/// collection, and its equipment lines by variant. This is
/// `world_fold.rs:1527`'s `BTreeMap<DivisionEquipmentKey, Vec<..>>` with the
/// entry vector keyed rather than positional, which is the same content — the
/// fold's own duplicate check is on `(owner, div_id, variant_id)` (`:17527`).
#[derive(Clone, Debug)]
struct AccRoot {
    record: Record,
    entries: BTreeMap<u32, Record>,
}

type Accumulator = BTreeMap<(u32, u32), AccRoot>;

/// One usable d26 hour: its game hour, its byte span in the source stream (the
/// shipping arm is those bytes verbatim, header included) and the payload span
/// plus mode the encode pass re-folds from.
///
/// The state is deliberately NOT retained here: d26's accumulator runs to ~3.3k
/// roots and ~21k equipment lines, and holding 903 hours of it would cost
/// gigabytes. The scan still folds every hour in full, so every way a row can be
/// rejected is exercised in the pass that either accepts the stream or refuses
/// it; the encode pass then re-folds through the identical code path, which
/// cannot fail where the scan did not.
struct D26Hour {
    game_hour: i32,
    ship_span: Range<usize>,
    payload_span: Range<usize>,
    mode: FrameMode,
}

#[derive(Default)]
struct ScanCounts {
    /// d26 records whose payload status marks the domain-hour failed.
    failure_records: usize,
    /// Tolerated-partial records that arrived before a complete baseline.
    partial_before_baseline: usize,
    /// Rowless tolerated-partial records.
    rowless_partial: usize,
    /// Delta records that arrived before a complete baseline.
    delta_before_baseline: usize,
    /// Measured hours whose frame is the fold's third, `sparse_partial` mode.
    sparse_partial_hours: usize,
    /// Measured hours that do not follow the previous measured one by one.
    non_contiguous: usize,
    /// D20 delete rows seen, all of which the prune guard cleared.
    d20_delete_rows: usize,
}

struct StreamScan {
    hours: Vec<D26Hour>,
    counts: ScanCounts,
}

/// Apply one d26 payload to the accumulator exactly as `fold_division_equipment`
/// does (`world_fold.rs:17336`-`:17542`), returning its header.
///
/// Fail-closed: every rejection the fold makes is an `Err` here. Total: no
/// payload of any content panics.
fn apply_frame(
    acc: &mut Accumulator,
    payload: &[u8],
    mode: FrameMode,
    divisions: &Collection,
    equipment: &Collection,
) -> Result<d26::PayloadHeader, String> {
    let (header, root_bytes, entry_bytes) = d26::split_payload(payload, mode)?;
    let entry_limit = header.equipment_entry_count as usize;

    let roots: Vec<d26::DivisionRow> = root_bytes
        .chunks_exact(d26::ROOT_ROW_LEN)
        .map(d26::parse_division_row)
        .collect::<Result<_, String>>()?;

    // `world_fold.rs:17465`-`:17487`: EVERY entry row in the frame is decoded
    // and checked in one pass before any root is applied, so a single invalid
    // entry rejects the whole frame. That has to happen here, before the root
    // loop, for two reasons a per-root walk gets wrong: a row belonging to a
    // duplicate-key root would otherwise never be looked at, and entry ranges
    // may legally overlap (the fold constrains their bounds and their sum, not
    // that each row is referenced once), so a row no root's range names would
    // never be looked at either. Both are rows the recorder refuses and this
    // would have accepted.
    let entries: Vec<(u32, Record)> = entry_bytes
        .chunks_exact(d26::ENTRY_ROW_LEN)
        .enumerate()
        .map(|(index, row)| {
            let parsed =
                d26::parse_equipment_row(row).map_err(|e| format!("entry row {index}: {e}"))?;
            let record = d26::record_from_equipment_row(equipment, row, 0)
                .map_err(|e| format!("entry row {index}: {e}"))?;
            Ok((parsed.variant_id, record))
        })
        .collect::<Result<_, String>>()?;

    // `world_fold.rs:17494`-`:17512`: a key serialized twice in one frame is
    // ambiguous, so ALL of its rows are dropped AND the prior equipment for that
    // key is purged. This is a `note`, not an error — a frame carrying one is
    // still folded.
    let mut key_counts: BTreeMap<(u32, u32), usize> = BTreeMap::new();
    for root in &roots {
        *key_counts
            .entry((root.owner_tag_db_id, root.div_id))
            .or_default() += 1;
    }

    // Build every surviving root before touching the accumulator, so a frame
    // this rejects leaves the accumulator exactly as it was. A duplicate key
    // appears at most once among the surviving roots, so `removals` and
    // `upserts` name disjoint keys and their order does not matter.
    let mut removals: Vec<(u32, u32)> = Vec::new();
    let mut upserts: Vec<((u32, u32), AccRoot)> = Vec::new();
    for (index, root) in roots.iter().enumerate() {
        let key = (root.owner_tag_db_id, root.div_id);
        if key_counts.get(&key).copied().unwrap_or(0) > 1 {
            continue;
        }
        if root.deleted {
            // `world_fold.rs:17519`-`:17522`. On the wire this becomes a
            // tombstone in both collections, derived by `diff` from the state
            // this removal produces.
            removals.push(key);
            continue;
        }
        let row_bytes = &root_bytes[index * d26::ROOT_ROW_LEN..(index + 1) * d26::ROOT_ROW_LEN];
        let record = d26::record_from_division_row(divisions, row_bytes, 0)
            .map_err(|e| format!("root row {index}: {e}"))?;
        let range = d26::entry_index_range(root, entry_limit)
            .map_err(|e| format!("root row {index}: {e}"))?;
        let mut lines: BTreeMap<u32, Record> = BTreeMap::new();
        for i in range {
            let (variant_id, entry_record) = entries
                .get(i)
                .ok_or_else(|| format!("root row {index}: entry index {i} is not in the frame"))?;
            // `world_fold.rs:17527`-`:17533`: a variant repeated under one root
            // is a rejected frame, never a last-writer-wins overwrite.
            if lines.insert(*variant_id, entry_record.clone()).is_some() {
                return Err(format!(
                    "division_equipment root owner={} div_id={} duplicates variant {variant_id}",
                    root.owner_tag_db_id, root.div_id
                ));
            }
        }
        upserts.push((key, AccRoot { record, entries: lines }));
    }

    // `world_fold.rs:17486`-`:17489`: a clean full frame replaces the world.
    if !mode.delta && !mode.sparse_partial {
        acc.clear();
    }
    for (key, count) in &key_counts {
        if *count > 1 {
            acc.remove(key);
        }
    }
    for key in removals {
        acc.remove(&key);
    }
    for (key, root) in upserts {
        acc.insert(key, root);
    }

    Ok(header)
}

// --- The D20 cross-domain prune guard ---------------------------------------
//
// `world_fold.rs:5074`-`:5099` runs `prune_division_equipment_for_deleted_divisions_current_hour`
// at hour close: when D20 reports a division deleted and the D20 lane is present
// and not unknown that hour, the recorder removes that division's key from the
// **d26 accumulator itself**. That mutation is outside `fold_division_equipment`
// and outside d26's own record stream, so a harness reading d26 records alone
// cannot reproduce it — and if it fired, this harness's state would diverge from
// the recorder's and the measured churn and tombstones would be wrong.
//
// Rather than carry that as a caveat, the harness proves the prune is inert on
// the stream it is measuring, and refuses the stream otherwise. The proof is a
// deliberate OVER-approximation of the recorder's condition: it ignores whether
// the deleted uid was known, whether its owner resolved, and whether the D20
// lane was unknown that hour — every one of which can only make the real prune
// fire less often. So "no D20-deleted div_id collides with a live d26 key at any
// hour close" implies the prune removed nothing, without this harness having to
// reimplement `fold_divisions`.
//
// It needs exactly three facts about a D20 payload, all from `fold_divisions`
// (`world_fold.rs:18923`-`:18945`, `:19095`): the 32-byte header, the row length
// (120 extended or 104 legacy, selected by the payload length), and `div_id` at
// `+0` with `flags` at `+8` carrying `DIVISION_FLAG_DELETED` (`:371`). A payload
// that does not fit that shape is not proof of anything, so it is refused too.

/// `DIVISIONS_DOMAIN_ID`, `world_fold.rs:629`.
const D20: u16 = 20;
/// `world_fold.rs:18924`: `need(payload, 32, "divisions header")`.
const D20_HEADER_LEN: usize = 32;
/// `DIVISION_ROW_LEN` / `LEGACY_DIVISION_ROW_LEN`, `world_fold.rs:350`-`:351`.
const D20_ROW_LEN: usize = 120;
const D20_LEGACY_ROW_LEN: usize = 104;
/// `DIVISION_FLAG_DELETED`, `world_fold.rs:371`.
const D20_FLAG_DELETED: u32 = 1 << 12;

/// The `div_id`s a D20 DATA payload reports deleted this hour.
fn d20_deleted_div_ids(payload: &[u8]) -> Result<Vec<u32>, String> {
    if payload.len() < D20_HEADER_LEN {
        return Err(format!(
            "divisions payload {} shorter than the {D20_HEADER_LEN}-byte header",
            payload.len()
        ));
    }
    let row_count = u32::from_le_bytes(payload[8..12].try_into().expect("4 bytes")) as usize;
    let row_len = [D20_ROW_LEN, D20_LEGACY_ROW_LEN]
        .into_iter()
        .find(|len| {
            row_count
                .checked_mul(*len)
                .and_then(|body| body.checked_add(D20_HEADER_LEN))
                == Some(payload.len())
        })
        .ok_or_else(|| {
            format!(
                "divisions len {} matches neither the extended nor the legacy row length for \
                 row_count {row_count}",
                payload.len()
            )
        })?;
    let mut deleted = Vec::new();
    for row in payload[D20_HEADER_LEN..].chunks_exact(row_len) {
        let flags = u32::from_le_bytes(row[8..12].try_into().expect("4 bytes"));
        if flags & D20_FLAG_DELETED != 0 {
            deleted.push(u32::from_le_bytes(row[0..4].try_into().expect("4 bytes")));
        }
    }
    Ok(deleted)
}

/// The two keyed states the accumulator represents, in schema order.
fn states_of(acc: &Accumulator) -> Vec<KeyedState> {
    let mut roots = KeyedState::new();
    let mut equipment = KeyedState::new();
    for ((owner, div_id), root) in acc {
        roots.insert(d26::division_key(*owner, *div_id), root.record.clone());
        for (variant, record) in &root.entries {
            equipment.insert(
                d26::equipment_key(*owner, *div_id, *variant),
                record.clone(),
            );
        }
    }
    vec![roots, equipment]
}

/// Validate the whole stream against the canonical contract and fold every
/// usable d26 DATA record. Every rejection path is an `Err`, never a silently
/// shorter measurement and never a panic.
fn scan_stream(stream: &[u8], schema: &Schema) -> Result<StreamScan, String> {
    let divisions = &schema.collections[0];
    let equipment = &schema.collections[1];

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

    let mut hours: Vec<D26Hour> = Vec::new();
    let mut counts = ScanCounts::default();

    let mut open_hour: Option<i32> = None;
    let mut last_closed_hour: Option<i32> = None;
    let mut last_record_type: Option<u16> = None;
    // Hour of the last d26 DATA record of ANY status — the uniqueness rule.
    let mut last_d26_hour: Option<i32> = None;
    // Hour of the last d26 record that entered the measurement — the contiguity
    // report, which a skipped failure hour legitimately breaks.
    let mut last_measured_hour: Option<i32> = None;
    // `delta_baselines_seen` for d26 (`world_fold.rs:17487`, `:17490`).
    let mut baseline_seen = false;
    let mut acc: Accumulator = Accumulator::new();
    // The `div_id`s D20 reported deleted in the open hour, checked against the
    // d26 accumulator when that hour closes — the recorder's prune point.
    let mut d20_deleted_this_hour: Vec<u32> = Vec::new();

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
                // The recorder's hour-close prune point
                // (`world_fold.rs:5074`-`:5099`). Over-approximated: a
                // collision on `div_id` alone is enough to refuse, because
                // owner resolution, uid knowledge and D20 freshness can only
                // make the real prune fire less often.
                if !d20_deleted_this_hour.is_empty() {
                    for div_id in &d20_deleted_this_hour {
                        if acc.keys().any(|(_, live)| live == div_id) {
                            return Err(format!(
                                "game hour {}: D20 deleted division {div_id}, which d26 still \
                                 holds; the recorder would prune it from the d26 accumulator \
                                 (world_fold.rs:5074) and this harness cannot reproduce that \
                                 from d26 records alone",
                                header.game_hour_raw
                            ));
                        }
                    }
                    d20_deleted_this_hour.clear();
                }
                open_hour = None;
                last_closed_hour = Some(header.game_hour_raw);
            }
            _ => {}
        }
        last_record_type = Some(header.record_type);

        if header.domain_id == D20 && header.record_type == COMPACT_RECORD_TYPE_DOMAIN_DATA {
            let deleted = d20_deleted_div_ids(payload)
                .map_err(|e| format!("d20 payload at byte {pos}: {e}"))?;
            counts.d20_delete_rows += deleted.len();
            d20_deleted_this_hour.extend(deleted);
        }

        if header.domain_id == D26 && header.record_type == COMPACT_RECORD_TYPE_DOMAIN_DATA {
            // The header words the fold's own mode classification reads, through
            // d26's boundary rather than a second copy of the header layout.
            let d26_header = d26::peek_header(payload)
                .map_err(|e| format!("d26 payload at byte {pos}: {e}"))?;

            // One d26 DATA record per game hour, checked over EVERY d26 record
            // and BEFORE any status-based exclusion. A second record for the
            // same hour would become a second measurement hour built from the
            // same clock time — and gating this on the record being usable would
            // let a success/failure pair through the rule entirely.
            if let Some(previous) = last_d26_hour {
                if header.game_hour_raw <= previous {
                    return Err(format!(
                        "d26 DATA at byte {pos} carries hour {} after hour {previous}",
                        header.game_hour_raw
                    ));
                }
            }
            last_d26_hour = Some(header.game_hour_raw);

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
            // measured as healthy. The production lane rejects first
            // (`lanes/division_state_movement_live/domain20_compact.rs:419`);
            // this harness does the same rather than re-deriving a lenient rule.
            // No capture carries an unassigned bit, so no measured number moves.
            let unknown_status = compact_status_unknown_flags_v1(d26_header.status_flags);
            if unknown_status != 0 {
                return Err(format!(
                    "d26 DATA at byte {pos}: hour {} carries unassigned v1 status bits \
                     {unknown_status:#x}; an unassigned bit has no defined classification",
                    header.game_hour_raw
                ));
            }

            // The fold's classification, from `compact_contract`'s predicates —
            // never a hand-copied mask literal, so it cannot drift from the
            // recorder's own tolerance table.
            let tolerance_applies =
                expected_partial_tolerance_applies(D26, d26_header.status_flags).is_some();
            let mode = FrameMode {
                delta: d26_header.status_flags & COMPACT_STATUS_DELTA_DATA != 0,
                sparse_partial: d26_header.row_count > 0 && tolerance_applies,
            };

            // The four fold-nothing paths, in the fold's own order
            // (`world_fold.rs:17301`-`:17335`).
            if tolerance_applies && !baseline_seen {
                counts.partial_before_baseline += 1;
            } else if d26_header.row_count == 0 && tolerance_applies {
                counts.rowless_partial += 1;
            } else if !mode.sparse_partial
                && compact_status_failure_flags_after_expected_partial_tolerance(
                    D26,
                    d26_header.status_flags,
                ) != 0
            {
                counts.failure_records += 1;
            } else if !mode.sparse_partial && mode.delta && !baseline_seen {
                counts.delta_before_baseline += 1;
            } else {
                if let Some(previous) = last_measured_hour {
                    if header.game_hour_raw != previous + 1 {
                        counts.non_contiguous += 1;
                    }
                }
                last_measured_hour = Some(header.game_hour_raw);
                if mode.sparse_partial {
                    counts.sparse_partial_hours += 1;
                }

                apply_frame(&mut acc, payload, mode, divisions, equipment)
                    .map_err(|e| format!("d26 payload at byte {pos}: {e}"))?;
                if mode.sparse_partial || !mode.delta {
                    baseline_seen = true;
                }

                hours.push(D26Hour {
                    game_hour: header.game_hour_raw,
                    ship_span: pos..record_end,
                    payload_span: payload_start..record_end,
                    mode,
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
        return Err("stream carries no usable d26 DATA records".to_string());
    }

    Ok(StreamScan { hours, counts })
}

fn main() {
    let mut args = std::env::args().skip(1);
    let input = args
        .next()
        .expect("usage: bench_real_d26 <collector_stream.bin> <out dir>");
    let out_dir = args
        .next()
        .expect("usage: bench_real_d26 <collector_stream.bin> <out dir>");
    let stream = std::fs::read(&input).expect("read stream");
    let schema = d26::schema();

    // ---- Pass 1: validate the WHOLE stream and fold its d26 DATA records. ---
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
    // The fold runs through the identical `apply_frame` the scan proved every
    // hour against, so nothing here can fail on stream content; every assertion
    // below is about the codec.
    let mut ship = Vec::new();
    let mut ship_offsets = vec![0usize];
    let mut wire = Vec::new();
    let mut wire_offsets = vec![0usize];

    let mut acc: Accumulator = Accumulator::new();
    let mut prior: Option<Vec<KeyedState>> = None;
    let mut reader: Option<Vec<KeyedState>> = None;
    let mut hours_seen = 0usize;
    let mut baselines = 0usize;
    let mut empty_hours = 0usize;
    let mut delta_hours_with_change = 0usize;
    let mut changed_rows = [0usize; 2];
    let mut tombstones = [0usize; 2];
    let mut rows_total = [0usize; 2];

    for hour in &scan.hours {
        // Shipping arm: the record's exact bytes, header included.
        ship.extend_from_slice(&stream[hour.ship_span.clone()]);
        ship_offsets.push(ship.len());

        let payload = &stream[hour.payload_span.clone()];
        if let Err(reason) = apply_frame(
            &mut acc,
            payload,
            hour.mode,
            &schema.collections[0],
            &schema.collections[1],
        ) {
            eprintln!("{input}: game hour {} re-fold: {reason}", hour.game_hour);
            std::process::exit(1);
        }
        let states = states_of(&acc);
        for i in 0..2 {
            rows_total[i] += states[i].len();
        }

        // New arm: BASELINE at each block start, stored-minimal DELTA between.
        let hour_start = wire.len();
        let block_start = hours_seen % BLOCK_HOURS == 0;
        let mut emitted = true;
        if block_start || prior.is_none() {
            encode_baseline(&schema, &states, &mut wire).expect("baseline encodes");
            baselines += 1;
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
                reader = Some(states);
            } else {
                let mut reader_state = reader.take().expect("reader exists");
                let (deltas, consumed) = decode_delta(&schema, &reader_state, hour_bytes, 0)
                    .expect("delta decodes");
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
    for (name, bytes) in [("ship_d26.bin", &ship), ("wire_new.bin", &wire)] {
        let mut f = std::fs::File::create(std::path::Path::new(&out_dir).join(name)).unwrap();
        f.write_all(bytes).unwrap();
    }
    for (name, offsets) in [
        ("ship_d26.offsets", &ship_offsets),
        ("wire_new.offsets", &wire_offsets),
    ] {
        let mut f = std::fs::File::create(std::path::Path::new(&out_dir).join(name)).unwrap();
        for o in offsets {
            writeln!(f, "{o}").unwrap();
        }
    }

    let c = &scan.counts;
    println!("hours                {hours_seen} ({} non-contiguous steps)", c.non_contiguous);
    println!(
        "excluded hours       {} failure-status, {} partial-before-baseline, {} rowless-partial, {} delta-before-baseline",
        c.failure_records, c.partial_before_baseline, c.rowless_partial, c.delta_before_baseline
    );
    println!(
        "sparse_partial hours {} (measured; domain-hour unknown, see the module docs)",
        c.sparse_partial_hours
    );
    println!(
        "D20 delete rows      {} (none collided with a live d26 key: the recorder's \
         cross-domain prune is inert on this stream)",
        c.d20_delete_rows
    );
    println!(
        "rows avg             {:.1} divisions, {:.1} equipment lines",
        rows_total[0] as f64 / hours_seen as f64,
        rows_total[1] as f64 / hours_seen as f64
    );
    println!("baselines            {baselines} (every {BLOCK_HOURS} h)");
    println!("delta hours          {delta_hours_with_change}   empty hours {empty_hours}");
    let denominator = delta_hours_with_change.max(1) as f64;
    println!(
        "real churn divisions {:.2} changed rows/hour, {:.3} tombstones/hour (stored precision)",
        changed_rows[0] as f64 / denominator,
        tombstones[0] as f64 / denominator
    );
    println!(
        "real churn equipment {:.2} changed rows/hour, {:.3} tombstones/hour (stored precision)",
        changed_rows[1] as f64 / denominator,
        tombstones[1] as f64 / denominator
    );
    println!("ship_d26.bin         {} bytes", ship.len());
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
        COMPACT_DOMAIN_FRAME, COMPACT_FRAME_END_PAYLOAD_LEN, COMPACT_STATUS_PARTIAL_DATA,
    };

    fn schema() -> Schema {
        d26::schema()
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

    fn root_row(
        owner: u32,
        div_id: u32,
        flags: u32,
        template_id: u32,
        entry_first: u32,
        entry_count: u32,
    ) -> Vec<u8> {
        let mut row = Vec::new();
        for word in [owner, div_id, flags, template_id, entry_first, entry_count] {
            row.extend_from_slice(&word.to_le_bytes());
        }
        row
    }

    fn entry_row(variant_id: u32, amount_raw: i64) -> Vec<u8> {
        let mut row = Vec::new();
        row.extend_from_slice(&variant_id.to_le_bytes());
        row.extend_from_slice(&0u32.to_le_bytes());
        row.extend_from_slice(&amount_raw.to_le_bytes());
        row
    }

    /// A d26 DATA payload with explicit header counts, so a test can build both
    /// well-formed frames and the malformed ones the scan must refuse.
    fn payload_with(
        status_flags: u32,
        roots: &[Vec<u8>],
        entries: &[Vec<u8>],
        root_rows: u32,
        observed_equipment_rows: u32,
    ) -> Vec<u8> {
        let mut p = Vec::new();
        p.extend_from_slice(&0u64.to_le_bytes());
        p.extend_from_slice(&(roots.len() as u32).to_le_bytes());
        p.extend_from_slice(&(roots.len() as u32).to_le_bytes());
        p.extend_from_slice(&status_flags.to_le_bytes());
        p.extend_from_slice(&d26::FIXED_SCALE_RAW.to_le_bytes());
        p.extend_from_slice(&root_rows.to_le_bytes());
        p.extend_from_slice(&root_rows.to_le_bytes());
        p.extend_from_slice(&(entries.len() as u32).to_le_bytes());
        p.extend_from_slice(&observed_equipment_rows.to_le_bytes());
        for row in roots {
            p.extend_from_slice(row);
        }
        for row in entries {
            p.extend_from_slice(row);
        }
        p
    }

    /// A clean full baseline: one division with one equipment line.
    fn baseline_payload(amount_raw: i64) -> Vec<u8> {
        payload_with(
            0,
            &[root_row(7, 42, d26::ROOT_FLAG_TEMPLATE_PRESENT, 5, 0, 1)],
            &[entry_row(11, amount_raw)],
            1,
            1,
        )
    }

    /// A clean delta carrying one changed root.
    fn delta_payload(roots: &[Vec<u8>], entries: &[Vec<u8>]) -> Vec<u8> {
        payload_with(
            COMPACT_STATUS_DELTA_DATA,
            roots,
            entries,
            9,
            entries.len() as u32 + 5,
        )
    }

    /// Two closed hours: a baseline then a delta that changes the amount.
    fn good_stream() -> Vec<u8> {
        let mut s = Vec::new();
        s.extend(record(1, 100, D26, COMPACT_RECORD_TYPE_DOMAIN_DATA, &baseline_payload(100_000)));
        s.extend(frame_end(2, 100));
        s.extend(record(
            3,
            101,
            D26,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &delta_payload(
                &[root_row(7, 42, d26::ROOT_FLAG_TEMPLATE_PRESENT, 5, 0, 1)],
                &[entry_row(11, 200_000)],
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

    /// Re-fold a scan's hours the way `main`'s second pass does.
    fn fold_hours(stream: &[u8], scan: &StreamScan, schema: &Schema) -> Vec<Vec<KeyedState>> {
        let mut acc = Accumulator::new();
        let mut out = Vec::new();
        for hour in &scan.hours {
            apply_frame(
                &mut acc,
                &stream[hour.payload_span.clone()],
                hour.mode,
                &schema.collections[0],
                &schema.collections[1],
            )
            .expect("the scan already proved this frame");
            out.push(states_of(&acc));
        }
        out
    }

    #[test]
    fn a_well_formed_stream_yields_its_d26_hours() {
        let s = good_stream();
        let schema = schema();
        let scan = scan_stream(&s, &schema).expect("accepted");
        assert_eq!(scan.hours.len(), 2);
        assert_eq!(scan.hours[0].game_hour, 100);
        assert_eq!(scan.hours[1].game_hour, 101);
        assert_eq!(scan.counts.failure_records, 0);
        assert_eq!(scan.counts.non_contiguous, 0);
        assert!(!scan.hours[0].mode.delta, "the first frame is a full baseline");
        assert!(scan.hours[1].mode.delta);

        let states = fold_hours(&s, &scan, &schema);
        assert_eq!(states[0][0].len(), 1, "one division root");
        assert_eq!(states[0][1].len(), 1, "one equipment line");
        // The delta upserts the same key, so the row counts do not move but the
        // amount does.
        assert_eq!(states[1][1].len(), 1);
        assert_ne!(states[0][1], states[1][1], "the amount changed");
    }

    #[test]
    fn a_deleted_root_becomes_a_tombstone_in_both_collections() {
        // The property this domain is the first to exercise. A DELETED root row
        // removes the key from the fold's accumulator, so the derived delta must
        // carry a tombstone for the root AND for every equipment line it held —
        // and `encode_delta` must accept them against the reader's own prior.
        let schema = schema();
        let mut s = Vec::new();
        s.extend(record(1, 100, D26, COMPACT_RECORD_TYPE_DOMAIN_DATA, &baseline_payload(100_000)));
        s.extend(frame_end(2, 100));
        s.extend(record(
            3,
            101,
            D26,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &delta_payload(&[root_row(7, 42, d26::ROOT_FLAG_DELETED, 0, 0, 0)], &[]),
        ));
        s.extend(frame_end(4, 101));
        let scan = scan_stream(&s, &schema).expect("accepted");
        let states = fold_hours(&s, &scan, &schema);
        assert_eq!(states[1][0].len(), 0, "the root is gone");
        assert_eq!(states[1][1].len(), 0, "and so are its equipment lines");

        let deltas: Vec<Option<KeyedDelta>> = schema
            .collections
            .iter()
            .enumerate()
            .map(|(i, c)| Some(diff(c, &states[0][i], &states[1][i])))
            .collect();
        for (i, delta) in deltas.iter().enumerate() {
            let d = delta.as_ref().expect("both collections changed");
            assert!(d.changed.is_empty(), "collection {i} upserts nothing");
            assert_eq!(d.tombstones.len(), 1, "collection {i} tombstones its key");
        }
        let mut bytes = Vec::new();
        encode_delta(&schema, &states[0], &deltas, &mut bytes).expect("tombstones encode");
        let (decoded, consumed) =
            decode_delta(&schema, &states[0], &bytes, 0).expect("tombstones decode");
        assert_eq!(consumed, bytes.len());
        let mut replayed = states[0].clone();
        for (i, delta) in decoded.iter().enumerate() {
            apply(&mut replayed[i], delta.as_ref().expect("present"));
        }
        assert_eq!(replayed, states[1], "the reader reaches the fold's state");
    }

    #[test]
    fn dropping_one_variant_tombstones_only_that_line() {
        // A root upsert replaces its whole entry set (`world_fold.rs:17541`), so
        // a variant that disappears from an otherwise-unchanged root is a
        // tombstone in the equipment collection and nothing in the divisions one.
        let schema = schema();
        let mut s = Vec::new();
        s.extend(record(
            1,
            100,
            D26,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &payload_with(
                0,
                &[root_row(7, 42, d26::ROOT_FLAG_TEMPLATE_PRESENT, 5, 0, 2)],
                &[entry_row(11, 100_000), entry_row(12, 300_000)],
                1,
                2,
            ),
        ));
        s.extend(frame_end(2, 100));
        s.extend(record(
            3,
            101,
            D26,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &delta_payload(
                &[root_row(7, 42, d26::ROOT_FLAG_TEMPLATE_PRESENT, 5, 0, 1)],
                &[entry_row(11, 100_000)],
            ),
        ));
        s.extend(frame_end(4, 101));
        let scan = scan_stream(&s, &schema).expect("accepted");
        let states = fold_hours(&s, &scan, &schema);
        assert_eq!(states[0][1].len(), 2);
        assert_eq!(states[1][1].len(), 1);
        let roots_delta = diff(&schema.collections[0], &states[0][0], &states[1][0]);
        assert!(roots_delta.is_empty(), "the root itself did not change");
        let equipment_delta = diff(&schema.collections[1], &states[0][1], &states[1][1]);
        assert!(equipment_delta.changed.is_empty());
        assert_eq!(equipment_delta.tombstones.len(), 1);
        assert_eq!(equipment_delta.tombstones[0], d26::equipment_key(7, 42, 12));
    }

    #[test]
    fn a_duplicate_root_key_purges_that_key_rather_than_overwriting_it() {
        // `world_fold.rs:17494`-`:17512`: every row of an ambiguous key is
        // dropped AND the prior equipment for it is cleared. Last-writer-wins
        // would publish one of two contradictory rows as fact.
        let schema = schema();
        let mut s = Vec::new();
        s.extend(record(1, 100, D26, COMPACT_RECORD_TYPE_DOMAIN_DATA, &baseline_payload(100_000)));
        s.extend(frame_end(2, 100));
        s.extend(record(
            3,
            101,
            D26,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &delta_payload(
                &[
                    root_row(7, 42, d26::ROOT_FLAG_TEMPLATE_PRESENT, 5, 0, 1),
                    root_row(7, 42, d26::ROOT_FLAG_TEMPLATE_PRESENT, 6, 1, 1),
                ],
                &[entry_row(11, 500_000), entry_row(11, 600_000)],
            ),
        ));
        s.extend(frame_end(4, 101));
        let scan = scan_stream(&s, &schema).expect("a duplicate key is a note, not an error");
        let states = fold_hours(&s, &scan, &schema);
        assert_eq!(states[1][0].len(), 0, "the ambiguous root is purged");
        assert_eq!(states[1][1].len(), 0);
    }

    #[test]
    fn every_entry_row_in_the_frame_is_checked_even_if_no_root_reads_it() {
        // The reviewer's input, generalized. The fold decodes and range-checks
        // EVERY entry row in one pass before applying any root
        // (`world_fold.rs:17465`-`:17487`), so an invalid row rejects the frame
        // even when the root loop would never look at it. Two ways that
        // happens, and both must be refused:
        //   (1) the row belongs to a duplicate-key root, whose rows are all
        //       dropped (`:17513`-`:17517`);
        //   (2) entry ranges overlap — legal, since the fold constrains bounds
        //       and the SUM only — leaving a row no root's range names.
        let schema = schema();
        for (label, roots) in [
            (
                "duplicate-key root",
                vec![
                    root_row(7, 42, d26::ROOT_FLAG_TEMPLATE_PRESENT, 5, 0, 1),
                    root_row(7, 42, d26::ROOT_FLAG_TEMPLATE_PRESENT, 6, 1, 1),
                ],
            ),
            (
                "overlapping ranges",
                vec![
                    root_row(7, 42, d26::ROOT_FLAG_TEMPLATE_PRESENT, 5, 0, 1),
                    root_row(7, 43, d26::ROOT_FLAG_TEMPLATE_PRESENT, 6, 0, 1),
                ],
            ),
        ] {
            // The row nobody reads is invalid in each of the three ways the
            // fold rejects an entry row.
            for (case, bad) in [
                ("variant_id 0", entry_row(0, 100_000)),
                ("amount_raw 0", entry_row(12, 0)),
                ("reserved nonzero", {
                    let mut r = entry_row(12, 100_000);
                    r[4..8].copy_from_slice(&1u32.to_le_bytes());
                    r
                }),
            ] {
                let mut s = Vec::new();
                s.extend(record(
                    1,
                    100,
                    D26,
                    COMPACT_RECORD_TYPE_DOMAIN_DATA,
                    &payload_with(
                        0,
                        &roots,
                        &[entry_row(11, 100_000), bad],
                        roots.len() as u32,
                        2,
                    ),
                ));
                s.extend(frame_end(2, 100));
                assert!(
                    scan_stream(&s, &schema).is_err(),
                    "{label} with an unread {case} entry must refuse the frame"
                );
            }
            // And the same shape with every entry row valid is accepted, so the
            // rule rejects the row and not the shape.
            let mut s = Vec::new();
            s.extend(record(
                1,
                100,
                D26,
                COMPACT_RECORD_TYPE_DOMAIN_DATA,
                &payload_with(
                    0,
                    &roots,
                    &[entry_row(11, 100_000), entry_row(12, 200_000)],
                    roots.len() as u32,
                    2,
                ),
            ));
            s.extend(frame_end(2, 100));
            assert!(scan_stream(&s, &schema).is_ok(), "{label} with valid rows is legal");
        }
    }

    #[test]
    fn a_rejected_frame_leaves_the_accumulator_untouched() {
        // The corollary of validating before mutating: a frame this refuses must
        // not have half-applied its roots first, or the diagnosis would be about
        // a state no recorder ever held.
        let schema = schema();
        let mut acc = Accumulator::new();
        apply_frame(
            &mut acc,
            &baseline_payload(100_000),
            FrameMode::FULL,
            &schema.collections[0],
            &schema.collections[1],
        )
        .expect("baseline applies");
        let before = states_of(&acc);
        // A delta whose first root is fine and whose second entry row is not.
        let bad = payload_with(
            COMPACT_STATUS_DELTA_DATA,
            &[
                root_row(7, 42, d26::ROOT_FLAG_TEMPLATE_PRESENT, 5, 0, 1),
                root_row(8, 43, d26::ROOT_FLAG_TEMPLATE_PRESENT, 6, 1, 1),
            ],
            &[entry_row(11, 900_000), entry_row(0, 100_000)],
            9,
            7,
        );
        assert!(apply_frame(
            &mut acc,
            &bad,
            FrameMode::DELTA,
            &schema.collections[0],
            &schema.collections[1],
        )
        .is_err());
        assert_eq!(states_of(&acc), before, "the refused frame changed nothing");
    }

    #[test]
    fn a_d20_deletion_that_would_prune_a_live_d26_key_refuses_the_stream() {
        // `world_fold.rs:5074`-`:5099` removes the d26 key at hour close, which
        // this harness cannot reproduce from d26 records alone. Rather than
        // measure a divergent state series it refuses the stream — and it must
        // NOT refuse a stream whose D20 deletions touch no live d26 key.
        let schema = schema();
        let d20_row = |div_id: u32, flags: u32| {
            let mut row = vec![0u8; D20_ROW_LEN];
            row[0..4].copy_from_slice(&div_id.to_le_bytes());
            row[8..12].copy_from_slice(&flags.to_le_bytes());
            row
        };
        let d20_payload = |rows: &[Vec<u8>]| {
            let mut p = vec![0u8; D20_HEADER_LEN];
            p[8..12].copy_from_slice(&(rows.len() as u32).to_le_bytes());
            for row in rows {
                p.extend_from_slice(row);
            }
            p
        };
        // d26 holds (owner 7, div_id 42) after the baseline.
        for (deleted_div_id, expect_ok) in [(42u32, false), (43, true)] {
            let mut s = Vec::new();
            s.extend(record(1, 100, D26, COMPACT_RECORD_TYPE_DOMAIN_DATA, &baseline_payload(100_000)));
            s.extend(record(
                2,
                100,
                D20,
                COMPACT_RECORD_TYPE_DOMAIN_DATA,
                &d20_payload(&[d20_row(deleted_div_id, D20_FLAG_DELETED)]),
            ));
            s.extend(frame_end(3, 100));
            assert_eq!(
                scan_stream(&s, &schema).is_ok(),
                expect_ok,
                "D20 deleting division {deleted_div_id}"
            );
        }
        // A D20 row that is not a deletion never triggers the guard, at either
        // row length.
        for row_len in [D20_ROW_LEN, D20_LEGACY_ROW_LEN] {
            let mut row = vec![0u8; row_len];
            row[0..4].copy_from_slice(&42u32.to_le_bytes());
            let mut s = Vec::new();
            s.extend(record(1, 100, D26, COMPACT_RECORD_TYPE_DOMAIN_DATA, &baseline_payload(100_000)));
            s.extend(record(2, 100, D20, COMPACT_RECORD_TYPE_DOMAIN_DATA, &d20_payload(&[row])));
            s.extend(frame_end(3, 100));
            assert!(scan_stream(&s, &schema).is_ok(), "row length {row_len}");
        }
        // A D20 payload whose shape the guard cannot read is not proof of
        // anything, so it is refused rather than skipped.
        let mut s = Vec::new();
        s.extend(record(1, 100, D26, COMPACT_RECORD_TYPE_DOMAIN_DATA, &baseline_payload(100_000)));
        s.extend(record(2, 100, D20, COMPACT_RECORD_TYPE_DOMAIN_DATA, &vec![0u8; 7]));
        s.extend(frame_end(3, 100));
        assert!(scan_stream(&s, &schema).is_err(), "a short D20 payload");
        let mut ragged = d20_payload(&[d20_row(1, 0)]);
        ragged.push(0);
        let mut s = Vec::new();
        s.extend(record(1, 100, D26, COMPACT_RECORD_TYPE_DOMAIN_DATA, &baseline_payload(100_000)));
        s.extend(record(2, 100, D20, COMPACT_RECORD_TYPE_DOMAIN_DATA, &ragged));
        s.extend(frame_end(3, 100));
        assert!(scan_stream(&s, &schema).is_err(), "a D20 payload of no legal row length");
        // The guard is per hour: a deletion in an hour whose d26 key arrives
        // only later must not be carried forward and refuse that later hour.
        let mut s = Vec::new();
        s.extend(record(1, 100, D20, COMPACT_RECORD_TYPE_DOMAIN_DATA, &d20_payload(&[d20_row(42, D20_FLAG_DELETED)])));
        s.extend(frame_end(2, 100));
        s.extend(record(3, 101, D26, COMPACT_RECORD_TYPE_DOMAIN_DATA, &baseline_payload(100_000)));
        s.extend(frame_end(4, 101));
        assert!(scan_stream(&s, &schema).is_ok(), "the deletion preceded the key");
    }

    #[test]
    fn d20_deleted_div_ids_never_panics_on_arbitrary_bytes() {
        let mut state = 0x1234_5678_9ABC_DEF1u64;
        for _ in 0..8_000 {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            let len = (state as usize) % 600;
            let mut bytes = vec![0u8; len];
            for byte in &mut bytes {
                state ^= state << 13;
                state ^= state >> 7;
                state ^= state << 17;
                *byte = state as u8;
            }
            let _ = d20_deleted_div_ids(&bytes);
        }
        // And a row_count whose byte length overflows `usize` is rejected, not
        // wrapped into a plausible short body.
        let mut p = vec![0u8; D20_HEADER_LEN];
        p[8..12].copy_from_slice(&u32::MAX.to_le_bytes());
        assert!(d20_deleted_div_ids(&p).is_err());
    }

    #[test]
    fn a_variant_repeated_under_one_root_is_refused() {
        // `world_fold.rs:17527`-`:17533` rejects the frame outright.
        let schema = schema();
        let mut s = Vec::new();
        s.extend(record(
            1,
            100,
            D26,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &payload_with(
                0,
                &[root_row(7, 42, d26::ROOT_FLAG_TEMPLATE_PRESENT, 5, 0, 2)],
                &[entry_row(11, 100_000), entry_row(11, 200_000)],
                1,
                2,
            ),
        ));
        s.extend(frame_end(2, 100));
        assert!(scan_stream(&s, &schema).is_err());
    }

    #[test]
    fn a_stream_truncated_on_a_record_boundary_is_rejected() {
        // A cut immediately after a d26 DATA record leaves that hour open and
        // ends the stream on a record that is not a FRAME_END.
        let full = good_stream();
        let schema = schema();
        for records in [1usize, 3] {
            let cut = boundary(&full, records);
            assert!(
                scan_stream(&full[..cut], &schema).is_err(),
                "a stream cut after {records} record(s) leaves an hour open"
            );
        }
        // A cut exactly at a FRAME_END boundary IS a well-formed shorter stream
        // and `eof_check` accepts it. What matters is that it reports the hours
        // it actually closed, not the hours the untruncated file had.
        for (records, expected_hours) in [(2usize, 1usize), (4, 2)] {
            let cut = boundary(&full, records);
            let scan = scan_stream(&full[..cut], &schema).expect("closed stream is well-formed");
            assert_eq!(scan.hours.len(), expected_hours, "cut after {records}");
        }
    }

    #[test]
    fn a_stream_ending_mid_record_is_rejected() {
        let full = good_stream();
        let schema = schema();
        let boundaries: Vec<usize> = (0..=4).map(|n| boundary(&full, n)).collect();
        for cut in 1..full.len() {
            if boundaries.contains(&cut) {
                continue;
            }
            assert!(scan_stream(&full[..cut], &schema).is_err(), "mid-record cut at {cut}");
        }
        assert!(scan_stream(&[], &schema).is_err(), "an empty stream is not a measurement");
    }

    #[test]
    fn a_header_the_canonical_parser_rejects_is_rejected_here() {
        let schema = schema();
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
                    scan_stream(&s, &schema).is_err(),
                    "record {records} patched at +{offset} must be refused"
                );
            }
        }
    }

    #[test]
    fn a_sequence_discontinuity_is_rejected() {
        let schema = schema();
        for records in 0..4 {
            let at = boundary(&good_stream(), records);
            for wrong in [0u64, 9] {
                let mut s = good_stream();
                s[at + 8..at + 16].copy_from_slice(&wrong.to_le_bytes());
                assert!(scan_stream(&s, &schema).is_err(), "record {records} sequence {wrong}");
            }
        }
    }

    #[test]
    fn a_frame_end_that_does_not_close_its_open_hour_is_rejected() {
        let schema = schema();
        let mut s = Vec::new();
        s.extend(record(1, 100, D26, COMPACT_RECORD_TYPE_DOMAIN_DATA, &baseline_payload(100_000)));
        s.extend(frame_end(2, 999)); // closes an hour that is not open
        assert!(scan_stream(&s, &schema).is_err());

        let mut s = Vec::new();
        s.extend(frame_end(1, 100));
        assert!(scan_stream(&s, &schema).is_err());
    }

    #[test]
    fn non_monotonic_hours_are_rejected() {
        let schema = schema();
        let mut s = Vec::new();
        s.extend(record(1, 101, D26, COMPACT_RECORD_TYPE_DOMAIN_DATA, &baseline_payload(100_000)));
        s.extend(frame_end(2, 101));
        s.extend(record(3, 100, D26, COMPACT_RECORD_TYPE_DOMAIN_DATA, &baseline_payload(200_000)));
        s.extend(frame_end(4, 100));
        assert!(scan_stream(&s, &schema).is_err());
    }

    #[test]
    fn a_second_d26_record_in_one_hour_is_rejected() {
        // The rule is checked over EVERY d26 DATA record and before status-based
        // exclusion, so all four success/failure orderings are refused — gating
        // it on the record being usable lets a success + failure pair through.
        const FAILED: u32 = 1; // COMPACT_STATUS_STATE_UNAVAILABLE
        let schema = schema();
        for (first, second) in [(0u32, 0u32), (0, FAILED), (FAILED, 0), (FAILED, FAILED)] {
            let mut s = Vec::new();
            let mut a = baseline_payload(100_000);
            a[16..20].copy_from_slice(&first.to_le_bytes());
            let mut b = baseline_payload(200_000);
            b[16..20].copy_from_slice(&second.to_le_bytes());
            s.extend(record(1, 100, D26, COMPACT_RECORD_TYPE_DOMAIN_DATA, &a));
            s.extend(record(2, 100, D26, COMPACT_RECORD_TYPE_DOMAIN_DATA, &b));
            s.extend(frame_end(3, 100));
            assert!(
                scan_stream(&s, &schema).is_err(),
                "two d26 records in hour 100 with statuses {first:#x}/{second:#x}"
            );
        }
    }

    #[test]
    fn a_failure_status_record_is_excluded_rather_than_measured() {
        // Every failure bit in `COMPACT_STATUS_FAILURE_MASK` except the one d26
        // tolerates, one at a time: the fold retains the accumulator and folds
        // nothing, so the hour is not d26 data and must not enter either arm.
        let schema = schema();
        for bit in [0u32, 1, 2, 3, 6, 7, 8, 12] {
            let mut failed = baseline_payload(100_000);
            failed[16..20].copy_from_slice(&(1u32 << bit).to_le_bytes());
            let mut s = Vec::new();
            s.extend(record(1, 100, D26, COMPACT_RECORD_TYPE_DOMAIN_DATA, &failed));
            s.extend(frame_end(2, 100));
            s.extend(record(3, 101, D26, COMPACT_RECORD_TYPE_DOMAIN_DATA, &baseline_payload(200_000)));
            s.extend(frame_end(4, 101));
            let scan = scan_stream(&s, &schema).expect("the stream itself is well-formed");
            assert_eq!(scan.counts.failure_records, 1, "bit {bit}");
            assert_eq!(scan.hours.len(), 1, "bit {bit}");
            assert_eq!(scan.hours[0].game_hour, 101, "bit {bit}");
        }
        // And a non-failure bit (DELTA_DATA) is NOT treated as failure: it is
        // the ordinary shape of 902 of this capture's 903 d26 hours.
        let mut s = Vec::new();
        s.extend(record(1, 100, D26, COMPACT_RECORD_TYPE_DOMAIN_DATA, &baseline_payload(100_000)));
        s.extend(frame_end(2, 100));
        s.extend(record(
            3,
            101,
            D26,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &delta_payload(
                &[root_row(7, 42, d26::ROOT_FLAG_TEMPLATE_PRESENT, 5, 0, 1)],
                &[entry_row(11, 400_000)],
            ),
        ));
        s.extend(frame_end(4, 101));
        let scan = scan_stream(&s, &schema).expect("well-formed");
        assert_eq!(scan.counts.failure_records, 0);
        assert_eq!(scan.hours.len(), 2);
    }

    #[test]
    fn the_four_fold_nothing_paths_all_exclude_their_hour() {
        let schema = schema();

        // (1) a tolerated partial before any baseline, and (4) a delta before
        // any baseline: both fold nothing, so neither hour is d26 data — and
        // neither may establish a baseline for the hours that follow.
        for first in [
            payload_with(
                COMPACT_STATUS_PARTIAL_DATA,
                &[root_row(7, 42, d26::ROOT_FLAG_TEMPLATE_PRESENT, 5, 0, 1)],
                &[entry_row(11, 100_000)],
                9,
                6,
            ),
            delta_payload(
                &[root_row(7, 42, d26::ROOT_FLAG_TEMPLATE_PRESENT, 5, 0, 1)],
                &[entry_row(11, 100_000)],
            ),
        ] {
            let mut s = Vec::new();
            s.extend(record(1, 100, D26, COMPACT_RECORD_TYPE_DOMAIN_DATA, &first));
            s.extend(frame_end(2, 100));
            s.extend(record(3, 101, D26, COMPACT_RECORD_TYPE_DOMAIN_DATA, &baseline_payload(200_000)));
            s.extend(frame_end(4, 101));
            let scan = scan_stream(&s, &schema).expect("well-formed");
            assert_eq!(scan.hours.len(), 1, "only the baseline hour is measured");
            assert_eq!(scan.hours[0].game_hour, 101);
            assert_eq!(
                scan.counts.partial_before_baseline + scan.counts.delta_before_baseline,
                1
            );
        }

        // (2) a rowless tolerated partial after a baseline: `row_count == 0`
        // makes it not `sparse_partial`, and the fold marks the hour unknown
        // without folding.
        let mut s = Vec::new();
        s.extend(record(1, 100, D26, COMPACT_RECORD_TYPE_DOMAIN_DATA, &baseline_payload(100_000)));
        s.extend(frame_end(2, 100));
        s.extend(record(
            3,
            101,
            D26,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &payload_with(COMPACT_STATUS_PARTIAL_DATA, &[], &[], 9, 6),
        ));
        s.extend(frame_end(4, 101));
        let scan = scan_stream(&s, &schema).expect("well-formed");
        assert_eq!(scan.counts.rowless_partial, 1);
        assert_eq!(scan.hours.len(), 1);
        assert_eq!(scan.hours[0].game_hour, 100);
    }

    #[test]
    fn an_unassigned_status_bit_is_refused_not_classified() {
        // `compact_contract/src/lib.rs:121`-`:123`: the failure helpers and the
        // expected-partial tolerance classify the v1 vocabulary and are not
        // validity predicates, so an unassigned bit must be rejected BEFORE
        // classification — otherwise it silently reads as "not a failure", is
        // given a frame mode, and the hour is measured as healthy.
        let schema = schema();
        for bit in 14..32u32 {
            let status = 1u32 << bit;
            // Alone, combined with a known failure bit, combined with the
            // informational DELTA_DATA bit, and combined with the one failure bit
            // d26 tolerates (PARTIAL_DATA). The rejection must not depend on any
            // of it, nor on whether a baseline has been seen.
            for extra in [0u32, 1, COMPACT_STATUS_DELTA_DATA, COMPACT_STATUS_PARTIAL_DATA] {
                let frame = payload_with(
                    status | extra,
                    &[root_row(7, 42, d26::ROOT_FLAG_TEMPLATE_PRESENT, 5, 0, 1)],
                    &[entry_row(11, 100_000)],
                    1,
                    1,
                );
                let mut first = Vec::new();
                first.extend(record(1, 100, D26, COMPACT_RECORD_TYPE_DOMAIN_DATA, &frame));
                first.extend(frame_end(2, 100));
                assert!(
                    scan_stream(&first, &schema).is_err(),
                    "status bit {bit} (+{extra:#x}) as the first record must be refused"
                );
                let mut after = Vec::new();
                after.extend(record(
                    1,
                    100,
                    D26,
                    COMPACT_RECORD_TYPE_DOMAIN_DATA,
                    &baseline_payload(100_000),
                ));
                after.extend(frame_end(2, 100));
                after.extend(record(3, 101, D26, COMPACT_RECORD_TYPE_DOMAIN_DATA, &frame));
                after.extend(frame_end(4, 101));
                assert!(
                    scan_stream(&after, &schema).is_err(),
                    "status bit {bit} (+{extra:#x}) after a baseline must be refused"
                );
            }
        }
        // And the whole known v1 mask, minus the failure bits, is still accepted:
        // the check must reject unassigned bits and nothing else. DELTA_DATA is
        // the one d26 itself reads as a frame mode; PARTIAL_DATA's tolerated
        // sparse-partial path has its own test above.
        for informational in [1u32 << 4, 1 << 5, COMPACT_STATUS_DELTA_DATA, 1 << 11, 1 << 13] {
            let mut s = Vec::new();
            s.extend(record(
                1,
                100,
                D26,
                COMPACT_RECORD_TYPE_DOMAIN_DATA,
                &baseline_payload(100_000),
            ));
            s.extend(frame_end(2, 100));
            s.extend(record(
                3,
                101,
                D26,
                COMPACT_RECORD_TYPE_DOMAIN_DATA,
                &payload_with(
                    informational,
                    &[root_row(7, 42, d26::ROOT_FLAG_TEMPLATE_PRESENT, 5, 0, 1)],
                    &[entry_row(11, 200_000)],
                    if informational == COMPACT_STATUS_DELTA_DATA { 9 } else { 1 },
                    if informational == COMPACT_STATUS_DELTA_DATA { 6 } else { 1 },
                ),
            ));
            s.extend(frame_end(4, 101));
            let scan = scan_stream(&s, &schema)
                .unwrap_or_else(|e| panic!("informational bit {informational:#x}: {e}"));
            assert_eq!(scan.hours.len(), 2, "bit {informational:#x}");
            assert_eq!(scan.counts.failure_records, 0, "bit {informational:#x}");
            assert_eq!(scan.counts.delta_before_baseline, 0, "bit {informational:#x}");
        }
    }

    #[test]
    fn a_sparse_partial_hour_after_a_baseline_is_folded_and_measured() {
        // The third frame mode: rows applied to the retained accumulator, and
        // the domain-hour marked unknown. It folds, so it is measured — see the
        // module docs for why, and for what the wire format cannot express here.
        let schema = schema();
        let mut s = Vec::new();
        s.extend(record(1, 100, D26, COMPACT_RECORD_TYPE_DOMAIN_DATA, &baseline_payload(100_000)));
        s.extend(frame_end(2, 100));
        s.extend(record(
            3,
            101,
            D26,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &payload_with(
                COMPACT_STATUS_PARTIAL_DATA,
                &[root_row(8, 43, d26::ROOT_FLAG_TEMPLATE_PRESENT, 6, 0, 1)],
                &[entry_row(12, 700_000)],
                9,
                6,
            ),
        ));
        s.extend(frame_end(4, 101));
        let scan = scan_stream(&s, &schema).expect("well-formed");
        assert_eq!(scan.hours.len(), 2);
        assert_eq!(scan.counts.sparse_partial_hours, 1);
        assert!(scan.hours[1].mode.sparse_partial);
        assert!(!scan.hours[1].mode.delta);
        let states = fold_hours(&s, &scan, &schema);
        // The retained root survives and the sparse row is added: a sparse
        // partial patches, it never replaces.
        assert_eq!(states[1][0].len(), 2);
        assert_eq!(states[1][1].len(), 2);
    }

    #[test]
    fn a_stream_with_no_d26_records_is_refused_not_reported_as_zero() {
        let schema = schema();
        let mut s = Vec::new();
        s.extend(record(1, 100, 20, COMPACT_RECORD_TYPE_DOMAIN_DATA, &[0u8; 8]));
        s.extend(frame_end(2, 100));
        assert!(scan_stream(&s, &schema).is_err());
    }

    #[test]
    fn a_malformed_d26_payload_is_refused_not_skipped() {
        let schema = schema();
        // A short payload is a corrupt stream, not an hour to drop quietly.
        let mut short = baseline_payload(100_000);
        short.truncate(short.len() - 1);
        let mut s = Vec::new();
        s.extend(record(1, 100, D26, COMPACT_RECORD_TYPE_DOMAIN_DATA, &short));
        s.extend(frame_end(2, 100));
        assert!(scan_stream(&s, &schema).is_err());

        // A row the FOLD rejects must make the SCAN fail, not be accepted and
        // then unwind while measuring. The whole row-rejection space, not one
        // sampled case.
        let mut invalid: Vec<Vec<u8>> = Vec::new();
        // zero owner_tag_db_id / zero div_id (`world_fold.rs:17404`, `:17409`)
        invalid.push(payload_with(0, &[root_row(0, 42, 0, 0, 0, 0)], &[], 1, 0));
        invalid.push(payload_with(0, &[root_row(7, 0, 0, 0, 0, 0)], &[], 1, 0));
        // an unknown flag bit (`:17421`)
        invalid.push(payload_with(0, &[root_row(7, 42, 1 << 5, 0, 0, 0)], &[], 1, 0));
        // a template_id without its flag (`:17437`)
        invalid.push(payload_with(0, &[root_row(7, 42, 0, 9, 0, 0)], &[], 1, 0));
        // a delete row carrying a payload (`:17427`)
        invalid.push(payload_with(
            0,
            &[root_row(7, 42, d26::ROOT_FLAG_DELETED, 9, 0, 0)],
            &[],
            1,
            0,
        ));
        // an entry range that leaves the shared array (`:17442`)
        invalid.push(payload_with(
            0,
            &[root_row(7, 42, d26::ROOT_FLAG_TEMPLATE_PRESENT, 5, 1, 1)],
            &[entry_row(11, 100_000)],
            1,
            1,
        ));
        // declared entries that do not sum to the header count (`:17456`)
        invalid.push(payload_with(
            0,
            &[root_row(7, 42, d26::ROOT_FLAG_TEMPLATE_PRESENT, 5, 0, 0)],
            &[entry_row(11, 100_000)],
            1,
            1,
        ));
        // an invalid variant id and an out-of-range amount (`:17471`, `:17477`)
        invalid.push(payload_with(
            0,
            &[root_row(7, 42, d26::ROOT_FLAG_TEMPLATE_PRESENT, 5, 0, 1)],
            &[entry_row(0, 100_000)],
            1,
            1,
        ));
        invalid.push(payload_with(
            0,
            &[root_row(7, 42, d26::ROOT_FLAG_TEMPLATE_PRESENT, 5, 0, 1)],
            &[entry_row(11, 0)],
            1,
            1,
        ));
        // a wrong scale_raw (`:17340`)
        let mut wrong_scale = baseline_payload(100_000);
        wrong_scale[20..24].copy_from_slice(&32_768u32.to_le_bytes());
        invalid.push(wrong_scale);
        // a full frame whose row_count does not match root_rows (`:17358`)
        invalid.push(payload_with(
            0,
            &[root_row(7, 42, d26::ROOT_FLAG_TEMPLATE_PRESENT, 5, 0, 1)],
            &[entry_row(11, 100_000)],
            9,
            1,
        ));
        for (i, payload) in invalid.into_iter().enumerate() {
            let mut s = Vec::new();
            s.extend(record(1, 100, D26, COMPACT_RECORD_TYPE_DOMAIN_DATA, &payload));
            s.extend(frame_end(2, 100));
            assert!(
                scan_stream(&s, &schema).is_err(),
                "invalid payload {i} must make the scan fail"
            );
        }
    }

    #[test]
    fn scan_stream_never_panics_on_arbitrary_bytes() {
        // Total over hostile input: the harness reads a file from disk and must
        // return a diagnosis, never unwind.
        let schema = schema();
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
            let _ = scan_stream(&bytes, &schema);
        }
        // And corruption of a genuinely well-formed stream, byte by byte.
        let good = good_stream();
        for i in 0..good.len() {
            let mut s = good.clone();
            s[i] ^= 0xFF;
            let _ = scan_stream(&s, &schema);
        }
    }
}
