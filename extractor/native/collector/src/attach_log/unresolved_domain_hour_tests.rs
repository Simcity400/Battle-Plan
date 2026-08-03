//! A scheduled domain-hour must resolve to a DATA record or to a gap naming
//! why there is none. These cover the seal pass that enforces it.
//!
//! The shape being prevented is real and shipped: in run t15
//! `pov_spotted_units_live` (d95) emitted a DOMAIN_PERF record and then, on its
//! reader's error path, no DATA record and no gap -- for 396 of 617 frames. No
//! ledger in the sealed manifest counted those hours as missing, because none
//! of them was counting anything for d95 at all, so the loss was
//! indistinguishable from success. `pov_province_visibility_live` (d94) is the
//! same code shape and balanced only because its read happened to succeed.

use std::collections::{BTreeMap, BTreeSet};

use compact_contract::domain_gap::{DomainGapPart, DomainGapReason, DomainGapRecordV1};

use super::*;

const HOUR: i32 = 60_832_384;
/// d95 itself: the domain the shape was found on.
const SILENT_DOMAIN: u16 = COMPACT_DOMAIN_POV_SPOTTED_UNITS_LIVE;
/// d94: the same code shape, silent only when its read fails.
const RESOLVED_DOMAIN: u16 = COMPACT_DOMAIN_POV_PROVINCE_VISIBILITY_LIVE;

fn perf_record(out: &mut Vec<u8>, sequence: u64, domain_id: u16) {
    append_compact_domain_perf_record(
        out,
        sequence,
        HOUR,
        CompactDomainPerfRecord {
            domain_id,
            read_time_us: 3,
            row_count: 0,
            changed_rows: 0,
            unit_count: 0,
            changed_unit_count: 0,
            status_flags: COMPACT_STATUS_PARTIAL_DATA,
        },
        0,
    );
}

fn data_record(out: &mut Vec<u8>, sequence: u64, domain_id: u16) {
    append_compact_record(
        out,
        sequence,
        HOUR,
        domain_id,
        COMPACT_RECORD_TYPE_DOMAIN_DATA,
        0,
        &[0u8; 8],
    );
}

/// Every (domain, record type) pair on the wire, in order.
fn wire_shape(records: &[u8]) -> Vec<(u16, u16)> {
    let mut out = Vec::new();
    let mut offset = 0usize;
    while offset < records.len() {
        let header =
            compact_contract::parse_compact_record_header(&records[offset..]).expect("header");
        out.push((header.domain_id, header.record_type));
        offset += header.record_len();
    }
    out
}

fn decode_gap_for(records: &[u8], domain_id: u16) -> DomainGapRecordV1 {
    let mut offset = 0usize;
    while offset < records.len() {
        let header =
            compact_contract::parse_compact_record_header(&records[offset..]).expect("header");
        let end = offset + header.record_len();
        if header.domain_id == domain_id
            && header.record_type == compact_contract::COMPACT_RECORD_TYPE_DOMAIN_GAP
        {
            let payload_start = offset + compact_contract::COMPACT_RECORD_HEADER_LEN;
            return DomainGapRecordV1::decode(&records[payload_start..end]).expect("H4DG payload");
        }
        offset = end;
    }
    panic!("no DOMAIN_GAP for domain {domain_id}");
}

/// The exact t15 d95 shape: a PERF record, no DATA, no gap. The seal must turn
/// it into a gap, and must not leave the PERF record behind next to it --
/// `stream_fold` admits one terminal outcome per domain per hour.
///
/// Mutation proof: delete the `close_unresolved_domain_hours` call from the
/// seal and this fails on `silenced`; return `Ok` early from the function
/// without minting and it fails on the missing gap; keep the superseded PERF
/// record and it fails on `wire_shape`.
#[test]
fn a_domain_that_emits_perf_but_no_data_is_closed_with_a_named_gap() {
    let mut records = Vec::new();
    perf_record(&mut records, 1, SILENT_DOMAIN);
    let mut ordinals = BTreeMap::new();

    let closed = close_unresolved_domain_hours(
        &mut records,
        1,
        HOUR,
        &BTreeSet::from([SILENT_DOMAIN]),
        &mut ordinals,
    )
    .expect("seal closes the unresolved hour");

    assert_eq!(closed.silenced, BTreeSet::from([SILENT_DOMAIN]));
    assert_eq!(
        wire_shape(&records),
        vec![(
            SILENT_DOMAIN,
            compact_contract::COMPACT_RECORD_TYPE_DOMAIN_GAP
        )],
        "the superseded PERF record must be replaced by the gap, not joined to it"
    );
    let gap = decode_gap_for(&records, SILENT_DOMAIN);
    assert_eq!(gap.reason, DomainGapReason::ReaderSilent);
    assert_eq!(gap.target_hour_raw, HOUR);
    // The domain already counted itself as an emission when it appended the
    // PERF record, and the gap replacing it is the same single terminal
    // outcome, so FRAME_END must not be moved.
    assert_eq!(closed.newly_emitted, 0);
}

/// `ReaderSilent` says only that no outcome was recorded. It must therefore
/// carry no hour witness at all -- borrowing the frame's tick would assert the
/// reader observed something it never observed.
///
/// Mutation proof: set `tick_before`/`tick_after` to the frame hour in
/// `close_unresolved_domain_hours` and `encode` rejects the payload, because
/// the reason's witness contract requires both to be zero; the `expect` in the
/// helper then fires.
#[test]
fn a_reader_silent_gap_claims_no_clock_witness() {
    let mut records = Vec::new();
    perf_record(&mut records, 1, SILENT_DOMAIN);
    let mut ordinals = BTreeMap::new();

    close_unresolved_domain_hours(
        &mut records,
        1,
        HOUR,
        &BTreeSet::from([SILENT_DOMAIN]),
        &mut ordinals,
    )
    .expect("seal closes the unresolved hour");

    let gap = decode_gap_for(&records, SILENT_DOMAIN);
    assert_eq!((gap.tick_before, gap.tick_after), (0, 0));
    assert_eq!(gap.range_start_hour_raw, gap.range_end_hour_raw);
    assert_eq!(
        (gap.queue_capacity, gap.queue_depth, gap.queue_dropped),
        (0, 0, 0),
        "nothing was queued or dropped; a queue witness would be a fabricated cause"
    );
}

/// A domain that resolved is left exactly as it was. Without this the pass
/// would be free to mint gaps over real data and still look like it was
/// working.
///
/// Mutation proof: drop the `!resolved.contains(domain)` filter and this fails
/// with a gap minted for a domain that published a DATA record -- which
/// `stream_fold` separately rejects as DATA coexisting with a GAP.
#[test]
fn a_domain_that_published_data_is_left_untouched() {
    let mut records = Vec::new();
    perf_record(&mut records, 1, RESOLVED_DOMAIN);
    data_record(&mut records, 2, RESOLVED_DOMAIN);
    let before = records.clone();
    let mut ordinals = BTreeMap::new();

    let closed = close_unresolved_domain_hours(
        &mut records,
        1,
        HOUR,
        &BTreeSet::from([RESOLVED_DOMAIN]),
        &mut ordinals,
    )
    .expect("nothing to close");

    assert!(closed.silenced.is_empty());
    assert_eq!(closed.newly_emitted, 0);
    assert_eq!(records, before, "a resolved domain-hour is not rewritten");
    assert!(ordinals.is_empty(), "no outcome ordinal may be spent");
}

/// A scheduled domain that put NOTHING on the wire is the second shape of the
/// same hole: it is not even in the frame's terminal-outcome set, so the
/// FRAME_END counts stay self-consistent while the roster is silently short.
/// This one DOES owe an emission increment, because no site ever counted it.
///
/// Mutation proof: build `unresolved` from the PERF-bearing domains instead of
/// the roster and the gap is never minted; count `newly_emitted` over all
/// silenced domains instead of only the record-less ones and the previous test
/// fails with 1.
#[test]
fn a_scheduled_domain_that_emits_nothing_is_named_and_counted() {
    let mut records = Vec::new();
    perf_record(&mut records, 1, RESOLVED_DOMAIN);
    data_record(&mut records, 2, RESOLVED_DOMAIN);
    let mut ordinals = BTreeMap::new();

    let closed = close_unresolved_domain_hours(
        &mut records,
        1,
        HOUR,
        &BTreeSet::from([RESOLVED_DOMAIN, SILENT_DOMAIN]),
        &mut ordinals,
    )
    .expect("seal closes the absent domain");

    assert_eq!(closed.silenced, BTreeSet::from([SILENT_DOMAIN]));
    assert_eq!(
        closed.newly_emitted, 1,
        "a domain with no record at all was never counted as an emission"
    );
    assert_eq!(decode_gap_for(&records, SILENT_DOMAIN).reason, DomainGapReason::ReaderSilent);
    // The resolved domain's pair is still there and still first.
    assert_eq!(
        wire_shape(&records)[..2],
        [
            (RESOLVED_DOMAIN, COMPACT_RECORD_TYPE_DOMAIN_PERF),
            (RESOLVED_DOMAIN, COMPACT_RECORD_TYPE_DOMAIN_DATA),
        ]
    );
}

/// The reserved probe domain emits PERF records and no DATA by design and is
/// excluded from every coverage gate, so it must not be swept up.
///
/// Mutation proof: remove either `RC_PROBE_DOMAIN_ID` guard and this fails with
/// the probe's PERF record replaced by a gap.
#[test]
fn the_reserved_probe_domain_is_not_owed_an_outcome() {
    let mut records = Vec::new();
    perf_record(&mut records, 1, RC_PROBE_DOMAIN_ID);
    let before = records.clone();
    let mut ordinals = BTreeMap::new();

    let closed =
        close_unresolved_domain_hours(&mut records, 1, HOUR, &BTreeSet::new(), &mut ordinals)
            .expect("probe is exempt");

    assert!(closed.silenced.is_empty());
    assert_eq!(records, before);
}

/// Outcome ordinals are a per-(domain, part) sequence that consumers use to
/// detect a dropped outcome. A minted gap has to spend exactly one, or it opens
/// the very hole it was minted to close.
///
/// Mutation proof: stop writing back to `outcome_ordinals` and the second hour
/// re-uses ordinal 0.
#[test]
fn each_minted_gap_spends_exactly_one_outcome_ordinal() {
    let key = (SILENT_DOMAIN, domain_gap_part(SILENT_DOMAIN).code());
    let mut ordinals = BTreeMap::new();
    let mut ordinals_seen = Vec::new();

    for _ in 0..3 {
        let mut records = Vec::new();
        perf_record(&mut records, 1, SILENT_DOMAIN);
        close_unresolved_domain_hours(
            &mut records,
            1,
            HOUR,
            &BTreeSet::from([SILENT_DOMAIN]),
            &mut ordinals,
        )
        .expect("seal closes the unresolved hour");
        ordinals_seen.push(decode_gap_for(&records, SILENT_DOMAIN).outcome_ordinal);
    }

    assert_eq!(ordinals_seen, vec![0, 1, 2]);
    assert_eq!(ordinals.get(&key), Some(&3));
}

/// The behavioural tests above call the pass directly, so on their own they
/// keep saying "ok" if the seal simply stops calling it -- which is exactly how
/// this hole existed in the first place. This pins the call: one definition,
/// one production call site, and it runs before FRAME_END is appended, because
/// FRAME_END carries the emitted-domain counts the pass adjusts and
/// `stream_fold` checks those words against the terminal outcomes it observes.
///
/// Mutation proof: delete the call from the seal and the count is 1; move it
/// below `append_compact_frame_end_record` and the ordering assertion fails.
#[test]
fn the_seal_closes_unresolved_hours_exactly_once_before_frame_end() {
    const COLLECTOR_SOURCE: &str = include_str!("../attach_log.rs");
    let code = COLLECTOR_SOURCE
        .lines()
        .map(|line| match line.split_once("//") {
            Some((code, _)) => code,
            None => line,
        })
        .collect::<Vec<_>>()
        .join("\n");

    assert_eq!(
        code.matches("close_unresolved_domain_hours(").count(),
        2,
        "the definition and ONE production call site; an unresolved domain-hour is only \
         closed if the seal actually asks"
    );
    let call = code
        .find("let silent_domains = match close_unresolved_domain_hours(")
        .expect("the seal closes unresolved domain-hours");
    // Anchored to a newline plus exactly four spaces: the seal's own call sits
    // at the top level of the gate function, while the two earlier ones are
    // nested deeper and would otherwise match on their indent's tail.
    let frame_end = code
        .find("\n    append_compact_frame_end_record(")
        .expect("the seal appends FRAME_END");
    assert!(
        call < frame_end,
        "the pass adjusts the emitted-domain count that FRAME_END seals, so it must run first"
    );
}

/// d95's part is `Both`, and the gap must carry the part the rest of the
/// pipeline expects for that domain rather than a hardcoded one -- the ordinal
/// lane is keyed on it.
#[test]
fn a_minted_gap_uses_the_domains_declared_part() {
    let mut records = Vec::new();
    perf_record(&mut records, 1, SILENT_DOMAIN);
    let mut ordinals = BTreeMap::new();

    close_unresolved_domain_hours(
        &mut records,
        1,
        HOUR,
        &BTreeSet::from([SILENT_DOMAIN]),
        &mut ordinals,
    )
    .expect("seal closes the unresolved hour");

    assert_eq!(
        decode_gap_for(&records, SILENT_DOMAIN).part,
        domain_gap_part(SILENT_DOMAIN)
    );
    assert_eq!(domain_gap_part(SILENT_DOMAIN), DomainGapPart::Both);
}
