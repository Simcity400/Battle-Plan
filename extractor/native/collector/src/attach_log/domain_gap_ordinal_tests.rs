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

/// Reproduces the live t4 shape: an eleven-hour coalesced range followed by
/// two one-hour ranges, with an unpublished pending DATA frame between the
/// first and second H4DG publications.
///
/// Mutation proof: changing the scheduler increment back to the represented
/// hour count yields `0, 11, 12`; changing pending-frame staging back to a
/// plain `mem::take` loses the first committed ordinal and yields `0, 0, 1`.
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

/// Reproduces the five remaining t8 ordinal holes. D81 can build its own H4DG
/// before the common clock-bracket reconciliation discovers that the same
/// domain tore. Replacing that already-staged H4DG is one outcome rewrite, not
/// a second outcome, so it must keep the ordinal already present in the bytes.
///
/// Mutation proof: selecting the current map value unconditionally in
/// `replace_rejected_domain_records_with_gaps` changes the replacement from 0
/// to 1 and the following scheduler outcome from 1 to 2.
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

/// The retained buffer is now built LAZILY: it does not exist until the walk
/// reaches the first record that is actually dropped, and at that moment it is
/// back-filled with everything the walk already passed. Before, it was
/// allocated at the frame's size up front and every retained record was copied
/// into it as the walk went — a 4.59 MB allocation and copy paid on every
/// frame and kept only on the frames that rejected something.
///
/// The back-fill is the load-bearing half. Omitting it does not make the frame
/// slower, it silently DELETES every record physically before the first
/// rejected one — here, the surviving domain's PERF and DATA pair — and the
/// published frame would be missing domains that read perfectly well.
///
/// MUTATION RUN: the back-fill `buffer.extend_from_slice(&records[..offset]);`
/// was deleted, leaving `Vec::with_capacity(records.len())`, and the suite run
/// in `--release`. Observed: this test failed with
/// `left: [70, 70]  right: [10, 10, 70, 70]` — the two records ahead of the
/// rejected one gone from the output while the two behind it survived.
/// Restored.
#[test]
fn the_prefix_before_the_first_rejected_record_survives() {
    let hour = FIRST_HOUR;
    let kept_before: u16 = 10;
    let torn_domain: u16 = 40;
    let kept_after: u16 = 70;

    // Physical order: two records for a domain that reads cleanly, then the
    // domain that tore, then two more clean records. The first two are the
    // prefix the back-fill is responsible for.
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

    // Every domain present needs a bracket or it is rejected as
    // clock-unreadable; the torn one's brackets straddle the hour.
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
    let (_next_sequence, rejected) =
        replace_rejected_domain_records_with_gaps(
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

/// A frame with nothing to reject is returned BYTE-IDENTICAL and untouched.
///
/// This is the path the lazy buffer exists for, and the property it must not
/// break: the early return hands the caller the original `records`, so the
/// caller's frame is exactly what the build produced.
///
/// MUTATION RUN: `*records = retained.clone().unwrap_or_default();` was
/// inserted above the `present_rejected.is_empty()` early return, so a clean
/// frame is overwritten by a buffer that was never built. Observed: this test
/// failed with `a clean frame must be returned unchanged, left: []`, and eight
/// unrelated state-fixture tests failed with it — the frame had been emptied.
/// Restored.
///
/// What this test does NOT pin, stated plainly: the SAVING. Building `retained`
/// eagerly and dropping it unused is byte-identical here and would still pass.
/// The saving is a measurement — `buildtail_previous_frame` in the next
/// capture — and not an assertion this or any unit test can make.
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
    let (_next_sequence, rejected) =
        replace_rejected_domain_records_with_gaps(
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
