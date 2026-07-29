//! Measure the wire format against the shipping compact stream on REAL d56
//! `naval_missions_status_live` dynamics.
//!
//! Input: a real `collector_stream.bin` from a d56-scheduled capture
//! (`runs/batch3/deepwar_a`). The stream's own d56 DATA records are the shipping
//! format's cost for the domain: a 44-byte header, a fixed 64-byte row per task
//! force, and an optional 32-byte-per-row `NSUP` naval-supremacy trailer, exactly
//! what the recorder stores today. This tool replays those records into per-hour
//! states — the same fold `world_fold::fold_naval_missions_status` performs
//! (`world_fold.rs:22119`) — and re-encodes every hour with this crate's grammar:
//! a BASELINE at each 48-hour block start, field-granular stored-minimal DELTAs
//! between, over BOTH collections.
//!
//! **Nothing here re-derives a contract it can call.** Payload, row and trailer
//! decoding, the fold's accumulator semantics and its non-folding exits all go
//! through `wire_format::d56`'s public boundary
//! (`split_payload` / `decode_payload` / `key_from_*_row` / `record_from_*_row` /
//! `State::fold_payload`); H4CR framing, record semantics, sequence continuity and
//! status classification go through `compact_contract`'s own parser, stateful
//! validator and status predicates.
//!
//! **The whole stream is validated before any artifact is written**, fail-closed,
//! exactly as `bench_real_d63.rs` documents it: every record header through
//! `compact_contract::parse_compact_record_header`; every record through
//! `StatefulStreamValidator`; the hour-boundary and DOMAIN_PERF-hour-binding rules
//! of `recorder_host::stream_fold::BoundaryTracker` (`stream_fold.rs:277`-`:365`);
//! the FRAME_END seal fields (`:46`-`:89`) and their domain-count cross-check
//! (`:411`-`:419`); and that tracker's EOF rules (`:459`). Those two live in
//! `recorder_host`, which `wire_format` must not depend on, so this harness
//! implements their documented subset itself.
//!
//! **Exactly one production check is outside that subset, and it is a
//! crate-boundary limitation rather than a choice** — the same one
//! `bench_real_d63.rs` records. Production calls `validate_domain_data_payload_at`
//! on EVERY DOMAIN_DATA record (`recorder_host/src/stream_fold.rs:593`), decoding
//! each domain's payload grammar through `recorder_host::domain_data_authority`,
//! and a malformed payload for any domain makes the record
//! `malformed_payload_records`, which `strict_stream_certification_verdict`
//! (`:507`-`:510`) refuses. `wire_format` cannot depend on `recorder_host` (it
//! pulls in every capture lane) and this crate implements one domain's grammar, so
//! the only DOMAIN_DATA payload this harness can validate is d56's — which it does,
//! fully and fail-closed, on every record whatever its status (see
//! [`check_authority_payload`]). **Consequence, stated rather than hidden:** a
//! stream carrying a corrupt payload for some OTHER domain is refused there and
//! accepted here. The measured capture is a recorder-validated run, so its other
//! domains' payloads have already been through that check; a hand-made stream
//! carrying a corrupt non-d56 payload would measure here and not there. Closing
//! the gap would mean either depending on `recorder_host` from `wire_format` or
//! reimplementing 40-odd domains' payload grammars in this bin, and both are worse
//! than the stated limitation. That unvalidated remainder is explicitly
//! BOUNDED: every closed game hour must carry exactly one d56 DATA record of
//! any status (the `d56_records == closed_hours` completeness check in
//! [`scan_stream`]), so a d56 record relabelled to another domain id — or
//! dropped — refuses the run instead of silently shrinking it.
//!
//! **d56's exclusion order is the d83 one, not the d63 one.** The fold's first act
//! is `naval56::decode_status_payload(...)?` (`world_fold.rs:22120`-`:22122`), so
//! the ENTIRE payload — header, rows, duplicate-id rule, and the whole trailer walk
//! — is validated *before* the status word is consulted. A malformed payload is
//! therefore a **stream rejection** whatever its status; only a well-formed payload
//! can be an **excluded hour**, on one of the fold's two non-folding exits
//! (failure status `:22123`-`:22129`, delta-before-baseline `:22130`-`:22136`).
//! Those are the three outcomes, named as such in the code below.
//!
//! Outputs, for the compression step (`compress_arms.py` beside the run):
//!   ship_d56.bin   — the d56 DATA records verbatim (32-byte H4CR header +
//!                    payload), concatenated in stream order
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
    COMPACT_RECORD_TYPE_FRAME_END, COMPACT_STATUS_FAILURE_MASK,
};
use wire_format::body::{
    apply_shaped, collection_stored_eq, decode_shaped_baseline, decode_shaped_delta, diff_shaped,
    encode_shaped_baseline, encode_shaped_delta, CollectionDelta, CollectionState,
};
use wire_format::d56;
use wire_format::Schema;

const D56: u16 = 56;
const BLOCK_HOURS: usize = 48;
const COLLECTIONS: usize = 2;

/// One usable d56 hour: its game hour, the folded state of both collections, and
/// its byte span in the source stream (the shipping arm is those bytes verbatim,
/// header included).
struct D56Hour {
    game_hour: i32,
    states: Vec<CollectionState>,
    ship_span: Range<usize>,
    /// True when this measured hour is exactly one game hour after the previous
    /// MEASURED hour. False on the first measured hour and after any gap — an
    /// excluded hour (failure-status or delta-before-baseline) or a
    /// non-contiguous hour step — which is what forces the encoder to open a
    /// fresh baseline (`opens_baseline`).
    follows_previous: bool,
    /// Payload composition, for the shipping-arm byte split in the report.
    row_bytes: usize,
    /// The ported `NSUP` trailer's bytes in this record, and its row count.
    /// The trailer is the payload's suffix, so it is also what the cut shipping
    /// arm removes.
    trailer_bytes: usize,
    supremacy_rows: u32,
}

struct StreamScan {
    hours: Vec<D56Hour>,
    /// d56 records whose payload status flags mark the domain-hour failed.
    failure_records: usize,
    /// d56 records the fold would refuse as a delta before any baseline.
    delta_before_baseline_records: usize,
    /// Measured d56 hours that do not follow the previous measured one by one.
    non_contiguous: usize,
}

/// Whether the wire arm opens a BASELINE at this measured hour.
///
/// Two reasons, and both are `WIRE_FORMAT.md` rules, not conveniences:
/// the 48-hour block cadence (every block opens with its own baseline, §1.5's
/// closing paragraph), and a gap — `WIRE_FORMAT.md:187`-`:190`: "A gap
/// affecting the state part forces the next state observation to be a
/// `BASELINE`." Every hour this domain did not successfully sample is such a
/// gap — the excluded failure-status and delta-before-baseline hours, and any
/// non-contiguous hour step — and d56's cadence is hourly, so a measured hour
/// that does not follow the previous measured one by exactly one is preceded
/// by one. Neither a delta nor a retained reader state can be justified
/// across it.
fn opens_baseline(hours_seen: usize, follows_previous: bool) -> bool {
    hours_seen % BLOCK_HOURS == 0 || !follows_previous
}

/// One wire-arm hour: the §1.5/block baseline decision, the encode, and the
/// per-hour reader replay over BOTH collections. Factored out of `main` so the
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
    changed_by_collection: Vec<usize>,
    tombstones_by_collection: Vec<usize>,
    replaced_by_collection: Vec<bool>,
}

fn encode_wire_hour(
    schema: &wire_format::Schema,
    hours_seen: usize,
    hour: &D56Hour,
    prior: Option<&[CollectionState]>,
    reader: &mut Option<Vec<CollectionState>>,
    wire: &mut Vec<u8>,
) -> WireHourOutcome {
    let states = &hour.states;
    let hour_start = wire.len();
    // BASELINE at each block start — and at every §1.5 gap, which forces a
    // fresh baseline and restarts the delta chain (`opens_baseline`).
    let block_start = opens_baseline(hours_seen, hour.follows_previous);
    let gap_forced = block_start && hours_seen % BLOCK_HOURS != 0;
    let mut emitted = true;
    let mut changed_by_collection = vec![0usize; schema.collections.len()];
    let mut tombstones_by_collection = vec![0usize; schema.collections.len()];
    let mut replaced_by_collection = vec![false; schema.collections.len()];
    let opened_baseline = block_start || prior.is_none();
    if opened_baseline {
        encode_shaped_baseline(schema, states, wire).expect("baseline encodes");
    } else {
        let prev = prior.unwrap();
        let deltas: Vec<Option<CollectionDelta>> = schema
            .collections
            .iter()
            .enumerate()
            .map(|(i, collection)| {
                diff_shaped(collection, &prev[i], &states[i]).expect("fold state matches schema")
            })
            .collect();
        if deltas.iter().all(Option::is_none) {
            emitted = false;
        } else {
            for (i, d) in deltas.iter().enumerate() {
                if let Some(d) = d {
                    match d {
                        CollectionDelta::Keyed(d) => {
                            changed_by_collection[i] = d.changed.len();
                            tombstones_by_collection[i] = d.tombstones.len();
                        }
                        CollectionDelta::OrderedKeyed(rows) => {
                            changed_by_collection[i] = rows.len();
                            replaced_by_collection[i] = true;
                        }
                        CollectionDelta::OrderedOccurrence(rows) => {
                            changed_by_collection[i] = rows.len();
                            replaced_by_collection[i] = true;
                        }
                    }
                }
            }
            encode_shaped_delta(schema, prev, &deltas, wire).expect("delta encodes");
        }
    }
    let bytes = wire.len() - hour_start;

    // Reader replay: decode the bytes just written against the reader's OWN
    // reconstructed prior, exactly as a conforming reader would, and require
    // the result to be stored-equal to this hour's state in BOTH collections.
    // An hour that emitted nothing is still checked: the reader's retained
    // state must already equal the fold's, which is what "nothing changed"
    // asserts.
    if emitted {
        let hour_bytes = &wire[hour_start..];
        if block_start || reader.is_none() {
            let (decoded, consumed) =
                decode_shaped_baseline(schema, hour_bytes, 0).expect("baseline decodes");
            assert_eq!(consumed, hour_bytes.len(), "baseline consumed exactly");
            *reader = Some(decoded);
        } else {
            let reader_prior = reader.take().unwrap();
            let (deltas, consumed) =
                decode_shaped_delta(schema, &reader_prior, hour_bytes, 0).expect("delta decodes");
            assert_eq!(consumed, hour_bytes.len(), "delta consumed exactly");
            let mut next = reader_prior;
            for (i, d) in deltas.iter().enumerate() {
                if let Some(d) = d {
                    apply_shaped(&mut next[i], d).expect("delta shape matches state");
                }
            }
            *reader = Some(next);
        }
    }
    let replayed = reader.as_ref().unwrap();
    for (i, collection) in schema.collections.iter().enumerate() {
        assert_eq!(
            replayed[i].len(),
            states[i].len(),
            "game hour {}: collection {} row count",
            hour.game_hour,
            collection.collection_id
        );
        assert!(
            collection_stored_eq(collection, &replayed[i], &states[i])
                .expect("states encode"),
            "game hour {}: collection {} reader state is not stored-equal to the fold",
            hour.game_hour,
            collection.collection_id
        );
    }
    WireHourOutcome {
        opened_baseline,
        gap_forced,
        emitted,
        bytes,
        changed_by_collection,
        tombstones_by_collection,
        replaced_by_collection,
    }
}

/// Every globally-defined compact status occupies one of bits 0..=13
/// (`recorder_host/src/stream_fold.rs:43`).
const FRAME_END_KNOWN_STATUS_MASK: u32 = (1 << 14) - 1;

/// The reader-context probe domain, excluded from the seal's domain accounting
/// (`recorder_host/src/stream_fold.rs:330`).
const PROBE_DOMAIN_ID: u16 = 250;

/// The FRAME_END seal check, re-implemented from
/// `recorder_host::stream_fold::validate_frame_end_seal_fields` (`:46`-`:89`)
/// because `wire_format` must not depend on `recorder_host`. Returns the seal's
/// `(emitted_domains, scheduled_domains)` for production's cross-check
/// (`:411`-`:419`). Total: no payload of any length panics.
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

/// The d56 checks production's **second** authority makes on every DOMAIN_DATA
/// record — status-independent, and therefore also on the records this harness
/// excludes.
///
/// Production runs two independent passes over a d56 DATA record and refuses the
/// stream if EITHER complains:
///
/// 1. `WorldFold::ingest_record` → `fold_naval_missions_status`, whose `Err`
///    aborts the stream fold. Its whole decode runs BEFORE its status early exit
///    (`world_fold.rs:22120`-`:22127`), so an excluded record's rows and trailer
///    have already been validated there.
/// 2. `validate_domain_data_payload_at` → `decode_compact_naval_missions_status_live_data_payload`
///    (`recorder_host/src/domain_data_authority.rs:8628`-`:8653`), which calls the
///    same lane decoder on EVERY DOMAIN_DATA record whatever its status; any error
///    it reports makes the record `malformed_payload_records`, which
///    `strict_stream_certification_verdict` refuses.
///
/// Both are `naval56::decode_status_payload`/`decode_payload`, and
/// `d56::decode_payload` is this crate's single implementation of it — header,
/// every row, the duplicate-`task_force_id` rule (which for d56 lives in the
/// DECODER, `domain56_compact.rs:269`-`:274`, not in the fold, so unlike d63's it
/// applies to excluded records too) and the whole trailer walk.
fn check_authority_payload(schema: &Schema, payload: &[u8], at_byte: usize) -> Result<(), String> {
    d56::decode_payload(schema, payload, 0)
        .map(|_| ())
        .map_err(|e| format!("d56 payload at byte {at_byte}: {e}"))
}

/// Validate the whole stream against the canonical contract and fold its usable
/// d56 DATA records into per-hour states. Every rejection path is an `Err`, never
/// a silently shorter measurement and never a panic.
fn scan_stream(
    stream: &[u8],
    schema: &Schema,
    identity_mode: StreamIdentityMode,
) -> Result<StreamScan, String> {
    // The identity protocol is an INPUT, never sniffed: an `IdentityV1` stream
    // whose mandatory CID1 record has been spliced off is byte-indistinguishable
    // from a legacy stream, so a sniffing harness measures a decapitated stream
    // that production refuses (`stream_validator.rs:328`, `stream_fold.rs:220`).
    let mut validator = match identity_mode {
        StreamIdentityMode::IdentityV1 => StatefulStreamValidator::identity_v1(),
        StreamIdentityMode::Legacy => StatefulStreamValidator::legacy(),
    };

    let mut hours: Vec<D56Hour> = Vec::new();
    let mut failure_records = 0usize;
    let mut delta_before_baseline_records = 0usize;
    let mut non_contiguous = 0usize;
    // COMPLETENESS accounting: every d56 DATA record of ANY status, against
    // every closed game hour — see the check after the EOF rules.
    let mut d56_records = 0usize;
    let mut closed_hours = 0usize;

    // The fold's accumulator, carried across hours: d56 has a delta grammar (a
    // delta frame upserts without clearing, `world_fold.rs:22138`-`:22144`), so an
    // hour's state is the accumulator after that hour, not the hour's own rows.
    let mut fold_state = d56::State::new();

    let mut open_hour: Option<i32> = None;
    let mut last_closed_hour: Option<i32> = None;
    let mut last_record_type: Option<u16> = None;
    // The DOMAIN_PERF hour-binding state, `BoundaryTracker`'s own
    // (`stream_fold.rs:328`-`:365`, reset at `:449`-`:451`).
    let mut pending_perf_hour: Option<i32> = None;
    let mut non_probe_perf_domains: BTreeSet<u16> = BTreeSet::new();
    // Hour of the last d56 DATA record of ANY status — the uniqueness rule.
    let mut last_d56_hour: Option<i32> = None;
    // Hour of the last d56 record that entered the measurement — the contiguity
    // report, which a skipped excluded hour legitimately breaks.
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
                    // `stream_fold.rs:378`-`:386`, and it comes BEFORE the
                    // monotonicity check, as it does there.
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

        if header.domain_id == D56 && header.record_type == COMPACT_RECORD_TYPE_DOMAIN_DATA {
            d56_records += 1;
            // One d56 DATA record per game hour, checked over EVERY d56 record and
            // BEFORE any status-based exclusion: gating this on the record being
            // usable would let a success/failure pair in one hour through the rule
            // entirely.
            //
            // DELIBERATELY STRICTER THAN PRODUCTION, and fail-closed. The
            // production boundary tracker permits several DOMAIN_DATA records
            // while an hour is open (`recorder_host/src/stream_fold.rs:369`) and
            // `WorldFold::ingest_record` would fold each in turn, so a same-hour
            // pair is a stream production certifies and this harness refuses. That
            // is the recipe's standing rule for every measurement bin ("Enforce
            // hour uniqueness across ALL of your domain's records — including
            // failure-status ones — BEFORE status exclusion",
            // `DOMAIN_SCHEMA_RECIPE.md` step 9): a second record for one hour
            // would otherwise become a second measurement hour built from the same
            // clock time, and for a full-replace domain the two would silently
            // disagree about what that hour's state is. Refusing produces no
            // number; measuring would produce a wrong one. No capture carries such
            // a pair.
            if let Some(previous) = last_d56_hour {
                if header.game_hour_raw <= previous {
                    return Err(format!(
                        "d56 DATA at byte {pos} carries hour {} after hour {previous}",
                        header.game_hour_raw
                    ));
                }
            }
            last_d56_hour = Some(header.game_hour_raw);

            // VALIDITY BEFORE CLASSIFICATION. The failure predicate classifies the
            // assigned v1 status vocabulary and is not a validity predicate
            // (`compact_contract/src/lib.rs:121`-`:123`); bits 14..=31 are
            // unassigned (`status_flags_base.rs:21`-`:24`) and an unassigned bit
            // "is not evidence of a future informational status", so a record
            // carrying one refuses the stream rather than reading as "not a
            // failure" and being measured as healthy.
            //
            // ALSO DELIBERATELY STRICTER THAN PRODUCTION, and fail-closed. d56's
            // lane decoder rejects only the retired bits 16..=19
            // (`domain56_compact.rs:232`-`:237`) and the fold reads the word only
            // through the failure and delta predicates, so production would fold a
            // record carrying, say, bit 14 — and would classify it as a success by
            // the very masking this rule exists to distrust. This is the recipe's
            // standing rule for every bin written since d16 ("Reject an unassigned
            // status bit BEFORE classifying the record",
            // `DOMAIN_SCHEMA_RECIPE.md` step 9), and it is a measurement rule, not
            // a claim about what the recorder must do: measuring an hour whose
            // status word has no defined classification would publish a number
            // whose meaning depends on a bit nobody has assigned. Batch-3's survey
            // records d53 shipping unassigned bits on 379 of 514 hours, which is
            // what the rule catches. No d56 record in any capture carries one, so
            // no measured number depends on this.
            let split = d56::split_payload(payload)
                .map_err(|e| format!("d56 payload at byte {pos}: {e}"))?;
            let unknown_status = compact_status_unknown_flags_v1(split.header.status_flags);
            if unknown_status != 0 {
                return Err(format!(
                    "d56 DATA at byte {pos}: hour {} carries unassigned v1 status bits \
                     {unknown_status:#x}; an unassigned bit has no defined classification",
                    header.game_hour_raw
                ));
            }

            // Production's SECOND authority, on every record whatever its status.
            // For d56 it is the whole lane decode, so an excluded record's rows,
            // duplicate-id rule and trailer are all checked here.
            check_authority_payload(schema, payload, pos)?;

            // The fold's own classification: `compact_contract`'s predicate, not a
            // copied bit mask — it already applies d56's lane-local mask (none) and
            // expected partial tolerance (none), so it cannot drift.
            let status_is_failure = compact_status_failure_flags_after_expected_partial_tolerance(
                D56,
                split.header.status_flags,
            ) != 0;

            // The fold, through the domain module: an `Err` is the payload not
            // being this grammar (a stream rejection, already impossible here
            // because the authority check ran first), and the two non-folding
            // exits are excluded hours.
            match fold_state
                .fold_payload(schema, payload, status_is_failure, 0)
                .map_err(|e| format!("d56 payload at byte {pos}: {e}"))?
            {
                d56::FoldOutcome::FailureStatus => failure_records += 1,
                d56::FoldOutcome::DeltaBeforeBaseline => delta_before_baseline_records += 1,
                d56::FoldOutcome::Folded => {
                    let follows_previous = last_measured_hour
                        .is_some_and(|previous| header.game_hour_raw == previous + 1);
                    if last_measured_hour.is_some() && !follows_previous {
                        non_contiguous += 1;
                    }
                    last_measured_hour = Some(header.game_hour_raw);
                    hours.push(D56Hour {
                        game_hour: header.game_hour_raw,
                        states: fold_state.states().to_vec(),
                        ship_span: pos..record_end,
                        follows_previous,
                        row_bytes: split.rows.len(),
                        trailer_bytes: payload.len() - d56::HEADER_LEN - split.rows.len(),
                        supremacy_rows: fold_state.naval_supremacy.len() as u32,
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
        return Err("stream carries no usable d56 DATA records".to_string());
    }

    // COMPLETENESS: every closed game hour must carry exactly one d56 DATA
    // record, of any status. This bounds what this harness canNOT validate:
    // production runs `validate_domain_data_payload_at` on EVERY domain's DATA
    // payload (`stream_fold.rs:593`), while this harness owns only d56's
    // grammar, so a d56 record whose header was relabelled to another domain
    // id — or lost outright — would otherwise be silently ignored and the run
    // would report a plausible measurement over fewer hours. A per-capture
    // completeness assertion, NOT a grammar rule: a capture where d56 is not
    // scheduled every hour fails here loudly and a human adjusts, instead of a
    // number quietly shrinking (`bench_real_d65.rs` precedent).
    if d56_records != closed_hours {
        return Err(format!(
            "stream closes {closed_hours} game hour(s) but carries {d56_records} d56 DATA \
             record(s); every closed hour of a d56-scheduled capture must carry exactly one, \
             and this harness cannot validate the payloads of other domains to see where the \
             difference went"
        ));
    }

    Ok(StreamScan {
        hours,
        failure_records,
        delta_before_baseline_records,
        non_contiguous,
    })
}

fn main() {
    const USAGE: &str = "usage: bench_real_d56 <collector_stream.bin> <out dir> \
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
    let schema = d56::schema();

    // ---- Pass 1: validate the WHOLE stream and fold its d56 DATA records. ----
    let scan = match scan_stream(&stream, &schema, identity_mode) {
        Ok(scan) => scan,
        Err(reason) => {
            eprintln!("{input}: {reason}");
            std::process::exit(1);
        }
    };
    std::fs::create_dir_all(&out_dir).expect("out dir");

    // ---- Pass 2: encode both arms. ------------------------------------------

    let mut ship = Vec::new();
    let mut ship_offsets = vec![0usize];
    // The second, CUT shipping arm. The formerly stopped `NSUP` trailer is a suffix
    // of each payload (the decoder's walk rejects anything after it,
    // `domain56_compact.rs:370`-`:388`), so it is cuttable, and d11's `PDQ1`
    // accounting applies: each record with the trailer removed and its payload
    // length left as written. **That is a size-accounting artifact, not a valid
    // record** — a decoder handed this arm would read a length past the bytes —
    // and it is retained only to compare against the previously published cut-arm
    // numbers. The complete shipping arm is now the honest headline.
    let mut ship_cut = Vec::new();
    let mut ship_cut_offsets = vec![0usize];
    let mut wire = Vec::new();
    let mut wire_offsets = vec![0usize];

    let mut prior: Option<Vec<CollectionState>> = None;
    let mut reader: Option<Vec<CollectionState>> = None;
    let mut hours_seen = 0usize;
    let mut baselines = 0usize;
    let mut gap_forced_baselines = 0usize;
    let mut baseline_bytes = 0usize;
    let mut delta_bytes = 0usize;
    let mut empty_hours = 0usize;
    // Cadence-independent churn: changed rows over MEASURED-HOUR TRANSITIONS,
    // taken before the block decision, so a change landing on a block boundary
    // still counts.
    let mut transition_changed = [0usize; COLLECTIONS];
    let mut transition_tombstones = [0usize; COLLECTIONS];
    let mut transition_replacements = [0usize; COLLECTIONS];
    let mut transitions = 0usize;
    // Delta-path churn: the same counts over the hours that actually emitted a
    // delta. The denominators are different questions and cannot share a number.
    let mut delta_changed = [0usize; COLLECTIONS];
    let mut delta_tombstones = [0usize; COLLECTIONS];
    let mut delta_replacements = [0usize; COLLECTIONS];
    let mut rows_total = [0usize; COLLECTIONS];
    let mut ship_row_bytes = 0usize;
    let mut ship_trailer_bytes = 0usize;
    let mut supremacy_rows_total = 0u64;

    for hour in &scan.hours {
        let record = &stream[hour.ship_span.clone()];
        ship.extend_from_slice(record);
        ship_offsets.push(ship.len());
        ship_cut.extend_from_slice(&record[..record.len() - hour.trailer_bytes]);
        ship_cut_offsets.push(ship_cut.len());
        ship_row_bytes += hour.row_bytes;
        ship_trailer_bytes += hour.trailer_bytes;
        supremacy_rows_total += hour.supremacy_rows as u64;

        let states = &hour.states;
        for i in 0..COLLECTIONS {
            rows_total[i] += states[i].len();
        }

        // Cadence-independent churn, before the block decision.
        if let Some(prev) = &prior {
            transitions += 1;
            for (i, collection) in schema.collections.iter().enumerate() {
                if let Some(delta) =
                    diff_shaped(collection, &prev[i], &states[i]).expect("states encode")
                {
                    match delta {
                        CollectionDelta::Keyed(delta) => {
                            transition_changed[i] += delta.changed.len();
                            transition_tombstones[i] += delta.tombstones.len();
                        }
                        CollectionDelta::OrderedKeyed(rows) => {
                            transition_changed[i] += rows.len();
                            transition_replacements[i] += 1;
                        }
                        CollectionDelta::OrderedOccurrence(rows) => {
                            transition_changed[i] += rows.len();
                            transition_replacements[i] += 1;
                        }
                    }
                }
            }
        }

        // The wire-arm hour — baseline decision, encode and reader replay —
        // through the SAME function the tests execute (`encode_wire_hour`).
        let outcome = encode_wire_hour(
            &schema,
            hours_seen,
            hour,
            prior.as_deref(),
            &mut reader,
            &mut wire,
        );
        if outcome.gap_forced {
            gap_forced_baselines += 1;
        }
        if outcome.opened_baseline {
            baselines += 1;
            baseline_bytes += outcome.bytes;
        } else if outcome.emitted {
            for i in 0..schema.collections.len() {
                delta_changed[i] += outcome.changed_by_collection[i];
                delta_tombstones[i] += outcome.tombstones_by_collection[i];
                delta_replacements[i] += usize::from(outcome.replaced_by_collection[i]);
            }
            delta_bytes += outcome.bytes;
        } else {
            empty_hours += 1;
        }
        wire_offsets.push(wire.len());

        prior = Some(states.clone());
        hours_seen += 1;
    }

    for (name, bytes) in [
        ("ship_d56.bin", &ship),
        ("ship_d56_no_nsup.bin", &ship_cut),
        ("wire_new.bin", &wire),
    ] {
        let mut f = std::fs::File::create(std::path::Path::new(&out_dir).join(name)).unwrap();
        f.write_all(bytes).unwrap();
    }
    for (name, offsets) in [
        ("ship_d56.offsets", &ship_offsets),
        ("ship_d56_no_nsup.offsets", &ship_cut_offsets),
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
        "excluded hours       {} failure-status, {} delta-before-baseline",
        scan.failure_records, scan.delta_before_baseline_records
    );
    println!(
        "task forces avg      {:.1}   supremacy occurrence rows avg {:.1}, total {}",
        rows_total[0] as f64 / hours_seen as f64,
        supremacy_rows_total as f64 / hours_seen as f64,
        supremacy_rows_total
    );
    println!(
        "baselines            {baselines} ({} at a {BLOCK_HOURS}-h block start, \
         {gap_forced_baselines} forced by a WIRE_FORMAT \u{a7}1.5 state gap)",
        baselines - gap_forced_baselines
    );
    println!("delta hours          {delta_hours}   empty hours {empty_hours}");
    println!(
        "wire raw split       baseline {baseline_bytes} B, delta {delta_bytes} B ({:.1}% baseline)",
        100.0 * baseline_bytes as f64 / (baseline_bytes + delta_bytes).max(1) as f64
    );
    println!(
        "churn cadence-indep  task_forces {:.2} changed + {:.3} tombstones per transition ({transitions} transitions)",
        transition_changed[0] as f64 / transitions.max(1) as f64,
        transition_tombstones[0] as f64 / transitions.max(1) as f64,
    );
    println!(
        "churn delta-path     task_forces {:.2} changed + {:.3} tombstones per delta hour ({delta_hours} delta hours)",
        delta_changed[0] as f64 / delta_hours.max(1) as f64,
        delta_tombstones[0] as f64 / delta_hours.max(1) as f64,
    );
    println!(
        "ordered replacements naval_supremacy {} events / {transitions} measured transitions, \
         {} events / {delta_hours} delta hours; {:.2} rows per measured transition, {:.2} rows \
         per delta hour",
        transition_replacements[1],
        delta_replacements[1],
        transition_changed[1] as f64 / transitions.max(1) as f64,
        delta_changed[1] as f64 / delta_hours.max(1) as f64,
    );
    // The newly ported segment, accounted for both ways so the old cut-arm
    // measurement remains directly comparable.
    println!(
        "ship_d56.bin         {} bytes ({} B task-force rows {:.1}%, {} B ported NSUP trailer {:.1}%, {} B payload header + H4CR framing {:.1}%)",
        ship.len(),
        ship_row_bytes,
        100.0 * ship_row_bytes as f64 / ship.len() as f64,
        ship_trailer_bytes,
        100.0 * ship_trailer_bytes as f64 / ship.len() as f64,
        ship.len() - ship_row_bytes - ship_trailer_bytes,
        100.0 * (ship.len() - ship_row_bytes - ship_trailer_bytes) as f64 / ship.len() as f64,
    );
    println!(
        "ship_d56_no_nsup.bin {} bytes (retired headline arm: retained only for before/after comparability; not a valid record stream)",
        ship_cut.len()
    );
    println!(
        "wire_new.bin         {} bytes ({:.1}% of shipping, {:.1}% of the cut arm)",
        wire.len(),
        100.0 * wire.len() as f64 / ship.len() as f64,
        100.0 * wire.len() as f64 / ship_cut.len() as f64
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use compact_contract::{COMPACT_DOMAIN_FRAME, COMPACT_STATUS_DELTA_DATA};

    fn schema() -> Schema {
        d56::schema()
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

    /// One 64-byte task-force row with a live location gate.
    fn task_force_row(task_force_id: u32, location: u32) -> Vec<u8> {
        let words: [u32; 16] = [
            task_force_id,
            615,
            47,
            d56::FLAG_VALIDATED_DIRECT | d56::FLAG_HAS_LOCATION,
            0,
            0,
            location,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
        ];
        let mut row = Vec::new();
        for w in words {
            row.extend_from_slice(&w.to_le_bytes());
        }
        row
    }

    fn supremacy_row(region: u32, controller: u32, current: u32) -> Vec<u8> {
        let words: [u32; 8] = [region, controller, current, current, current, current, current, 1];
        let mut row = Vec::new();
        for w in words {
            row.extend_from_slice(&w.to_le_bytes());
        }
        row
    }

    /// A d56 DATA payload: header, rows, and an `NSUP` trailer when non-empty.
    fn d56_payload(status_flags: u32, rows: &[Vec<u8>], supremacy: &[Vec<u8>]) -> Vec<u8> {
        let mut p = Vec::new();
        p.extend_from_slice(&0u64.to_le_bytes()); // elapsed_wall_ms
        p.extend_from_slice(&(rows.len() as u32).to_le_bytes());
        p.extend_from_slice(&(rows.len() as u32).to_le_bytes()); // changed_rows
        p.extend_from_slice(&status_flags.to_le_bytes());
        p.extend_from_slice(&(rows.len() as u32).to_le_bytes()); // observed
        p.extend_from_slice(&0u32.to_le_bytes());
        p.extend_from_slice(&0u32.to_le_bytes());
        p.extend_from_slice(&0u32.to_le_bytes());
        p.extend_from_slice(&0u32.to_le_bytes());
        p.extend_from_slice(&d56::SCHEMA_VERSION.to_le_bytes());
        for row in rows {
            p.extend_from_slice(row);
        }
        if !supremacy.is_empty() {
            p.extend_from_slice(&d56::TRAILER_MAGIC);
            p.extend_from_slice(&d56::TRAILER_VERSION.to_le_bytes());
            p.extend_from_slice(&(d56::TRAILER_HEADER_LEN as u16).to_le_bytes());
            p.extend_from_slice(&(supremacy.len() as u32).to_le_bytes());
            for row in supremacy {
                p.extend_from_slice(row);
            }
        }
        p
    }

    fn one_row_payload(status_flags: u32, location: u32) -> Vec<u8> {
        d56_payload(
            status_flags,
            &[task_force_row(617, location)],
            &[supremacy_row(16, 2, 100_000)],
        )
    }

    /// Two closed hours, each carrying one d56 DATA record.
    fn good_stream() -> Vec<u8> {
        let mut s = Vec::new();
        s.extend(record(
            1,
            100,
            D56,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &one_row_payload(0, 12_389),
        ));
        s.extend(frame_end(2, 100));
        s.extend(record(
            3,
            101,
            D56,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &one_row_payload(0, 12_390),
        ));
        s.extend(frame_end(4, 101));
        s
    }

    fn boundary(stream: &[u8], records: usize) -> usize {
        let mut pos = 0usize;
        for _ in 0..records {
            let h = parse_compact_record_header(&stream[pos..]).expect("framed");
            pos += COMPACT_RECORD_HEADER_LEN + h.payload_len;
        }
        pos
    }

    #[test]
    fn the_library_constants_agree_with_the_shipping_contract() {
        // `d56.rs` names these locally because d56 is not admitted to the
        // production-codec dependency surface (the d24/d32 pattern); this is
        // the assertion that makes that safe.
        assert_eq!(d56::STATUS_DELTA_DATA, COMPACT_STATUS_DELTA_DATA);
        assert_eq!(
            d56::MAX_OWNER_TAG_DB_ID,
            compact_contract::build_profile::MAX_COMPACT_OWNER_TAG_DB_ID
        );
    }

    #[test]
    fn a_well_formed_stream_yields_its_d56_hours() {
        let scan = scan_stream(&good_stream(), &schema(), StreamIdentityMode::Legacy)
            .expect("accepted");
        assert_eq!(scan.hours.len(), 2);
        assert_eq!(scan.hours[0].game_hour, 100);
        assert_eq!(scan.hours[1].game_hour, 101);
        assert_eq!(scan.failure_records, 0);
        assert_eq!(scan.delta_before_baseline_records, 0);
        assert_eq!(scan.non_contiguous, 0);
        assert_eq!(scan.hours[0].states.len(), 2, "both ported collections");
        assert_eq!(scan.hours[0].states[0].len(), 1);
        assert_eq!(scan.hours[0].states[1].len(), 1);
        assert_eq!(
            scan.hours[0].supremacy_rows, 1,
            "the ported trailer row is counted and folded into its collection"
        );
    }

    #[test]
    fn an_unassigned_status_bit_is_refused_not_classified() {
        for bit in 14..32u32 {
            let status = 1u32 << bit;
            for extra in [0u32, 1, COMPACT_STATUS_DELTA_DATA] {
                for (label, payload) in [
                    ("rowless", d56_payload(status | extra, &[], &[])),
                    ("one row", one_row_payload(status | extra, 12_389)),
                ] {
                    let mut s = Vec::new();
                    s.extend(record(1, 100, D56, COMPACT_RECORD_TYPE_DOMAIN_DATA, &payload));
                    s.extend(frame_end(2, 100));
                    assert!(
                        scan_stream(&s, &schema(), StreamIdentityMode::Legacy).is_err(),
                        "status bit {bit} (+{extra:#x}), {label}, must be refused"
                    );
                }
            }
        }
        // And the known v1 mask minus the failure bits is still accepted: the
        // check must reject unassigned bits and nothing else. Bit 10 (DELTA_DATA)
        // is excluded here — on a fresh accumulator it is a legitimate
        // delta-before-baseline exclusion, which its own test covers.
        for informational in [1u32 << 4, 1 << 5, 1 << 11, 1 << 13] {
            let mut s = Vec::new();
            s.extend(record(
                1,
                100,
                D56,
                COMPACT_RECORD_TYPE_DOMAIN_DATA,
                &one_row_payload(informational, 12_389),
            ));
            s.extend(frame_end(2, 100));
            let scan = scan_stream(&s, &schema(), StreamIdentityMode::Legacy)
                .unwrap_or_else(|e| panic!("informational bit {informational:#x} refused: {e}"));
            assert_eq!(scan.hours.len(), 1);
        }
    }

    #[test]
    fn a_stream_truncated_on_a_record_boundary_is_rejected_unless_it_closes_an_hour() {
        // A stream cut exactly at a FRAME_END boundary is a well-formed shorter
        // stream and is accepted; cut anywhere else it is not.
        let good = good_stream();
        let after_first_hour = boundary(&good, 2);
        let scan = scan_stream(&good[..after_first_hour], &schema(), StreamIdentityMode::Legacy)
            .expect("a stream cut at a FRAME_END boundary is well formed");
        assert_eq!(scan.hours.len(), 1);
        // Cut after the DATA record of the second hour: the hour never closes.
        let after_second_data = boundary(&good, 3);
        assert!(scan_stream(
            &good[..after_second_data],
            &schema(),
            StreamIdentityMode::Legacy
        )
        .is_err());
    }

    #[test]
    fn a_stream_ending_mid_record_is_rejected() {
        let good = good_stream();
        for cut in [good.len() - 1, good.len() - 16, boundary(&good, 1) + 8] {
            assert!(
                scan_stream(&good[..cut], &schema(), StreamIdentityMode::Legacy).is_err(),
                "cut at {cut}"
            );
        }
    }

    #[test]
    fn a_sequence_discontinuity_is_rejected() {
        let mut s = Vec::new();
        s.extend(record(
            1,
            100,
            D56,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &one_row_payload(0, 12_389),
        ));
        s.extend(frame_end(9, 100)); // sequence jumps
        assert!(scan_stream(&s, &schema(), StreamIdentityMode::Legacy).is_err());
    }

    #[test]
    fn a_frame_end_that_does_not_close_its_open_hour_is_rejected() {
        let mut s = Vec::new();
        s.extend(record(
            1,
            100,
            D56,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &one_row_payload(0, 12_389),
        ));
        s.extend(frame_end(2, 101)); // closes a different hour
        assert!(scan_stream(&s, &schema(), StreamIdentityMode::Legacy).is_err());
        // And a FRAME_END with no open hour at all.
        let mut orphan = Vec::new();
        orphan.extend(frame_end(1, 100));
        assert!(scan_stream(&orphan, &schema(), StreamIdentityMode::Legacy).is_err());
    }

    #[test]
    fn non_monotonic_hours_are_rejected() {
        let mut s = Vec::new();
        s.extend(record(
            1,
            101,
            D56,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &one_row_payload(0, 12_389),
        ));
        s.extend(frame_end(2, 101));
        s.extend(record(
            3,
            100,
            D56,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &one_row_payload(0, 12_389),
        ));
        s.extend(frame_end(4, 100));
        assert!(scan_stream(&s, &schema(), StreamIdentityMode::Legacy).is_err());
    }

    #[test]
    fn a_second_d56_record_in_one_hour_is_rejected_whatever_its_status() {
        // Including a success/failure pair, which is why the uniqueness rule runs
        // over EVERY d56 record and before the status exclusion.
        for second_status in [0u32, 1 /* a failure bit */] {
            let mut s = Vec::new();
            s.extend(record(
                1,
                100,
                D56,
                COMPACT_RECORD_TYPE_DOMAIN_DATA,
                &one_row_payload(0, 12_389),
            ));
            s.extend(record(
                2,
                100,
                D56,
                COMPACT_RECORD_TYPE_DOMAIN_DATA,
                &one_row_payload(second_status, 12_390),
            ));
            s.extend(frame_end(3, 100));
            assert!(
                scan_stream(&s, &schema(), StreamIdentityMode::Legacy).is_err(),
                "second record with status {second_status:#x}"
            );
        }
    }

    #[test]
    fn an_excluded_hour_forces_the_next_observation_to_be_a_baseline() {
        // `WIRE_FORMAT.md:187`-`:190`: "A gap affecting the state part forces
        // the next state observation to be a `BASELINE`." Both gap shapes d56
        // can meet — an excluded failure-status hour, and a whole hour the
        // capture never scheduled — must clear `follows_previous`, and
        // `opens_baseline` must open a baseline on it wherever it lands in a
        // block. A delta would relate two states across unobserved time.
        //
        // Shape 1: hour 101 carries a failure-status d56 record (excluded).
        let mut s = Vec::new();
        s.extend(record(
            1,
            100,
            D56,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &one_row_payload(0, 12_389),
        ));
        s.extend(frame_end(2, 100));
        s.extend(record(
            3,
            101,
            D56,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &one_row_payload(1 << 0, 12_389), // failure status -> excluded
        ));
        s.extend(frame_end(4, 101));
        s.extend(record(
            5,
            102,
            D56,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &one_row_payload(0, 12_390),
        ));
        s.extend(frame_end(6, 102));
        let scan =
            scan_stream(&s, &schema(), StreamIdentityMode::Legacy).expect("well-formed");
        assert_eq!(scan.hours.len(), 2);
        assert_eq!(scan.failure_records, 1);
        assert_eq!(scan.non_contiguous, 1);
        assert!(!scan.hours[0].follows_previous, "the first hour follows none");
        assert!(
            !scan.hours[1].follows_previous,
            "hour 102 does not follow hour 100"
        );
        // ...and that is what makes the encoder open a baseline for it, even
        // though it is only the second measured hour of the block — through the
        // PRODUCTION encode path, not just the decision function.
        assert!(opens_baseline(1, scan.hours[1].follows_previous));
        let full_schema = schema();
        let mut wire = Vec::new();
        let mut reader: Option<Vec<CollectionState>> = None;
        let o0 = encode_wire_hour(&full_schema, 0, &scan.hours[0], None, &mut reader, &mut wire);
        assert!(o0.opened_baseline && !o0.gap_forced);
        let o1 = encode_wire_hour(
            &full_schema,
            1,
            &scan.hours[1],
            Some(&scan.hours[0].states),
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
            D56,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &one_row_payload(0, 12_389),
        ));
        s.extend(frame_end(2, 100));
        s.extend(record(
            3,
            102,
            D56,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &one_row_payload(0, 12_390),
        ));
        s.extend(frame_end(4, 102));
        let scan =
            scan_stream(&s, &schema(), StreamIdentityMode::Legacy).expect("well-formed");
        assert_eq!(scan.hours.len(), 2);
        assert_eq!(scan.non_contiguous, 1);
        assert!(!scan.hours[1].follows_previous);
        assert!(opens_baseline(1, scan.hours[1].follows_previous));

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
        let scan = scan_stream(&good_stream(), &schema(), StreamIdentityMode::Legacy)
            .expect("well-formed");
        assert_eq!(scan.non_contiguous, 0);
        assert!(scan.hours[1].follows_previous);
        assert!(!opens_baseline(1, scan.hours[1].follows_previous));
        let mut wire = Vec::new();
        let mut reader: Option<Vec<CollectionState>> = None;
        encode_wire_hour(&full_schema, 0, &scan.hours[0], None, &mut reader, &mut wire);
        let o1 = encode_wire_hour(
            &full_schema,
            1,
            &scan.hours[1],
            Some(&scan.hours[0].states),
            &mut reader,
            &mut wire,
        );
        assert!(
            !o1.opened_baseline,
            "a contiguous non-block hour is not baselined"
        );
    }

    #[test]
    fn a_closed_hour_without_a_d56_record_refuses_the_stream() {
        // The cross-domain bound: this harness validates only d56 payloads, so
        // a d56 record relabelled to another domain — or dropped — would
        // otherwise be silently ignored and the run would report a plausible
        // measurement over fewer hours. Hour 100 carries a d56 record; hour
        // 101 carries only another domain's DATA record. Every hour is
        // structurally well formed — a DOMAIN_DATA record opens it and a
        // FRAME_END closes it — so only the completeness check can see it.
        let mut s = Vec::new();
        s.extend(record(
            1,
            100,
            D56,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &one_row_payload(0, 12_389),
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
        let err = match scan_stream(&s, &schema(), StreamIdentityMode::Legacy) {
            Err(err) => err,
            Ok(_) => panic!("a closed hour with no d56 record must refuse the stream"),
        };
        assert!(
            err.contains("every closed hour"),
            "the refusal names the completeness rule: {err}"
        );
    }

    #[test]
    fn a_failure_status_record_is_excluded_rather_than_measured() {
        // `world_fold.rs:22123`-`:22129`: the accumulator is retained and the hour
        // folds nothing, so it is neither a measured hour nor a stream rejection.
        for bit in [0u32, 1, 2, 3, 8, 9] {
            let mut s = Vec::new();
            s.extend(record(
                1,
                100,
                D56,
                COMPACT_RECORD_TYPE_DOMAIN_DATA,
                &one_row_payload(0, 12_389),
            ));
            s.extend(frame_end(2, 100));
            s.extend(record(
                3,
                101,
                D56,
                COMPACT_RECORD_TYPE_DOMAIN_DATA,
                &one_row_payload(1 << bit, 12_390),
            ));
            s.extend(frame_end(4, 101));
            s.extend(record(
                5,
                102,
                D56,
                COMPACT_RECORD_TYPE_DOMAIN_DATA,
                &one_row_payload(0, 12_391),
            ));
            s.extend(frame_end(6, 102));
            let scan = scan_stream(&s, &schema(), StreamIdentityMode::Legacy)
                .unwrap_or_else(|e| panic!("status bit {bit} must exclude, not refuse: {e}"));
            if compact_status_failure_flags_after_expected_partial_tolerance(D56, 1 << bit) != 0 {
                assert_eq!(scan.failure_records, 1, "status bit {bit}");
                assert_eq!(scan.hours.len(), 2);
                assert_eq!(scan.non_contiguous, 1, "the skipped hour breaks contiguity");
            } else {
                assert_eq!(scan.hours.len(), 3, "status bit {bit} is not a failure");
            }
        }
    }

    #[test]
    fn a_delta_before_any_baseline_is_excluded_rather_than_measured() {
        // `world_fold.rs:22130`-`:22136`, and the sidecar is NOT replaced on that
        // exit either — the fold returns before it touches anything.
        let mut s = Vec::new();
        s.extend(record(
            1,
            100,
            D56,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &one_row_payload(COMPACT_STATUS_DELTA_DATA, 12_389),
        ));
        s.extend(frame_end(2, 100));
        s.extend(record(
            3,
            101,
            D56,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &one_row_payload(0, 12_390),
        ));
        s.extend(frame_end(4, 101));
        let scan = scan_stream(&s, &schema(), StreamIdentityMode::Legacy).expect("accepted");
        assert_eq!(scan.delta_before_baseline_records, 1);
        assert_eq!(scan.hours.len(), 1);
        assert_eq!(scan.hours[0].game_hour, 101);
    }

    #[test]
    fn a_delta_after_a_baseline_upserts_without_clearing() {
        // The measured capture never carries a delta frame, so this is the only
        // place the delta arm of the fold is exercised at stream level.
        let mut s = Vec::new();
        s.extend(record(
            1,
            100,
            D56,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &d56_payload(0, &[task_force_row(617, 12_389)], &[supremacy_row(16, 2, 1)]),
        ));
        s.extend(frame_end(2, 100));
        s.extend(record(
            3,
            101,
            D56,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &d56_payload(
                COMPACT_STATUS_DELTA_DATA,
                &[task_force_row(618, 12_390)],
                &[],
            ),
        ));
        s.extend(frame_end(4, 101));
        let scan = scan_stream(&s, &schema(), StreamIdentityMode::Legacy).expect("accepted");
        assert_eq!(scan.hours.len(), 2);
        assert_eq!(
            scan.hours[1].states[0].len(),
            2,
            "a delta frame upserts; there is no tombstone contract"
        );
        assert_eq!(
            scan.hours[1].supremacy_rows, 0,
            "the sidecar is full-replaced even by a delta frame"
        );
    }

    #[test]
    fn a_stream_with_no_d56_records_is_refused_not_reported_as_zero() {
        let mut s = Vec::new();
        s.extend(record(
            1,
            100,
            99,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &[0u8; 8],
        ));
        s.extend(frame_end(2, 100));
        assert!(scan_stream(&s, &schema(), StreamIdentityMode::Legacy).is_err());
    }

    #[test]
    fn a_malformed_d56_payload_is_refused_not_skipped() {
        // Every way the lane decoder rejects a payload is a refused stream here,
        // because the fold decodes before it classifies.
        let mut cases: Vec<(&str, Vec<u8>)> = Vec::new();
        cases.push(("short header", vec![0u8; d56::HEADER_LEN - 1]));
        let mut version = one_row_payload(0, 12_389);
        version[40..44].copy_from_slice(&2u32.to_le_bytes());
        cases.push(("schema version", version));
        let mut short_body = one_row_payload(0, 12_389);
        short_body.truncate(d56::HEADER_LEN + d56::ROW_LEN - 1);
        cases.push(("body short of row_count", short_body));
        let mut changed = one_row_payload(0, 12_389);
        changed[12..16].copy_from_slice(&2u32.to_le_bytes());
        cases.push(("changed_rows > row_count", changed));
        let mut fixtures = one_row_payload(0, 12_389);
        fixtures[32..36].copy_from_slice(&1u32.to_le_bytes());
        cases.push(("retired fixture counter", fixtures));
        cases.push((
            "retired row flag",
            d56_payload(
                0,
                &[{
                    let mut row = task_force_row(617, 12_389);
                    let flags = d56::FLAG_VALIDATED_DIRECT | d56::FLAG_FIELD_REJECTED;
                    row[12..16].copy_from_slice(&flags.to_le_bytes());
                    row
                }],
                &[],
            ),
        ));
        cases.push((
            "duplicate task_force_id",
            d56_payload(
                0,
                &[task_force_row(617, 12_389), task_force_row(617, 12_390)],
                &[],
            ),
        ));
        cases.push((
            "supremacy ratio past 1.0",
            d56_payload(
                0,
                &[task_force_row(617, 12_389)],
                &[{
                    let mut row = supremacy_row(16, 2, 1);
                    row[28..32].copy_from_slice(&(d56::FIXED_SCALE_RAW + 1).to_le_bytes());
                    row
                }],
            ),
        ));
        cases.push((
            "unknown trailer magic",
            {
                let mut p = d56_payload(0, &[task_force_row(617, 12_389)], &[]);
                p.extend_from_slice(b"XSUP");
                p
            },
        ));
        for (label, payload) in cases {
            // Under a clean status AND under a failure status: the decode runs
            // first either way (`world_fold.rs:22120`-`:22127`).
            for status in [0u32, 1] {
                let mut p = payload.clone();
                if p.len() >= d56::HEADER_LEN {
                    p[16..20].copy_from_slice(&status.to_le_bytes());
                }
                let mut s = Vec::new();
                s.extend(record(1, 100, D56, COMPACT_RECORD_TYPE_DOMAIN_DATA, &p));
                s.extend(frame_end(2, 100));
                assert!(
                    scan_stream(&s, &schema(), StreamIdentityMode::Legacy).is_err(),
                    "{label} under status {status:#x} must refuse the stream"
                );
            }
        }
    }

    /// One DOMAIN_PERF record with the minimal valid payload.
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
        payload.extend_from_slice(&0u64.to_le_bytes()); // elapsed_wall_ms
        payload.extend_from_slice(&0u64.to_le_bytes()); // frame_time_us
        payload.extend_from_slice(&emitted.to_le_bytes());
        payload.extend_from_slice(&scheduled.to_le_bytes());
        payload.extend_from_slice(&status.to_le_bytes());
        payload.extend_from_slice(&reserved.to_le_bytes());
        assert_eq!(payload.len(), COMPACT_FRAME_END_PAYLOAD_LEN);
        record(
            sequence,
            game_hour,
            COMPACT_DOMAIN_FRAME,
            COMPACT_RECORD_TYPE_FRAME_END,
            &payload,
        )
    }

    #[test]
    fn a_frame_end_that_does_not_seal_its_hour_refuses_the_stream() {
        // The seal predicate of `validate_frame_end_seal_fields`
        // (`stream_fold.rs:46`-`:89`), swept over the whole predicate.
        let mut ok = Vec::new();
        ok.extend(record(
            1,
            100,
            D56,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &one_row_payload(0, 12_389),
        ));
        ok.extend(frame_end_sealed(2, 100, 0, 0, 0, 0));
        assert_eq!(
            scan_stream(&ok, &schema(), StreamIdentityMode::Legacy)
                .expect("a sealed stream is accepted")
                .hours
                .len(),
            1
        );

        let mut bad_seals: Vec<(String, Vec<u8>)> = vec![
            (
                "reserved nonzero".to_string(),
                frame_end_sealed(2, 100, 0, 0, 0, 1),
            ),
            (
                "emitted != scheduled".to_string(),
                frame_end_sealed(2, 100, 0, 1, 0, 0),
            ),
            (
                "counts without PERF records".to_string(),
                frame_end_sealed(2, 100, 1, 1, 0, 0),
            ),
        ];
        for bit in 14..32u32 {
            bad_seals.push((
                format!("unknown seal status bit {bit}"),
                frame_end_sealed(2, 100, 0, 0, 1 << bit, 0),
            ));
        }
        for bit in [0u32, 1, 2, 3, 6, 7, 8, 9, 12] {
            bad_seals.push((
                format!("fatal seal status bit {bit}"),
                frame_end_sealed(2, 100, 0, 0, 1 << bit, 0),
            ));
        }
        for (label, sealed) in bad_seals {
            let mut s = Vec::new();
            s.extend(record(
                1,
                100,
                D56,
                COMPACT_RECORD_TYPE_DOMAIN_DATA,
                &one_row_payload(0, 12_389),
            ));
            s.extend(sealed);
            assert!(
                scan_stream(&s, &schema(), StreamIdentityMode::Legacy).is_err(),
                "{label} must refuse the stream, not yield a measurement"
            );
        }

        // A seal payload of the wrong length is refused at every length, never a
        // panic.
        for len in [0usize, 1, 16, 31, 33, 64] {
            let mut s = Vec::new();
            s.extend(record(
                1,
                100,
                D56,
                COMPACT_RECORD_TYPE_DOMAIN_DATA,
                &one_row_payload(0, 12_389),
            ));
            s.extend(record(
                2,
                100,
                COMPACT_DOMAIN_FRAME,
                COMPACT_RECORD_TYPE_FRAME_END,
                &vec![0u8; len],
            ));
            assert!(
                scan_stream(&s, &schema(), StreamIdentityMode::Legacy).is_err(),
                "seal payload length {len}"
            );
        }
    }

    #[test]
    fn perf_hour_binding_is_enforced() {
        // The seal's domain-count cross-check against the observed non-probe PERF
        // domains (`stream_fold.rs:411`-`:419`), and the binding rules at
        // `:328`-`:365`.
        let mut ok = Vec::new();
        ok.extend(perf(1, 100, D56));
        ok.extend(record(
            2,
            100,
            D56,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &one_row_payload(0, 12_389),
        ));
        ok.extend(frame_end_sealed(3, 100, 1, 1, 0, 0));
        assert_eq!(
            scan_stream(&ok, &schema(), StreamIdentityMode::Legacy)
                .expect("one PERF domain, seal says one")
                .hours
                .len(),
            1
        );

        // A duplicate non-probe PERF domain inside one hour.
        let mut duplicate = Vec::new();
        duplicate.extend(perf(1, 100, D56));
        duplicate.extend(perf(2, 100, D56));
        duplicate.extend(record(
            3,
            100,
            D56,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &one_row_payload(0, 12_389),
        ));
        duplicate.extend(frame_end_sealed(4, 100, 1, 1, 0, 0));
        assert!(scan_stream(&duplicate, &schema(), StreamIdentityMode::Legacy).is_err());

        // A PERF record for an hour other than the open one.
        let mut interleaved = Vec::new();
        interleaved.extend(record(
            1,
            100,
            D56,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &one_row_payload(0, 12_389),
        ));
        interleaved.extend(perf(2, 101, D56));
        interleaved.extend(frame_end_sealed(3, 100, 1, 1, 0, 0));
        assert!(scan_stream(&interleaved, &schema(), StreamIdentityMode::Legacy).is_err());

        // A PERF record for an hour already closed.
        let mut reopened = Vec::new();
        reopened.extend(record(
            1,
            100,
            D56,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &one_row_payload(0, 12_389),
        ));
        reopened.extend(frame_end_sealed(2, 100, 0, 0, 0, 0));
        reopened.extend(perf(3, 100, D56));
        reopened.extend(frame_end_sealed(4, 100, 1, 1, 0, 0));
        assert!(scan_stream(&reopened, &schema(), StreamIdentityMode::Legacy).is_err());

        // Two different pending PERF hours before any DATA record opens one.
        let mut conflict = Vec::new();
        conflict.extend(perf(1, 100, D56));
        conflict.extend(perf(2, 101, 61));
        conflict.extend(record(
            3,
            100,
            D56,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &one_row_payload(0, 12_389),
        ));
        conflict.extend(frame_end_sealed(4, 100, 2, 2, 0, 0));
        assert!(scan_stream(&conflict, &schema(), StreamIdentityMode::Legacy).is_err());

        // The seal's counts must match what was observed.
        let mut miscounted = Vec::new();
        miscounted.extend(perf(1, 100, D56));
        miscounted.extend(record(
            2,
            100,
            D56,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &one_row_payload(0, 12_389),
        ));
        miscounted.extend(frame_end_sealed(3, 100, 2, 2, 0, 0));
        assert!(scan_stream(&miscounted, &schema(), StreamIdentityMode::Legacy).is_err());
    }

    #[test]
    fn a_data_record_must_open_the_hour_the_pending_perf_records_bound() {
        let mut s = Vec::new();
        s.extend(perf(1, 100, D56));
        s.extend(record(
            2,
            101,
            D56,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &one_row_payload(0, 12_389),
        ));
        s.extend(frame_end_sealed(3, 101, 1, 1, 0, 0));
        assert!(scan_stream(&s, &schema(), StreamIdentityMode::Legacy).is_err());
    }

    #[test]
    fn an_excluded_record_is_still_checked_by_the_payload_authority() {
        // d56's duplicate-`task_force_id` rule lives in the DECODER
        // (`domain56_compact.rs:269`-`:274`), which production runs on every
        // DOMAIN_DATA record whatever its status — unlike d63's, whose uniqueness
        // rule is the fold's alone. So a failure-status record carrying a
        // duplicate refuses the stream rather than being quietly skipped.
        let mut s = Vec::new();
        s.extend(record(
            1,
            100,
            D56,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &d56_payload(
                1, // a failure status bit
                &[task_force_row(617, 12_389), task_force_row(617, 12_390)],
                &[],
            ),
        ));
        s.extend(frame_end(2, 100));
        assert!(scan_stream(&s, &schema(), StreamIdentityMode::Legacy).is_err());

        // And a clean excluded record with a well-formed payload is still only an
        // exclusion.
        let mut clean = Vec::new();
        clean.extend(record(
            1,
            100,
            D56,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &one_row_payload(1, 12_389),
        ));
        clean.extend(frame_end(2, 100));
        clean.extend(record(
            3,
            101,
            D56,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &one_row_payload(0, 12_389),
        ));
        clean.extend(frame_end(4, 101));
        let scan = scan_stream(&clean, &schema(), StreamIdentityMode::Legacy).expect("accepted");
        assert_eq!(scan.failure_records, 1);
        assert_eq!(scan.hours.len(), 1);
    }

    #[test]
    fn the_identity_protocol_is_stated_not_sniffed() {
        use compact_contract::campaign_identity::COMPACT_RECORD_TYPE_STREAM_HEADER;

        let decapitated = good_stream();
        assert!(
            scan_stream(&decapitated, &schema(), StreamIdentityMode::IdentityV1).is_err(),
            "an identity-mode stream whose CID1 record is missing must be refused"
        );
        assert_eq!(
            scan_stream(&decapitated, &schema(), StreamIdentityMode::Legacy)
                .expect("a genuine legacy stream")
                .hours
                .len(),
            2
        );
        let mut identity_stream = Vec::new();
        identity_stream.extend(record(
            0,
            0,
            0,
            COMPACT_RECORD_TYPE_STREAM_HEADER,
            b"CID1-not-a-real-identity-payload",
        ));
        identity_stream.extend(&good_stream());
        assert!(
            scan_stream(&identity_stream, &schema(), StreamIdentityMode::Legacy).is_err(),
            "a CID1 record under the legacy claim must be refused"
        );
    }

    #[test]
    fn scan_stream_never_panics_on_arbitrary_bytes() {
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
            let _ = scan_stream(&bytes, &schema(), StreamIdentityMode::Legacy);
        }
        let good = good_stream();
        for i in 0..good.len() {
            let mut s = good.clone();
            s[i] ^= 0xFF;
            let _ = scan_stream(&s, &schema(), StreamIdentityMode::Legacy);
        }
    }
}
