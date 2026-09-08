use super::*;
use compact_contract::domain_gap::{DomainGapReason, DomainGapRecordV1};

const HOUR: i32 = 60_832_384;
const START: u64 = 53;
const REJECTED: u16 = 40;
const KEPT: u16 = 70;
const SILENT: u16 = COMPACT_DOMAIN_POV_SPOTTED_UNITS_LIVE;

fn bracket(domain: u16, opening: i32, closing: i32) -> DomainClockBracket {
    DomainClockBracket {
        tag: domain,
        opening: Some(clock_obs(opening, 1)),
        closing: Some(clock_obs(closing, 2)),
        worker_panicked: false,
    }
}

fn append(out: &mut Vec<u8>, sequence: u64, domain: u16, kind: u16, payload: &[u8]) {
    append_compact_record(out, sequence, HOUR, domain, kind, 7, payload);
}

fn gap(domain: u16, reason: DomainGapReason, ordinal: u64) -> Vec<u8> {
    let (tick_before, tick_after) = if reason == DomainGapReason::Torn {
        (HOUR as u64, (HOUR + 1) as u64)
    } else {
        (0, 0)
    };
    DomainGapRecordV1 {
        part: domain_gap_part(domain),
        reason,
        target_hour_raw: HOUR,
        range_start_hour_raw: HOUR,
        range_end_hour_raw: HOUR,
        tick_before,
        tick_after,
        queue_capacity: 0,
        queue_depth: 0,
        queue_dropped: 0,
        outcome_ordinal: ordinal,
    }
    .encode()
    .expect("test gap")
    .to_vec()
}

fn append_expected_gap(
    out: &mut Vec<u8>,
    sequence: u64,
    domain: u16,
    reason: DomainGapReason,
    ordinal: u64,
) {
    append_compact_record(
        out,
        sequence,
        HOUR,
        domain,
        compact_contract::COMPACT_RECORD_TYPE_DOMAIN_GAP,
        0,
        &gap(domain, reason, ordinal),
    );
}

#[test]
fn rejected_record_positions_preserve_exact_wire_and_existing_allocation() {
    let lengths = [0usize, 1, 7, 31, 257, 4097];
    for mask in 0..1u32 << lengths.len() {
        let mut records = Vec::with_capacity(16 * 1024);
        let mut expected = Vec::new();
        let mut next = START;
        for (index, length) in lengths.into_iter().enumerate() {
            let domain = if mask & (1 << index) == 0 {
                KEPT
            } else {
                REJECTED
            };
            let payload = vec![index as u8 + 1; length];
            let kind = if index % 2 == 0 {
                COMPACT_RECORD_TYPE_DOMAIN_DATA
            } else {
                COMPACT_RECORD_TYPE_DOMAIN_PERF
            };
            append(&mut records, START + index as u64, domain, kind, &payload);
            if domain == KEPT {
                append(&mut expected, next, domain, kind, &payload);
                next += 1;
            }
        }
        for (index, domain, kind) in [
            (6, RC_PROBE_DOMAIN_ID, COMPACT_RECORD_TYPE_DOMAIN_PERF),
            (7, REJECTED, COMPACT_RECORD_TYPE_FRAME_END),
        ] {
            append(&mut records, START + index, domain, kind, &[0xAD; 19]);
            append(&mut expected, next, domain, kind, &[0xAD; 19]);
            next += 1;
        }
        if mask != 0 {
            append_expected_gap(&mut expected, next, REJECTED, DomainGapReason::Torn, 0);
            next += 1;
        }
        let address = records.as_ptr();
        let capacity = records.capacity();
        let mut ordinals = BTreeMap::new();
        let (sequence, rejected) = replace_rejected_domain_records_with_root_dependencies(
            &mut records,
            START,
            HOUR,
            &[bracket(KEPT, HOUR, HOUR), bracket(REJECTED, HOUR, HOUR + 1)],
            &mut ordinals,
            HourLabelMismatchPolicy::Refuse,
            &[],
        )
        .expect("rejection compacts validated records");
        assert_eq!(records, expected, "rejection mask {mask}");
        assert_eq!(sequence, next, "rejection mask {mask}");
        assert_eq!(records.as_ptr(), address, "rejection mask {mask}");
        assert_eq!(records.capacity(), capacity, "rejection mask {mask}");
        if mask == 0 {
            assert!(rejected.is_empty());
            assert!(ordinals.is_empty());
        } else {
            assert_eq!(rejected, BTreeSet::from([REJECTED]));
            assert_eq!(
                ordinals,
                BTreeMap::from([((REJECTED, domain_gap_part(REJECTED).code()), 1)])
            );
        }
    }
}

#[test]
fn silent_perf_positions_preserve_exact_wire_allocation_counts_and_ordinals() {
    let absent = 93;
    let resolved = COMPACT_DOMAIN_POV_PROVINCE_VISIBILITY_LIVE;
    let lengths = [1usize, 33, 257, 4097];
    for mask in 0..1u32 << lengths.len() {
        let mut records = Vec::with_capacity(16 * 1024);
        let mut expected = Vec::new();
        let mut next = START;
        for (index, length) in lengths.into_iter().enumerate() {
            let silent = mask & (1 << index) != 0;
            let (domain, kind) = if silent {
                (SILENT, COMPACT_RECORD_TYPE_DOMAIN_PERF)
            } else {
                (resolved, COMPACT_RECORD_TYPE_DOMAIN_DATA)
            };
            let payload = vec![index as u8 + 1; length];
            append(&mut records, START + index as u64, domain, kind, &payload);
            if !silent {
                append(&mut expected, next, domain, kind, &payload);
                next += 1;
            }
        }
        for (index, domain, kind) in [
            (4, RC_PROBE_DOMAIN_ID, COMPACT_RECORD_TYPE_DOMAIN_PERF),
            (5, SILENT, COMPACT_RECORD_TYPE_FRAME_END),
        ] {
            append(&mut records, START + index, domain, kind, &[0xBA; 19]);
            append(&mut expected, next, domain, kind, &[0xBA; 19]);
            next += 1;
        }
        let mut ordinals = BTreeMap::new();
        for (domain, ordinal) in [(absent, 4), (SILENT, 11)] {
            ordinals.insert((domain, domain_gap_part(domain).code()), ordinal);
            append_expected_gap(
                &mut expected,
                next,
                domain,
                DomainGapReason::ReaderSilent,
                ordinal,
            );
            next += 1;
        }
        let address = records.as_ptr();
        let capacity = records.capacity();
        let closed = close_unresolved_domain_hours(
            &mut records,
            START,
            HOUR,
            &BTreeSet::from([absent, SILENT]),
            &mut ordinals,
        )
        .expect("silent records are superseded");
        assert_eq!(records, expected, "silent mask {mask}");
        assert_eq!(closed.sequence, next, "silent mask {mask}");
        assert_eq!(closed.silenced, BTreeSet::from([absent, SILENT]));
        assert_eq!(closed.newly_emitted, if mask == 0 { 2 } else { 1 });
        assert_eq!(records.as_ptr(), address, "silent mask {mask}");
        assert_eq!(records.capacity(), capacity, "silent mask {mask}");
        assert_eq!(
            ordinals.get(&(absent, domain_gap_part(absent).code())),
            Some(&5)
        );
        assert_eq!(
            ordinals.get(&(SILENT, domain_gap_part(SILENT).code())),
            Some(&12)
        );
    }
}

#[test]
fn late_framing_errors_leave_both_filter_inputs_and_ordinals_unchanged() {
    for corruption in 0..3 {
        for rejection_filter in [false, true] {
            let domain = if rejection_filter { REJECTED } else { SILENT };
            let mut records = Vec::with_capacity(16 * 1024);
            append(
                &mut records,
                START,
                domain,
                COMPACT_RECORD_TYPE_DOMAIN_PERF,
                &[1; 8],
            );
            append(
                &mut records,
                START + 1,
                KEPT,
                COMPACT_RECORD_TYPE_DOMAIN_DATA,
                &[2; 4097],
            );
            let last = records.len();
            append(
                &mut records,
                START + 2,
                KEPT,
                COMPACT_RECORD_TYPE_DOMAIN_DATA,
                &[3; 8],
            );
            match corruption {
                0 => records[last] = 0,
                1 => records.truncate(last + compact_contract::COMPACT_RECORD_HEADER_LEN - 1),
                _ => records.truncate(records.len() - 1),
            }
            let original = records.clone();
            let address = records.as_ptr();
            let capacity = records.capacity();
            let mut ordinals = BTreeMap::new();
            let error = if rejection_filter {
                replace_rejected_domain_records_with_root_dependencies(
                    &mut records,
                    START,
                    HOUR,
                    &[bracket(KEPT, HOUR, HOUR), bracket(REJECTED, HOUR, HOUR + 1)],
                    &mut ordinals,
                    HourLabelMismatchPolicy::Refuse,
                    &[],
                )
                .err()
            } else {
                close_unresolved_domain_hours(
                    &mut records,
                    START,
                    HOUR,
                    &BTreeSet::from([SILENT]),
                    &mut ordinals,
                )
                .err()
            }
            .expect("late malformed record is refused");
            assert!(error.contains(if corruption == 0 {
                "invalid record magic"
            } else {
                "truncated"
            }));
            assert_eq!(records, original);
            assert_eq!(records.as_ptr(), address);
            assert_eq!(records.capacity(), capacity);
            assert!(ordinals.is_empty());
        }
    }
}

#[test]
fn late_rejected_gap_decode_failure_preserves_the_original_frame() {
    let mut records = Vec::new();
    append(
        &mut records,
        START,
        REJECTED,
        COMPACT_RECORD_TYPE_DOMAIN_DATA,
        &[1; 8],
    );
    append(
        &mut records,
        START + 1,
        KEPT,
        COMPACT_RECORD_TYPE_DOMAIN_DATA,
        &[2; 4097],
    );
    append(
        &mut records,
        START + 2,
        REJECTED,
        compact_contract::COMPACT_RECORD_TYPE_DOMAIN_GAP,
        &[3; 8],
    );
    let original = records.clone();
    let mut ordinals = BTreeMap::new();
    let error = replace_rejected_domain_records_with_root_dependencies(
        &mut records,
        START,
        HOUR,
        &[bracket(KEPT, HOUR, HOUR), bracket(REJECTED, HOUR, HOUR + 1)],
        &mut ordinals,
        HourLabelMismatchPolicy::Refuse,
        &[],
    )
    .unwrap_err();
    assert!(error.contains("decode rejected DOMAIN_GAP"));
    assert_eq!(records, original);
    assert!(ordinals.is_empty());
}

#[test]
fn withdrawn_rejection_is_refused_before_any_frame_bytes_move() {
    let mut records = Vec::new();
    append(
        &mut records,
        START,
        REJECTED,
        COMPACT_RECORD_TYPE_DOMAIN_DATA,
        &[1; 8],
    );
    append(
        &mut records,
        START + 1,
        KEPT,
        COMPACT_RECORD_TYPE_DOMAIN_DATA,
        &[2; 4097],
    );
    append_expected_gap(
        &mut records,
        START + 2,
        REJECTED,
        DomainGapReason::SourceUnavailable,
        0,
    );
    let original = records.clone();
    let mut ordinals = BTreeMap::from([((REJECTED, domain_gap_part(REJECTED).code()), 1)]);
    let original_ordinals = ordinals.clone();
    let error = replace_rejected_domain_records_with_root_dependencies(
        &mut records,
        START,
        HOUR,
        &[
            bracket(KEPT, HOUR, HOUR),
            bracket(REJECTED, HOUR + 1, HOUR + 1),
        ],
        &mut ordinals,
        HourLabelMismatchPolicy::Refuse,
        &[],
    )
    .unwrap_err();
    assert_eq!(error, "domain 40 had records dropped from the frame but its rejection was withdrawn before the replacement was minted");
    assert_eq!(records, original);
    assert_eq!(ordinals, original_ordinals);
}
