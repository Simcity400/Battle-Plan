use std::collections::BTreeMap;

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
