//! Measure the wire format against the shipping compact stream on REAL d46
//! `air_variant_archetypes` dynamics.
//!
//! Input: a real `collector_stream.bin` from a d46-scheduled capture
//! (`runs/batch3/deepwar_a`, and `deepwar_b` for the coverage the first one does
//! not have). The stream's own d46 DATA records are the shipping format's cost for
//! the domain: a 48-byte header, a fixed 32-byte row per changed variant, three
//! length-prefixed strings per row, and an optional `ACE1` ace-roster trailer —
//! exactly what the recorder stores today. This tool replays those records into
//! per-hour states — the same fold `world_fold::fold_air_variant_archetypes`
//! performs (`world_fold.rs:16319`) — and re-encodes every hour with this crate's
//! grammar: a BASELINE at each 48-hour block start and at every `WIRE_FORMAT.md`
//! §1.5 state gap, field-granular stored-minimal DELTAs between.
//!
//! **The ace roster is stop-reported** (`d46.rs` module docs): its free text has no
//! representable target and its retain-on-incomplete membership needs §8's gap
//! record, which has no implementation. Its bytes are therefore accounted for in
//! the shipping arm both ways the recipe allows — a byte split, and a second cut
//! arm — and never silently charged to the wire arm.
//!
//! **Two passes, because this domain carries content-defined text.** §5 forbids
//! in-stream interning ("it must be fully read before any seek"), so the symbol
//! table is an input to the mapper and pass 2 cannot mint ids as it goes. Pass 1
//! validates the whole stream and collects the vocabulary with
//! [`wire_format::d46::tokens_in_payload`], retaining only record spans; pass 2
//! re-folds the same spans through the same code path with the frozen table while
//! encoding.
//!
//! **The proof that pass 2 can never need a symbol pass 1 did not see** (the recipe
//! requires it written down, not assumed): every value in every hour's state comes
//! from `State::fold_payload`, which builds records only from the rows of a payload
//! it folds; pass 1 runs `tokens_in_payload` — the same
//! `d46::decode_payload` walk — on exactly the payloads pass 2 folds (the same
//! records, in the same order, with the same status classification), and collects
//! every non-empty token of every row of each. So pass 2's symbol set is a subset
//! of pass 1's by construction. `pass_two_needs_no_symbol_pass_one_did_not_see`
//! pins it on a stream whose second hour introduces a token the first never
//! carried.
//!
//! **Nothing here re-derives a contract it can call.** Payload, row and trailer
//! decoding, the fold's accumulator semantics and its one non-folding exit all go
//! through `wire_format::d46`'s public boundary
//! (`peek_header` / `split_payload` / `decode_payload` / `check_authority_payload` /
//! `key_from_row` / `record_from_row` / `State::fold_payload`); H4CR framing, record
//! semantics, sequence continuity and status classification go through
//! `compact_contract`'s own parser, stateful validator and status predicates.
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
//! **d46's three outcomes, and where each is decided.**
//!
//! 1. **Stream rejection.** Anything [`check_authority_payload`] refuses — it is
//!    production's status-independent payload authority
//!    (`domain_data_authority.rs:7906`, dispatched at `:16966`), whose complaints
//!    make the record `malformed_payload_records` and are refused by
//!    `strict_stream_certification_verdict`. It walks the header, every row, every
//!    string **and the whole `ACE1` trailer**, on every record whatever its
//!    status — so a malformed trailer refuses the stream even on a failure-status
//!    hour, which the FOLD's own copy of the trailer walk cannot see (it sits
//!    behind the failure gate, `world_fold.rs:16342` then `:16493`). Also anything
//!    `State::fold_payload` returns `Err` for on a record it does not exclude.
//! 2. **Excluded hour.** A failure status (`world_fold.rs:16342`-`:16348`). That is
//!    the ONLY non-folding exit this fold has: its baseline is implicit, so there
//!    is no delta-before-baseline arm, and d46 is not in
//!    `COMPACT_EXPECTED_PARTIAL_TOLERANCES` (domains 26, 27, 28 and 54 only), which
//!    `the_expected_partial_tolerance_table_still_omits_d46` asserts so a future
//!    entry breaks the test rather than silently changing what is measured.
//! 3. **Measured hour.** Everything else.
//!
//! **Exactly one production check is outside that subset, and it is a
//! crate-boundary limitation rather than a choice.** Production calls
//! `validate_domain_data_payload_at` on EVERY DOMAIN_DATA record
//! (`recorder_host/src/stream_fold.rs:593`), decoding each domain's payload grammar;
//! `wire_format` cannot depend on `recorder_host` and this crate implements one
//! domain's grammar, so **the only DOMAIN_DATA payload this harness validates is
//! d46's — every other domain's payload bytes pass unread**. A stream carrying a
//! corrupt payload for some other domain is refused there and accepted here. That
//! unvalidated remainder is explicitly BOUNDED: every closed game hour must carry
//! exactly one d46 DATA record of any status (the completeness check in
//! [`scan_stream`]), so a d46 record relabelled to another domain id — or dropped —
//! refuses the run instead of silently shrinking it. A per-capture assertion, not a
//! grammar rule.
//!
//! Outputs, for the compression step (`compress_arms.py` beside the run):
//!   ship_d46.bin          — the d46 DATA records verbatim (32-byte H4CR header +
//!                           payload), concatenated in stream order
//!   ship_d46_no_ace1.bin  — the same records with the stop-reported trailer cut
//!                           and the payload length left as written: a
//!                           size-accounting artifact, NOT a valid record stream
//!   wire_new.bin          — the wire-format bodies, concatenated in hour order
//!   *.offsets             — hour-boundary byte offsets, one per line

use std::collections::BTreeSet;
use std::io::Write;
use std::ops::Range;

use compact_contract::stream_validator::{StatefulStreamValidator, StreamIdentityMode};
use compact_contract::{
    compact_status_failure_flags_after_expected_partial_tolerance, compact_status_unknown_flags_v1,
    expected_partial_tolerance, parse_compact_record_header, COMPACT_FRAME_END_PAYLOAD_LEN,
    COMPACT_RECORD_HEADER_LEN, COMPACT_RECORD_TYPE_DOMAIN_DATA, COMPACT_RECORD_TYPE_DOMAIN_PERF,
    COMPACT_RECORD_TYPE_FRAME_END, COMPACT_STATUS_FAILURE_MASK,
};
use wire_format::body::{
    apply, decode_baseline, decode_delta, diff, encode_baseline, encode_delta, KeyedDelta,
    KeyedState,
};
use wire_format::d46;
use wire_format::Schema;

const D46: u16 = 46;
const BLOCK_HOURS: usize = 48;
/// The ported collections, in schema order: the archetype table alone. The `ACE1`
/// ace roster is stop-reported, so it contributes no collection here and its bytes
/// are accounted for in the shipping arm instead — see the two shipping arms.
const COLLECTIONS: usize = 1;

/// One usable d46 hour: its game hour, its byte span in the source stream (the
/// shipping arm is those bytes verbatim, header included), and the payload
/// composition the byte split needs. **No state is retained here** — this is the
/// two-pass harness, so pass 2 re-folds the span.
#[derive(Debug)]
struct D46Hour {
    game_hour: i32,
    ship_span: Range<usize>,
    /// True when this measured hour is exactly one game hour after the previous
    /// MEASURED hour. False on the first measured hour and after any gap — an
    /// excluded (failure-status) hour or a non-contiguous hour step — which is what
    /// forces the encoder to open a fresh baseline (`opens_baseline`).
    follows_previous: bool,
    /// Wire `row_count` — changed variants this hour, deletes included.
    wire_rows: u32,
    /// Header `observed_variant_rows`, the population the collector walked. Not an
    /// invariant of this fold (it never reads the word); reported because the
    /// batch-3 survey's "rows/h" figure is this one, not `row_count`.
    observed_variant_rows: u32,
    /// The fixed 32-byte row region.
    row_bytes: usize,
    /// The three-strings-per-row region.
    string_bytes: usize,
    /// The stop-reported `ACE1` trailer's bytes in this record, and its row count.
    /// The trailer is the payload's suffix, so it is also what the cut arm removes.
    trailer_bytes: usize,
    ace_rows: u32,
    /// True when the trailer's own status says the fold RETAINED the prior roster
    /// instead of full-replacing it (`world_fold.rs:16564`-`:16584`) — the second,
    /// independent blocker in the stop report.
    ace_retained: bool,
}

/// A census over every wire row of every FOLDED record, so the report's coverage
/// caveats are measured rather than inferred. Each counter is "rows on which this
/// was true"; `distinct_*` are the value sets of the two stamped header words.
#[derive(Debug, Default)]
struct Coverage {
    rows: u64,
    deleted_rows: u64,
    archetype_token_present: u64,
    base_airframe_token_present: u64,
    role_token_present: u64,
    weight_present: u64,
    live_definition: u64,
    static_catalog: u64,
    live_token_dictionary: u64,
    interface_type_zero: u64,
    distinct_static_catalog_hash: BTreeSet<u32>,
    distinct_fixed_scale_raw: BTreeSet<u32>,
}

impl Coverage {
    fn observe(&mut self, row: &wire_format::d46::ParsedRow, stamp: wire_format::d46::HeaderStamp) {
        self.rows += 1;
        if row.is_deleted() {
            self.deleted_rows += 1;
            return;
        }
        self.archetype_token_present += u64::from(row.archetype_token.is_some());
        self.base_airframe_token_present += u64::from(row.base_airframe_token.is_some());
        self.role_token_present += u64::from(row.role_token.is_some());
        self.weight_present +=
            u64::from(row.flags & d46::ROW_FLAG_AIR_SUPERIORITY_WEIGHT != 0);
        self.live_definition += u64::from(row.source_mask & d46::SOURCE_LIVE_DEFINITION != 0);
        self.static_catalog += u64::from(row.source_mask & d46::SOURCE_STATIC_CATALOG != 0);
        self.live_token_dictionary +=
            u64::from(row.source_mask & d46::SOURCE_LIVE_TOKEN_DICTIONARY != 0);
        self.interface_type_zero += u64::from(row.interface_type_raw == 0);
        self.distinct_static_catalog_hash
            .insert(stamp.static_catalog_hash);
        self.distinct_fixed_scale_raw.insert(stamp.fixed_scale_raw);
    }
}

#[derive(Debug)]
struct StreamScan {
    hours: Vec<D46Hour>,
    /// d46 records whose payload status flags mark the domain-hour failed.
    failure_records: usize,
    /// Measured d46 hours that do not follow the previous measured one by one.
    non_contiguous: usize,
    /// Pass 1's §5 vocabulary — see the module docs' completeness proof.
    vocabulary: BTreeSet<String>,
    /// Records that carried an `ACE1` trailer at all.
    ace_trailer_records: usize,
    status_histogram: Vec<(u32, usize)>,
    /// The per-row census behind the report's coverage caveats.
    coverage: Coverage,
}

/// Whether the wire arm opens a BASELINE at this measured hour.
///
/// Two reasons, and both are `WIRE_FORMAT.md` rules, not conveniences: the
/// 48-hour block cadence (every block opens with its own baseline, §1.5's closing
/// paragraph), and a gap — `WIRE_FORMAT.md:187`-`:190`: "A gap affecting the state
/// part forces the next state observation to be a `BASELINE`." Every hour this
/// domain did not successfully sample is such a gap — the excluded failure-status
/// hours, and any non-contiguous hour step — and d46's cadence is hourly, so a
/// measured hour that does not follow the previous measured one by exactly one is
/// preceded by one. Neither a delta nor a retained reader state can be justified
/// across it.
fn opens_baseline(hours_seen: usize, follows_previous: bool) -> bool {
    hours_seen % BLOCK_HOURS == 0 || !follows_previous
}

/// One wire-arm hour: the §1.5/block baseline decision, the encode, and the
/// per-hour reader replay. Factored out of `main` so the tests execute the
/// PRODUCTION encode path over a scanned stream — a mutant that reverts the
/// baseline decision here to block-position-only must fail the gap test.
struct WireHourOutcome {
    opened_baseline: bool,
    /// The baseline sits outside a scheduled block start — §1.5 gap-forced.
    gap_forced: bool,
    emitted: bool,
    bytes: usize,
    changed: usize,
    tombstones: usize,
}

fn encode_wire_hour(
    schema: &Schema,
    hours_seen: usize,
    game_hour: i32,
    follows_previous: bool,
    states: &[KeyedState],
    prior: Option<&[KeyedState]>,
    reader: &mut Option<Vec<KeyedState>>,
    wire: &mut Vec<u8>,
) -> WireHourOutcome {
    let hour_start = wire.len();
    let block_start = opens_baseline(hours_seen, follows_previous);
    let gap_forced = block_start && hours_seen % BLOCK_HOURS != 0;
    let mut emitted = true;
    let mut changed = 0usize;
    let mut tombstones = 0usize;
    let opened_baseline = block_start || prior.is_none();
    if opened_baseline {
        encode_baseline(schema, states, wire).expect("baseline encodes");
    } else {
        let prev = prior.expect("a delta has a prior");
        let deltas: Vec<Option<KeyedDelta>> = schema
            .collections
            .iter()
            .enumerate()
            .map(|(i, collection)| {
                let d = diff(collection, &prev[i], &states[i]);
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
            for d in deltas.iter().flatten() {
                changed += d.changed.len();
                tombstones += d.tombstones.len();
            }
            encode_delta(schema, prev, &deltas, wire).expect("delta encodes");
        }
    }
    let bytes = wire.len() - hour_start;

    // Reader replay: decode the bytes just written against the reader's OWN
    // reconstructed prior, exactly as a conforming reader would, and require the
    // result to be stored-equal to this hour's state. An hour that emitted nothing
    // is still checked: the reader's retained state must already equal the fold's,
    // which is what "nothing changed" asserts.
    if emitted {
        let hour_bytes = &wire[hour_start..];
        if block_start || reader.is_none() {
            let (decoded, consumed) =
                decode_baseline(schema, hour_bytes, 0).expect("baseline decodes");
            assert_eq!(consumed, hour_bytes.len(), "baseline consumed exactly");
            *reader = Some(decoded);
        } else {
            let reader_prior = reader.take().expect("a delta has a reader prior");
            let (deltas, consumed) =
                decode_delta(schema, &reader_prior, hour_bytes, 0).expect("delta decodes");
            assert_eq!(consumed, hour_bytes.len(), "delta consumed exactly");
            let mut next = reader_prior;
            for (i, d) in deltas.iter().enumerate() {
                if let Some(d) = d {
                    apply(&mut next[i], d);
                }
            }
            *reader = Some(next);
        }
    }
    let replayed = reader.as_ref().expect("a reader state exists");
    for (i, collection) in schema.collections.iter().enumerate() {
        assert_eq!(
            replayed[i].len(),
            states[i].len(),
            "game hour {game_hour}: collection {} row count",
            collection.collection_id
        );
        assert!(
            diff(collection, &replayed[i], &states[i]).is_empty(),
            "game hour {game_hour}: collection {} reader state is not stored-equal to the fold",
            collection.collection_id
        );
    }
    WireHourOutcome {
        opened_baseline,
        gap_forced,
        emitted,
        bytes,
        changed,
        tombstones,
    }
}

/// Every globally-defined compact status occupies one of bits 0..=14
/// (`recorder_host/src/stream_fold.rs:43`).
// Re-exported, not re-declared: a local `(1 << 14) - 1` became wrong the moment
// the contract assigned bit 14 to the measured-empty badge.
const FRAME_END_KNOWN_STATUS_MASK: u32 = compact_contract::COMPACT_STATUS_V1_KNOWN_MASK;

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

/// Pass 1. Validate the whole stream against the canonical contract, classify every
/// d46 DATA record, and collect the §5 vocabulary and the per-hour byte composition.
/// Every rejection path is an `Err`, never a silently shorter measurement and never
/// a panic.
fn scan_stream(stream: &[u8], identity_mode: StreamIdentityMode) -> Result<StreamScan, String> {
    // The identity protocol is an INPUT, never sniffed: an `IdentityV1` stream
    // whose mandatory CID1 record has been spliced off is byte-indistinguishable
    // from a legacy stream, so a sniffing harness measures a decapitated stream
    // that production refuses (`stream_validator.rs:328`, `stream_fold.rs:220`).
    let mut validator = match identity_mode {
        StreamIdentityMode::IdentityV1 => StatefulStreamValidator::identity_v1(),
        StreamIdentityMode::Legacy => StatefulStreamValidator::legacy(),
    };

    // d46 has no expected-partial tolerance, so `handle_compact_failure` treats
    // `COMPACT_STATUS_PARTIAL_DATA` as a plain failure bit and a tolerance branch
    // here would be dead code (the d65 rule). Asserted rather than assumed, so a
    // future table entry fails the run instead of silently changing what is
    // measured.
    if expected_partial_tolerance(D46).is_some() {
        return Err(
            "d46 has gained a COMPACT_EXPECTED_PARTIAL_TOLERANCES entry; this harness models \
             the fold WITHOUT one and must be revisited before it measures anything"
                .to_string(),
        );
    }

    let mut hours: Vec<D46Hour> = Vec::new();
    let mut failure_records = 0usize;
    let mut non_contiguous = 0usize;
    let mut vocabulary: BTreeSet<String> = BTreeSet::new();
    let mut ace_trailer_records = 0usize;
    let mut status_counts: Vec<(u32, usize)> = Vec::new();
    let mut coverage = Coverage::default();
    // COMPLETENESS accounting: every d46 DATA record of ANY status, against every
    // closed game hour — see the check after the EOF rules.
    let mut d46_records = 0usize;
    let mut closed_hours = 0usize;

    let mut open_hour: Option<i32> = None;
    let mut last_closed_hour: Option<i32> = None;
    let mut last_record_type: Option<u16> = None;
    // The DOMAIN_PERF hour-binding state, `BoundaryTracker`'s own
    // (`stream_fold.rs:328`-`:365`, reset at `:449`-`:451`).
    let mut pending_perf_hour: Option<i32> = None;
    let mut non_probe_perf_domains: BTreeSet<u16> = BTreeSet::new();
    // Hour of the last d46 DATA record of ANY status — the uniqueness rule.
    let mut last_d46_hour: Option<i32> = None;
    // Hour of the last d46 record that entered the measurement — the contiguity
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
                    // `stream_fold.rs:378`-`:386`, and it comes BEFORE the
                    // monotonicity check, as it does there.
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

        if header.domain_id == D46 && header.record_type == COMPACT_RECORD_TYPE_DOMAIN_DATA {
            d46_records += 1;
            // One d46 DATA record per game hour, checked over EVERY d46 record and
            // BEFORE any status-based exclusion: gating this on the record being
            // usable would let a success/failure pair in one hour through the rule
            // entirely.
            //
            // DELIBERATELY STRICTER THAN PRODUCTION, and fail-closed. The production
            // boundary tracker permits several DOMAIN_DATA records while an hour is
            // open (`stream_fold.rs:369`) and `WorldFold::ingest_record` would fold
            // each in turn, so a same-hour pair is a stream production certifies and
            // this harness refuses. That is the recipe's standing rule for every
            // measurement bin: a second record for one hour would otherwise become a
            // second measurement hour built from the same clock time, and for a
            // sparse-patch domain the two would silently disagree about what that
            // hour's state is. Refusing produces no number; measuring would produce a
            // wrong one. No capture carries such a pair.
            if let Some(previous) = last_d46_hour {
                if header.game_hour_raw <= previous {
                    return Err(format!(
                        "d46 DATA at byte {pos} carries hour {} after hour {previous}",
                        header.game_hour_raw
                    ));
                }
            }
            last_d46_hour = Some(header.game_hour_raw);

            // VALIDITY BEFORE CLASSIFICATION. The failure predicate classifies the
            // assigned v1 status vocabulary and is not a validity predicate; bits
            // 15..=31 are unassigned (`status_flags_base.rs:40`-`:44`) and an
            // unassigned bit "is not evidence of a future informational status", so a
            // record carrying one refuses the stream rather than reading as "not a
            // failure" and being measured as healthy. The header read is
            // `peek_header`, which is exactly the fold's pre-status checks — the
            // classify boundary, reachable without paying for the body.
            let peeked = d46::peek_header(payload)
                .map_err(|e| format!("d46 payload at byte {pos}: {e}"))?;
            let unknown_status = compact_status_unknown_flags_v1(peeked.status_flags);
            if unknown_status != 0 {
                return Err(format!(
                    "d46 DATA at byte {pos}: hour {} carries unassigned v1 status bits \
                     {unknown_status:#x}; an unassigned bit has no defined classification",
                    header.game_hour_raw
                ));
            }
            match status_counts
                .iter_mut()
                .find(|(flags, _)| *flags == peeked.status_flags)
            {
                Some((_, count)) => *count += 1,
                None => status_counts.push((peeked.status_flags, 1)),
            }

            // Production's SECOND authority, on every record whatever its status —
            // header, rows, strings AND the whole `ACE1` trailer. This is what makes
            // a malformed trailer refuse the stream on a failure-status hour, where
            // the fold's own trailer walk never runs.
            d46::check_authority_payload(payload)
                .map_err(|e| format!("d46 payload at byte {pos}: {e}"))?;

            // The fold's own classification: `compact_contract`'s predicate, not a
            // copied bit mask — it already applies d46's lane-local mask (none) and
            // expected partial tolerance (none), so it cannot drift.
            let status_is_failure = compact_status_failure_flags_after_expected_partial_tolerance(
                D46,
                peeked.status_flags,
            ) != 0;
            if status_is_failure {
                failure_records += 1;
                pos = record_end;
                continue;
            }

            // A folded record: decode it once — the FOLD's own walk — for the §5
            // vocabulary and the byte composition. Pass 2 re-folds the same span
            // through `State::fold_payload`, which repeats this decode; an `Err`
            // here is therefore an `Err` there, which is why pass 2 cannot fail
            // after this pass has passed.
            let decoded = d46::decode_payload(payload)
                .map_err(|e| format!("d46 payload at byte {pos}: {e}"))?;
            let stamp = decoded.header.stamp();
            for row in &decoded.rows {
                coverage.observe(row, stamp);
                for token in [
                    row.archetype_token.as_deref(),
                    row.base_airframe_token.as_deref(),
                    row.role_token.as_deref(),
                ]
                .into_iter()
                .flatten()
                {
                    if !vocabulary.contains(token) {
                        vocabulary.insert(token.to_string());
                    }
                }
            }
            let row_bytes = decoded.rows.len() * d46::ROW_LEN;
            let string_bytes = decoded.base_len - d46::HEADER_LEN - row_bytes;
            let trailer_bytes = payload.len() - decoded.base_len;
            if decoded.ace.is_some() {
                ace_trailer_records += 1;
            }

            let follows_previous =
                last_measured_hour.is_some_and(|previous| header.game_hour_raw == previous + 1);
            if last_measured_hour.is_some() && !follows_previous {
                non_contiguous += 1;
            }
            last_measured_hour = Some(header.game_hour_raw);
            hours.push(D46Hour {
                game_hour: header.game_hour_raw,
                ship_span: pos..record_end,
                follows_previous,
                wire_rows: decoded.header.row_count,
                observed_variant_rows: decoded.header.observed_variant_rows,
                row_bytes,
                string_bytes,
                trailer_bytes,
                ace_rows: decoded.ace.map_or(0, |t| t.row_count),
                ace_retained: decoded.ace.is_some_and(|t| t.retains_prior_state()),
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
        return Err("stream carries no usable d46 DATA records".to_string());
    }

    // COMPLETENESS: every closed game hour must carry exactly one d46 DATA record,
    // of any status. This bounds what this harness canNOT validate — see the module
    // docs. A per-capture assertion, NOT a grammar rule: a capture where d46 is not
    // scheduled every hour fails here loudly and a human adjusts, instead of a
    // number quietly shrinking (`bench_real_d65.rs` precedent).
    if d46_records != closed_hours {
        return Err(format!(
            "stream closes {closed_hours} game hour(s) but carries {d46_records} d46 DATA \
             record(s); every closed hour of a d46-scheduled capture must carry exactly one, and \
             this harness cannot validate the payloads of other domains to see where the \
             difference went"
        ));
    }

    status_counts.sort_unstable();
    Ok(StreamScan {
        hours,
        failure_records,
        non_contiguous,
        vocabulary,
        ace_trailer_records,
        status_histogram: status_counts,
        coverage,
    })
}

/// Pass 2's per-hour fold: re-fold the retained span through the SAME public
/// boundary, with the frozen symbol table.
fn fold_hour(
    schema: &Schema,
    state: &mut d46::State,
    payload: &[u8],
    symbols: &d46::Symbols,
    game_hour: i32,
) -> Vec<KeyedState> {
    match state.fold_payload(schema, payload, false, 0, symbols) {
        Ok(d46::FoldOutcome::Folded) => {}
        Ok(other) => panic!("game hour {game_hour}: pass 2 reclassified the record as {other:?}"),
        Err(reason) => panic!("game hour {game_hour}: pass 2 refused a pass-1 record: {reason}"),
    }
    state.states().to_vec()
}

fn main() {
    const USAGE: &str = "usage: bench_real_d46 <collector_stream.bin> <out dir> \
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
    let schema = d46::schema();

    // ---- Pass 1: validate the WHOLE stream, classify, collect the vocabulary. ---
    let scan = match scan_stream(&stream, identity_mode) {
        Ok(scan) => scan,
        Err(reason) => {
            eprintln!("{input}: {reason}");
            std::process::exit(1);
        }
    };
    let symbols = d46::Symbols::from_vocabulary(scan.vocabulary.iter().cloned());

    // ---- Pass 2: re-fold the same records and encode both arms. ----------------

    let mut ship = Vec::new();
    let mut ship_offsets = vec![0usize];
    // The second, CUT shipping arm. The stop-reported `ACE1` trailer is a suffix of
    // each payload (the base payload is self-delimiting and the trailer must
    // consume the rest exactly, `domain46_air_ace_roster_live.rs:608`-`:613`), so
    // it is cuttable, and d11's `PDQ1` accounting applies: each record with the
    // trailer removed and its payload length left as written. **That is a
    // size-accounting artifact, not a valid record** — a decoder handed this arm
    // would read a length past the bytes — and it exists so the wire arm is
    // compared against a shipping arm carrying only what the wire arm is allowed to
    // encode.
    let mut ship_cut = Vec::new();
    let mut ship_cut_offsets = vec![0usize];
    let mut wire = Vec::new();
    let mut wire_offsets = vec![0usize];

    let mut fold_state = d46::State::new();
    let mut prior: Option<Vec<KeyedState>> = None;
    let mut reader: Option<Vec<KeyedState>> = None;
    let mut hours_seen = 0usize;
    let mut baselines = 0usize;
    let mut gap_forced_baselines = 0usize;
    let mut baseline_bytes = 0usize;
    let mut delta_bytes = 0usize;
    let mut empty_hours = 0usize;
    // Cadence-independent churn: changed rows over MEASURED-HOUR TRANSITIONS, taken
    // before the block decision, so a change landing on a block boundary still
    // counts.
    let mut transition_changed = [0usize; COLLECTIONS];
    let mut transition_tombstones = [0usize; COLLECTIONS];
    let mut transitions = 0usize;
    // Delta-path churn: the same counts over the hours that actually emitted a
    // delta. The denominators are different questions and cannot share a number.
    let mut delta_changed = 0usize;
    let mut delta_tombstones = 0usize;
    let mut rows_total = 0usize;
    let mut wire_rows_total = 0u64;
    let mut observed_rows_total = 0u64;
    let mut wire_rows_min = u32::MAX;
    let mut wire_rows_max = 0u32;
    let mut ship_row_bytes = 0usize;
    let mut ship_string_bytes = 0usize;
    let mut ship_trailer_bytes = 0usize;
    let mut ace_rows_total = 0u64;
    let mut ace_retained_hours = 0usize;

    for hour in &scan.hours {
        let record = &stream[hour.ship_span.clone()];
        ship.extend_from_slice(record);
        ship_offsets.push(ship.len());
        ship_cut.extend_from_slice(&record[..record.len() - hour.trailer_bytes]);
        ship_cut_offsets.push(ship_cut.len());
        ship_row_bytes += hour.row_bytes;
        ship_string_bytes += hour.string_bytes;
        ship_trailer_bytes += hour.trailer_bytes;
        ace_rows_total += u64::from(hour.ace_rows);
        if hour.ace_retained {
            ace_retained_hours += 1;
        }
        wire_rows_total += u64::from(hour.wire_rows);
        observed_rows_total += u64::from(hour.observed_variant_rows);
        wire_rows_min = wire_rows_min.min(hour.wire_rows);
        wire_rows_max = wire_rows_max.max(hour.wire_rows);

        let payload = &record[COMPACT_RECORD_HEADER_LEN..];
        let states = fold_hour(&schema, &mut fold_state, payload, &symbols, hour.game_hour);
        rows_total += states[0].len();

        // Cadence-independent churn, before the block decision.
        if let Some(prev) = &prior {
            transitions += 1;
            for (i, collection) in schema.collections.iter().enumerate() {
                let d = diff(collection, &prev[i], &states[i]);
                transition_changed[i] += d.changed.len();
                transition_tombstones[i] += d.tombstones.len();
            }
        }

        let outcome = encode_wire_hour(
            &schema,
            hours_seen,
            hour.game_hour,
            hour.follows_previous,
            &states,
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
            delta_changed += outcome.changed;
            delta_tombstones += outcome.tombstones;
            delta_bytes += outcome.bytes;
        } else {
            empty_hours += 1;
        }
        wire_offsets.push(wire.len());

        prior = Some(states);
        hours_seen += 1;
    }

    std::fs::create_dir_all(&out_dir).expect("out dir");
    for (name, bytes) in [
        ("ship_d46.bin", &ship),
        ("ship_d46_no_ace1.bin", &ship_cut),
        ("wire_new.bin", &wire),
    ] {
        let mut f = std::fs::File::create(std::path::Path::new(&out_dir).join(name)).unwrap();
        f.write_all(bytes).unwrap();
    }
    for (name, offsets) in [
        ("ship_d46.offsets", &ship_offsets),
        ("ship_d46_no_ace1.offsets", &ship_cut_offsets),
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
        "excluded hours       {} failure-status (the fold's only non-folding exit)",
        scan.failure_records
    );
    println!(
        "status histogram     {}",
        scan.status_histogram
            .iter()
            .map(|(flags, count)| format!("{flags:#05x}x{count}"))
            .collect::<Vec<_>>()
            .join(", ")
    );
    println!(
        "archetype rows avg   {:.1}   wire row_count/h {}…{} (mean {:.1})   \
         header observed_variant_rows/h mean {:.1}",
        rows_total as f64 / hours_seen as f64,
        wire_rows_min,
        wire_rows_max,
        wire_rows_total as f64 / hours_seen as f64,
        observed_rows_total as f64 / hours_seen as f64,
    );
    println!(
        "symbols              {} distinct archetype/airframe/role token(s)",
        symbols.len()
    );
    println!(
        "stopped ace roster   {} record(s) carry an ACE1 trailer, {:.1} ace row(s)/h, \
         {ace_retained_hours} retained-instead-of-replaced hour(s)",
        scan.ace_trailer_records,
        ace_rows_total as f64 / hours_seen as f64,
    );
    let cov = &scan.coverage;
    println!(
        "row census           {} wire row(s) folded, {} tombstone(s);          archetype_token present on {}, base_airframe on {}, role on {},          air_superiority_weight on {}",
        cov.rows,
        cov.deleted_rows,
        cov.archetype_token_present,
        cov.base_airframe_token_present,
        cov.role_token_present,
        cov.weight_present,
    );
    println!(
        "  source bits        LIVE_DEFINITION {}, STATIC_CATALOG {}, LIVE_TOKEN_DICTIONARY {}          (of {} live row(s)); interface_type_raw == 0 on {}",
        cov.live_definition,
        cov.static_catalog,
        cov.live_token_dictionary,
        cov.rows - cov.deleted_rows,
        cov.interface_type_zero,
    );
    println!(
        "  stamped words      static_catalog_hash values {:?}, fixed_scale_raw values {:?}",
        cov.distinct_static_catalog_hash, cov.distinct_fixed_scale_raw
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
        "churn cadence-indep  archetypes {:.2} changed + {:.3} tombstones per transition \
         ({transitions} transitions)",
        transition_changed[0] as f64 / transitions.max(1) as f64,
        transition_tombstones[0] as f64 / transitions.max(1) as f64,
    );
    println!(
        "churn delta-path     archetypes {:.2} changed + {:.3} tombstones per delta hour \
         ({delta_hours} delta hours)",
        delta_changed as f64 / delta_hours.max(1) as f64,
        delta_tombstones as f64 / delta_hours.max(1) as f64,
    );
    // The stop-reported segment, accounted for both ways the recipe allows: a byte
    // split of the shipping arm, and a second cut arm with the segment removed.
    let claimed = ship_row_bytes + ship_string_bytes;
    let unclaimable = ship.len() - claimed - ship_trailer_bytes;
    println!(
        "ship_d46.bin         {} bytes ({claimed} B archetype rows + tokens {:.1}%, \
         {ship_trailer_bytes} B stopped ACE1 trailer {:.1}%, {unclaimable} B payload header + \
         H4CR framing {:.1}%)",
        ship.len(),
        100.0 * claimed as f64 / ship.len() as f64,
        100.0 * ship_trailer_bytes as f64 / ship.len() as f64,
        100.0 * unclaimable as f64 / ship.len() as f64,
    );
    println!(
        "  of the claimed     {ship_row_bytes} B fixed rows, {ship_string_bytes} B token strings"
    );
    println!(
        "ship_d46_no_ace1.bin {} bytes (the same records with the stop-reported trailer cut; a \
         size-accounting artifact, not a valid record stream)",
        ship_cut.len()
    );
    println!(
        "wire_new.bin         {} bytes ({:.1}% of shipping, {:.1}% of the cut arm)",
        wire.len(),
        100.0 * wire.len() as f64 / ship.len() as f64,
        100.0 * wire.len() as f64 / ship_cut.len().max(1) as f64
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use compact_contract::{
        COMPACT_DOMAIN_FRAME, COMPACT_STATUS_DELTA_DATA, COMPACT_STATUS_PARTIAL_DATA,
    };

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
        let mut payload = vec![0u8; COMPACT_FRAME_END_PAYLOAD_LEN];
        // One non-probe DOMAIN_PERF domain per hour: d46's own.
        payload[16..20].copy_from_slice(&1u32.to_le_bytes());
        payload[20..24].copy_from_slice(&1u32.to_le_bytes());
        record(
            sequence,
            game_hour,
            COMPACT_DOMAIN_FRAME,
            COMPACT_RECORD_TYPE_FRAME_END,
            &payload,
        )
    }

    /// One DOMAIN_PERF record with the minimal valid payload.
    fn perf(sequence: u64, game_hour: i32) -> Vec<u8> {
        record(
            sequence,
            game_hour,
            D46,
            COMPACT_RECORD_TYPE_DOMAIN_PERF,
            &vec![0u8; compact_contract::COMPACT_DOMAIN_PERF_BASE_LEN],
        )
    }

    fn compact_string(text: &str) -> Vec<u8> {
        let mut out = (text.len() as u16).to_le_bytes().to_vec();
        out.extend_from_slice(text.as_bytes());
        out
    }

    /// One live 32-byte row with a resolved dictionary token.
    fn live_row(variant_id: u32, equipment_token_raw: u32) -> Vec<u8> {
        let mut row = Vec::with_capacity(d46::ROW_LEN);
        for word in [
            variant_id,
            d46::ROW_FLAG_ARCHETYPE,
            equipment_token_raw,
            1,
            7,
            0,
            d46::SOURCE_LIVE_STATS | d46::SOURCE_LIVE_TOKEN_DICTIONARY,
            0,
        ] {
            row.extend_from_slice(&word.to_le_bytes());
        }
        row
    }

    fn deleted_row(variant_id: u32) -> Vec<u8> {
        let mut row = Vec::with_capacity(d46::ROW_LEN);
        for word in [variant_id, d46::ROW_FLAG_DELETED, 0, 0, 0, 0, 0, 0] {
            row.extend_from_slice(&word.to_le_bytes());
        }
        row
    }

    /// A d46 DATA payload: header, rows, three strings per row, optional trailer.
    fn d46_payload(status_flags: u32, rows: &[(Vec<u8>, &str)], trailer: Option<&[u8]>) -> Vec<u8> {
        let row_count = rows.len() as u32;
        let classified = rows.iter().filter(|(_, token)| !token.is_empty()).count() as u32;
        let mut p = Vec::new();
        p.extend_from_slice(&0u64.to_le_bytes()); // elapsed_wall_ms
        p.extend_from_slice(&row_count.to_le_bytes());
        p.extend_from_slice(&row_count.to_le_bytes()); // changed_rows
        p.extend_from_slice(&status_flags.to_le_bytes());
        p.extend_from_slice(&row_count.to_le_bytes()); // observed_variant_rows
        p.extend_from_slice(&classified.to_le_bytes());
        p.extend_from_slice(&(row_count - classified).to_le_bytes());
        p.extend_from_slice(&0u32.to_le_bytes()); // static_catalog_hash
        p.extend_from_slice(&d46::AUTHORITY_FIXED_SCALE_RAW.to_le_bytes());
        p.extend_from_slice(&0u32.to_le_bytes()); // reserved0
        p.extend_from_slice(&0u32.to_le_bytes()); // reserved1
        for (row, _) in rows {
            p.extend_from_slice(row);
        }
        for (_, token) in rows {
            p.extend_from_slice(&compact_string(token));
            p.extend_from_slice(&compact_string(""));
            p.extend_from_slice(&compact_string(""));
        }
        if let Some(trailer) = trailer {
            p.extend_from_slice(trailer);
        }
        p
    }

    /// A clean, empty `ACE1` trailer — enough to exercise the trailer walk without
    /// building a roster.
    fn empty_ace_trailer() -> Vec<u8> {
        let mut t = Vec::new();
        t.extend_from_slice(&d46::ace::TRAILER_MAGIC);
        t.extend_from_slice(&d46::ace::TRAILER_VERSION_V2.to_le_bytes());
        t.extend_from_slice(&(d46::ace::TRAILER_HEADER_LEN as u16).to_le_bytes());
        t.extend_from_slice(&(d46::ace::TRAILER_ROW_LEN_V2 as u16).to_le_bytes());
        t.extend_from_slice(&0u16.to_le_bytes());
        for word in [0u32, 0, 3, 0, 0, 0, 0] {
            t.extend_from_slice(&word.to_le_bytes());
        }
        t
    }

    /// One closed hour carrying a d46 PERF + DATA record.
    fn hour(sequence: &mut u64, game_hour: i32, payload: &[u8]) -> Vec<u8> {
        let mut out = Vec::new();
        out.extend(perf(*sequence, game_hour));
        *sequence += 1;
        out.extend(record(
            *sequence,
            game_hour,
            D46,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            payload,
        ));
        *sequence += 1;
        out.extend(frame_end(*sequence, game_hour));
        *sequence += 1;
        out
    }

    /// Three contiguous hours: a baseline, a sparse patch, and a tombstone.
    fn good_stream() -> Vec<u8> {
        let mut sequence = 1u64;
        let mut s = Vec::new();
        s.extend(hour(
            &mut sequence,
            100,
            &d46_payload(
                COMPACT_STATUS_DELTA_DATA,
                &[
                    (live_row(1, 20_000), "small_plane_airframe_1"),
                    (live_row(2, 20_001), "small_plane_airframe_2"),
                ],
                Some(&empty_ace_trailer()),
            ),
        ));
        s.extend(hour(
            &mut sequence,
            101,
            &d46_payload(
                COMPACT_STATUS_DELTA_DATA,
                &[(live_row(3, 20_002), "cv_small_plane_airframe_3")],
                Some(&empty_ace_trailer()),
            ),
        ));
        s.extend(hour(
            &mut sequence,
            102,
            &d46_payload(
                COMPACT_STATUS_DELTA_DATA,
                &[(deleted_row(2), "")],
                Some(&empty_ace_trailer()),
            ),
        ));
        s
    }

    fn scan_good(stream: &[u8]) -> StreamScan {
        scan_stream(stream, StreamIdentityMode::Legacy).expect("the good stream validates")
    }

    /// Drive the PRODUCTION encode path over a scanned stream, exactly as `main`
    /// does, and report what each hour did.
    fn measure(scan: &StreamScan, stream: &[u8]) -> (Vec<WireHourOutcome>, usize) {
        let schema = d46::schema();
        let symbols = d46::Symbols::from_vocabulary(scan.vocabulary.iter().cloned());
        let mut fold_state = d46::State::new();
        let mut prior: Option<Vec<KeyedState>> = None;
        let mut reader: Option<Vec<KeyedState>> = None;
        let mut wire = Vec::new();
        let mut outcomes = Vec::new();
        let mut hours_seen = 0usize;
        for hour in &scan.hours {
            let payload = &stream[hour.ship_span.start + COMPACT_RECORD_HEADER_LEN..hour.ship_span.end];
            let states = fold_hour(&schema, &mut fold_state, payload, &symbols, hour.game_hour);
            let outcome = encode_wire_hour(
                &schema,
                hours_seen,
                hour.game_hour,
                hour.follows_previous,
                &states,
                prior.as_deref(),
                &mut reader,
                &mut wire,
            );
            outcomes.push(outcome);
            prior = Some(states);
            hours_seen += 1;
        }
        (outcomes, wire.len())
    }

    // --- §1.5: a gap forces a baseline ---------------------------------------

    #[test]
    fn a_gap_between_measured_hours_forces_a_baseline() {
        // The decision function, swept over every block position.
        for hours_seen in 0..(3 * BLOCK_HOURS) {
            assert!(
                opens_baseline(hours_seen, false),
                "a gap at block position {hours_seen} must open a baseline"
            );
            assert_eq!(
                opens_baseline(hours_seen, true),
                hours_seen % BLOCK_HOURS == 0,
                "a contiguous hour is cadence-only at {hours_seen}"
            );
        }
        // And the callsite: a stream whose middle hour is excluded by a failure
        // status must open a gap-forced baseline on the hour after it.
        let mut sequence = 1u64;
        let mut s = Vec::new();
        s.extend(hour(
            &mut sequence,
            100,
            &d46_payload(
                COMPACT_STATUS_DELTA_DATA,
                &[(live_row(1, 20_000), "a")],
                None,
            ),
        ));
        s.extend(hour(
            &mut sequence,
            101,
            &d46_payload(
                COMPACT_STATUS_DELTA_DATA | COMPACT_STATUS_PARTIAL_DATA,
                &[(live_row(2, 20_001), "b")],
                None,
            ),
        ));
        s.extend(hour(
            &mut sequence,
            102,
            &d46_payload(
                COMPACT_STATUS_DELTA_DATA,
                &[(live_row(3, 20_002), "c")],
                None,
            ),
        ));
        let scan = scan_good(&s);
        assert_eq!(scan.failure_records, 1, "the partial hour is excluded");
        assert_eq!(scan.hours.len(), 2);
        assert_eq!(scan.non_contiguous, 1);
        let (outcomes, _) = measure(&scan, &s);
        assert!(outcomes[0].opened_baseline && !outcomes[0].gap_forced);
        assert!(
            outcomes[1].opened_baseline && outcomes[1].gap_forced,
            "the hour after the excluded one is a \u{a7}1.5 gap-forced baseline"
        );
        // The excluded hour's rows were never folded, so the gap is real: variant 2
        // is absent from the post-gap state.
        assert!(!scan.hours.iter().any(|h| h.game_hour == 101));
    }

    #[test]
    fn the_reader_replay_is_asserted_on_every_measured_hour() {
        let s = good_stream();
        let scan = scan_good(&s);
        assert_eq!(scan.hours.len(), 3);
        let (outcomes, wire_len) = measure(&scan, &s);
        // `encode_wire_hour` panics if any hour's replay disagrees with the fold,
        // so reaching here is the assertion. The shape is also pinned: hour 1 is a
        // baseline, hours 2 and 3 are deltas that actually emit.
        assert!(outcomes[0].opened_baseline);
        assert!(!outcomes[1].opened_baseline && outcomes[1].emitted);
        assert_eq!(outcomes[1].changed, 1, "the sparse patch adds one row");
        assert_eq!(outcomes[2].tombstones, 1, "and the third hour deletes one");
        assert!(wire_len > 0);
    }

    // --- The cross-domain validation bound -----------------------------------

    #[test]
    fn an_hour_closed_without_a_d46_record_refuses_the_stream() {
        // Relabel the LAST d46 DATA record's domain id: the completeness check is
        // what must reject it, because this harness cannot validate the payload of
        // whatever domain the record now claims to be.
        let mut s = good_stream();
        let mut pos = 0usize;
        let mut last_d46 = None;
        while pos < s.len() {
            let header = parse_compact_record_header(&s[pos..]).expect("header parses");
            if header.domain_id == D46 && header.record_type == COMPACT_RECORD_TYPE_DOMAIN_DATA {
                last_d46 = Some(pos);
            }
            pos += COMPACT_RECORD_HEADER_LEN + header.payload_len;
        }
        let at = last_d46.expect("a d46 record");
        s[at + 20..at + 22].copy_from_slice(&99u16.to_le_bytes());
        let err = scan_stream(&s, StreamIdentityMode::Legacy).expect_err("refused");
        assert!(
            err.contains("closes 3 game hour(s) but carries 2"),
            "the completeness check is what rejected it: {err}"
        );
    }

    #[test]
    fn a_second_d46_record_in_one_hour_refuses_the_stream() {
        let mut sequence = 1u64;
        let mut s = Vec::new();
        s.extend(perf(sequence, 100));
        sequence += 1;
        for _ in 0..2 {
            s.extend(record(
                sequence,
                100,
                D46,
                COMPACT_RECORD_TYPE_DOMAIN_DATA,
                &d46_payload(
                    COMPACT_STATUS_DELTA_DATA,
                    &[(live_row(1, 20_000), "a")],
                    None,
                ),
            ));
            sequence += 1;
        }
        s.extend(frame_end(sequence, 100));
        let err = scan_stream(&s, StreamIdentityMode::Legacy).expect_err("refused");
        assert!(err.contains("carries hour 100 after hour 100"), "{err}");
    }

    // --- Status handling -----------------------------------------------------

    #[test]
    fn an_unassigned_status_bit_is_refused_not_classified() {
        // Bits 15..=31 have no defined classification, so a record carrying one
        // refuses the stream rather than falling through to "not a failure".
        for bit in 15..32u32 {
            let mut sequence = 1u64;
            let s = hour(
                &mut sequence,
                100,
                &d46_payload(
                    COMPACT_STATUS_DELTA_DATA | (1 << bit),
                    &[(live_row(1, 20_000), "a")],
                    None,
                ),
            );
            let err = scan_stream(&s, StreamIdentityMode::Legacy).expect_err("refused");
            assert!(
                err.contains("unassigned v1 status bits"),
                "bit {bit}: {err}"
            );
        }
    }

    #[test]
    fn a_failure_status_record_is_excluded_rather_than_measured() {
        let mut sequence = 1u64;
        let mut s = Vec::new();
        s.extend(hour(
            &mut sequence,
            100,
            &d46_payload(
                COMPACT_STATUS_DELTA_DATA | COMPACT_STATUS_PARTIAL_DATA,
                &[(live_row(1, 20_000), "a")],
                None,
            ),
        ));
        s.extend(hour(
            &mut sequence,
            101,
            &d46_payload(
                COMPACT_STATUS_DELTA_DATA,
                &[(live_row(2, 20_001), "b")],
                None,
            ),
        ));
        let scan = scan_good(&s);
        assert_eq!(scan.failure_records, 1);
        assert_eq!(scan.hours.len(), 1);
        // The excluded record's tokens are NOT in the vocabulary, because pass 2
        // never maps them — the two passes classify identically.
        assert!(!scan.vocabulary.contains("a"));
        assert!(scan.vocabulary.contains("b"));
        // And the first FOLDED hour is the implicit baseline: variant 1 never
        // entered the accumulator.
        let (outcomes, _) = measure(&scan, &s);
        assert!(outcomes[0].opened_baseline);
    }

    #[test]
    fn the_expected_partial_tolerance_table_still_omits_d46() {
        // `COMPACT_STATUS_PARTIAL_DATA` is inside `COMPACT_STATUS_FAILURE_MASK` for
        // an unlisted domain, so a tolerance branch in this harness would be dead
        // code (the d65 rule). If d46 ever gains an entry, `scan_stream` refuses
        // rather than silently changing what is measured.
        assert!(expected_partial_tolerance(D46).is_none());
        assert_ne!(
            compact_status_failure_flags_after_expected_partial_tolerance(
                D46,
                COMPACT_STATUS_DELTA_DATA | COMPACT_STATUS_PARTIAL_DATA,
            ),
            0,
            "a PARTIAL_DATA d46 record is a failure-status record"
        );
        assert_eq!(
            compact_status_failure_flags_after_expected_partial_tolerance(
                D46,
                COMPACT_STATUS_DELTA_DATA,
            ),
            0,
            "and a plain DELTA_DATA one is not"
        );
    }

    #[test]
    fn the_restated_delta_status_bit_agrees_with_the_shipping_one() {
        // `d46::STATUS_DELTA_DATA` is transcribed rather than imported (the library
        // keeps no `compact_contract` dependency); this is the bin asserting the
        // copy has not drifted. The fold never reads the bit — see the module
        // docs — so nothing measured depends on it, which the next assertion pins.
        assert_eq!(d46::STATUS_DELTA_DATA, COMPACT_STATUS_DELTA_DATA);
    }

    #[test]
    fn the_delta_bit_does_not_change_what_the_fold_does() {
        // The implicit baseline: the FIRST folded frame replaces whatever the
        // status word says, so the same two payloads with and without DELTA_DATA
        // fold to the same states.
        let schema = d46::schema();
        let symbols = d46::Symbols::from_vocabulary(["a", "b"]);
        let mut with = d46::State::new();
        let mut without = d46::State::new();
        for flags in [COMPACT_STATUS_DELTA_DATA, 0] {
            let first = d46_payload(flags, &[(live_row(1, 20_000), "a")], None);
            let second = d46_payload(flags, &[(live_row(2, 20_001), "b")], None);
            let state = if flags == 0 { &mut without } else { &mut with };
            state
                .fold_payload(&schema, &first, false, 0, &symbols)
                .expect("folds");
            state
                .fold_payload(&schema, &second, false, 0, &symbols)
                .expect("folds");
        }
        assert_eq!(with.archetypes.len(), 2);
        assert_eq!(without.archetypes.len(), 2, "the bit changes nothing");
        assert_eq!(with.states()[0], without.states()[0]);
    }

    // --- The stop: the trailer's refusal position ----------------------------

    #[test]
    fn a_malformed_trailer_refuses_the_stream_even_on_a_failure_status_hour() {
        // d52's lesson, and d46's own two positions. The FOLD parses the trailer at
        // `world_fold.rs:16493`, behind its failure gate at `:16342`, so on a
        // failure-status hour the fold never sees it. Production's payload
        // authority walks it on every DATA record whatever the status, so the
        // stream is refused there — and this harness runs that check first, which
        // is what makes the two agree.
        let mut bad_trailer = empty_ace_trailer();
        let last = bad_trailer.len() - 1;
        bad_trailer[last] ^= 0xff; // skipped_rows is free, so corrupt the magic
        bad_trailer[0] = b'X';
        for status in [
            COMPACT_STATUS_DELTA_DATA,
            COMPACT_STATUS_DELTA_DATA | COMPACT_STATUS_PARTIAL_DATA,
        ] {
            let mut sequence = 1u64;
            let s = hour(
                &mut sequence,
                100,
                &d46_payload(status, &[(live_row(1, 20_000), "a")], Some(&bad_trailer)),
            );
            let err = scan_stream(&s, StreamIdentityMode::Legacy).expect_err("refused");
            assert!(
                err.contains("ace trailer"),
                "status {status:#x} must refuse the stream: {err}"
            );
        }
        // The fold's own copy, for contrast: excluded, because it never reads it.
        let schema = d46::schema();
        let payload = d46_payload(
            COMPACT_STATUS_DELTA_DATA | COMPACT_STATUS_PARTIAL_DATA,
            &[(live_row(1, 20_000), "a")],
            Some(&bad_trailer),
        );
        assert_eq!(
            d46::State::new()
                .fold_payload(&schema, &payload, true, 0, &d46::Symbols::empty())
                .expect("the fold excludes it"),
            d46::FoldOutcome::FailureStatus
        );
    }

    #[test]
    fn the_cut_arm_removes_exactly_the_trailer_bytes() {
        let s = good_stream();
        let scan = scan_good(&s);
        for hour in &scan.hours {
            let record = &s[hour.ship_span.clone()];
            assert_eq!(
                hour.trailer_bytes,
                empty_ace_trailer().len(),
                "the trailer's own bytes"
            );
            let cut = &record[..record.len() - hour.trailer_bytes];
            assert_eq!(
                cut.len(),
                record.len() - empty_ace_trailer().len(),
                "the cut arm removes the suffix and nothing else"
            );
            // The cut record's payload length word is left as written — that is what
            // makes it a size-accounting artifact rather than a valid record.
            let declared = u32::from_le_bytes(cut[24..28].try_into().expect("4 bytes")) as usize;
            assert!(
                declared > cut.len() - COMPACT_RECORD_HEADER_LEN,
                "the cut arm is deliberately not a decodable stream"
            );
        }
        // And the accounting adds up on every hour.
        for hour in &scan.hours {
            let payload_len = hour.ship_span.len() - COMPACT_RECORD_HEADER_LEN;
            assert_eq!(
                d46::HEADER_LEN + hour.row_bytes + hour.string_bytes + hour.trailer_bytes,
                payload_len,
                "header + rows + strings + trailer is the whole payload"
            );
        }
    }

    // --- The two-pass symbol contract ----------------------------------------

    #[test]
    fn pass_two_needs_no_symbol_pass_one_did_not_see() {
        // The proof stated in the module docs, exercised: hour 2 introduces a token
        // hour 1 never carried, and pass 2 still maps every row.
        let s = good_stream();
        let scan = scan_good(&s);
        assert_eq!(
            scan.vocabulary.iter().cloned().collect::<Vec<_>>(),
            vec![
                "cv_small_plane_airframe_3".to_string(),
                "small_plane_airframe_1".to_string(),
                "small_plane_airframe_2".to_string(),
            ]
        );
        let (_, wire_len) = measure(&scan, &s);
        assert!(wire_len > 0, "pass 2 mapped every row");

        // The negative control: a table missing one symbol makes pass 2 refuse
        // rather than quietly dropping the token, which is what the fail-closed
        // rule buys.
        let schema = d46::schema();
        let short = d46::Symbols::from_vocabulary(["small_plane_airframe_1"]);
        let payload = &s[scan.hours[1].ship_span.start + COMPACT_RECORD_HEADER_LEN
            ..scan.hours[1].ship_span.end];
        assert!(d46::State::new()
            .fold_payload(&schema, payload, false, 0, &short)
            .is_err());
    }

    #[test]
    fn the_symbol_ids_are_the_sorted_vocabulary_index() {
        // The stand-in assignment is deterministic and order-independent, so two
        // callers over the same content set mint the same ids — which is what makes
        // the measurement reproducible while §5's rule is open.
        let a = d46::Symbols::from_vocabulary(["b", "a", "c", "a"]);
        let b = d46::Symbols::from_vocabulary(["c", "a", "b"]);
        for (symbol, id) in [("a", 0u64), ("b", 1), ("c", 2)] {
            assert_eq!(a.id(symbol), Some(id));
            assert_eq!(b.id(symbol), Some(id));
            assert_eq!(a.symbol(id), Some(symbol));
        }
        assert_eq!(a.len(), 3);
        assert!(d46::Symbols::empty().is_empty());
    }

    // --- Stream-level rules ---------------------------------------------------

    #[test]
    fn a_stream_cut_at_a_frame_end_boundary_is_a_shorter_well_formed_stream() {
        let full = good_stream();
        let mut pos = 0usize;
        let mut first_frame_end = None;
        while pos < full.len() {
            let header = parse_compact_record_header(&full[pos..]).expect("header parses");
            let end = pos + COMPACT_RECORD_HEADER_LEN + header.payload_len;
            if header.record_type == COMPACT_RECORD_TYPE_FRAME_END && first_frame_end.is_none() {
                first_frame_end = Some(end);
            }
            pos = end;
        }
        let cut = &full[..first_frame_end.expect("a FRAME_END")];
        let scan = scan_stream(cut, StreamIdentityMode::Legacy).expect("accepted");
        assert_eq!(scan.hours.len(), 1);
    }

    #[test]
    fn a_stream_that_ends_mid_record_refuses() {
        let full = good_stream();
        let cut = &full[..full.len() - 4];
        let err = scan_stream(cut, StreamIdentityMode::Legacy).expect_err("refused");
        assert!(err.contains("mid-record") || err.contains("must be a canonical FRAME_END"), "{err}");
    }

    #[test]
    fn a_malformed_d46_payload_refuses_the_stream_rather_than_skipping_an_hour() {
        // The third outcome: not an excluded hour, a rejection. Here via the
        // authority's `fixed_scale_raw == 100_000` rule, which the FOLD does not
        // make — so this is also the survey hazard's answer, exercised.
        let mut sequence = 1u64;
        let mut payload = d46_payload(
            COMPACT_STATUS_DELTA_DATA,
            &[(live_row(1, 20_000), "a")],
            None,
        );
        payload[36..40].copy_from_slice(&1_000u32.to_le_bytes());
        let s = hour(&mut sequence, 100, &payload);
        let err = scan_stream(&s, StreamIdentityMode::Legacy).expect_err("refused");
        assert!(err.contains("fixed scale 1000 != 100000"), "{err}");
        // The FOLD, by contrast, folds it — which is why the two boundaries are
        // separate functions.
        assert!(d46::peek_header(&payload).is_ok());
    }

    #[test]
    fn the_scan_reports_the_status_histogram_and_the_two_row_counts() {
        // The survey's "rows/h" for d46 is the header's `observed_variant_rows`,
        // not the wire `row_count`; both are carried so the measurement can say
        // which is which.
        let s = good_stream();
        let scan = scan_good(&s);
        assert_eq!(scan.status_histogram, vec![(COMPACT_STATUS_DELTA_DATA, 3)]);
        assert_eq!(scan.hours[0].wire_rows, 2);
        assert_eq!(scan.hours[0].observed_variant_rows, 2);
        assert_eq!(scan.ace_trailer_records, 3);
        assert!(scan.hours.iter().all(|h| !h.ace_retained));
    }

    #[test]
    fn the_scan_and_the_encoder_agree_on_which_hours_exist() {
        let s = good_stream();
        let scan = scan_good(&s);
        let seen: Vec<i32> = scan.hours.iter().map(|h| h.game_hour).collect();
        assert_eq!(seen, vec![100, 101, 102]);
        let (outcomes, _) = measure(&scan, &s);
        assert_eq!(outcomes.len(), scan.hours.len());
    }
}
