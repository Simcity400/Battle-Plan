use std::collections::{BTreeMap, BTreeSet};

use compact_contract::domain_gap::{DomainGapPart, DomainGapReason, DomainGapRecordV1};

use super::*;

const HOUR: i32 = 60_832_384;
const SILENT_DOMAIN: u16 = COMPACT_DOMAIN_POV_SPOTTED_UNITS_LIVE;
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
    assert_eq!(closed.newly_emitted, 0);
}

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
    assert_eq!(
        decode_gap_for(&records, SILENT_DOMAIN).reason,
        DomainGapReason::ReaderSilent
    );
    assert_eq!(
        wire_shape(&records)[..2],
        [
            (RESOLVED_DOMAIN, COMPACT_RECORD_TYPE_DOMAIN_PERF),
            (RESOLVED_DOMAIN, COMPACT_RECORD_TYPE_DOMAIN_DATA),
        ]
    );
}

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

#[test]
fn the_seal_is_wired_to_close_unresolved_hours_before_it_seals_a_read_frame() {
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

    let call_args = &code[call..call + 400];
    assert!(
        call_args.contains("&compact_scheduled_emitted_domain_ids(control)"),
        "the pass must be handed the expanded scheduled roster, or a domain that emitted \
         nothing at all is invisible to it too:\n{call_args}"
    );
    let frame_end = code
        .find("\n    append_compact_frame_end_record(")
        .expect("the seal appends FRAME_END");
    assert!(
        call < frame_end,
        "the pass adjusts the emitted-domain count that FRAME_END seals, so it must run first"
    );

    assert!(
        code[call..frame_end]
            .contains("emitted_domains = emitted_domains.saturating_add(closed.newly_emitted);"),
        "the pass's emission count must be assigned back to `emitted_domains` between the \
         call and the FRAME_END that seals it"
    );
}

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
