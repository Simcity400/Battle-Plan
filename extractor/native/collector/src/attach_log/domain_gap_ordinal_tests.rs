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
        opening: Some((hour, 1)),
        closing: Some((hour + 1, 2)),
        worker_panicked: false,
    };

    let (next_sequence, rejected) = replace_rejected_domain_records_with_gaps(
        &mut records,
        1,
        hour,
        &[torn],
        &mut ordinals,
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
