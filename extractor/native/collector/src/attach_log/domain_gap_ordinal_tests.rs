use std::collections::{BTreeMap, BTreeSet};

use compact_contract::domain_gap::DomainGapRecordV1;

use super::*;
use crate::read_only_scheduler::{DomainGapRange, GapKind};

const DOMAIN_ID: u16 = 10;
const FIRST_HOUR: i32 = 60_832_384;

fn backpressure_range(first_hour_raw: i32, last_hour_raw: i32) -> DomainGapRange {
    let represented = u64::try_from(i64::from(last_hour_raw) - i64::from(first_hour_raw) + 1)
        .expect("test range width");
    DomainGapRange {
        domain_id: DOMAIN_ID,
        epoch: 1,
        first_hour_raw,
        last_hour_raw,
        kind: GapKind::Backpressure,
        force_next_baseline: false,
        tick_before: 7,
        tick_after: 7,
        queue_capacity: 8,
        queue_depth: 8,
        queue_dropped: represented,
    }
}

fn build_and_decode(
    range: DomainGapRange,
    ordinals: &mut BTreeMap<(u16, u16), u64>,
) -> DomainGapRecordV1 {
    let frames = build_scheduler_gap_frames(&[range], &[DOMAIN_ID], 1, 0, ordinals)
        .expect("scheduler gap frame");
    assert_eq!(frames.len(), 1);
    let records = &frames[0].0;
    let header = compact_contract::parse_compact_record_header(records).expect("H4DG header");
    assert_eq!(
        header.record_type,
        compact_contract::COMPACT_RECORD_TYPE_DOMAIN_GAP
    );
    let payload_start = compact_contract::COMPACT_RECORD_HEADER_LEN;
    DomainGapRecordV1::decode(&records[payload_start..payload_start + header.payload_len])
        .expect("H4DG payload")
}

#[test]
fn coalesced_scheduler_gaps_remain_record_ordinaled_across_dropped_pending_frame() {
    let mut published = CompactPerfGateTrackers::default();

    let first = build_and_decode(
        backpressure_range(FIRST_HOUR, FIRST_HOUR + 10),
        &mut published.domain_gap_ordinals,
    );

    let pending = stage_compact_perf_gate_trackers(&mut published);
    drop(pending);

    let second = build_and_decode(
        backpressure_range(FIRST_HOUR + 11, FIRST_HOUR + 11),
        &mut published.domain_gap_ordinals,
    );
    let third = build_and_decode(
        backpressure_range(FIRST_HOUR + 12, FIRST_HOUR + 12),
        &mut published.domain_gap_ordinals,
    );

    assert_eq!(
        [
            first.outcome_ordinal,
            second.outcome_ordinal,
            third.outcome_ordinal,
        ],
        [0, 1, 2],
        "ordinals count emitted outcomes, not represented hours, and published state survives a dropped pending frame"
    );
}

#[test]
fn replacing_an_already_staged_h4dg_does_not_spend_a_second_ordinal() {
    use compact_contract::domain_gap::{DomainGapPart, DomainGapReason};

    let hour = FIRST_HOUR;
    let domain_id = COMPACT_DOMAIN_RAIL_REPLAY_STATE_LIVE;
    let key = (domain_id, DomainGapPart::State.code());
    let original = DomainGapRecordV1 {
        part: DomainGapPart::State,
        reason: DomainGapReason::ClockUnreadable,
        target_hour_raw: hour,
        range_start_hour_raw: hour,
        range_end_hour_raw: hour,
        tick_before: 0,
        tick_after: 0,
        queue_capacity: 0,
        queue_depth: 0,
        queue_dropped: 0,
        outcome_ordinal: 0,
    }
    .encode()
    .expect("canonical staged D81 gap");
    let mut records = Vec::new();
    append_compact_record(
        &mut records,
        1,
        hour,
        domain_id,
        compact_contract::COMPACT_RECORD_TYPE_DOMAIN_GAP,
        0,
        &original,
    );
    let mut ordinals = BTreeMap::from([(key, 1)]);
    let torn = DomainClockBracket {
        tag: domain_id,
        opening: Some(clock_obs(hour, 1)),
        closing: Some(clock_obs(hour + 1, 2)),
        worker_panicked: false,
    };

    let (next_sequence, rejected) = replace_rejected_domain_records_with_gaps(
        &mut records,
        1,
        hour,
        &[torn],
        &mut ordinals,
        HourLabelMismatchPolicy::Refuse,
    )
    .expect("staged H4DG replacement");
    assert_eq!(next_sequence, 2);
    assert_eq!(rejected, BTreeSet::from([domain_id]));
    let header = compact_contract::parse_compact_record_header(&records).unwrap();
    let replacement = DomainGapRecordV1::decode(
        &records[compact_contract::COMPACT_RECORD_HEADER_LEN..header.record_len()],
    )
    .unwrap();
    assert_eq!(replacement.reason, DomainGapReason::Torn);
    assert_eq!(replacement.outcome_ordinal, 0);
    assert_eq!(ordinals.get(&key), Some(&1));

    let next_range = DomainGapRange {
        domain_id,
        epoch: 1,
        first_hour_raw: hour + 1,
        last_hour_raw: hour + 1,
        kind: GapKind::Backpressure,
        force_next_baseline: false,
        tick_before: 7,
        tick_after: 7,
        queue_capacity: 1,
        queue_depth: 1,
        queue_dropped: 1,
    };
    let next = build_scheduler_gap_frames(&[next_range], &[domain_id], 2, 0, &mut ordinals)
        .expect("next scheduler gap");
    let next_header = compact_contract::parse_compact_record_header(&next[0].0).unwrap();
    let next_gap = DomainGapRecordV1::decode(
        &next[0].0[compact_contract::COMPACT_RECORD_HEADER_LEN..next_header.record_len()],
    )
    .unwrap();
    assert_eq!(next_gap.outcome_ordinal, 1);
    assert_eq!(ordinals.get(&key), Some(&2));
}

/// in `--release`. Observed: this test failed with
#[test]
fn the_prefix_before_the_first_rejected_record_survives() {
    let hour = FIRST_HOUR;
    let kept_before: u16 = 10;
    let torn_domain: u16 = 40;
    let kept_after: u16 = 70;

    let mut records = Vec::new();
    let mut sequence = 1u64;
    for (domain, record_type) in [
        (kept_before, COMPACT_RECORD_TYPE_DOMAIN_PERF),
        (kept_before, COMPACT_RECORD_TYPE_DOMAIN_DATA),
        (torn_domain, COMPACT_RECORD_TYPE_DOMAIN_DATA),
        (kept_after, COMPACT_RECORD_TYPE_DOMAIN_PERF),
        (kept_after, COMPACT_RECORD_TYPE_DOMAIN_DATA),
    ] {
        append_compact_record(
            &mut records,
            sequence,
            hour,
            domain,
            record_type,
            0,
            &[0xA5u8; 96],
        );
        sequence += 1;
    }

    let brackets = [
        DomainClockBracket {
            tag: kept_before,
            opening: Some(clock_obs(hour, 1)),
            closing: Some(clock_obs(hour, 2)),
            worker_panicked: false,
        },
        DomainClockBracket {
            tag: torn_domain,
            opening: Some(clock_obs(hour, 1)),
            closing: Some(clock_obs(hour + 1, 2)),
            worker_panicked: false,
        },
        DomainClockBracket {
            tag: kept_after,
            opening: Some(clock_obs(hour, 1)),
            closing: Some(clock_obs(hour, 2)),
            worker_panicked: false,
        },
    ];

    let mut ordinals = BTreeMap::new();
    let (_next_sequence, rejected) = replace_rejected_domain_records_with_gaps(
        &mut records,
        1,
        hour,
        &brackets,
        &mut ordinals,
        HOUR_LABEL_MISMATCH_POLICY,
    )
    .expect("the torn domain is replaced with a gap");
    assert_eq!(rejected, BTreeSet::from([torn_domain]));

    let mut surviving_domains = Vec::new();
    let mut offset = 0usize;
    while offset < records.len() {
        let header = compact_contract::parse_compact_record_header(&records[offset..]).unwrap();
        if header.domain_id != torn_domain {
            surviving_domains.push(header.domain_id);
        }
        offset += header.record_len();
    }
    assert_eq!(
        surviving_domains,
        vec![kept_before, kept_before, kept_after, kept_after],
        "the records ahead of the first rejected one are the back-fill's job; \
         losing them publishes a frame missing domains that read cleanly"
    );
}

#[test]
fn a_frame_that_rejects_nothing_is_returned_unchanged() {
    let hour = FIRST_HOUR;
    let domain: u16 = 10;
    let mut records = Vec::new();
    for (index, record_type) in [
        COMPACT_RECORD_TYPE_DOMAIN_PERF,
        COMPACT_RECORD_TYPE_DOMAIN_DATA,
    ]
    .into_iter()
    .enumerate()
    {
        append_compact_record(
            &mut records,
            1 + index as u64,
            hour,
            domain,
            record_type,
            0,
            &[0x3Cu8; 128],
        );
    }
    let before = records.clone();
    let brackets = [DomainClockBracket {
        tag: domain,
        opening: Some(clock_obs(hour, 1)),
        closing: Some(clock_obs(hour, 2)),
        worker_panicked: false,
    }];
    let mut ordinals = BTreeMap::new();
    let (_next_sequence, rejected) = replace_rejected_domain_records_with_gaps(
        &mut records,
        1,
        hour,
        &brackets,
        &mut ordinals,
        HOUR_LABEL_MISMATCH_POLICY,
    )
    .expect("a clean frame");
    assert!(rejected.is_empty());
    assert_eq!(records, before, "a clean frame must be returned unchanged");
}

#[test]
fn a_coalesced_backpressure_range_emits_one_record_per_domain_and_names_its_hours() {
    const ROSTER: [u16; 4] = [10, 11, 12, 13];
    const SPAN_HOURS: i32 = 11;
    let mut ordinals: BTreeMap<(u16, u16), u64> = BTreeMap::new();

    let ranges: Vec<DomainGapRange> = ROSTER
        .iter()
        .map(|domain_id| DomainGapRange {
            domain_id: *domain_id,
            ..backpressure_range(FIRST_HOUR, FIRST_HOUR + SPAN_HOURS - 1)
        })
        .collect();

    let frames = build_scheduler_gap_frames(&ranges, &ROSTER, 1, 0, &mut ordinals)
        .expect("one coalesced range over the full roster");
    assert_eq!(
        frames.len(),
        1,
        "one coalesced range is ONE frame, however many hours it covers"
    );

    let bytes = &frames[0].0;
    let mut offset = 0usize;
    let mut gap_records = 0u64;
    let mut covered_domain_hours = 0u64;
    let mut frame_ends = 0u64;
    let mut domains_named: BTreeSet<u16> = BTreeSet::new();
    while offset < bytes.len() {
        let header =
            compact_contract::parse_compact_record_header(&bytes[offset..]).expect("record header");
        let payload_start = offset + compact_contract::COMPACT_RECORD_HEADER_LEN;
        let payload = &bytes[payload_start..payload_start + header.payload_len];
        if header.record_type == compact_contract::COMPACT_RECORD_TYPE_DOMAIN_GAP {
            let gap = DomainGapRecordV1::decode(payload).expect("H4DG payload");
            gap_records += 1;
            domains_named.insert(header.domain_id);
            let hours = i64::from(gap.range_end_hour_raw) - i64::from(gap.range_start_hour_raw) + 1;
            assert_eq!(
                hours,
                i64::from(SPAN_HOURS),
                "each record must carry the WHOLE range it stands for"
            );
            assert_eq!(
                gap.queue_dropped, SPAN_HOURS as u64,
                "queue_dropped is the record's independent witness of that hour count"
            );
            covered_domain_hours += hours as u64;
        }
        if header.record_type == compact_contract::COMPACT_RECORD_TYPE_FRAME_END {
            frame_ends += 1;
        }
        offset = payload_start + header.payload_len;
    }

    assert_eq!(
        gap_records,
        ROSTER.len() as u64,
        "one H4DG per domain per range -- NOT one per domain-hour"
    );
    assert_eq!(
        domains_named,
        ROSTER.into_iter().collect::<BTreeSet<_>>(),
        "the range must exhaust the scheduled roster"
    );
    assert_eq!(frame_ends, 1, "the range is closed by one FRAME_END");
    assert_eq!(
        covered_domain_hours,
        ROSTER.len() as u64 * SPAN_HOURS as u64,
        "summing each record's own span gives the domain-hour count the \
         scheduler's counter holds"
    );
    assert!(
        covered_domain_hours > gap_records,
        "a coalesced range makes records ({gap_records}) and domain-hours \
         ({covered_domain_hours}) different quantities"
    );
}
