//! Measure the wire format against the shipping compact stream on REAL d51
//! `scripted_state_flags_variables_live` dynamics.
//!
//! Input: a real `collector_stream.bin` from a d51-scheduled capture
//! (`runs/batch3/deepwar_a`). The stream's own d51 DATA records are the shipping
//! format's cost for the domain: a 64-byte header, a fixed 48-byte row per
//! emitted row, and a trailing UTF-8 key table — exactly what the recorder stores
//! today. This tool replays those records through `wire_format::d51`'s own
//! `State::fold_payload`, the same fold `world_fold::fold_scripted_state`
//! performs, and re-encodes every hour with this crate's grammar: a BASELINE at
//! each 48-hour block start, field-granular stored-minimal DELTAs between.
//!
//! **Nothing here re-derives a contract it can call.** Header, row and
//! accumulator semantics go through `wire_format::d51`'s public boundary
//! (`peek_header`, `split_payload`, `parse_row`, `key_from_*`, `record_from_*`,
//! `State::fold_payload`, `KeyScanner::scan_payload`, `check_authority_payload`);
//! H4CR framing, record semantics and sequence continuity go through
//! `compact_contract`'s own parser and stateful validator; failure classification
//! goes through `compact_contract`'s own predicate rather than a copied bit mask.
//!
//! # Two passes, because the §5 symbol table must be frozen before any mapping
//!
//! §5 requires symbol ids to be file-global and forbids in-stream interning — the
//! table "must be fully read before any seek" — so the scripted-key vocabulary is
//! collected over the whole validated stream first and frozen into a
//! `d51::Symbols` that pass 2 only reads. Pass 1 therefore cannot call a mapper at
//! all: it runs `d51::KeyScanner`, the key-only mirror of the fold (d29's rule),
//! which shares `fold_input` with `State::fold_payload` so the two passes cannot
//! disagree about what is foldable. That sharing is also the completeness proof
//! pass 2 needs — the scanner appends the key string of EVERY row of every payload
//! it folds, live and tombstone alike, and the mapper resolves a symbol for
//! exactly one thing, a row's own key string, so pass 2 can never need a symbol
//! pass 1 did not see (`d51::KeyScanner`'s docs;
//! `the_key_scanner_and_the_fold_agree_on_every_payload` pins the agreement).
//! d51 is also a large-state domain — 7,745 rows in the capture's baseline — so
//! the second reason for two passes applies as well: pass 1 retains only record
//! spans.
//!
//! # What pass 1 enforces, fail-closed
//!
//! - every record header through `compact_contract::parse_compact_record_header`;
//! - every record through `compact_contract::stream_validator::StatefulStreamValidator`;
//! - the hour boundary rules of `recorder_host::stream_fold::BoundaryTracker`
//!   (`recorder_host/src/stream_fold.rs:277`), its DOMAIN_PERF hour binding
//!   (`:328`-`:365`), the FRAME_END seal fields (`:46`-`:89`) with the seal's
//!   domain-count cross-check (`:411`-`:419`), and `eof_check` (`:459`).
//!   `wire_format` must not depend on `recorder_host`, so that documented subset
//!   is re-implemented here exactly as `bench_real_d63.rs` does. The DATA payloads
//!   of every domain other than 51 pass UNVALIDATED beyond header sanity, the
//!   stream validator and the hour-boundary rules — production validates every
//!   domain's payload (`stream_fold.rs:593`). That unvalidated remainder is
//!   explicitly BOUNDED: every closed game hour must carry exactly one d51 DATA
//!   record of any status (the `d51_records == closed_hours` completeness check in
//!   [`scan_stream`]), so a d51 record relabelled to another domain id — or
//!   dropped — refuses the run instead of silently shrinking it;
//! - **one d51 DATA record per game hour, over EVERY d51 record and BEFORE any
//!   status exclusion**, so a success+failure pair in one hour cannot slip
//!   through;
//! - **unassigned v1 status bits refused BEFORE classification**
//!   (`compact_contract/src/status_flags_base.rs:21`-`:24`). d51 makes this rule
//!   sharper than usual: its three lane-local badges ALIAS globally-assigned bits
//!   11, 12 and 13, and are registered in `lane_local_status_mask`
//!   (`compact_contract/src/lib.rs:141`), so the validity check must run over
//!   `compact_status_for_global_interpretation` or every hour of this capture —
//!   which carries bit 11 on 514 of 514 — would misclassify;
//! - production's SECOND authority on every DOMAIN_DATA record whatever its
//!   status — `d51::check_authority_payload`, which is
//!   `lanes/scripted_state_flags_variables_live/scripted_state_compact.rs`'s own
//!   decoder. Any complaint there makes the record `malformed_payload_records`,
//!   which `strict_stream_certification_verdict` (`stream_fold.rs:507`-`:510`)
//!   refuses.
//!
//! # The three outcomes, named
//!
//! Every non-folding path of `fold_scripted_state` is classified explicitly:
//!
//! 1. **Stream rejection** — every `Err` the fold can return: a payload shorter
//!    than the header (`world_fold.rs:18912`), a schema version other than 2
//!    (`:18918`), a payload whose length is not exactly
//!    `64 + row_count*48 + key_bytes_len` (`:18947`), a `fixed_scale` other than
//!    100,000 (`:18954`), and every row-loop refusal (`:18966`-`:19062`). These
//!    invalidate the run.
//! 2. **Excluded hour, failure status** (`:18928`-`:18935`): the fold marks the
//!    domain-hour unknown and returns. The accumulator is retained, but the
//!    recorder then publishes the whole d51 surface as `Null` for that hour —
//!    world (`:3872`-`:3902`), country (`:5175` → `mask_scripted_country_extra`)
//!    and state (`:5393`-`:5397`) — so nothing of that hour survives the fold and
//!    excluding it from both arms loses nothing.
//! 3. **Excluded hour, delta before baseline** (`:18937`-`:18943`). Unlike d52,
//!    this is NOT a separate case for d51: `reject_delta_before_baseline` calls
//!    the same `mark_fold_domain_unknown`, this domain has no trailer that
//!    survives the exit, and the publish path masks everything. It is counted
//!    separately and treated as an excluded hour.
//!
//! **The exclusion's POSITION is the fold's own.** The fold checks the header
//! length and schema version, THEN the status word, THEN delta-before-baseline,
//! and only THEN the exact payload length, the fixed scale and the rows. So this
//! harness calls `d51::peek_header` (which carries exactly the first two checks),
//! excludes on status, and lets `State::fold_payload` take the
//! delta-before-baseline exit before `split_payload` runs. Copying d63's
//! "everything then status" order here would refuse a stream the recorder folds
//! nothing from and accepts.
//!
//! # Other writers of this domain's accumulator
//!
//! The five `scripted_world_*` maps (`world_fold.rs:1683`-`:1695`) are written by
//! `fold_scripted_state` and nothing else — the only other mentions in the file
//! are its own clear, its own row applier and the world projection that reads
//! them. The country and state JSON slices d51 also writes
//! (`extra.script_flags`, `extra._r2_script_variables`,
//! `extra._r2_script_variables_raw_fixed5`, `extra._r2_token_only_flags`,
//! `state._r2_scripted_state`) have exactly one potential other writer, and it is
//! **proved inert structurally rather than per capture**: `country_identity`'s
//! roster replacement (`world_fold.rs:14438`-`:14461`) removes only d63/d67/d48
//! fields from a DEPARTED tag and drops the country entry only when its whole
//! field map becomes empty — a country carrying any d51 row still carries its
//! `extra` object, so the map is never empty and the entry is never dropped.
//! Nothing else removes a country or a state.
//!
//! Outputs, for the compression step (`compress_arms.py` beside the run):
//!   ship_d51.bin   — the d51 DATA records verbatim (32-byte H4CR header +
//!                    payload), concatenated in stream order
//!   wire_new.bin   — the wire-format bodies, concatenated in hour order
//!   *.offsets      — hour-boundary byte offsets, one per line

use std::collections::{BTreeMap, BTreeSet};
use std::io::Write;
use std::ops::Range;

use compact_contract::stream_validator::{StatefulStreamValidator, StreamIdentityMode};
use compact_contract::{
    compact_status_failure_flags_after_expected_partial_tolerance,
    compact_status_for_global_interpretation, compact_status_unknown_flags_v1,
    parse_compact_record_header, COMPACT_FRAME_END_PAYLOAD_LEN, COMPACT_RECORD_HEADER_LEN,
    COMPACT_RECORD_TYPE_DOMAIN_DATA, COMPACT_RECORD_TYPE_DOMAIN_PERF,
    COMPACT_RECORD_TYPE_FRAME_END, COMPACT_STATUS_FAILURE_MASK,
};
use wire_format::body::{
    apply, decode_baseline, decode_delta, diff, encode_baseline, encode_delta, KeyedDelta,
    KeyedState,
};
use wire_format::d51::{self, FoldOutcome, KeyScanner, State, Symbols, COLLECTION_COUNT};

const D51: u16 = 51;
/// The default re-baseline cadence, as every earlier bench bin uses. It is an
/// argument rather than a constant because the block ratio measures re-baseline
/// POLICY as much as grammar: the shipping lane baselines once per campaign and
/// this arm baselines every `BLOCK_HOURS`, so a run at the shipping cadence is
/// what makes the two grammars comparable. See the measurement write-up.
const DEFAULT_BLOCK_HOURS: usize = 48;

/// One usable d51 hour: its game hour, whether its payload was a baseline, and
/// the byte span of its record in the source stream (the shipping arm is those
/// bytes verbatim, header included).
#[derive(Debug)]
struct D51Hour {
    game_hour: i32,
    is_baseline: bool,
    /// True when this measured hour is exactly one game hour after the previous
    /// MEASURED hour. False on the first measured hour and after any gap — an
    /// excluded hour or a non-contiguous hour step — which is what forces the
    /// encoder to open a fresh baseline (`opens_baseline`).
    follows_previous: bool,
    /// The `row_count` the shipping payload carried — the collector's own notion
    /// of how many rows it emitted this hour, reported beside the wire arm's.
    shipping_rows: u32,
    record_span: Range<usize>,
    payload_span: Range<usize>,
}

/// Whether the wire arm opens a BASELINE at this measured hour.
///
/// Two reasons, and both are `WIRE_FORMAT.md` rules, not conveniences: the block
/// cadence (every block opens with its own baseline, §1.5's closing paragraph),
/// and a gap — `WIRE_FORMAT.md:187`-`:190`: "A gap affecting the state part forces
/// the next state observation to be a `BASELINE`." Every hour this domain did not
/// successfully sample is such a gap — an excluded hour, and any non-contiguous
/// hour step — and d51's cadence is hourly, so a measured hour that does not
/// follow the previous measured one by exactly one is preceded by one. Neither a
/// delta nor a retained reader state can be justified across it.
fn opens_baseline(hours_seen: usize, block_hours: usize, follows_previous: bool) -> bool {
    hours_seen % block_hours == 0 || !follows_previous
}

/// One wire-arm hour: the §1.5/block baseline decision, the encode, and the
/// per-hour reader replay over every collection. Factored out of `main` so the
/// tests execute the PRODUCTION encode path over a scanned stream: a mutant that
/// reverts the baseline decision here to block-position-only fails the gap test.
struct WireHourOutcome {
    opened_baseline: bool,
    /// The baseline sits outside a scheduled block start — §1.5 gap-forced.
    gap_forced: bool,
    emitted: bool,
    changed_by_collection: [usize; COLLECTION_COUNT],
    tombstones_by_collection: [usize; COLLECTION_COUNT],
}

#[allow(clippy::too_many_arguments)]
fn encode_wire_hour(
    schema: &wire_format::Schema,
    hours_seen: usize,
    block_hours: usize,
    follows_previous: bool,
    game_hour: i32,
    prior: Option<&[KeyedState]>,
    state: &[KeyedState],
    reader: &mut Option<Vec<KeyedState>>,
    wire: &mut Vec<u8>,
    symbol_occurrences: &mut SymbolOccurrences,
) -> WireHourOutcome {
    let hour_start = wire.len();
    // BASELINE at each block start — and at every §1.5 gap, which forces a fresh
    // baseline and restarts the delta chain (`opens_baseline`).
    let block_start = opens_baseline(hours_seen, block_hours, follows_previous);
    let gap_forced = block_start && hours_seen % block_hours != 0;
    let mut emitted = true;
    let mut changed_by_collection = [0usize; COLLECTION_COUNT];
    let mut tombstones_by_collection = [0usize; COLLECTION_COUNT];
    let opened_baseline = block_start || prior.is_none();
    if opened_baseline {
        encode_baseline(schema, state, wire).expect("baseline encodes");
        // A baseline writes every key and every record, so every symbol in the
        // state is paid for here. Counted at the encode site, not beside it, so a
        // future edit cannot encode one thing and count another.
        symbol_occurrences.count_state(schema, state);
    } else {
        let prev = prior.expect("a prior exists past the first hour");
        let deltas: Vec<Option<KeyedDelta>> = schema
            .collections
            .iter()
            .enumerate()
            .map(|(i, collection)| {
                let d = diff(collection, &prev[i], &state[i]);
                if d.is_empty() {
                    None
                } else {
                    Some(d)
                }
            })
            .collect();
        if deltas.iter().all(Option::is_none) {
            emitted = false;
        } else {
            for (i, delta) in deltas.iter().enumerate() {
                if let Some(delta) = delta {
                    changed_by_collection[i] = delta.changed.len();
                    tombstones_by_collection[i] = delta.tombstones.len();
                    // A delta writes the key of every changed row and of every
                    // tombstone, and those keys carry symbol ids too. Omitting
                    // them understates the §5 assignment band (round-1 finding).
                    symbol_occurrences.count_delta(&schema.collections[i], delta);
                }
            }
            encode_delta(schema, prev, &deltas, wire).expect("delta encodes");
        }
    }

    // Reader replay: decode the bytes just written against the reader's OWN
    // reconstructed prior and require the result to be stored-equal to this
    // hour's folded state. An hour that emitted nothing is still checked.
    if emitted {
        let hour_bytes = &wire[hour_start..];
        if block_start || reader.is_none() {
            let (states, consumed) =
                decode_baseline(schema, hour_bytes, 0).expect("baseline decodes");
            assert_eq!(consumed, hour_bytes.len(), "baseline consumed exactly");
            *reader = Some(states);
        } else {
            let reader_prior = reader.take().expect("a reader prior exists");
            let (deltas, consumed) =
                decode_delta(schema, &reader_prior, hour_bytes, 0).expect("delta decodes");
            assert_eq!(consumed, hour_bytes.len(), "delta consumed exactly");
            let mut next = reader_prior;
            for (state, delta) in next.iter_mut().zip(deltas.iter()) {
                if let Some(delta) = delta {
                    apply(state, delta);
                }
            }
            *reader = Some(next);
        }
    }
    let replayed = reader.as_ref().expect("a reader state exists");
    for (i, collection) in schema.collections.iter().enumerate() {
        assert_eq!(
            replayed[i].len(),
            state[i].len(),
            "game hour {game_hour}: collection {} row count",
            collection.collection_id
        );
        assert!(
            diff(collection, &replayed[i], &state[i]).is_empty(),
            "game hour {game_hour}: collection {} is not stored-equal to the fold",
            collection.collection_id
        );
    }
    WireHourOutcome {
        opened_baseline,
        gap_forced,
        emitted,
        changed_by_collection,
        tombstones_by_collection,
    }
}

#[derive(Debug, Default)]
struct ScanCounts {
    /// d51 records whose payload status flags mark the domain-hour failed.
    failure_records: usize,
    /// d51 records that arrived as a delta before any baseline. An excluded hour
    /// for this domain (see the module docs), not a run rejection.
    delta_before_baseline_records: usize,
    /// Measured d51 hours that do not follow the previous measured one by one.
    non_contiguous: usize,
    /// Rows the fold walked, over every measured record.
    rows_folded: usize,
    /// Tombstone rows among them.
    tombstones_folded: usize,
    /// Live rows per collection, in schema order.
    live_rows_by_collection: [usize; COLLECTION_COUNT],
    /// Union of every status bit seen on a d51 record.
    status_flags_or: u32,
    /// Hours carrying the lane-local names-partial badge.
    names_partial_hours: usize,
}

#[derive(Debug)]
struct StreamScan {
    hours: Vec<D51Hour>,
    counts: ScanCounts,
    /// The scripted-key vocabulary of the whole stream, frozen before any record
    /// is mapped (§5).
    vocabulary: BTreeSet<String>,
}

/// Every globally-defined compact status occupies one of bits 0..=13
/// (`recorder_host/src/stream_fold.rs:43`).
const FRAME_END_KNOWN_STATUS_MASK: u32 = (1 << 14) - 1;

/// The reader-context probe domain, excluded from the seal's domain accounting
/// (`recorder_host/src/stream_fold.rs:330`).
const PROBE_DOMAIN_ID: u16 = 250;

/// The FRAME_END seal check, re-implemented from
/// `recorder_host::stream_fold::validate_frame_end_seal_fields`
/// (`recorder_host/src/stream_fold.rs:46`-`:89`) because `wire_format` must not
/// depend on `recorder_host`. Total: no payload of any length panics.
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
            "FRAME_END seal violation at byte {at_byte}: reserved word is nonzero ({reserved:#010x})"
        ));
    }
    if emitted_domains != scheduled_domains {
        return Err(format!(
            "FRAME_END seal violation at byte {at_byte}: emitted domain count {emitted_domains} \
             does not match scheduled domain count {scheduled_domains}"
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

/// Validate the whole stream against the canonical contract and scan every usable
/// d51 DATA record. Every rejection path is an `Err`, never a silently shorter
/// measurement and never a panic.
fn scan_stream(stream: &[u8], identity_mode: StreamIdentityMode) -> Result<StreamScan, String> {
    // The identity protocol is an INPUT, never sniffed: an `IdentityV1` stream
    // whose mandatory CID1 record has been spliced off is byte-indistinguishable
    // from a legacy stream, so a harness that sniffs accepts a decapitated
    // identity stream and measures it.
    let mut validator = match identity_mode {
        StreamIdentityMode::IdentityV1 => StatefulStreamValidator::identity_v1(),
        StreamIdentityMode::Legacy => StatefulStreamValidator::legacy(),
    };

    // COMPLETENESS accounting: every d51 DATA record of ANY status, against every
    // closed game hour — see the check after the EOF rules.
    let mut d51_records = 0usize;
    let mut closed_hours = 0usize;
    let mut scan = StreamScan {
        hours: Vec::new(),
        counts: ScanCounts::default(),
        vocabulary: BTreeSet::new(),
    };

    let mut open_hour: Option<i32> = None;
    let mut last_closed_hour: Option<i32> = None;
    let mut last_record_type: Option<u16> = None;
    let mut pending_perf_hour: Option<i32> = None;
    let mut non_probe_perf_domains: BTreeSet<u16> = BTreeSet::new();
    // Hour of the last d51 DATA record of ANY status — the uniqueness rule.
    let mut last_d51_hour: Option<i32> = None;
    // Hour of the last d51 record that entered the measurement.
    let mut last_measured_hour: Option<i32> = None;
    // Pass 1 scans into the key-only mirror, because every way a row can be
    // rejected is a property of the stream and so belongs to the pass that either
    // accepts the stream or refuses it — and because `delta_before_baseline` is
    // decided from the mirror's own baseline bit.
    let mut scanner = KeyScanner::new();

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
                            "hour interleaving at byte {pos}: DOMAIN_PERF for hour {} while hour \
                             {open} is still open",
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
                            "FRAME_END at byte {pos} closes hour {} without an open DOMAIN_DATA \
                             hour",
                            header.game_hour_raw
                        ))
                    }
                }
                if let Some(last_closed) = last_closed_hour {
                    if header.game_hour_raw <= last_closed {
                        return Err(format!(
                            "non-monotonic FRAME_END at byte {pos}: closed hour {last_closed}, \
                             then {}",
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

        if header.domain_id == D51 && header.record_type == COMPACT_RECORD_TYPE_DOMAIN_DATA {
            d51_records += 1;
            // One d51 DATA record per game hour, over EVERY d51 record and BEFORE
            // any status-based exclusion: gating this on the record being usable
            // would let a success/failure pair through the rule entirely.
            if let Some(previous) = last_d51_hour {
                if header.game_hour_raw <= previous {
                    return Err(format!(
                        "d51 DATA at byte {pos} carries hour {} after hour {previous}",
                        header.game_hour_raw
                    ));
                }
            }
            last_d51_hour = Some(header.game_hour_raw);

            // The fold's first two checks, in the fold's own order and before the
            // status word is classified (`world_fold.rs:18912`, `:18918`).
            let d51_header = d51::peek_header(payload)
                .map_err(|e| format!("d51 payload at byte {pos}: {e}"))?;
            scan.counts.status_flags_or |= d51_header.status_flags;
            if d51_header.names_partial() {
                scan.counts.names_partial_hours += 1;
            }

            // VALIDITY BEFORE CLASSIFICATION. Bits 14..=31 are unassigned and an
            // unassigned bit has no defined classification, so it refuses the
            // stream rather than falling through to "success". The three
            // lane-local badges of THIS domain alias assigned bits 11..13 and are
            // registered in `lane_local_status_mask`, so the validity check runs
            // over the globally-interpretable word — otherwise every hour of this
            // capture, which carries bit 11, would be misread.
            let unknown_status = compact_status_unknown_flags_v1(
                compact_status_for_global_interpretation(D51, d51_header.status_flags),
            );
            if unknown_status != 0 {
                return Err(format!(
                    "d51 DATA at byte {pos}: hour {} carries unassigned v1 status bits \
                     {unknown_status:#x}; an unassigned bit has no defined classification",
                    header.game_hour_raw
                ));
            }

            // Production's SECOND authority, which runs on every DOMAIN_DATA
            // record whatever its status and whose complaint refuses the stream at
            // certification. Before the exclusion, or a record the recorder would
            // refuse gets quietly skipped instead.
            d51::check_authority_payload(payload)
                .map_err(|e| format!("d51 payload at byte {pos}: {e}"))?;

            // `handle_compact_failure`-equivalent (`world_fold.rs:18928`-`:18935`):
            // the predicate is `compact_contract`'s own, not a copied bit mask, so
            // it cannot drift from the recorder. The fold's hand-written
            // expression is `(FAILURE_MASK & !OPAQUE_STATUS_MASK) | SOURCE_UNAVAILABLE
            // | PARTIAL_DATA`, which is exactly `COMPACT_STATUS_FAILURE_MASK` over
            // the globally-interpretable status word — the helper below —
            // because the only failure-mask bit the opaque mask removes is bit 12,
            // and `compact_status_for_global_interpretation` removes exactly bits
            // 11, 12 and 13 for this domain. `the_failure_predicate_matches_the_folds_own`
            // pins the equality over the whole 32-bit status space.
            if compact_status_failure_flags_after_expected_partial_tolerance(
                D51,
                d51_header.status_flags,
            ) != 0
            {
                scan.counts.failure_records += 1;
            } else {
                match scanner
                    .scan_payload(payload, &mut scan.vocabulary)
                    .map_err(|e| format!("d51 payload at byte {pos}: {e}"))?
                {
                    (FoldOutcome::DeltaBeforeBaseline, _) => {
                        // An excluded hour for this domain: the fold folds nothing
                        // and the recorder publishes the whole d51 surface as Null
                        // for the hour, so skipping it from both arms loses
                        // nothing. It still forces the next measured hour to open
                        // a §1.5 baseline, via `follows_previous`.
                        scan.counts.delta_before_baseline_records += 1;
                    }
                    (outcome, contribution) => {
                        scan.counts.rows_folded += contribution.rows;
                        scan.counts.tombstones_folded += contribution.tombstones;
                        for i in 0..COLLECTION_COUNT {
                            scan.counts.live_rows_by_collection[i] +=
                                contribution.live_rows_by_collection[i];
                        }
                        let follows_previous = last_measured_hour
                            .is_some_and(|previous| header.game_hour_raw == previous + 1);
                        if last_measured_hour.is_some() && !follows_previous {
                            scan.counts.non_contiguous += 1;
                        }
                        last_measured_hour = Some(header.game_hour_raw);
                        scan.hours.push(D51Hour {
                            game_hour: header.game_hour_raw,
                            is_baseline: outcome == FoldOutcome::Baseline,
                            follows_previous,
                            shipping_rows: d51_header.row_count,
                            record_span: pos..record_end,
                            payload_span: payload_start..record_end,
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
    if scan.hours.is_empty() {
        return Err("stream carries no usable d51 DATA records".to_string());
    }

    // COMPLETENESS: every closed game hour must carry exactly one d51 DATA record,
    // of any status. This bounds what this harness canNOT validate: production
    // runs `validate_domain_data_payload_at` on EVERY domain's DATA payload
    // (`stream_fold.rs:593`), while this harness owns only d51's grammar, so a d51
    // record whose header was relabelled to another domain id — or lost outright —
    // would otherwise be silently ignored and the run would report a plausible
    // measurement over fewer hours. A per-capture completeness assertion, NOT a
    // grammar rule: a capture where d51 is not scheduled every hour fails here
    // loudly and a human adjusts, instead of a number quietly shrinking
    // (`bench_real_d65.rs` precedent).
    if d51_records != closed_hours {
        return Err(format!(
            "stream closes {closed_hours} game hour(s) but carries {d51_records} d51 DATA \
             record(s); every closed hour of a d51-scheduled capture must carry exactly one, and \
             this harness cannot validate the payloads of other domains to see where the \
             difference went"
        ));
    }

    Ok(scan)
}

fn snapshot(state: &State) -> Vec<KeyedState> {
    state.collections().iter().map(|c| (*c).clone()).collect()
}

/// The number of bytes a varint id occupies (`varint.rs`), for the symbol-id
/// assignment band.
fn varint_len(value: u64) -> usize {
    let mut out = Vec::new();
    wire_format::varint::write_varint(&mut out, value);
    out.len()
}

/// The symbol ids the wire arm actually wrote, counted per id.
///
/// Both collections that carry a symbol carry it as key component 2, which
/// §7.4 writes as an ABSOLUTE varint (`body.rs:88`-`:116`) — only component 0 is
/// delta-chained. So a re-assignment of the symbol ids changes the wire arm's
/// size by exactly the change in `sum(varint_len(id))` over these occurrences,
/// and the band over all assignments is computable rather than a permutation
/// problem. `OPAQUE_BADGES`'s `BADGE_KEY` is a symbol too, as a VALUE field, and
/// is counted here for the same reason.
///
/// **Every hour the wire arm emits is counted, baseline and delta alike.** A
/// baseline writes every key of every collection; a delta writes the key of each
/// changed row and of each tombstone (`body.rs:encode_delta`), and those keys
/// carry symbol ids too. Counting baselines only understates the band — a round-1
/// review finding, regression-tested by
/// [`a_delta_key_is_counted_toward_the_symbol_band`].
#[derive(Default)]
struct SymbolOccurrences(BTreeMap<u64, usize>);

impl SymbolOccurrences {
    /// The key position at which this collection carries a §5 symbol, if any.
    fn symbol_key_position(collection: &wire_format::schema::Collection) -> Option<usize> {
        collection.key_fields().iter().position(|f| {
            f.field_id == d51::field::VARIABLE_NAME || f.field_id == d51::field::NAMED_FLAG_NAME
        })
    }

    fn count_key(&mut self, collection: &wire_format::schema::Collection, key: &wire_format::body::Key) {
        if let Some(position) = Self::symbol_key_position(collection) {
            *self.0.entry(key.0[position] as u64).or_default() += 1;
        }
    }

    /// `OPAQUE_BADGES` carries its symbol as a VALUE field, so it is counted from
    /// the record rather than the key — and only where the record is present
    /// (a tombstone carries none) and the value is a `ContentId` rather than the
    /// declared `Null`.
    fn count_record(
        &mut self,
        collection: &wire_format::schema::Collection,
        record: &wire_format::record::Record,
    ) {
        if collection.collection_id == d51::OPAQUE_BADGES {
            if let Some(wire_format::value::Value::ContentId(id)) = record.values.first() {
                *self.0.entry(*id).or_default() += 1;
            }
        }
    }

    /// Everything a BASELINE writes: every key and every record.
    fn count_state(&mut self, schema: &wire_format::Schema, states: &[KeyedState]) {
        for (i, collection) in schema.collections.iter().enumerate() {
            for (key, record) in &states[i] {
                self.count_key(collection, key);
                self.count_record(collection, record);
            }
        }
    }

    /// Everything a DELTA writes: the key and record of each changed row, and the
    /// key of each tombstone.
    fn count_delta(&mut self, collection: &wire_format::schema::Collection, delta: &KeyedDelta) {
        for row in &delta.changed {
            self.count_key(collection, &row.key);
            self.count_record(collection, &row.record);
        }
        for key in &delta.tombstones {
            self.count_key(collection, key);
        }
    }

    /// `(stand_in, best, worst)` symbol-id bytes over all assignments of the
    /// frozen vocabulary.
    ///
    /// **`vocabulary_len` is the whole `Symbols` table, not the number of symbols
    /// the wire arm happened to emit**, and the difference is the round-2 review
    /// finding. §5 assigns ids across the entire content set, so an assignment is
    /// a bijection onto `0..vocabulary_len-1`; the emitted symbols may receive
    /// ANY distinct ids from that range, including the highest ones. A vocabulary
    /// member the wire arm never emits — a key that only ever appeared on a
    /// tombstone, say — still occupies an id and can push an emitted symbol into
    /// a wider varint. Sizing the id space by `self.0.len()` instead reported a
    /// band that the stand-in itself could fall outside of.
    ///
    /// Cheapest: the most frequent emitted symbols take the SHORTEST ids in the
    /// range. Dearest: they take the LONGEST.
    fn band(&self, vocabulary_len: usize) -> (usize, usize, usize) {
        let stand_in: usize = self.0.iter().map(|(id, n)| varint_len(*id) * n).sum();
        // Occurrence counts of the emitted symbols, most frequent first.
        let mut occurrences: Vec<usize> = self.0.values().copied().collect();
        occurrences.sort_unstable_by(|a, b| b.cmp(a));
        // A vocabulary cannot be smaller than the set of symbols actually
        // emitted from it; `max` keeps the arithmetic total if a caller passes a
        // smaller number rather than silently underflowing below.
        let id_space = vocabulary_len.max(occurrences.len());
        let best: usize = occurrences
            .iter()
            .enumerate()
            .map(|(rank, count)| count * varint_len(rank as u64))
            .sum();
        let worst: usize = occurrences
            .iter()
            .enumerate()
            .map(|(rank, count)| count * varint_len((id_space - 1 - rank) as u64))
            .sum();
        (stand_in, best, worst)
    }
}

fn main() {
    const USAGE: &str = "usage: bench_real_d51 <collector_stream.bin> <out dir> \
                         [legacy|identity-v1] [block hours]";
    let mut args = std::env::args().skip(1);
    let input = args.next().expect(USAGE);
    let out_dir = args.next().expect(USAGE);
    let identity_mode = match args.next().as_deref() {
        None | Some("legacy") => StreamIdentityMode::Legacy,
        Some("identity-v1") => StreamIdentityMode::IdentityV1,
        Some(other) => panic!("unknown identity mode {other:?}; {USAGE}"),
    };
    let block_hours: usize = match args.next() {
        None => DEFAULT_BLOCK_HOURS,
        Some(raw) => raw
            .parse()
            .ok()
            .filter(|n| *n > 0)
            .unwrap_or_else(|| panic!("block hours must be a positive integer; {USAGE}")),
    };
    let stream = std::fs::read(&input).expect("read stream");
    let schema = d51::schema();

    // ---- Pass 1: validate the WHOLE stream and scan its d51 DATA records. ----
    let scan = match scan_stream(&stream, identity_mode) {
        Ok(scan) => scan,
        Err(reason) => {
            eprintln!("{input}: {reason}");
            std::process::exit(1);
        }
    };
    std::fs::create_dir_all(&out_dir).expect("out dir");

    // §5: the table is frozen from the whole validated stream before any record is
    // mapped, and never grown by the mapper.
    let symbols = Symbols::from_vocabulary(scan.vocabulary.iter().cloned());

    // ---- Pass 2: re-fold through the SAME boundary while encoding. ----------

    let mut ship = Vec::new();
    let mut ship_offsets = vec![0usize];
    let mut wire = Vec::new();
    let mut wire_offsets = vec![0usize];

    let mut fold = State::new();
    let mut prior: Option<Vec<KeyedState>> = None;
    let mut reader: Option<Vec<KeyedState>> = None;
    let mut hours_seen = 0usize;
    let mut baselines = 0usize;
    let mut gap_forced_baselines = 0usize;
    let mut baseline_flags: Vec<bool> = Vec::new();
    let mut empty_hours = 0usize;
    // Cadence-independent churn: changed rows over MEASURED-HOUR transitions,
    // taken before the block decision, so a change landing on a block boundary
    // still counts.
    let mut transition_changed = [0usize; COLLECTION_COUNT];
    let mut transition_tombstones = [0usize; COLLECTION_COUNT];
    let mut transitions = 0usize;
    // Delta-path churn: over the hours that actually emitted a delta.
    let mut delta_changed = [0usize; COLLECTION_COUNT];
    let mut delta_tombstones = [0usize; COLLECTION_COUNT];
    let mut delta_hours = 0usize;
    let mut rows_total = 0usize;
    let mut shipping_rows_total = 0usize;
    let mut shipping_payload_bytes = 0usize;
    let mut shipping_key_table_bytes = 0usize;
    let mut symbol_occurrences = SymbolOccurrences::default();
    let mut max_rows_by_collection = [0usize; COLLECTION_COUNT];

    for hour in &scan.hours {
        ship.extend_from_slice(&stream[hour.record_span.clone()]);
        ship_offsets.push(ship.len());
        shipping_payload_bytes += hour.payload_span.len();
        shipping_rows_total += hour.shipping_rows as usize;

        // Re-fold through the same public boundary pass 1 validated. Any error
        // here would be a disagreement between the two passes, not stream content.
        let payload = &stream[hour.payload_span.clone()];
        shipping_key_table_bytes += d51::peek_header(payload)
            .expect("pass 1 accepted this header")
            .key_bytes_len as usize;
        fold.fold_payload(&schema, payload, &symbols)
            .unwrap_or_else(|e| {
                panic!("game hour {}: pass 2 disagrees with pass 1: {e}", hour.game_hour)
            });
        let state = snapshot(&fold);
        let lengths = d51::collection_lengths(&fold);
        for i in 0..COLLECTION_COUNT {
            rows_total += lengths[i];
            max_rows_by_collection[i] = max_rows_by_collection[i].max(lengths[i]);
        }

        // Cadence-independent churn, before the block decision.
        if let Some(prev) = &prior {
            transitions += 1;
            for (i, collection) in schema.collections.iter().enumerate() {
                let d = diff(collection, &prev[i], &state[i]);
                transition_changed[i] += d.changed.len();
                transition_tombstones[i] += d.tombstones.len();
            }
        }

        let outcome = encode_wire_hour(
            &schema,
            hours_seen,
            block_hours,
            hour.follows_previous,
            hour.game_hour,
            prior.as_deref(),
            &state,
            &mut reader,
            &mut wire,
            &mut symbol_occurrences,
        );
        if outcome.gap_forced {
            gap_forced_baselines += 1;
        }
        baseline_flags.push(outcome.opened_baseline);
        if outcome.opened_baseline {
            baselines += 1;
        } else if outcome.emitted {
            delta_hours += 1;
            for i in 0..COLLECTION_COUNT {
                delta_changed[i] += outcome.changed_by_collection[i];
                delta_tombstones[i] += outcome.tombstones_by_collection[i];
            }
        } else {
            empty_hours += 1;
        }
        wire_offsets.push(wire.len());

        prior = Some(state);
        hours_seen += 1;
    }

    for (name, bytes) in [("ship_d51.bin", &ship), ("wire_new.bin", &wire)] {
        let mut f = std::fs::File::create(std::path::Path::new(&out_dir).join(name)).unwrap();
        f.write_all(bytes).unwrap();
    }
    for (name, offsets) in [
        ("ship_d51.offsets", &ship_offsets),
        ("wire_new.offsets", &wire_offsets),
    ] {
        let mut f = std::fs::File::create(std::path::Path::new(&out_dir).join(name)).unwrap();
        for o in offsets {
            writeln!(f, "{o}").unwrap();
        }
    }

    // The wire arm's baseline/delta raw split, from the per-hour baseline
    // decisions the encode loop actually made (a §1.5 gap-forced baseline is a
    // baseline wherever it sits in the block, so block position alone cannot
    // classify these bytes).
    let mut baseline_bytes = 0usize;
    let mut delta_bytes = 0usize;
    for (index, window) in wire_offsets.windows(2).enumerate() {
        let len = window[1] - window[0];
        if baseline_flags[index] {
            baseline_bytes += len;
        } else {
            delta_bytes += len;
        }
    }

    let counts = &scan.counts;
    // The band is over the FROZEN vocabulary's whole id space, not over the
    // symbols this arm happened to emit — see `SymbolOccurrences::band`.
    let (stand_in_bytes, best, worst) = symbol_occurrences.band(symbols.len());
    let names = ["variables", "named_flags", "token_flags", "badges", "delayed"];

    println!(
        "hours                {hours_seen} ({} non-contiguous steps)",
        counts.non_contiguous
    );
    println!(
        "excluded hours       {} failure-status, {} delta-before-baseline",
        counts.failure_records, counts.delta_before_baseline_records
    );
    println!(
        "shipping frames      {} baseline, {} delta (status OR {:#x}; names-partial on {} hours)",
        scan.hours.iter().filter(|h| h.is_baseline).count(),
        scan.hours.iter().filter(|h| !h.is_baseline).count(),
        counts.status_flags_or,
        counts.names_partial_hours
    );
    println!(
        "shipping rows        {shipping_rows_total} total, {:.2}/hour ({} folded, {} tombstones)",
        shipping_rows_total as f64 / hours_seen as f64,
        counts.rows_folded,
        counts.tombstones_folded
    );
    print!("rows by kind         ");
    for (i, name) in names.iter().enumerate() {
        print!("{name}={} ", counts.live_rows_by_collection[i]);
    }
    println!();
    println!(
        "state accumulated    {:.1} rows avg over all collections",
        rows_total as f64 / hours_seen as f64
    );
    print!("max rows held        ");
    for (i, name) in names.iter().enumerate() {
        print!("{name}={} ", max_rows_by_collection[i]);
    }
    println!();
    println!(
        "symbols              {} scripted keys (\u{a7}5 vocabulary, frozen before mapping)",
        symbols.len()
    );
    println!(
        "baselines            {baselines} ({} at a {block_hours}-h block start, \
         {gap_forced_baselines} forced by a WIRE_FORMAT \u{a7}1.5 state gap)",
        baselines - gap_forced_baselines
    );
    println!("delta hours          {delta_hours}   empty hours {empty_hours}");
    println!("churn, cadence-independent (over {transitions} measured-hour transitions):");
    for (i, collection) in schema.collections.iter().enumerate() {
        println!(
            "  {:<12} {:.4} changed rows/hour, {:.4} tombstones/hour",
            names[i],
            transition_changed[i] as f64 / transitions.max(1) as f64,
            transition_tombstones[i] as f64 / transitions.max(1) as f64
        );
        let _ = collection;
    }
    println!("churn, delta path (over {delta_hours} hours that emitted a delta):");
    for (i, _) in schema.collections.iter().enumerate() {
        println!(
            "  {:<12} {:.4} changed rows/hour, {:.4} tombstones/hour",
            names[i],
            delta_changed[i] as f64 / delta_hours.max(1) as f64,
            delta_tombstones[i] as f64 / delta_hours.max(1) as f64
        );
    }
    println!(
        "ship_d51.bin         {} bytes ({} payload = {} rows+header + {} key table, {} framing)",
        ship.len(),
        shipping_payload_bytes,
        shipping_payload_bytes - shipping_key_table_bytes,
        shipping_key_table_bytes,
        ship.len() - shipping_payload_bytes
    );
    println!(
        "wire_new.bin         {} bytes ({:.2}%) — {baseline_bytes} baseline + {delta_bytes} delta",
        wire.len(),
        100.0 * wire.len() as f64 / ship.len() as f64
    );
    println!(
        "symbol-id bytes      {stand_in_bytes} under the sorted stand-in; band over all \
         assignments of this {}-symbol vocabulary: {best}..{worst} B ({:.2}%..{:.2}% of the wire \
         arm); {} of those symbols were actually emitted",
        symbols.len(),
        100.0 * best as f64 / wire.len().max(1) as f64,
        100.0 * worst as f64 / wire.len().max(1) as f64,
        symbol_occurrences.0.len()
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use compact_contract::{
        compact_status_is_failure, COMPACT_DOMAIN_FRAME, COMPACT_STATUS_CONTROL_INVALID,
        COMPACT_STATUS_DELTA_DATA, COMPACT_STATUS_PARTIAL_DATA,
    };
    use wire_format::d51::RowSpec;

    /// Build one H4CR record the way the recorder frames it
    /// (`compact_contract/src/record_header.rs:60`).
    fn record(sequence: u64, game_hour: i32, domain: u16, kind: u16, payload: &[u8]) -> Vec<u8> {
        let mut out = Vec::with_capacity(COMPACT_RECORD_HEADER_LEN + payload.len());
        out.extend_from_slice(b"H4CR");
        out.extend_from_slice(&1u16.to_le_bytes());
        out.extend_from_slice(&(COMPACT_RECORD_HEADER_LEN as u16).to_le_bytes());
        out.extend_from_slice(&sequence.to_le_bytes());
        out.extend_from_slice(&game_hour.to_le_bytes());
        out.extend_from_slice(&domain.to_le_bytes());
        out.extend_from_slice(&kind.to_le_bytes());
        out.extend_from_slice(&(payload.len() as u32).to_le_bytes());
        out.extend_from_slice(&0u32.to_le_bytes());
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

    /// A one-variable d51 payload in the shape every hour of the measured capture
    /// carries: one global scripted variable, names-partial badged.
    fn d51_payload(status_flags: u32, name: &str, value: i64) -> Vec<u8> {
        d51::build_payload(
            status_flags,
            &[RowSpec {
                scope_kind: d51::SCOPE_GLOBAL,
                owner_id: 0,
                raw_value_kind: d51::VALUE_VARIABLE_FIXED_I64,
                flags: d51::ROW_FLAG_KEY_PRESENT | d51::ROW_FLAG_VALUE_PRESENT,
                key: Some(name.to_string()),
                value_raw: value,
                ..RowSpec::default()
            }],
        )
    }

    /// Two closed hours: a baseline then a delta, each carrying one d51 record.
    fn good_stream() -> Vec<u8> {
        let mut s = Vec::new();
        s.extend(record(
            1,
            100,
            D51,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &d51_payload(0, "alpha", 10),
        ));
        s.extend(frame_end(2, 100));
        s.extend(record(
            3,
            101,
            D51,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &d51_payload(COMPACT_STATUS_DELTA_DATA, "alpha", 11),
        ));
        s.extend(frame_end(4, 101));
        s
    }

    fn scan(stream: &[u8]) -> Result<StreamScan, String> {
        scan_stream(stream, StreamIdentityMode::Legacy)
    }

    #[test]
    fn a_well_formed_stream_yields_its_d51_hours() {
        let s = scan(&good_stream()).expect("accepted");
        assert_eq!(s.hours.len(), 2);
        assert_eq!(s.hours[0].game_hour, 100);
        assert!(s.hours[0].is_baseline);
        assert!(!s.hours[1].is_baseline);
        assert_eq!(s.counts.failure_records, 0);
        assert_eq!(s.counts.delta_before_baseline_records, 0);
        assert_eq!(s.counts.non_contiguous, 0);
        assert_eq!(s.counts.rows_folded, 2);
        assert_eq!(s.counts.live_rows_by_collection[0], 2);
        assert_eq!(s.vocabulary.iter().cloned().collect::<Vec<_>>(), ["alpha"]);
    }

    #[test]
    fn the_failure_predicate_matches_the_folds_own() {
        // `world_fold.rs:18928`-`:18932` computes
        //   (FAILURE_MASK & !OPAQUE_STATUS_MASK) | SOURCE_UNAVAILABLE | PARTIAL_DATA
        // by hand. The harness uses `compact_contract`'s own predicate over the
        // globally-interpretable word instead of copying that expression, so the
        // equality is asserted over the WHOLE status space rather than assumed.
        const SOURCE_UNAVAILABLE: u32 = 1 << 8;
        let opaque_mask =
            d51::STATUS_DELAYED_EVENTS_ROOT_UNRESOLVED | d51::STATUS_SCRIPTED_GUI_ROOT_UNRESOLVED;
        for status in (0..32).flat_map(|bit| {
            let single = 1u32 << bit;
            [single, single | COMPACT_STATUS_DELTA_DATA, single | opaque_mask]
        }) {
            let fold_says = (status & (COMPACT_STATUS_FAILURE_MASK & !opaque_mask)) != 0
                || (status & (SOURCE_UNAVAILABLE | COMPACT_STATUS_PARTIAL_DATA)) != 0;
            let harness_says =
                compact_status_failure_flags_after_expected_partial_tolerance(D51, status) != 0;
            assert_eq!(fold_says, harness_says, "status {status:#x}");
        }
        // The two facts that make this non-trivial: bit 12 is globally
        // CONTROL_INVALID and inside the failure mask, but is d51's
        // delayed-events badge and NOT a failure here; and d51 is not in
        // `COMPACT_EXPECTED_PARTIAL_TOLERANCES`, so PARTIAL_DATA stays a failure.
        assert!(compact_status_is_failure(COMPACT_STATUS_CONTROL_INVALID));
        assert_eq!(
            compact_status_failure_flags_after_expected_partial_tolerance(
                D51,
                d51::STATUS_FLAG_NAMES_PARTIAL
                    | d51::STATUS_DELAYED_EVENTS_ROOT_UNRESOLVED
                    | d51::STATUS_SCRIPTED_GUI_ROOT_UNRESOLVED
            ),
            0
        );
        assert_ne!(
            compact_status_failure_flags_after_expected_partial_tolerance(
                D51,
                COMPACT_STATUS_PARTIAL_DATA
            ),
            0
        );
    }

    #[test]
    fn an_unassigned_status_bit_is_refused_not_classified() {
        // The failure helper classifies the assigned v1 vocabulary and is not a
        // validity predicate, so an unassigned bit must be rejected BEFORE
        // classification — otherwise it silently reads as "not a failure".
        for bit in 14..32u32 {
            for extra in [0u32, 1, COMPACT_STATUS_DELTA_DATA] {
                let mut s = Vec::new();
                s.extend(record(
                    1,
                    100,
                    D51,
                    COMPACT_RECORD_TYPE_DOMAIN_DATA,
                    &d51_payload((1 << bit) | extra, "alpha", 10),
                ));
                s.extend(frame_end(2, 100));
                assert!(
                    scan(&s).is_err(),
                    "status bit {bit} (+{extra:#x}) must be refused"
                );
            }
        }
        // ...and this domain's three lane-local badges, which ALIAS assigned bits
        // 11, 12 and 13, are still accepted. Bit 11 rides every hour of the real
        // capture, so a harness that stripped nothing would refuse the whole
        // stream; bits 12 and 13 need their badge rows, so they are checked
        // through the classifier rather than a whole stream here.
        let mut s = Vec::new();
        s.extend(record(
            1,
            100,
            D51,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &d51_payload(d51::STATUS_FLAG_NAMES_PARTIAL, "alpha", 10),
        ));
        s.extend(frame_end(2, 100));
        assert_eq!(scan(&s).expect("names-partial is accepted").hours.len(), 1);
        for badge in [
            d51::STATUS_DELAYED_EVENTS_ROOT_UNRESOLVED,
            d51::STATUS_SCRIPTED_GUI_ROOT_UNRESOLVED,
        ] {
            assert_eq!(
                compact_status_unknown_flags_v1(compact_status_for_global_interpretation(
                    D51, badge
                )),
                0
            );
        }
    }

    #[test]
    fn the_authority_runs_before_the_exclusion() {
        // A malformed payload on a FAILURE-status record still refuses the stream,
        // because production's payload authority is status-independent.
        let mut payload = d51_payload(1 << 0, "alpha", 10);
        // Break `changed_rows`, which the FOLD never reads and the authority does.
        payload[16..20].copy_from_slice(&99u32.to_le_bytes());
        let mut s = Vec::new();
        s.extend(record(1, 100, D51, COMPACT_RECORD_TYPE_DOMAIN_DATA, &payload));
        s.extend(frame_end(2, 100));
        assert!(scan(&s).is_err());
    }

    #[test]
    fn an_excluded_hour_forces_the_next_observation_to_be_a_baseline() {
        // `WIRE_FORMAT.md:187`-`:190`: "A gap affecting the state part forces the
        // next state observation to be a `BASELINE`." Both gap shapes d51 can
        // meet — an excluded failure-status hour, and a whole hour the capture
        // never scheduled — must clear `follows_previous`, and `opens_baseline`
        // must open a baseline on it wherever it lands in a block. A delta would
        // relate two states across unobserved time.
        //
        // Shape 1: hour 101 carries a failure-status d51 record (excluded).
        let mut s = Vec::new();
        s.extend(record(
            1,
            100,
            D51,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &d51_payload(0, "alpha", 10),
        ));
        s.extend(frame_end(2, 100));
        s.extend(record(
            3,
            101,
            D51,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &d51_payload(1 << 0, "alpha", 11), // failure status -> excluded
        ));
        s.extend(frame_end(4, 101));
        s.extend(record(
            5,
            102,
            D51,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &d51_payload(COMPACT_STATUS_DELTA_DATA, "alpha", 12),
        ));
        s.extend(frame_end(6, 102));
        let scanned = scan(&s).expect("well-formed");
        assert_eq!(scanned.hours.len(), 2);
        assert_eq!(scanned.counts.failure_records, 1);
        assert_eq!(scanned.counts.non_contiguous, 1);
        assert!(!scanned.hours[0].follows_previous, "the first hour follows none");
        assert!(
            !scanned.hours[1].follows_previous,
            "hour 102 does not follow hour 100"
        );
        assert!(opens_baseline(1, DEFAULT_BLOCK_HOURS, scanned.hours[1].follows_previous));

        // ...and that is what makes the encoder open a baseline for it, through
        // the PRODUCTION encode path and not just the decision function.
        let schema = d51::schema();
        let symbols = Symbols::from_vocabulary(scanned.vocabulary.iter().cloned());
        let mut fold = State::new();
        let mut states = Vec::new();
        for hour in &scanned.hours {
            fold.fold_payload(&schema, &s[hour.payload_span.clone()], &symbols)
                .expect("pass 1 accepted this payload");
            states.push(snapshot(&fold));
        }
        let mut wire = Vec::new();
        let mut reader: Option<Vec<KeyedState>> = None;
        let mut occurrences = SymbolOccurrences::default();
        let o0 = encode_wire_hour(
            &schema,
            0,
            DEFAULT_BLOCK_HOURS,
            scanned.hours[0].follows_previous,
            scanned.hours[0].game_hour,
            None,
            &states[0],
            &mut reader,
            &mut wire,
            &mut occurrences,
        );
        assert!(o0.opened_baseline && !o0.gap_forced);
        let o1 = encode_wire_hour(
            &schema,
            1,
            DEFAULT_BLOCK_HOURS,
            scanned.hours[1].follows_previous,
            scanned.hours[1].game_hour,
            Some(&states[0]),
            &states[1],
            &mut reader,
            &mut wire,
            &mut occurrences,
        );
        assert!(
            o1.opened_baseline && o1.gap_forced,
            "the post-gap hour must be a gap-forced baseline, not a delta"
        );

        // Shape 2: hour 101 is never scheduled at all.
        let mut s = Vec::new();
        s.extend(record(
            1,
            100,
            D51,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &d51_payload(0, "alpha", 10),
        ));
        s.extend(frame_end(2, 100));
        s.extend(record(
            3,
            102,
            D51,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &d51_payload(COMPACT_STATUS_DELTA_DATA, "alpha", 12),
        ));
        s.extend(frame_end(4, 102));
        let scanned = scan(&s).expect("well-formed");
        assert_eq!(scanned.counts.non_contiguous, 1);
        assert!(!scanned.hours[1].follows_previous);
    }

    #[test]
    fn the_gap_rule_decides_the_encoding_not_what_changed() {
        // d50's regression, restated: a gap-forced baseline must not disturb the
        // churn counters, which count a transition between every pair of
        // consecutive MEASURED states — across an excluded hour included.
        let schema = d51::schema();
        let symbols = Symbols::from_vocabulary(["alpha".to_string()]);
        let mut fold = State::new();
        let mut states = Vec::new();
        for (status, value) in [
            (0u32, 1i64),
            (COMPACT_STATUS_DELTA_DATA, 2),
            (COMPACT_STATUS_DELTA_DATA, 3),
        ] {
            fold.fold_payload(&schema, &d51_payload(status, "alpha", value), &symbols)
                .expect("folds");
            states.push(snapshot(&fold));
        }
        // The transition count is over measured states, whatever the encoder did.
        let mut transitions = 0usize;
        let mut changed = 0usize;
        for pair in states.windows(2) {
            transitions += 1;
            for (i, collection) in schema.collections.iter().enumerate() {
                changed += diff(collection, &pair[0][i], &pair[1][i]).changed.len();
            }
        }
        assert_eq!((transitions, changed), (2, 2));
    }

    #[test]
    fn a_delta_before_any_baseline_is_an_excluded_hour_and_forces_a_baseline_after_it() {
        let mut s = Vec::new();
        s.extend(record(
            1,
            100,
            D51,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &d51_payload(COMPACT_STATUS_DELTA_DATA, "alpha", 10),
        ));
        s.extend(frame_end(2, 100));
        s.extend(record(
            3,
            101,
            D51,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &d51_payload(0, "alpha", 11),
        ));
        s.extend(frame_end(4, 101));
        let scanned = scan(&s).expect("not a run rejection");
        assert_eq!(scanned.counts.delta_before_baseline_records, 1);
        assert_eq!(scanned.hours.len(), 1);
        assert!(!scanned.hours[0].follows_previous);
    }

    #[test]
    fn two_d51_records_in_one_hour_refuse_the_stream_whatever_their_status() {
        let mut s = Vec::new();
        s.extend(record(
            1,
            100,
            D51,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &d51_payload(1 << 0, "alpha", 10),
        ));
        s.extend(record(
            2,
            100,
            D51,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &d51_payload(0, "alpha", 10),
        ));
        s.extend(frame_end(3, 100));
        assert!(scan(&s).is_err());
    }

    #[test]
    fn a_missing_d51_record_refuses_the_run_instead_of_shrinking_it() {
        let mut s = Vec::new();
        s.extend(record(
            1,
            100,
            D51,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &d51_payload(0, "alpha", 10),
        ));
        s.extend(frame_end(2, 100));
        // Hour 101 closes with no d51 record at all.
        s.extend(record(
            3,
            101,
            77,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &[0u8; 8],
        ));
        s.extend(frame_end(4, 101));
        let err = scan(&s).expect_err("refused");
        assert!(err.contains("closes 2 game hour(s)"), "{err}");
    }

    #[test]
    fn a_stream_cut_at_a_frame_end_is_accepted_and_anything_else_is_not() {
        let full = good_stream();
        let mut cut = 0usize;
        // The first FRAME_END boundary.
        let mut pos = 0usize;
        for _ in 0..2 {
            let h = parse_compact_record_header(&full[pos..]).expect("framed");
            pos += COMPACT_RECORD_HEADER_LEN + h.payload_len;
            cut = pos;
        }
        assert_eq!(scan(&full[..cut]).expect("well-formed").hours.len(), 1);
        // A stream cut mid-record, and one that ends after a DATA record.
        assert!(scan(&full[..cut - 1]).is_err());
        let mut pos = cut;
        let h = parse_compact_record_header(&full[pos..]).expect("framed");
        pos += COMPACT_RECORD_HEADER_LEN + h.payload_len;
        assert!(scan(&full[..pos]).is_err(), "an open hour at EOF is refused");
    }

    #[test]
    fn a_stream_rejection_from_the_fold_invalidates_the_run() {
        // A payload whose length is not exactly `64 + rows*48 + key_bytes` is an
        // `Err` from the fold, not an excluded hour (`world_fold.rs:18947`).
        let mut payload = d51_payload(0, "alpha", 10);
        payload.push(0);
        let mut s = Vec::new();
        s.extend(record(1, 100, D51, COMPACT_RECORD_TYPE_DOMAIN_DATA, &payload));
        s.extend(frame_end(2, 100));
        assert!(scan(&s).is_err());
        // ...and so is a schema version the fold does not understand.
        let mut payload = d51_payload(0, "alpha", 10);
        payload[52..56].copy_from_slice(&1u32.to_le_bytes());
        let mut s = Vec::new();
        s.extend(record(1, 100, D51, COMPACT_RECORD_TYPE_DOMAIN_DATA, &payload));
        s.extend(frame_end(2, 100));
        assert!(scan(&s).is_err());
    }

    #[test]
    fn the_symbol_band_is_the_exact_cost_of_a_reassignment() {
        assert_eq!(varint_len(0), 1);
        assert_eq!(varint_len(127), 1);
        assert_eq!(varint_len(128), 2);

        // A two-symbol vocabulary: both ids are one-byte under every assignment,
        // so the band is tight and the stand-in sits inside it.
        let mut occurrences = SymbolOccurrences::default();
        occurrences.0.insert(0, 10);
        occurrences.0.insert(1, 1);
        let (stand_in, best, worst) = occurrences.band(2);
        assert_eq!((stand_in, best, worst), (11, 11, 11));

        // A vocabulary that straddles the one-byte boundary is where the
        // assignment actually matters: 129 symbols, ids 0..=127 one byte and 128
        // two. The cheapest assignment gives the two-byte id to the rarest
        // symbol, the dearest to the most frequent.
        let mut occurrences = SymbolOccurrences::default();
        for id in 0..128u64 {
            occurrences.0.insert(id, 1);
        }
        occurrences.0.insert(128, 100);
        let (stand_in, best, worst) = occurrences.band(129);
        assert_eq!(stand_in, 128 + 200, "the frequent symbol drew the long id");
        assert_eq!(best, 100 + 127 + 2, "the rarest symbol pays the second byte");
        assert_eq!(worst, 200 + 128, "the most frequent one does");
        assert!(best <= stand_in && stand_in <= worst);
    }

    /// Round-2 review finding, regression-tested with the reviewer's own input.
    ///
    /// The band sized the id space by the number of symbols the wire arm EMITTED
    /// rather than by the frozen vocabulary. §5 assigns ids across the whole
    /// content set, so a vocabulary member the arm never emits — a key that only
    /// ever appeared on a tombstone, for instance — still occupies an id and can
    /// push an emitted symbol into a wider varint.
    ///
    /// The reviewer's input: a 130-symbol vocabulary of which the wire arm emits
    /// exactly one symbol, once. Sizing by the emitted set reported `1..1`, a band
    /// the sorted stand-in (id 129, two bytes) falls OUTSIDE. The correct band is
    /// `1..2`.
    #[test]
    fn the_band_ranges_over_the_frozen_vocabulary_not_the_emitted_subset() {
        let mut occurrences = SymbolOccurrences::default();
        occurrences.0.insert(129, 1);
        let (stand_in, best, worst) = occurrences.band(130);
        assert_eq!(stand_in, 2, "the stand-in's own id is a two-byte varint");
        assert_eq!((best, worst), (1, 2));
        assert!(
            best <= stand_in && stand_in <= worst,
            "the stand-in must lie inside the band it is compared against"
        );
        // Sizing the id space by the emitted set alone is the defect: it would
        // report a degenerate 1..1.
        assert_eq!(occurrences.band(1), (2, 1, 1));

        // The property, swept: for any vocabulary size and any emitted subset,
        // the stand-in lies inside the band, and the band widens (never narrows)
        // as unemitted vocabulary members are added.
        for vocabulary in [1usize, 2, 128, 129, 200, 4_354] {
            for emitted in [1usize, 2, 64, 130] {
                if emitted > vocabulary {
                    continue;
                }
                let mut occurrences = SymbolOccurrences::default();
                for rank in 0..emitted as u64 {
                    // Ids taken from the top of the space, as a sorted stand-in
                    // over a vocabulary whose emitted members sort last would.
                    occurrences.0.insert(vocabulary as u64 - 1 - rank, rank as usize + 1);
                }
                let (stand_in, best, worst) = occurrences.band(vocabulary);
                assert!(
                    best <= stand_in && stand_in <= worst,
                    "vocabulary {vocabulary}, emitted {emitted}: {best} <= {stand_in} <= {worst}"
                );
                let (_, wider_best, wider_worst) = occurrences.band(vocabulary + 1_000);
                assert!(wider_best <= best.max(wider_best));
                assert!(wider_worst >= worst, "a larger id space cannot cost less");
            }
        }
    }

    #[test]
    fn the_reader_replays_every_hour_of_a_small_stream() {
        // The end-to-end property the measurement rests on, exercised through the
        // production encode path.
        let s = good_stream();
        let scanned = scan(&s).expect("well-formed");
        let schema = d51::schema();
        let symbols = Symbols::from_vocabulary(scanned.vocabulary.iter().cloned());
        let mut fold = State::new();
        let mut prior: Option<Vec<KeyedState>> = None;
        let mut reader: Option<Vec<KeyedState>> = None;
        let mut wire = Vec::new();
        let mut occurrences = SymbolOccurrences::default();
        for (index, hour) in scanned.hours.iter().enumerate() {
            fold.fold_payload(&schema, &s[hour.payload_span.clone()], &symbols)
                .expect("folds");
            let state = snapshot(&fold);
            // `encode_wire_hour` asserts the replay internally.
            encode_wire_hour(
                &schema,
                index,
                DEFAULT_BLOCK_HOURS,
                hour.follows_previous,
                hour.game_hour,
                prior.as_deref(),
                &state,
                &mut reader,
                &mut wire,
                &mut occurrences,
            );
            prior = Some(state);
        }
        assert!(!wire.is_empty());
    }

    /// Round-1 review finding, regression-tested with the reviewer's own input.
    ///
    /// The band counted only the symbols a BASELINE writes, but `encode_delta`
    /// writes the key of every changed row and of every tombstone, and those keys
    /// carry symbol ids too. The measured consequence on the real capture was 89
    /// omitted varints (87 variable changes + 2 tombstones). The fix counts at the
    /// encode site, so this test drives the PRODUCTION encode path: an hour that
    /// opens a baseline, then a delta that changes a key, then a delta that
    /// tombstones it.
    #[test]
    fn a_delta_key_is_counted_toward_the_symbol_band() {
        let schema = d51::schema();
        let symbols = Symbols::from_vocabulary(["alpha".to_string()]);
        let alpha = symbols.id("alpha").expect("in the table");

        // Hour 1 baseline (one variable), hour 2 delta (value changes), hour 3
        // delta (the row is tombstoned).
        let payloads = [
            d51_payload(0, "alpha", 1),
            d51_payload(COMPACT_STATUS_DELTA_DATA, "alpha", 2),
            d51::build_payload(
                COMPACT_STATUS_DELTA_DATA,
                &[RowSpec {
                    scope_kind: d51::SCOPE_GLOBAL,
                    owner_id: 0,
                    raw_value_kind: d51::VALUE_VARIABLE_FIXED_I64 | d51::VALUE_KIND_DELETED,
                    flags: d51::ROW_FLAG_KEY_PRESENT,
                    key: Some("alpha".to_string()),
                    ..RowSpec::default()
                }],
            ),
        ];

        let mut fold = State::new();
        let mut prior: Option<Vec<KeyedState>> = None;
        let mut reader: Option<Vec<KeyedState>> = None;
        let mut wire = Vec::new();
        let mut occurrences = SymbolOccurrences::default();
        for (index, payload) in payloads.iter().enumerate() {
            fold.fold_payload(&schema, payload, &symbols).expect("folds");
            let state = snapshot(&fold);
            let outcome = encode_wire_hour(
                &schema,
                index,
                DEFAULT_BLOCK_HOURS,
                index != 0,
                100 + index as i32,
                prior.as_deref(),
                &state,
                &mut reader,
                &mut wire,
                &mut occurrences,
            );
            assert_eq!(outcome.opened_baseline, index == 0);
            assert!(outcome.emitted, "hour {index} changed something");
            prior = Some(state);
        }
        // One occurrence from the baseline, one from the changed row's key, one
        // from the tombstone's key. Counting baselines alone would report 1.
        assert_eq!(occurrences.0.get(&alpha).copied(), Some(3));
        let (stand_in, best, worst) = occurrences.band(symbols.len());
        assert_eq!((stand_in, best, worst), (3, 3, 3));
    }
}
