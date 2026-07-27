//! Measure the wire format against the shipping compact stream on REAL d70
//! `sunk_convoy_history_live` dynamics.
//!
//! Input: a real `collector_stream.bin` from a d70-scheduled capture
//! (`runs/batch3/deepwar_b`). The stream's own d70 DATA records are the shipping
//! format's cost for the domain: a 272-byte SCH1 v3 header — 48 bytes of scalars
//! plus seven 32-byte pinned receipt hashes — followed by a fixed 24-byte row per
//! `(month, killer, owner)` bucket, full-replace every hour, exactly what the
//! recorder stores today. This tool replays those records into per-hour states —
//! the same fold `world_fold::fold_sunk_convoy_history` performs — and re-encodes
//! every hour with this crate's grammar: a BASELINE at each 48-hour block start,
//! field-granular stored-minimal DELTAs between.
//!
//! **Nothing here re-derives a contract it can call.** The payload grammar, the
//! key, the ordinal and the fold's five decisions go through `wire_format::d70`'s
//! public boundary; H4CR framing, record semantics and sequence continuity go
//! through `compact_contract`'s own parser and stateful validator.
//!
//! **And the one contract this crate cannot call, it cross-checks.** d70's row
//! grammar lives in `compact_contract::naval_history`, not in the fold, and
//! `wire_format`'s library half must not depend on `compact_contract`
//! (`Cargo.toml`) — so `d70.rs` mirrors that codec rather than calling it. This
//! bin *can* call it, and does: [`check_authority_payload`] runs the real
//! `naval_history::decode_payload` on every d70 record and
//! [`cross_check_against_codec`] requires the codec's decode and `d70::decode` to
//! agree, record for record, on the header scalars, the authority, the ordering,
//! and every row in wire order. A mirror that drifts fails the run rather than
//! measuring a plausible number under a wrong grammar.
//!
//! **The two typed inputs the payload does not carry come from the record.**
//! `fold_sunk_convoy_history` refuses a payload whose `frame_hour` or
//! `source_sequence` disagrees with the H4CR record that carried it
//! (`world_fold.rs:10259`-`:10273`), and only the production dispatcher supplies
//! them (`:4801`-`:4806`). This harness supplies them the same way —
//! `hour_index_from_raw(header.game_hour_raw)` and `Some(header.sequence)` — which
//! is why it is the H4CR header, not the payload, that decides whether an hour is
//! measurable at all.
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
//!   (`recorder_host/src/stream_fold.rs:277`): a DOMAIN_DATA record opens its
//!   game hour, a FRAME_END closes the open hour and only that hour, and closed
//!   hours strictly increase;
//! - that tracker's **DOMAIN_PERF hour binding** (`:328`-`:365`): no duplicate
//!   non-probe PERF domain inside one hour, no PERF for an hour other than the
//!   open one, no PERF for an hour already closed, and no conflict with the
//!   pending PERF hour before a DATA record opens one;
//! - the **FRAME_END seal fields**, `validate_frame_end_seal_fields` (`:46`-`:89`),
//!   re-implemented here as [`check_frame_end_seal`]: the 32-byte payload length,
//!   `reserved == 0`, `emitted_domains == scheduled_domains`, no status bit above
//!   13, and no fatal status bit; plus the seal's **domain-count cross-check**
//!   against the distinct non-probe PERF domains actually observed in the hour
//!   (`:411`-`:419`);
//! - the EOF rules of that tracker's `eof_check` (`:459`): no residual bytes, no
//!   hour left open, at least one FRAME_END, and the final record is a FRAME_END.
//!
//! That is a documented **subset** of the production boundary contract:
//! `recorder_host::stream_fold::verify_stream_logical_boundary` (`:537`) is the
//! full authority. Exactly one production check is outside this subset, and it is
//! a crate-boundary limitation rather than a choice: production calls
//! `validate_domain_data_payload_at` on EVERY DOMAIN_DATA record (`:593`), so a
//! stream carrying a malformed payload for some *other* domain is refused there
//! and accepted here. For **d70's own** records that authority is
//! `decode_compact_sunk_convoy_history_data_payload`
//! (`domain_data_authority.rs:13102`-`:13150`), which is a bare
//! `naval_history::decode_payload` call — and [`check_authority_payload`] runs
//! exactly that, on every record whatever its status.
//!
//! `wire_format` cannot depend on `recorder_host` (it pulls in every capture
//! lane) and this crate implements one domain's grammar, so d70's is the only
//! DOMAIN_DATA payload this harness can validate. The measured capture is a
//! recorder-validated run, so its other domains' payloads have already been
//! through that check; a hand-made stream carrying a corrupt non-d70 payload —
//! a flipped `DMI2` magic byte in a d67 record, say — would measure here and not
//! there. That gap is shared by every bin in this crate and is a property of the
//! crate graph, not of this domain's port.
//!
//! # Three outcomes, named
//!
//! Every non-folding path of `fold_sunk_convoy_history` is classified explicitly:
//!
//! - **stream rejection** — a `decode_payload` error (`world_fold.rs:10233`), a
//!   non-`StaticExact20Receipt` authority on a clean frame (`:10255`), a
//!   `frame_hour` mismatch (`:10262`) or a `source_sequence` mismatch (`:10269`).
//!   The payload is not this grammar, or is not bound to the record that carried
//!   it; either invalidates the run.
//! - **excluded hour** — a failure status (`:10243`). The fold sets the
//!   accumulator to `None`, so the hour carries no d70 data and enters neither
//!   arm.
//! - **measured hour** — everything else.
//!
//! **An excluded d70 hour is not d63's "retain the accumulator".** It *discards*
//! it: the recorder is saying the table is unavailable, not that it is unchanged.
//! A keyed-set delta cannot express that (§8 gap normalisation has no code in this
//! crate), so this harness forces a BASELINE on the first measured hour after any
//! excluded one rather than emitting a delta against a table the recorder has
//! thrown away. The reference capture has **zero** excluded hours, so that path is
//! exercised only by this bin's tests — a stated coverage caveat, not a silence.
//!
//! Outputs, for the compression step (`compress_arms.py` beside the run):
//!   ship_d70.bin   — the d70 DATA records verbatim (32-byte H4CR header +
//!                    payload), concatenated in stream order
//!   wire_new.bin   — the wire-format bodies, concatenated in hour order
//!   *.offsets      — hour-boundary byte offsets, one per line, for exact
//!                    block-span slicing
//!
//! It also prints the real churn surface twice — cadence-independent over
//! measured-hour transitions, and along the delta path — because one denominator
//! cannot answer both questions.

use std::collections::BTreeSet;
use std::io::Write;
use std::ops::Range;

use compact_contract::calendar::hour_index_from_raw;
use compact_contract::naval_history;
use compact_contract::stream_validator::{StatefulStreamValidator, StreamIdentityMode};
use compact_contract::{
    compact_status_failure_flags_after_expected_partial_tolerance, compact_status_unknown_flags_v1,
    expected_partial_tolerance, lane_local_status_mask, parse_compact_record_header,
    COMPACT_FRAME_END_PAYLOAD_LEN, COMPACT_RECORD_HEADER_LEN, COMPACT_RECORD_TYPE_DOMAIN_DATA,
    COMPACT_RECORD_TYPE_DOMAIN_PERF, COMPACT_RECORD_TYPE_FRAME_END, COMPACT_STATUS_FAILURE_MASK,
};
use wire_format::body::{
    apply, decode_baseline, decode_delta, diff, encode_baseline, encode_delta, KeyedState,
};
use wire_format::d70;
use wire_format::Value;

const D70: u16 = 70;
const BLOCK_HOURS: usize = 48;

/// One usable d70 hour: its game hour, the folded state of that hour, and its
/// byte span in the source stream (the shipping arm is those bytes verbatim,
/// header included).
///
/// The state is built during the scan, not later: every way a payload or a row
/// can be rejected is a property of the stream, so it belongs to the pass that
/// either accepts the stream or refuses it.
#[derive(Debug)]
struct D70Hour {
    game_hour: i32,
    state: KeyedState,
    ship_span: Range<usize>,
    /// The previous d70 record marked the domain-hour unavailable, so the
    /// recorder's accumulator was `None` immediately before this hour. A delta
    /// against the last measured hour would assert continuity the recorder does
    /// not have, so this hour is baselined. See the module docs.
    follows_unavailable: bool,
}

#[derive(Debug)]
struct StreamScan {
    hours: Vec<D70Hour>,
    /// d70 records whose payload status flags mark the domain-hour failed. The
    /// fold sets the accumulator to `None` on each: an excluded, unavailable hour.
    unavailable_records: usize,
    /// Measured d70 hours that do not follow the previous measured one by one.
    non_contiguous: usize,
    /// Hours whose wire row order happens to equal the canonical
    /// `(month, killer, owner)` sort — the redundancy check for the carried
    /// ordinal, reported rather than relied on.
    canonical_order_hours: usize,
    /// The largest `source_ordinal` any measured hour carried.
    max_ordinal: u64,
}

/// Every globally-defined compact status occupies one of bits 0..=13. FRAME_END
/// carries the frame-wide aggregate, so lane-local interpretations do not apply
/// there and any higher bit is an unknown wire-contract extension
/// (`recorder_host/src/stream_fold.rs:43`).
const FRAME_END_KNOWN_STATUS_MASK: u32 = (1 << 14) - 1;

/// The reader-context probe domain, excluded from the seal's domain accounting
/// (`recorder_host/src/stream_fold.rs:330`).
const PROBE_DOMAIN_ID: u16 = 250;

/// d70 carries no lane-local status bits and no expected-partial tolerance, so
/// `compact_contract`'s failure predicate reduces exactly to the global mask —
/// which is what `d70::FAILURE_STATUS_MASK` (the library's copy, kept because
/// `wire_format`'s library half must not depend on `compact_contract`) has to
/// equal. Asserting the three facts proves the two predicates agree for every
/// status word, rather than for the words this capture happens to contain.
///
/// The same argument covers `d70::KNOWN_STATUS_MASK_V1` against
/// `compact_status_unknown_flags_v1`, which the SCH1 decoder itself calls
/// (`naval_history.rs:344`).
fn assert_status_predicates_agree() {
    assert_eq!(
        lane_local_status_mask(D70),
        0,
        "d70 gained a lane-local status bit; d70::FAILURE_STATUS_MASK must be revisited"
    );
    assert!(
        expected_partial_tolerance(D70).is_none(),
        "d70 gained an expected-partial tolerance; d70::FAILURE_STATUS_MASK must be revisited"
    );
    assert_eq!(
        d70::FAILURE_STATUS_MASK,
        COMPACT_STATUS_FAILURE_MASK,
        "the library's failure mask has drifted from compact_contract's"
    );
    // Every status word, not a sample: the two must partition the u32 the same
    // way. 2^32 words is too many, so sweep the single bits and the whole known
    // mask, which is what the two predicates are defined over.
    for bit in 0..32u32 {
        let word = 1u32 << bit;
        assert_eq!(
            compact_status_unknown_flags_v1(word),
            word & !d70::KNOWN_STATUS_MASK_V1,
            "status bit {bit}: the library's assigned-bit mask has drifted"
        );
    }
    assert_eq!(
        compact_status_unknown_flags_v1(d70::KNOWN_STATUS_MASK_V1),
        0,
        "every bit of the library's known mask must be an assigned v1 bit"
    );
}

/// The seven pinned receipts a v3 exact payload must carry, taken from
/// `compact_contract`'s own constants rather than copied into `wire_format`
/// (`naval_history.rs:34`-`:45`, in the order `encode_v3` writes them at
/// `:241`-`:249`).
fn pinned_receipts() -> d70::AuthorityReceipts {
    d70::AuthorityReceipts::new([
        naval_history::EXECUTABLE_SHA256,
        naval_history::OWNER_SERIALIZER_MANIFEST_SHA256,
        naval_history::OWNER_SERIALIZER_ARTIFACT_SHA256,
        naval_history::ROW_MUTATION_MANIFEST_SHA256,
        naval_history::ROW_SERIALIZER_ARTIFACT_SHA256,
        naval_history::MUTATION_ARTIFACT_SHA256,
        naval_history::LIVE_FIXTURE_RECEIPT_SHA256,
    ])
}

/// The FRAME_END seal check, re-implemented from
/// `recorder_host::stream_fold::validate_frame_end_seal_fields`
/// (`recorder_host/src/stream_fold.rs:46`-`:89`) because `wire_format` must not
/// depend on `recorder_host`.
///
/// Wire layout: `elapsed_wall_ms:u64`, `frame_time_us:u64`, `emitted_domains:u32`,
/// `scheduled_domains:u32`, `status_flags:u32`, `reserved:u32`. The two timing
/// fields carry no seal-time predicate; the rest are authoritative. A violation is
/// a refused stream, exactly as it is in production (`:402`-`:409`).
///
/// Returns the seal's `(emitted_domains, scheduled_domains)` so the caller can run
/// production's cross-check against the PERF domains it actually observed
/// (`:411`-`:419`).
///
/// Total: no payload of any length panics.
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

/// The d70 check production's **second** authority makes on every DOMAIN_DATA
/// record — status-independent, and therefore also on the records this harness
/// excludes.
///
/// Production runs two independent passes over a d70 DATA record and a stream is
/// refused if EITHER complains:
///
/// 1. `WorldFold::ingest_record` → `fold_sunk_convoy_history`, whose `Err` aborts
///    the stream fold. Its `decode_payload` call runs BEFORE the status early exit
///    (`world_fold.rs:10231`-`:10243`), so unlike d63 there is no part of the
///    payload an excluded record's fold does not reach.
/// 2. `validate_domain_data_payload_at` (`recorder_host/src/stream_fold.rs:594`) →
///    `decode_compact_sunk_convoy_history_data_payload`
///    (`domain_data_authority.rs:13102`-`:13150`). This one runs on EVERY
///    DOMAIN_DATA record whatever its status, and any error it reports makes the
///    record `malformed_payload_records`, which
///    `strict_stream_certification_verdict` (`stream_fold.rs:507`-`:510`) refuses.
///
/// That authority is a bare `naval_history::decode_payload` call and nothing else
/// — it does not check the frame binding, the sequence binding or the authority
/// state, which are the fold's alone. So this function is that same call, and
/// stops exactly where production's does.
///
/// It returns the codec's own reading of the status word, which the caller uses
/// to check the fold's exclusion decision against `compact_contract`'s real
/// failure predicate on every single record.
fn check_authority_payload(payload: &[u8], at_byte: usize) -> Result<u32, String> {
    naval_history::decode_payload(payload)
        .map(|decoded| decoded.status_flags)
        .map_err(|error| format!("d70 payload authority at byte {at_byte}: {error}"))
}

/// Require `wire_format::d70`'s mirror of the SCH1 grammar and the real codec to
/// agree on this payload, field for field and row for row.
///
/// d70 is the first ported domain whose row grammar lives in a codec crate rather
/// than in the fold, and `wire_format`'s library half cannot depend on that crate.
/// A comment citing `naval_history.rs:LINE` is a claim; this is the check. It runs
/// on every d70 record of the capture, so a transposed offset, a missed
/// predicate or a version-gate mistake fails the run instead of producing a
/// plausible number under a wrong grammar.
///
/// The two decodes are compared on their **classification** (both accept or both
/// reject) and, where both accept, on every decoded value. The message of a
/// doubly-malformed payload may legitimately differ: the codec checks the
/// per-row non-zero rules after reading every row, while `d70::RawRow::parse`
/// checks them as it reads, because `key_from_row` and `record_from_row` see one
/// row at a time. The predicates are the same set, so the classification is not
/// affected — which is what this function pins.
fn cross_check_against_codec(
    payload: &[u8],
    collection: &wire_format::Collection,
    at_byte: usize,
) -> Result<(), String> {
    let mine = d70::decode(payload, &pinned_receipts(), collection, 0);
    let theirs = naval_history::decode_payload(payload);
    let (mine, theirs) = match (mine, theirs) {
        (Ok(mine), Ok(theirs)) => (mine, theirs),
        (Err(_), Err(_)) => return Ok(()),
        (Ok(_), Err(error)) => {
            return Err(format!(
                "d70 payload at byte {at_byte}: wire_format::d70 accepted a payload \
                 compact_contract rejects ({error})"
            ))
        }
        (Err(error), Ok(_)) => {
            return Err(format!(
                "d70 payload at byte {at_byte}: wire_format::d70 rejected a payload \
                 compact_contract accepts ({error})"
            ))
        }
    };

    let header = mine.header;
    let mut disagreements: Vec<String> = Vec::new();
    if header.schema_version != theirs.schema_version {
        disagreements.push(format!(
            "schema_version {} != {}",
            header.schema_version, theirs.schema_version
        ));
    }
    if header.status_flags != theirs.status_flags {
        disagreements.push(format!(
            "status_flags {:#x} != {:#x}",
            header.status_flags, theirs.status_flags
        ));
    }
    if header.elapsed_wall_ms != theirs.elapsed_wall_ms {
        disagreements.push("elapsed_wall_ms".to_string());
    }
    if header.observed_rows != theirs.observed_rows {
        disagreements.push("observed_rows".to_string());
    }
    if header.frame_hour != theirs.frame_hour {
        disagreements.push(format!(
            "frame_hour {:?} != {:?}",
            header.frame_hour, theirs.frame_hour
        ));
    }
    if header.source_sequence != theirs.source_sequence {
        disagreements.push(format!(
            "source_sequence {:?} != {:?}",
            header.source_sequence, theirs.source_sequence
        ));
    }
    let their_authority = match theirs.authority {
        naval_history::SunkConvoyHistoryAuthority::Unavailable => d70::Authority::Unavailable,
        naval_history::SunkConvoyHistoryAuthority::LegacyUnbound => d70::Authority::LegacyUnbound,
        naval_history::SunkConvoyHistoryAuthority::StaticExact20Receipt => {
            d70::Authority::StaticExact20Receipt
        }
    };
    if header.authority != their_authority {
        disagreements.push(format!(
            "authority {:?} != {their_authority:?}",
            header.authority
        ));
    }
    let their_ordering = match theirs.ordering {
        naval_history::SunkConvoyHistoryOrdering::CanonicalIdentity => {
            d70::Ordering::CanonicalIdentity
        }
        naval_history::SunkConvoyHistoryOrdering::FirstSeenAggregateIdentity => {
            d70::Ordering::FirstSeenAggregateIdentity
        }
    };
    if header.ordering != their_ordering {
        disagreements.push(format!("ordering {:?} != {their_ordering:?}", header.ordering));
    }
    if mine.rows.len() != theirs.rows.len() {
        disagreements.push(format!(
            "row count {} != {}",
            mine.rows.len(),
            theirs.rows.len()
        ));
    }

    // Every row, by identity and by both value fields — including the ordinal,
    // which is the whole reason the port carries one. `theirs.rows` is in wire
    // order, so its index IS the expected ordinal.
    let ordinal_index = value_index(collection, d70::field::SOURCE_ORDINAL);
    let convoys_index = value_index(collection, d70::field::CONVOYS);
    for (index, row) in theirs.rows.iter().enumerate() {
        let key = wire_format::body::Key(vec![
            row.month as i128,
            row.killer_country_tag_db_id as i128,
            row.owner_tag_db_id as i128,
        ]);
        let Some(record) = mine.rows.get(&key) else {
            disagreements.push(format!("row {index} identity {key:?} missing"));
            continue;
        };
        if record.values[convoys_index] != Value::Unsigned(row.convoys as u64) {
            disagreements.push(format!(
                "row {index} convoys {:?} != {}",
                record.values[convoys_index], row.convoys
            ));
        }
        if record.values[ordinal_index] != Value::Unsigned(index as u64) {
            disagreements.push(format!(
                "row {index} source_ordinal {:?} != {index}",
                record.values[ordinal_index]
            ));
        }
    }

    if disagreements.is_empty() {
        Ok(())
    } else {
        Err(format!(
            "d70 payload at byte {at_byte}: wire_format::d70 disagrees with \
             compact_contract::naval_history: {}",
            disagreements.join("; ")
        ))
    }
}

fn value_index(collection: &wire_format::Collection, field_id: u32) -> usize {
    collection
        .value_fields()
        .position(|f| f.field_id == field_id)
        .expect("field is in the d70 schema")
}

/// Validate the whole stream against the canonical contract and fold the usable
/// d70 DATA records into per-hour states. Every rejection path is an `Err`, never
/// a silently shorter measurement and never a panic.
fn scan_stream(
    stream: &[u8],
    collection: &wire_format::Collection,
    identity_mode: StreamIdentityMode,
) -> Result<StreamScan, String> {
    // The identity protocol is an INPUT, never sniffed. `StatefulStreamValidator`
    // deliberately never sniffs (`compact_contract/src/stream_validator.rs:328`)
    // and production takes the mode from its caller
    // (`recorder_host/src/stream_fold.rs:220`), for a reason this harness has to
    // honour: an `IdentityV1` stream whose mandatory CID1 record has been spliced
    // off is byte-indistinguishable from a legacy stream — both begin with an
    // ordinary record at sequence 1. A harness that sniffs therefore accepts a
    // decapitated identity stream as a healthy legacy one and measures it, which
    // production (invoked with `IdentityV1`) refuses as `WrongRecordSequence(1)`.
    let mut validator = match identity_mode {
        StreamIdentityMode::IdentityV1 => StatefulStreamValidator::identity_v1(),
        StreamIdentityMode::Legacy => StatefulStreamValidator::legacy(),
    };
    let receipts = pinned_receipts();

    let mut hours: Vec<D70Hour> = Vec::new();
    let mut unavailable_records = 0usize;
    let mut non_contiguous = 0usize;
    let mut canonical_order_hours = 0usize;
    let mut max_ordinal = 0u64;

    let mut open_hour: Option<i32> = None;
    let mut last_closed_hour: Option<i32> = None;
    let mut last_record_type: Option<u16> = None;
    // The DOMAIN_PERF hour-binding state, `BoundaryTracker`'s own
    // (`stream_fold.rs:328`-`:365`, reset at `:449`-`:451`).
    let mut pending_perf_hour: Option<i32> = None;
    let mut non_probe_perf_domains: BTreeSet<u16> = BTreeSet::new();
    // Hour of the last d70 DATA record of ANY status — the uniqueness rule.
    let mut last_d70_hour: Option<i32> = None;
    // Hour of the last d70 record that entered the measurement — the contiguity
    // report, which a skipped unavailable hour legitimately breaks.
    let mut last_measured_hour: Option<i32> = None;
    // The recorder's accumulator, replayed through `d70`'s own fold boundary.
    let mut accumulator = d70::State::new();
    // Set by an unavailable record, cleared by the measured hour that consumes it.
    let mut pending_unavailable = false;

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
                // The seal fields come first, as they do in the production tracker
                // (`stream_fold.rs:401`-`:409`).
                let (emitted_domains, scheduled_domains) = check_frame_end_seal(payload, pos)?;
                // `:411`-`:419`.
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
                // `stream_fold.rs:449`-`:451`.
                open_hour = None;
                pending_perf_hour = None;
                non_probe_perf_domains.clear();
                last_closed_hour = Some(header.game_hour_raw);
            }
            _ => {}
        }
        last_record_type = Some(header.record_type);

        if header.domain_id == D70 && header.record_type == COMPACT_RECORD_TYPE_DOMAIN_DATA {
            // One d70 DATA record per game hour, checked over EVERY d70 record and
            // BEFORE any status-based exclusion. d70 is full-replace, so a second
            // record for the same hour would become a second measurement hour built
            // from the same clock time — and gating this on the record being usable
            // would let a success/failure pair through the rule entirely.
            if let Some(previous) = last_d70_hour {
                if header.game_hour_raw <= previous {
                    return Err(format!(
                        "d70 DATA at byte {pos} carries hour {} after hour {previous}",
                        header.game_hour_raw
                    ));
                }
            }
            last_d70_hour = Some(header.game_hour_raw);

            // Production's SECOND authority, which runs on every DOMAIN_DATA record
            // whatever its status and whose complaint refuses the stream at
            // certification. It must come before any exclusion, or a record the
            // recorder would refuse gets quietly skipped instead.
            let authority_status = check_authority_payload(payload, pos)?;
            // ...and the mirror check, on the same records, for the same reason.
            cross_check_against_codec(payload, collection, pos)?;

            // The two typed inputs the payload does not carry, exactly as
            // `apply_domain_data` supplies them (`world_fold.rs:4801`-`:4806`).
            let expected_frame_hour =
                hour_index_from_raw(header.game_hour_raw).and_then(|hour| u64::try_from(hour).ok());
            let expected_record_sequence = Some(header.sequence);

            // The fold itself, through `d70`'s own boundary — five decisions in
            // the fold's own order, and the accumulator semantics with them.
            let outcome = d70::fold_payload(
                &mut accumulator,
                collection,
                payload,
                &receipts,
                expected_frame_hour,
                expected_record_sequence,
                0,
            )
            .map_err(|e| format!("d70 payload at byte {pos}: {e}"))?;

            // The exclusion decision, checked against `compact_contract`'s OWN
            // failure predicate rather than only against the library's copy of
            // the mask — the recipe's "use the fold's own predicate, never a
            // hand-copied mask literal", applied per record. `d70::fold_payload`
            // reaches its decision through `d70::FAILURE_STATUS_MASK`;
            // `assert_status_predicates_agree` proves the two masks are equal for
            // every status word, and this proves it again on every record the
            // capture actually carries.
            let production_failure = compact_status_failure_flags_after_expected_partial_tolerance(
                D70,
                authority_status,
            ) != 0;
            if production_failure != matches!(outcome, d70::FoldOutcome::Unavailable { .. }) {
                return Err(format!(
                    "d70 record at byte {pos}: status {authority_status:#x} classifies as                      failure={production_failure} under compact_contract but the fold                      boundary decided otherwise"
                ));
            }

            match outcome {
                d70::FoldOutcome::Unavailable { .. } => {
                    // An excluded hour AND a discarded accumulator: the next
                    // measured hour cannot be a delta against the last one.
                    unavailable_records += 1;
                    pending_unavailable = true;
                }
                d70::FoldOutcome::Folded { .. } => {
                    if let Some(previous) = last_measured_hour {
                        if header.game_hour_raw != previous + 1 {
                            non_contiguous += 1;
                        }
                    }
                    last_measured_hour = Some(header.game_hour_raw);

                    let state = accumulator
                        .rows
                        .clone()
                        .ok_or_else(|| format!("d70 payload at byte {pos}: folded without rows"))?;
                    // Report whether the carried ordinal was redundant with key
                    // order this hour, rather than relying on either answer.
                    let ordinals: Vec<u64> = state
                        .values()
                        .map(|record| match record.values[value_index(
                            collection,
                            d70::field::SOURCE_ORDINAL,
                        )] {
                            Value::Unsigned(v) => v,
                            _ => u64::MAX,
                        })
                        .collect();
                    if ordinals.windows(2).all(|w| w[0] < w[1]) {
                        canonical_order_hours += 1;
                    }
                    max_ordinal = max_ordinal.max(ordinals.iter().copied().max().unwrap_or(0));

                    hours.push(D70Hour {
                        game_hour: header.game_hour_raw,
                        state,
                        ship_span: pos..record_end,
                        follows_unavailable: pending_unavailable,
                    });
                    pending_unavailable = false;
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
        return Err("stream carries no usable d70 DATA records".to_string());
    }

    Ok(StreamScan {
        hours,
        unavailable_records,
        non_contiguous,
        canonical_order_hours,
        max_ordinal,
    })
}

fn main() {
    const USAGE: &str = "usage: bench_real_d70 <collector_stream.bin> <out dir> \
                         [legacy|identity-v1]";
    assert_status_predicates_agree();
    let mut args = std::env::args().skip(1);
    let input = args.next().expect(USAGE);
    let out_dir = args.next().expect(USAGE);
    // The identity protocol is stated, never sniffed — see `scan_stream`.
    let identity_mode = match args.next().as_deref() {
        None | Some("legacy") => StreamIdentityMode::Legacy,
        Some("identity-v1") => StreamIdentityMode::IdentityV1,
        Some(other) => panic!("unknown identity mode {other:?}; {USAGE}"),
    };
    let stream = std::fs::read(&input).expect("read stream");
    let schema = d70::schema();
    let collection = &schema.collections[0];

    // ---- Pass 1: validate the WHOLE stream and fold its d70 DATA records. ----
    // No artifact is created before this succeeds, so a refused stream leaves no
    // partial measurement behind to be mistaken for a result.
    let scan = match scan_stream(&stream, collection, identity_mode) {
        Ok(scan) => scan,
        Err(reason) => {
            eprintln!("{input}: {reason}");
            std::process::exit(1);
        }
    };
    std::fs::create_dir_all(&out_dir).expect("out dir");

    // ---- Pass 2: encode both arms. Nothing here reads stream content again. ---

    let mut ship = Vec::new();
    let mut ship_offsets = vec![0usize];
    let mut wire = Vec::new();
    let mut wire_offsets = vec![0usize];

    let mut prior: Option<&KeyedState> = None;
    let mut reader: Option<KeyedState> = None;
    let mut hours_seen = 0usize;
    let mut baselines = 0usize;
    let mut forced_baselines = 0usize;
    let mut empty_hours = 0usize;
    let mut changed_rows_total = 0usize;
    let mut tombstones_total = 0usize;
    let mut rows_total = 0usize;
    // Cadence-independent churn: measured-hour transitions, counted before the
    // block decision so a change landing on a block boundary still counts.
    let mut real_changed_rows = 0usize;
    let mut real_tombstones = 0usize;
    let mut real_changed_hours = 0usize;

    for hour in &scan.hours {
        // Shipping arm: the record's exact bytes, header included.
        ship.extend_from_slice(&stream[hour.ship_span.clone()]);
        ship_offsets.push(ship.len());

        // d70 is full-replace, so the hour's rows ARE the hour's whole state.
        let state: &KeyedState = &hour.state;
        rows_total += state.len();

        if let Some(prev) = prior {
            let real = diff(collection, prev, state);
            real_changed_rows += real.changed.len();
            real_tombstones += real.tombstones.len();
            if !real.is_empty() {
                real_changed_hours += 1;
            }
        }

        // New arm: BASELINE at each block start, stored-minimal DELTA between —
        // and a FORCED baseline after an unavailable hour, because the recorder
        // discarded its accumulator there and a delta would assert continuity it
        // does not have.
        let hour_start = wire.len();
        let block_start = hours_seen % BLOCK_HOURS == 0 || hour.follows_unavailable;
        if hour.follows_unavailable && hours_seen % BLOCK_HOURS != 0 {
            forced_baselines += 1;
        }
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
        // the result to be stored-equal to this hour's state.
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

    for (name, bytes) in [("ship_d70.bin", &ship), ("wire_new.bin", &wire)] {
        let mut f = std::fs::File::create(std::path::Path::new(&out_dir).join(name)).unwrap();
        f.write_all(bytes).unwrap();
    }
    for (name, offsets) in [
        ("ship_d70.offsets", &ship_offsets),
        ("wire_new.offsets", &wire_offsets),
    ] {
        let mut f = std::fs::File::create(std::path::Path::new(&out_dir).join(name)).unwrap();
        for o in offsets {
            writeln!(f, "{o}").unwrap();
        }
    }

    // The wire arm's baseline/delta raw split, recovered from the offsets.
    let mut baseline_bytes = 0usize;
    let mut delta_bytes = 0usize;
    {
        let mut index = 0usize;
        let mut is_block_start = true;
        for hour in &scan.hours {
            let span = wire_offsets[index + 1] - wire_offsets[index];
            if is_block_start || index == 0 || hour.follows_unavailable {
                baseline_bytes += span;
            } else {
                delta_bytes += span;
            }
            index += 1;
            is_block_start = index % BLOCK_HOURS == 0;
        }
    }

    let delta_hours = hours_seen - baselines - empty_hours;
    let transitions = hours_seen.saturating_sub(1);
    println!(
        "hours                {hours_seen} ({} non-contiguous steps)",
        scan.non_contiguous
    );
    println!(
        "unavailable hours    {} (failure status; accumulator discarded, excluded from both arms)",
        scan.unavailable_records
    );
    println!(
        "rows avg             {:.1}   max source_ordinal {}",
        rows_total as f64 / hours_seen as f64,
        scan.max_ordinal
    );
    println!(
        "wire order == key order in {} of {hours_seen} hours (the carried ordinal's redundancy check)",
        scan.canonical_order_hours
    );
    println!(
        "baselines            {baselines} (every {BLOCK_HOURS} h; {forced_baselines} forced after an unavailable hour)"
    );
    println!("delta hours          {delta_hours}   empty hours {empty_hours}");
    println!(
        "real churn           {real_changed_rows} changed row(s) and {real_tombstones} \
         tombstone(s) over {real_changed_hours} changed transition(s) of {transitions}; \
         {:.4} rows/hour (cadence-independent)",
        real_changed_rows as f64 / transitions.max(1) as f64
    );
    println!(
        "delta-path churn     {:.2} changed rows/hour, {:.3} tombstones/hour over \
         {delta_hours} emitted delta hour(s)",
        changed_rows_total as f64 / delta_hours.max(1) as f64,
        tombstones_total as f64 / delta_hours.max(1) as f64
    );
    println!("ship_d70.bin         {} bytes", ship.len());
    println!(
        "wire_new.bin         {} bytes ({:.1}%) = {baseline_bytes} baseline + {delta_bytes} delta",
        wire.len(),
        100.0 * wire.len() as f64 / ship.len() as f64
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use compact_contract::{COMPACT_DOMAIN_FRAME, COMPACT_STATUS_SOURCE_UNAVAILABLE};

    const EPOCH: i32 = 43_799_999;
    /// A game hour inside `raw_date_in_range`'s 1850..2200 window
    /// (`compact_contract/src/calendar.rs:14`-`:18`), so `hour_index_from_raw`
    /// yields a frame hour at all.
    const HOUR_A: i32 = EPOCH + 17_000_000;
    const HOUR_B: i32 = HOUR_A + 1;

    fn test_collection() -> wire_format::Collection {
        d70::schema().collections[0].clone()
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

    fn rows(convoys: u32) -> Vec<naval_history::SunkConvoyHistoryRow> {
        vec![
            naval_history::SunkConvoyHistoryRow {
                month: 23_310,
                convoys,
                killer_country_tag_db_id: 8,
                owner_tag_db_id: 9,
            },
            naval_history::SunkConvoyHistoryRow {
                month: 23_308,
                convoys: 2,
                killer_country_tag_db_id: 4,
                owner_tag_db_id: 9,
            },
        ]
    }

    /// A d70 DATA payload bound to the record that will carry it. Built through
    /// `compact_contract`'s own encoder, so a test can never accidentally assert
    /// against a payload the real codec would reject.
    fn d70_payload(sequence: u64, game_hour: i32, convoys: u32) -> Vec<u8> {
        let frame_hour = hour_index_from_raw(game_hour)
            .and_then(|h| u64::try_from(h).ok())
            .expect("test hours are in range");
        naval_history::encode_exact_payload(&rows(convoys), 0, frame_hour, sequence, 0, 2)
            .expect("encodes")
    }

    /// The explicit-unavailable v3 payload: failure status, zero rows, all-zero
    /// receipts (`naval_history.rs:174`-`:198`).
    fn d70_unavailable(sequence: u64, game_hour: i32, status: u32) -> Vec<u8> {
        let frame_hour = hour_index_from_raw(game_hour)
            .and_then(|h| u64::try_from(h).ok())
            .expect("test hours are in range");
        naval_history::encode_unavailable_payload(0, frame_hour, sequence, status)
            .expect("encodes")
    }

    /// Two closed hours, each carrying one d70 DATA record.
    fn good_stream() -> Vec<u8> {
        let mut s = Vec::new();
        s.extend(record(
            1,
            HOUR_A,
            D70,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &d70_payload(1, HOUR_A, 5),
        ));
        s.extend(frame_end(2, HOUR_A));
        s.extend(record(
            3,
            HOUR_B,
            D70,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &d70_payload(3, HOUR_B, 6),
        ));
        s.extend(frame_end(4, HOUR_B));
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
    fn the_library_and_the_contract_agree_on_every_status_predicate() {
        assert_status_predicates_agree();
    }

    #[test]
    fn a_well_formed_stream_yields_its_d70_hours() {
        let scan = scan_stream(
            &good_stream(),
            &test_collection(),
            StreamIdentityMode::Legacy,
        )
        .expect("accepted");
        assert_eq!(scan.hours.len(), 2);
        assert_eq!(scan.hours[0].game_hour, HOUR_A);
        assert_eq!(scan.hours[1].game_hour, HOUR_B);
        assert_eq!(scan.unavailable_records, 0);
        assert_eq!(scan.non_contiguous, 0);
        assert_eq!(scan.hours[0].state.len(), 2);
        assert!(!scan.hours[0].follows_unavailable);
        assert_eq!(scan.max_ordinal, 1);
    }

    #[test]
    fn the_record_binding_is_what_makes_an_hour_measurable() {
        // The headline, at the harness boundary: the payload's `frame_hour` and
        // `source_sequence` must agree with the H4CR record that carried it
        // (`world_fold.rs:10259`-`:10273`), and the harness supplies both from
        // the record exactly as `apply_domain_data` does (`:4801`-`:4806`).
        // Swept over both fields and in both directions.
        for (seq_in_payload, hour_in_payload) in [
            (2u64, HOUR_A),  // sequence disagrees with the record's 1
            (0u64, HOUR_A),  // ...including the zero the codec itself refuses
            (1, HOUR_B),     // frame hour disagrees
            (1, HOUR_A - 1), // ...in the other direction
        ] {
            let payload = if seq_in_payload == 0 {
                // The codec refuses a zero source sequence outright, so build the
                // bytes by hand for that one: it is still a refused stream, and
                // it must be refused rather than measured.
                let mut bytes = d70_payload(1, HOUR_A, 5);
                bytes[40..48].copy_from_slice(&0u64.to_le_bytes());
                bytes
            } else {
                d70_payload(seq_in_payload, hour_in_payload, 5)
            };
            let mut s = Vec::new();
            s.extend(record(
                1,
                HOUR_A,
                D70,
                COMPACT_RECORD_TYPE_DOMAIN_DATA,
                &payload,
            ));
            s.extend(frame_end(2, HOUR_A));
            assert!(
                scan_stream(&s, &test_collection(), StreamIdentityMode::Legacy).is_err(),
                "payload bound to sequence {seq_in_payload} hour {hour_in_payload} \
                 must refuse a record at sequence 1 hour {HOUR_A}"
            );
        }
        // A record whose game hour is outside the calendar window makes
        // `hour_index_from_raw` return `None` (`calendar.rs:22`-`:27`), which no
        // v3 payload's `Some(frame_hour)` can match — a refusal, not a measured
        // hour.
        let payload = d70_payload(1, HOUR_A, 5);
        let mut s = Vec::new();
        s.extend(record(1, 0, D70, COMPACT_RECORD_TYPE_DOMAIN_DATA, &payload));
        s.extend(frame_end(2, 0));
        assert!(scan_stream(&s, &test_collection(), StreamIdentityMode::Legacy).is_err());
    }

    #[test]
    fn an_unavailable_hour_is_excluded_and_forces_the_next_hour_to_baseline() {
        // The fold sets the accumulator to `None` (`world_fold.rs:10249`), so the
        // hour carries no data AND the next measured hour cannot be a delta
        // against the previous one. Every failure bit, one at a time.
        for bit in [0u32, 1, 2, 3, 6, 7, 8, 9, 12] {
            let status = 1u32 << bit;
            let mut s = Vec::new();
            s.extend(record(
                1,
                HOUR_A,
                D70,
                COMPACT_RECORD_TYPE_DOMAIN_DATA,
                &d70_payload(1, HOUR_A, 5),
            ));
            s.extend(frame_end(2, HOUR_A));
            s.extend(record(
                3,
                HOUR_B,
                D70,
                COMPACT_RECORD_TYPE_DOMAIN_DATA,
                &d70_unavailable(3, HOUR_B, status),
            ));
            s.extend(frame_end(4, HOUR_B));
            s.extend(record(
                5,
                HOUR_B + 1,
                D70,
                COMPACT_RECORD_TYPE_DOMAIN_DATA,
                &d70_payload(5, HOUR_B + 1, 7),
            ));
            s.extend(frame_end(6, HOUR_B + 1));
            let scan = scan_stream(&s, &test_collection(), StreamIdentityMode::Legacy)
                .unwrap_or_else(|e| panic!("failure bit {bit}: {e}"));
            assert_eq!(scan.unavailable_records, 1, "bit {bit}");
            assert_eq!(scan.hours.len(), 2, "bit {bit}");
            assert_eq!(scan.hours[0].game_hour, HOUR_A, "bit {bit}");
            assert_eq!(scan.hours[1].game_hour, HOUR_B + 1, "bit {bit}");
            assert!(
                !scan.hours[0].follows_unavailable,
                "bit {bit}: the first hour follows nothing"
            );
            assert!(
                scan.hours[1].follows_unavailable,
                "bit {bit}: the hour after an unavailable one must be baselined"
            );
            assert_eq!(scan.non_contiguous, 1, "bit {bit}: the skipped hour shows");
        }
    }

    #[test]
    fn a_legacy_authority_payload_refuses_the_stream_on_a_clean_frame() {
        // `world_fold.rs:10255`: v1 and v2 are readable and cannot supply
        // production authority. This is a stream rejection, not an excluded hour
        // — and the distinction is exactly what the failure gate's position
        // decides, which the library test pins and this one confirms end to end.
        let mut sorted = rows(5);
        sorted.sort();
        for (label, payload) in [
            (
                "v2",
                naval_history::encode_legacy_ordered_payload(&rows(5), 0, 0, 2).expect("encodes"),
            ),
            ("v1", {
                let mut bytes = naval_history::encode_legacy_ordered_payload(&sorted, 0, 0, 2)
                    .expect("encodes");
                bytes[4..6].copy_from_slice(&naval_history::LEGACY_PAYLOAD_VERSION.to_le_bytes());
                bytes
            }),
        ] {
            let mut s = Vec::new();
            s.extend(record(
                1,
                HOUR_A,
                D70,
                COMPACT_RECORD_TYPE_DOMAIN_DATA,
                &payload,
            ));
            s.extend(frame_end(2, HOUR_A));
            let error = scan_stream(&s, &test_collection(), StreamIdentityMode::Legacy)
                .expect_err("a legacy payload cannot supply production authority");
            assert!(error.contains("legacy-unbound"), "{label}: {error}");
        }
        // Under a FAILURE status the same payload is an excluded hour instead:
        // the failure gate runs first (`world_fold.rs:10243`). A trailing
        // measured hour keeps this from passing for the wrong reason.
        let failed = naval_history::encode_legacy_ordered_payload(
            &rows(5),
            0,
            COMPACT_STATUS_SOURCE_UNAVAILABLE,
            2,
        )
        .expect("encodes");
        let mut s = Vec::new();
        s.extend(record(
            1,
            HOUR_A,
            D70,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &failed,
        ));
        s.extend(frame_end(2, HOUR_A));
        s.extend(record(
            3,
            HOUR_B,
            D70,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &d70_payload(3, HOUR_B, 6),
        ));
        s.extend(frame_end(4, HOUR_B));
        let scan = scan_stream(&s, &test_collection(), StreamIdentityMode::Legacy)
            .expect("the failure gate fires before the authority check");
        assert_eq!(scan.unavailable_records, 1);
        assert_eq!(scan.hours.len(), 1);
        assert_eq!(scan.hours[0].game_hour, HOUR_B);
        assert!(scan.hours[0].follows_unavailable);
    }

    #[test]
    fn an_unassigned_status_bit_is_refused_not_classified() {
        // For d70 this rule is part of the grammar rather than a harness
        // convention: the SCH1 v3 decoder rejects an unassigned bit itself
        // (`naval_history.rs:344`-`:346`), before anything classifies the word.
        // The check is here anyway, swept, because the *reason* it holds must not
        // depend on which layer happens to implement it today.
        for bit in 14..32u32 {
            for extra in [0u32, 1, 1 << 10] {
                let mut payload = d70_payload(1, HOUR_A, 5);
                payload[12..16].copy_from_slice(&((1u32 << bit) | extra).to_le_bytes());
                let mut s = Vec::new();
                s.extend(record(
                    1,
                    HOUR_A,
                    D70,
                    COMPACT_RECORD_TYPE_DOMAIN_DATA,
                    &payload,
                ));
                s.extend(frame_end(2, HOUR_A));
                assert!(
                    scan_stream(&s, &test_collection(), StreamIdentityMode::Legacy).is_err(),
                    "status bit {bit} (+{extra:#x}) must be refused"
                );
            }
        }
        // And the informational bits are still accepted, so the check rejects
        // unassigned bits and nothing else.
        for informational in [1u32 << 4, 1 << 5, 1 << 10, 1 << 11, 1 << 13] {
            let payload =
                naval_history::encode_exact_payload(&rows(5), 0, (HOUR_A - EPOCH) as u64, 1, informational, 2)
                    .expect("encodes");
            let mut s = Vec::new();
            s.extend(record(
                1,
                HOUR_A,
                D70,
                COMPACT_RECORD_TYPE_DOMAIN_DATA,
                &payload,
            ));
            s.extend(frame_end(2, HOUR_A));
            let scan = scan_stream(&s, &test_collection(), StreamIdentityMode::Legacy)
                .unwrap_or_else(|e| panic!("informational bit {informational:#x}: {e}"));
            assert_eq!(scan.hours.len(), 1, "bit {informational:#x}");
        }
    }

    #[test]
    fn an_excluded_record_is_still_checked_by_the_payload_authority() {
        // `validate_domain_data_payload_at` runs
        // `decode_compact_sunk_convoy_history_data_payload`
        // (`domain_data_authority.rs:13102`) on EVERY DOMAIN_DATA record whatever
        // its status, and its complaint is `malformed_payload_records`, which
        // `strict_stream_certification_verdict` refuses. For d70 the fold reaches
        // the same decode anyway (it decodes before consulting status), so this
        // check is belt and braces — and it is the layer that would still catch a
        // malformed excluded record if that order ever changed.
        //
        // A trailing measured hour keeps the assertion from passing merely
        // because a lone excluded record leaves "no usable records".
        let mut bad = d70_unavailable(1, HOUR_A, COMPACT_STATUS_SOURCE_UNAVAILABLE);
        let last = bad.len() - 1;
        bad[last] ^= 0xFF; // corrupt the final receipt byte
        let mut s = Vec::new();
        s.extend(record(1, HOUR_A, D70, COMPACT_RECORD_TYPE_DOMAIN_DATA, &bad));
        s.extend(frame_end(2, HOUR_A));
        s.extend(record(
            3,
            HOUR_B,
            D70,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &d70_payload(3, HOUR_B, 6),
        ));
        s.extend(frame_end(4, HOUR_B));
        assert!(
            scan_stream(&s, &test_collection(), StreamIdentityMode::Legacy).is_err(),
            "a malformed EXCLUDED record must refuse the stream, not be skipped"
        );

        // A WELL-FORMED excluded record is still excluded, not refused.
        let mut s = Vec::new();
        s.extend(record(
            1,
            HOUR_A,
            D70,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &d70_unavailable(1, HOUR_A, COMPACT_STATUS_SOURCE_UNAVAILABLE),
        ));
        s.extend(frame_end(2, HOUR_A));
        s.extend(record(
            3,
            HOUR_B,
            D70,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &d70_payload(3, HOUR_B, 6),
        ));
        s.extend(frame_end(4, HOUR_B));
        let scan =
            scan_stream(&s, &test_collection(), StreamIdentityMode::Legacy).expect("well-formed");
        assert_eq!(scan.unavailable_records, 1);
        assert_eq!(scan.hours.len(), 1);
    }

    #[test]
    fn the_mirror_and_the_codec_agree_on_every_payload_shape() {
        // `cross_check_against_codec` is what keeps `d70.rs`'s hand-mirrored SCH1
        // grammar honest. Run it over the shapes the capture contains and over
        // every single-byte corruption of a good payload, so the check itself is
        // exercised rather than merely present.
        let c = test_collection();
        let good = d70_payload(1, HOUR_A, 5);
        assert!(cross_check_against_codec(&good, &c, 0).is_ok());
        assert!(cross_check_against_codec(
            &d70_unavailable(1, HOUR_A, COMPACT_STATUS_SOURCE_UNAVAILABLE),
            &c,
            0
        )
        .is_ok());
        let mut sorted = rows(5);
        sorted.sort();
        let v2 = naval_history::encode_legacy_ordered_payload(&rows(5), 0, 0, 2).expect("encodes");
        assert!(cross_check_against_codec(&v2, &c, 0).is_ok());
        let mut v1 =
            naval_history::encode_legacy_ordered_payload(&sorted, 0, 0, 2).expect("encodes");
        v1[4..6].copy_from_slice(&naval_history::LEGACY_PAYLOAD_VERSION.to_le_bytes());
        assert!(cross_check_against_codec(&v1, &c, 0).is_ok());

        for index in 0..good.len() {
            let mut bad = good.clone();
            bad[index] ^= 0xFF;
            cross_check_against_codec(&bad, &c, 0)
                .unwrap_or_else(|e| panic!("byte {index}: {e}"));
        }
        // Truncations too, at every length.
        for len in 0..good.len() {
            cross_check_against_codec(&good[..len], &c, 0)
                .unwrap_or_else(|e| panic!("length {len}: {e}"));
        }
    }

    #[test]
    fn the_ordinal_carries_the_wire_order_through_the_scan() {
        // The rows are emitted in an order that is NOT the canonical
        // `(month, killer, owner)` sort — the reference capture's order never is
        // — so the scan must reproduce the payload's positions, not the keyed
        // set's.
        let c = test_collection();
        let scan = scan_stream(&good_stream(), &c, StreamIdentityMode::Legacy).expect("accepted");
        let ordinal_index = value_index(&c, d70::field::SOURCE_ORDINAL);
        let state = &scan.hours[0].state;
        // Row 0 of the payload is month 23_310; row 1 is month 23_308. The keyed
        // set orders them the other way round.
        let first = &state[&wire_format::body::Key(vec![23_310, 8, 9])];
        let second = &state[&wire_format::body::Key(vec![23_308, 4, 9])];
        assert_eq!(first.values[ordinal_index], Value::Unsigned(0));
        assert_eq!(second.values[ordinal_index], Value::Unsigned(1));
        assert_eq!(
            scan.canonical_order_hours, 0,
            "neither hour is in canonical key order, so the ordinal is load-bearing"
        );
    }

    #[test]
    fn a_stream_truncated_on_a_record_boundary_is_rejected() {
        let full = good_stream();
        for records in [1usize, 3] {
            let cut = boundary(&full, records);
            assert!(
                scan_stream(&full[..cut], &test_collection(), StreamIdentityMode::Legacy).is_err(),
                "a stream cut after {records} record(s) leaves an hour open"
            );
        }
        // A cut exactly at a FRAME_END boundary IS a well-formed shorter stream.
        for (records, expected_hours) in [(2usize, 1usize), (4, 2)] {
            let cut = boundary(&full, records);
            let scan = scan_stream(&full[..cut], &test_collection(), StreamIdentityMode::Legacy)
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
                scan_stream(&full[..cut], &test_collection(), StreamIdentityMode::Legacy).is_err(),
                "mid-record cut at {cut}"
            );
        }
        assert!(
            scan_stream(&[], &test_collection(), StreamIdentityMode::Legacy).is_err(),
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
                    scan_stream(&s, &test_collection(), StreamIdentityMode::Legacy).is_err(),
                    "record {records} patched at +{offset} must be refused"
                );
            }
        }
    }

    #[test]
    fn a_sequence_discontinuity_is_rejected() {
        // For d70 the record sequence is doubly load-bearing: the stream
        // validator's chain, and the payload's own `source_sequence` binding.
        for records in 0..4 {
            let at = boundary(&good_stream(), records);
            for wrong in [0u64, 9] {
                let mut s = good_stream();
                s[at + 8..at + 16].copy_from_slice(&wrong.to_le_bytes());
                assert!(
                    scan_stream(&s, &test_collection(), StreamIdentityMode::Legacy).is_err(),
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
            HOUR_A,
            D70,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &d70_payload(1, HOUR_A, 5),
        ));
        s.extend(frame_end(2, HOUR_A + 999));
        assert!(scan_stream(&s, &test_collection(), StreamIdentityMode::Legacy).is_err());

        let mut s = Vec::new();
        s.extend(frame_end(1, HOUR_A));
        assert!(scan_stream(&s, &test_collection(), StreamIdentityMode::Legacy).is_err());
    }

    #[test]
    fn non_monotonic_hours_are_rejected() {
        let mut s = Vec::new();
        s.extend(record(
            1,
            HOUR_B,
            D70,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &d70_payload(1, HOUR_B, 5),
        ));
        s.extend(frame_end(2, HOUR_B));
        s.extend(record(
            3,
            HOUR_A,
            D70,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &d70_payload(3, HOUR_A, 6),
        ));
        s.extend(frame_end(4, HOUR_A));
        assert!(scan_stream(&s, &test_collection(), StreamIdentityMode::Legacy).is_err());
    }

    #[test]
    fn a_second_d70_record_in_one_hour_is_rejected() {
        // d70 is full-replace; two records in one hour would become two
        // measurement hours built from the same clock time. The rule is checked
        // over EVERY d70 DATA record and before status-based exclusion, so all
        // four success/unavailable orderings are refused.
        const FAILED: u32 = COMPACT_STATUS_SOURCE_UNAVAILABLE;
        let good = |seq: u64| d70_payload(seq, HOUR_A, 5);
        let failed = |seq: u64| d70_unavailable(seq, HOUR_A, FAILED);
        for (first, second) in [
            (good(1), good(2)),
            (good(1), failed(2)),
            (failed(1), good(2)),
            (failed(1), failed(2)),
        ] {
            let mut s = Vec::new();
            s.extend(record(
                1,
                HOUR_A,
                D70,
                COMPACT_RECORD_TYPE_DOMAIN_DATA,
                &first,
            ));
            s.extend(record(
                2,
                HOUR_A,
                D70,
                COMPACT_RECORD_TYPE_DOMAIN_DATA,
                &second,
            ));
            s.extend(frame_end(3, HOUR_A));
            assert!(
                scan_stream(&s, &test_collection(), StreamIdentityMode::Legacy).is_err(),
                "two d70 records in one hour"
            );
        }
    }

    #[test]
    fn a_stream_with_no_d70_records_is_refused_not_reported_as_zero() {
        let mut s = Vec::new();
        s.extend(record(
            1,
            HOUR_A,
            20,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &[0u8; 8],
        ));
        s.extend(frame_end(2, HOUR_A));
        assert!(scan_stream(&s, &test_collection(), StreamIdentityMode::Legacy).is_err());
    }

    #[test]
    fn a_malformed_d70_payload_is_refused_not_skipped() {
        // Every way the SCH1 grammar can be violated, at the stream level: a
        // truncation, a bad row and a bad cross-row rule.
        let mut truncated = d70_payload(1, HOUR_A, 5);
        truncated.truncate(truncated.len() - 1);

        let mut zero_convoys = d70_payload(1, HOUR_A, 5);
        zero_convoys[naval_history::HEADER_LEN + 4..naval_history::HEADER_LEN + 8]
            .copy_from_slice(&0u32.to_le_bytes());

        let mut zero_owner = d70_payload(1, HOUR_A, 5);
        zero_owner[naval_history::HEADER_LEN + 12..naval_history::HEADER_LEN + 16]
            .copy_from_slice(&0u32.to_le_bytes());

        let mut bad_tail = d70_payload(1, HOUR_A, 5);
        bad_tail[naval_history::HEADER_LEN + 16..naval_history::HEADER_LEN + 24]
            .copy_from_slice(&1u64.to_le_bytes());

        let mut observed = d70_payload(1, HOUR_A, 5);
        observed[28..32].copy_from_slice(&3u32.to_le_bytes());

        // A duplicated identity: copy row 0 over row 1.
        let mut duplicated = d70_payload(1, HOUR_A, 5);
        let row0: Vec<u8> = duplicated
            [naval_history::HEADER_LEN..naval_history::HEADER_LEN + naval_history::ROW_LEN]
            .to_vec();
        duplicated[naval_history::HEADER_LEN + naval_history::ROW_LEN
            ..naval_history::HEADER_LEN + 2 * naval_history::ROW_LEN]
            .copy_from_slice(&row0);

        for (index, payload) in [
            truncated,
            zero_convoys,
            zero_owner,
            bad_tail,
            observed,
            duplicated,
        ]
        .into_iter()
        .enumerate()
        {
            let mut s = Vec::new();
            s.extend(record(
                1,
                HOUR_A,
                D70,
                COMPACT_RECORD_TYPE_DOMAIN_DATA,
                &payload,
            ));
            s.extend(frame_end(2, HOUR_A));
            assert!(
                scan_stream(&s, &test_collection(), StreamIdentityMode::Legacy).is_err(),
                "malformed payload {index} must refuse the stream"
            );
        }
    }

    /// One DOMAIN_PERF record with the minimal valid payload
    /// (`COMPACT_DOMAIN_PERF_BASE_LEN`).
    fn perf(sequence: u64, game_hour: i32, domain: u16) -> Vec<u8> {
        record(
            sequence,
            game_hour,
            domain,
            COMPACT_RECORD_TYPE_DOMAIN_PERF,
            &vec![0u8; compact_contract::COMPACT_DOMAIN_PERF_BASE_LEN],
        )
    }

    /// One FRAME_END with explicit seal fields, so a test can perturb exactly one.
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
        let data = |seq: u64, hour: i32| {
            record(
                seq,
                hour,
                D70,
                COMPACT_RECORD_TYPE_DOMAIN_DATA,
                &d70_payload(seq, hour, 5),
            )
        };
        // A well-formed seal, for contrast: these streams carry no DOMAIN_PERF
        // records, so the seal's domain counts must be 0.
        let mut ok = Vec::new();
        ok.extend(data(1, HOUR_A));
        ok.extend(frame_end_sealed(2, HOUR_A, 0, 0, 0, 0));
        assert_eq!(
            scan_stream(&ok, &test_collection(), StreamIdentityMode::Legacy)
                .expect("a sealed stream is accepted")
                .hours
                .len(),
            1
        );

        let mut bad_seals: Vec<(String, Vec<u8>)> = vec![
            (
                "reserved nonzero".to_string(),
                frame_end_sealed(2, HOUR_A, 0, 0, 0, 1),
            ),
            (
                "emitted != scheduled".to_string(),
                frame_end_sealed(2, HOUR_A, 0, 1, 0, 0),
            ),
        ];
        for bit in 14..32u32 {
            bad_seals.push((
                format!("unknown seal status bit {bit}"),
                frame_end_sealed(2, HOUR_A, 0, 0, 1 << bit, 0),
            ));
        }
        for bit in [0u32, 1, 2, 3, 6, 7, 8, 9, 12] {
            bad_seals.push((
                format!("fatal seal status bit {bit}"),
                frame_end_sealed(2, HOUR_A, 0, 0, 1 << bit, 0),
            ));
        }
        for (label, sealed) in bad_seals {
            let mut s = Vec::new();
            s.extend(data(1, HOUR_A));
            s.extend(sealed);
            assert!(
                scan_stream(&s, &test_collection(), StreamIdentityMode::Legacy).is_err(),
                "{label} must refuse the stream, not yield a measurement"
            );
        }

        for len in [0usize, 1, 16, 31, 33, 64] {
            let mut s = Vec::new();
            s.extend(data(1, HOUR_A));
            s.extend(record(
                2,
                HOUR_A,
                COMPACT_DOMAIN_FRAME,
                COMPACT_RECORD_TYPE_FRAME_END,
                &vec![0u8; len],
            ));
            assert!(
                scan_stream(&s, &test_collection(), StreamIdentityMode::Legacy).is_err(),
                "a {len}-byte FRAME_END payload is not a seal"
            );
        }

        for informational in [1u32 << 4, 1 << 5, 1 << 10, 1 << 11, 1 << 13] {
            let mut s = Vec::new();
            s.extend(data(1, HOUR_A));
            s.extend(frame_end_sealed(2, HOUR_A, 0, 0, informational, 0));
            assert!(
                scan_stream(&s, &test_collection(), StreamIdentityMode::Legacy).is_ok(),
                "informational seal status {informational:#x} must be accepted"
            );
        }

        // A seal violation on a LATER frame refuses the whole stream rather than
        // silently truncating the measurement to the hours before it.
        let mut s = Vec::new();
        s.extend(data(1, HOUR_A));
        s.extend(frame_end_sealed(2, HOUR_A, 0, 0, 0, 0));
        s.extend(data(3, HOUR_B));
        s.extend(frame_end_sealed(4, HOUR_B, 0, 0, 0, 1));
        assert!(scan_stream(&s, &test_collection(), StreamIdentityMode::Legacy).is_err());
    }

    #[test]
    fn perf_hour_binding_is_enforced() {
        let data = |seq: u64, hour: i32| {
            record(
                seq,
                hour,
                D70,
                COMPACT_RECORD_TYPE_DOMAIN_DATA,
                &d70_payload(seq, hour, 5),
            )
        };
        let mut ok = Vec::new();
        ok.extend(perf(1, HOUR_A, 20));
        ok.extend(perf(2, HOUR_A, D70));
        ok.extend(perf(3, HOUR_A, PROBE_DOMAIN_ID)); // the probe is not counted
        ok.extend(data(4, HOUR_A));
        ok.extend(frame_end_sealed(5, HOUR_A, 2, 2, 0, 0));
        assert_eq!(
            scan_stream(&ok, &test_collection(), StreamIdentityMode::Legacy)
                .expect("a PERF-bound hour is accepted")
                .hours
                .len(),
            1
        );

        let mut probe_twice = Vec::new();
        probe_twice.extend(perf(1, HOUR_A, PROBE_DOMAIN_ID));
        probe_twice.extend(perf(2, HOUR_A, PROBE_DOMAIN_ID));
        probe_twice.extend(data(3, HOUR_A));
        probe_twice.extend(frame_end_sealed(4, HOUR_A, 0, 0, 0, 0));
        assert!(scan_stream(&probe_twice, &test_collection(), StreamIdentityMode::Legacy).is_ok());

        let mut dup = Vec::new();
        dup.extend(perf(1, HOUR_A, 20));
        dup.extend(perf(2, HOUR_A, 20));
        dup.extend(data(3, HOUR_A));
        dup.extend(frame_end_sealed(4, HOUR_A, 1, 1, 0, 0));
        assert!(
            scan_stream(&dup, &test_collection(), StreamIdentityMode::Legacy).is_err(),
            "duplicate PERF domain"
        );

        let mut interleaved = Vec::new();
        interleaved.extend(data(1, HOUR_A));
        interleaved.extend(perf(2, HOUR_A + 999, 20));
        interleaved.extend(frame_end_sealed(3, HOUR_A, 1, 1, 0, 0));
        assert!(
            scan_stream(&interleaved, &test_collection(), StreamIdentityMode::Legacy).is_err(),
            "PERF for an hour other than the open one"
        );

        let mut stale = Vec::new();
        stale.extend(data(1, HOUR_A));
        stale.extend(frame_end_sealed(2, HOUR_A, 0, 0, 0, 0));
        stale.extend(perf(3, HOUR_A, 20));
        stale.extend(data(4, HOUR_B));
        stale.extend(frame_end_sealed(5, HOUR_B, 1, 1, 0, 0));
        assert!(
            scan_stream(&stale, &test_collection(), StreamIdentityMode::Legacy).is_err(),
            "non-monotonic PERF"
        );

        let mut conflict = Vec::new();
        conflict.extend(perf(1, HOUR_A, 20));
        conflict.extend(perf(2, HOUR_B, 21));
        conflict.extend(data(3, HOUR_A));
        conflict.extend(frame_end_sealed(4, HOUR_A, 2, 2, 0, 0));
        assert!(
            scan_stream(&conflict, &test_collection(), StreamIdentityMode::Legacy).is_err(),
            "PERF hour conflict"
        );

        for (emitted, scheduled) in [(1u32, 1u32), (3, 3), (2, 3), (3, 2)] {
            let mut s = Vec::new();
            s.extend(perf(1, HOUR_A, 20));
            s.extend(perf(2, HOUR_A, D70));
            s.extend(data(3, HOUR_A));
            s.extend(frame_end_sealed(4, HOUR_A, emitted, scheduled, 0, 0));
            assert!(
                scan_stream(&s, &test_collection(), StreamIdentityMode::Legacy).is_err(),
                "seal claiming emitted={emitted} scheduled={scheduled} against 2 observed \
                 PERF domains must be refused"
            );
        }

        let mut two_hours = Vec::new();
        two_hours.extend(perf(1, HOUR_A, 20));
        two_hours.extend(data(2, HOUR_A));
        two_hours.extend(frame_end_sealed(3, HOUR_A, 1, 1, 0, 0));
        two_hours.extend(perf(4, HOUR_B, 20));
        two_hours.extend(data(5, HOUR_B));
        two_hours.extend(frame_end_sealed(6, HOUR_B, 1, 1, 0, 0));
        assert_eq!(
            scan_stream(&two_hours, &test_collection(), StreamIdentityMode::Legacy)
                .expect("the PERF domain set resets per hour")
                .hours
                .len(),
            2
        );
    }

    #[test]
    fn a_data_record_must_open_the_hour_the_pending_perf_records_bound() {
        // `stream_fold.rs:378`-`:386`. The PERF arm alone does not cover this,
        // because one pending PERF hour is legal until a DATA record disagrees.
        let data = |seq: u64, hour: i32| {
            record(
                seq,
                hour,
                D70,
                COMPACT_RECORD_TYPE_DOMAIN_DATA,
                &d70_payload(seq, hour, 5),
            )
        };
        for (perf_hour, data_hour) in [
            (HOUR_A, HOUR_B),
            (HOUR_B, HOUR_A),
            (HOUR_A, HOUR_A + 200),
            (HOUR_A + 200, HOUR_A),
        ] {
            let mut s = Vec::new();
            s.extend(perf(1, perf_hour, 20));
            s.extend(data(2, data_hour));
            s.extend(frame_end_sealed(3, data_hour, 1, 1, 0, 0));
            assert!(
                scan_stream(&s, &test_collection(), StreamIdentityMode::Legacy).is_err(),
                "pending PERF hour {perf_hour} then DATA hour {data_hour}"
            );
        }
        let mut agreeing = Vec::new();
        agreeing.extend(perf(1, HOUR_A, 20));
        agreeing.extend(data(2, HOUR_A));
        agreeing.extend(frame_end_sealed(3, HOUR_A, 1, 1, 0, 0));
        assert_eq!(
            scan_stream(&agreeing, &test_collection(), StreamIdentityMode::Legacy)
                .expect("a DATA record matching the pending PERF hour is accepted")
                .hours
                .len(),
            1
        );
    }

    #[test]
    fn the_identity_protocol_is_stated_not_sniffed() {
        use compact_contract::campaign_identity::COMPACT_RECORD_TYPE_STREAM_HEADER;

        let decapitated = good_stream();
        assert!(
            scan_stream(
                &decapitated,
                &test_collection(),
                StreamIdentityMode::IdentityV1
            )
            .is_err(),
            "an identity-mode stream whose CID1 record is missing must be refused"
        );
        assert_eq!(
            scan_stream(&decapitated, &test_collection(), StreamIdentityMode::Legacy)
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
            scan_stream(
                &identity_stream,
                &test_collection(),
                StreamIdentityMode::Legacy
            )
            .is_err(),
            "a CID1 record under the legacy claim must be refused"
        );
    }

    #[test]
    fn scan_stream_never_panics_on_arbitrary_bytes() {
        // Total over hostile input: the harness reads a file from disk and must
        // return a diagnosis, never unwind.
        let c = test_collection();
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
            let _ = scan_stream(&bytes, &c, StreamIdentityMode::Legacy);
        }
        let good = good_stream();
        for i in 0..good.len() {
            let mut s = good.clone();
            s[i] ^= 0xFF;
            let _ = scan_stream(&s, &c, StreamIdentityMode::Legacy);
        }
    }
}
