use std::collections::{BTreeMap, BTreeSet};

use compact_contract::domain_gap::{DomainGapPart, DomainGapReason, DomainGapRecordV1};

use super::*;

const LABEL_HOUR: i32 = 60_832_037;
const DOMAIN_ID: u16 = COMPACT_DOMAIN_DIVISIONS;

fn bracket(tag: u16, opening: i32, closing: i32) -> DomainClockBracket {
    DomainClockBracket {
        tag,
        opening: Some(clock_obs(opening, 1)),
        closing: Some(clock_obs(closing, 2)),
        worker_panicked: false,
    }
}

#[test]
fn a_bracket_is_graded_against_the_frames_label_not_only_against_itself() {
    assert_eq!(
        bracket_hour_verdict(&bracket(DOMAIN_ID, LABEL_HOUR, LABEL_HOUR), LABEL_HOUR),
        BracketHourVerdict::InFrameHour,
        "a bracket that opened and closed in the frame's own hour publishes"
    );
    assert_eq!(
        bracket_hour_verdict(&bracket(DOMAIN_ID, LABEL_HOUR, LABEL_HOUR + 1), LABEL_HOUR),
        BracketHourVerdict::Straddled,
        "the hour changed mid-read, which the tear check already saw"
    );
    assert_eq!(
        bracket_hour_verdict(
            &bracket(DOMAIN_ID, LABEL_HOUR + 1, LABEL_HOUR + 1),
            LABEL_HOUR
        ),
        BracketHourVerdict::LabelMismatch,
        "a bracket that ran wholly in another hour is a label mismatch"
    );
    assert_eq!(
        bracket_hour_verdict(
            &DomainClockBracket {
                tag: DOMAIN_ID,
                opening: Some(clock_obs(LABEL_HOUR, 1)),
                closing: None,
                worker_panicked: false,
            },
            LABEL_HOUR
        ),
        BracketHourVerdict::ClockUnreadable,
        "half a bracket proves nothing; 'could not observe' is not 'observed \
         the right hour'"
    );
    assert_eq!(
        bracket_hour_verdict(
            &DomainClockBracket {
                worker_panicked: true,
                ..bracket(DOMAIN_ID, LABEL_HOUR + 1, LABEL_HOUR + 1)
            },
            LABEL_HOUR
        ),
        BracketHourVerdict::WorkerPanic,
        "the panic outranks the hour: nothing this bracket says is trustworthy"
    );
}

#[test]
fn a_wrong_hour_bracket_replaces_its_domains_data_with_a_typed_gap() {
    let mut records = Vec::new();
    append_compact_record(
        &mut records,
        1,
        LABEL_HOUR,
        DOMAIN_ID,
        compact_contract::COMPACT_RECORD_TYPE_DOMAIN_DATA,
        0,
        &[7u8; 8],
    );
    let mut ordinals = BTreeMap::new();

    let (_next, rejected) = replace_rejected_domain_records_with_gaps(
        &mut records,
        1,
        LABEL_HOUR,
        &[bracket(DOMAIN_ID, LABEL_HOUR + 1, LABEL_HOUR + 1)],
        &mut ordinals,
        HourLabelMismatchPolicy::Refuse,
    )
    .expect("a wrong-hour refusal must be encodable");

    assert_eq!(rejected, BTreeSet::from([DOMAIN_ID]));
    let header = compact_contract::parse_compact_record_header(&records).unwrap();
    assert_eq!(
        header.record_type,
        compact_contract::COMPACT_RECORD_TYPE_DOMAIN_GAP,
        "the wrong-hour domain's DATA record must not survive"
    );
    let gap = DomainGapRecordV1::decode(
        &records[compact_contract::COMPACT_RECORD_HEADER_LEN..header.record_len()],
    )
    .expect("the replacement must satisfy the H4DG grammar");
    assert_eq!(gap.reason, DomainGapReason::HourLabelMismatch);
    assert_eq!(
        gap.target_hour_raw, LABEL_HOUR,
        "the lost hour is the one the frame was publishing under"
    );
    assert_eq!(
        (gap.tick_before, gap.tick_after),
        (
            u64::try_from(LABEL_HOUR + 1).unwrap(),
            u64::try_from(LABEL_HOUR + 1).unwrap()
        ),
        "the witnesses are the hour the reader ACTUALLY observed, unmodified; \
         nothing here is fabricated"
    );

    let mut clean = Vec::new();
    append_compact_record(
        &mut clean,
        1,
        LABEL_HOUR,
        DOMAIN_ID,
        compact_contract::COMPACT_RECORD_TYPE_DOMAIN_DATA,
        0,
        &[7u8; 8],
    );
    let (_next, rejected) = replace_rejected_domain_records_with_gaps(
        &mut clean,
        1,
        LABEL_HOUR,
        &[bracket(DOMAIN_ID, LABEL_HOUR, LABEL_HOUR)],
        &mut BTreeMap::new(),
        HourLabelMismatchPolicy::Refuse,
    )
    .expect("an in-hour frame replaces nothing");
    assert!(rejected.is_empty());
    assert_eq!(
        compact_contract::parse_compact_record_header(&clean)
            .unwrap()
            .record_type,
        compact_contract::COMPACT_RECORD_TYPE_DOMAIN_DATA
    );
}

#[test]
fn the_policy_switch_changes_the_refusal_and_not_the_measurement() {
    let brackets = [
        bracket(DOMAIN_ID, LABEL_HOUR + 1, LABEL_HOUR + 1),
        bracket(COMPACT_DOMAIN_LAND_COMBATS, LABEL_HOUR, LABEL_HOUR),
    ];

    assert_eq!(
        hour_label_mismatched_bracket_count(&brackets, LABEL_HOUR, &BTreeSet::new()),
        (1, 2),
        "the instrument counts the mismatch under BOTH policies, over the \
         DOMAINS each bracket covered"
    );
    assert!(
        rejected_bracket_domains(&brackets, LABEL_HOUR, HourLabelMismatchPolicy::CountOnly)
            .is_empty(),
        "CountOnly must publish the frame unchanged"
    );
    assert_eq!(
        rejected_bracket_domains(&brackets, LABEL_HOUR, HourLabelMismatchPolicy::Refuse)
            .get(&DOMAIN_ID)
            .map(|outcome| outcome.0),
        Some(DomainGapReason::HourLabelMismatch),
        "Refuse must reject exactly the mismatched domain"
    );
}

#[test]
fn the_instrument_prices_a_mismatch_in_domain_hours_not_in_brackets() {
    let brackets = [
        bracket(
            COMPACT_DOMAIN_NAVAL_COMBAT_LIFECYCLE_LIVE,
            LABEL_HOUR + 1,
            LABEL_HOUR + 1,
        ),
        bracket(RC_PROBE_DOMAIN_ID, LABEL_HOUR + 1, LABEL_HOUR + 1),
        bracket(DOMAIN_ID, LABEL_HOUR, LABEL_HOUR),
    ];
    assert_eq!(
        hour_label_mismatched_bracket_count(&brackets, LABEL_HOUR, &BTreeSet::new()),
        (2, 3),
        "one d49 bracket costs two domain-hours, not one; and the probe \
         domain costs none"
    );
    assert_eq!(
        rejected_bracket_domains(&brackets, LABEL_HOUR, HourLabelMismatchPolicy::Refuse).len(),
        2,
        "the instrument must equal what Refuse would actually reject"
    );
}

#[test]
fn a_straddle_is_torn_under_either_policy_even_when_it_missed_the_label_too() {
    let late_straddle = [bracket(DOMAIN_ID, LABEL_HOUR + 1, LABEL_HOUR + 2)];
    for policy in [
        HourLabelMismatchPolicy::Refuse,
        HourLabelMismatchPolicy::CountOnly,
    ] {
        assert_eq!(
            rejected_bracket_domains(&late_straddle, LABEL_HOUR, policy)
                .get(&DOMAIN_ID)
                .map(|outcome| outcome.0),
            Some(DomainGapReason::Torn),
            "a straddle is Torn regardless of policy"
        );
    }
    assert_eq!(
        hour_label_mismatched_bracket_count(&late_straddle, LABEL_HOUR, &BTreeSet::new()),
        (0, 1),
        "and it is not double-counted as a mismatch"
    );
}

#[test]
fn a_no_clock_sentinel_label_is_not_something_a_bracket_can_mismatch() {
    const NO_CLOCK_SENTINEL: i32 = 0;
    let brackets = [bracket(DOMAIN_ID, LABEL_HOUR, LABEL_HOUR)];

    assert_eq!(
        hour_label_mismatched_bracket_count(&brackets, NO_CLOCK_SENTINEL, &BTreeSet::new()),
        (0, 1),
        "a sentinel label must not refuse anything: it was never an observation"
    );

    let mut records = Vec::new();
    append_compact_record(
        &mut records,
        1,
        NO_CLOCK_SENTINEL,
        DOMAIN_ID,
        compact_contract::COMPACT_RECORD_TYPE_DOMAIN_DATA,
        0,
        &[7u8; 8],
    );
    replace_rejected_domain_records_with_gaps(
        &mut records,
        1,
        NO_CLOCK_SENTINEL,
        &brackets,
        &mut BTreeMap::new(),
        HourLabelMismatchPolicy::Refuse,
    )
    .expect("a NO_CLOCK frame must still publish, not fail to encode a gap");
}

#[test]
fn the_hour_label_mismatch_grammar_refuses_a_witness_that_does_not_prove_it() {
    let record = |tick_before: u64, tick_after: u64| DomainGapRecordV1 {
        part: DomainGapPart::State,
        reason: DomainGapReason::HourLabelMismatch,
        target_hour_raw: LABEL_HOUR,
        range_start_hour_raw: LABEL_HOUR,
        range_end_hour_raw: LABEL_HOUR,
        tick_before,
        tick_after,
        queue_capacity: 0,
        queue_depth: 0,
        queue_dropped: 0,
        outcome_ordinal: 0,
    };
    let observed = u64::try_from(LABEL_HOUR + 1).unwrap();
    let label = u64::try_from(LABEL_HOUR).unwrap();

    record(observed, observed)
        .encode()
        .expect("equal, nonzero, not the label: the reason is proven");
    record(observed, observed + 1)
        .encode()
        .expect_err("a mismatch whose ticks DIFFER is a straddle wearing another name");
    record(label, label)
        .encode()
        .expect_err("a mismatch whose ticks ARE the label proves the opposite of its reason");
    record(0, 0)
        .encode()
        .expect_err("a mismatch whose ticks are the wire's absent-witness value proves nothing");
}

#[test]
fn a_tear_on_one_bracket_outranks_a_label_mismatch_on_another_for_the_same_domain() {
    let brackets = [
        bracket(DOMAIN_ID, LABEL_HOUR + 1, LABEL_HOUR + 1),
        bracket(DOMAIN_ID, LABEL_HOUR + 1, LABEL_HOUR + 2),
    ];

    let refused = rejected_bracket_domains(&brackets, LABEL_HOUR, HourLabelMismatchPolicy::Refuse);
    let counted =
        rejected_bracket_domains(&brackets, LABEL_HOUR, HourLabelMismatchPolicy::CountOnly);

    assert_eq!(
        refused.get(&DOMAIN_ID).map(|outcome| outcome.0),
        Some(DomainGapReason::Torn),
        "a real tear outranks a label mismatch for the same domain"
    );
    assert_eq!(
        refused, counted,
        "the policy must not change the reason for a domain that is refused          either way"
    );
    assert_eq!(
        hour_label_mismatched_bracket_count(&brackets, LABEL_HOUR, &BTreeSet::new()),
        (0, 1),
        "and a domain already lost to a tear is not counted a second time as          the mismatch's price"
    );
}

#[test]
fn an_out_of_calendar_label_suppresses_the_rule_the_same_way_the_sentinel_does() {
    const CORRUPT_LABEL: i32 = 1;
    assert!(
        !frame_label_is_a_real_hour(CORRUPT_LABEL),
        "premise: the calendar refuses this label"
    );
    let brackets = [bracket(DOMAIN_ID, LABEL_HOUR, LABEL_HOUR)];

    assert!(
        rejected_bracket_domains(&brackets, CORRUPT_LABEL, HourLabelMismatchPolicy::Refuse)
            .is_empty(),
        "a label the calendar refuses cannot be mismatched against"
    );
    assert!(
        DomainGapRecordV1 {
            part: DomainGapPart::State,
            reason: DomainGapReason::HourLabelMismatch,
            target_hour_raw: CORRUPT_LABEL,
            range_start_hour_raw: CORRUPT_LABEL,
            range_end_hour_raw: CORRUPT_LABEL,
            tick_before: u64::try_from(LABEL_HOUR).unwrap(),
            tick_after: u64::try_from(LABEL_HOUR).unwrap(),
            queue_capacity: 0,
            queue_depth: 0,
            queue_dropped: 0,
            outcome_ordinal: 0,
        }
        .encode()
        .is_err(),
        "refusing against such a label could not be expressed on the wire,          which is why the rule declines rather than tries"
    );
}

#[test]
fn a_label_mismatch_leaves_an_already_published_gap_alone() {
    let staged = |reason| {
        DomainGapRecordV1 {
            part: domain_gap_part(DOMAIN_ID),
            reason,
            target_hour_raw: LABEL_HOUR,
            range_start_hour_raw: LABEL_HOUR,
            range_end_hour_raw: LABEL_HOUR,
            tick_before: u64::try_from(LABEL_HOUR).unwrap(),
            tick_after: u64::try_from(LABEL_HOUR).unwrap(),
            queue_capacity: 0,
            queue_depth: 0,
            queue_dropped: 0,
            outcome_ordinal: 0,
        }
        .encode()
        .expect("staged gap")
    };
    let seeded = |payload: &[u8]| {
        let mut records = Vec::new();
        append_compact_record(
            &mut records,
            1,
            LABEL_HOUR,
            DOMAIN_ID,
            compact_contract::COMPACT_RECORD_TYPE_DOMAIN_GAP,
            0,
            payload,
        );
        records
    };
    let decode = |records: &[u8]| {
        let header = compact_contract::parse_compact_record_header(records).unwrap();
        DomainGapRecordV1::decode(
            &records[compact_contract::COMPACT_RECORD_HEADER_LEN..header.record_len()],
        )
        .expect("reconciled record is a valid H4DG")
        .reason
    };

    let mut records = seeded(&staged(DomainGapReason::SourceUnavailable));
    replace_rejected_domain_records_with_gaps(
        &mut records,
        1,
        LABEL_HOUR,
        &[bracket(DOMAIN_ID, LABEL_HOUR + 1, LABEL_HOUR + 1)],
        &mut BTreeMap::from([((DOMAIN_ID, domain_gap_part(DOMAIN_ID).code()), 1)]),
        HourLabelMismatchPolicy::Refuse,
    )
    .expect("reconciliation succeeds");
    assert_eq!(
        decode(&records),
        DomainGapReason::SourceUnavailable,
        "an existing gap keeps the reason it was minted with"
    );

    let mut torn = seeded(&staged(DomainGapReason::SourceUnavailable));
    replace_rejected_domain_records_with_gaps(
        &mut torn,
        1,
        LABEL_HOUR,
        &[bracket(DOMAIN_ID, LABEL_HOUR, LABEL_HOUR + 1)],
        &mut BTreeMap::from([((DOMAIN_ID, domain_gap_part(DOMAIN_ID).code()), 1)]),
        HourLabelMismatchPolicy::Refuse,
    )
    .expect("reconciliation succeeds");
    assert_eq!(
        decode(&torn),
        DomainGapReason::Torn,
        "a torn read invalidates the reader's own account of itself"
    );
}

#[test]
fn the_instrument_does_not_charge_for_a_domain_whose_gap_is_already_published() {
    let brackets = [bracket(DOMAIN_ID, LABEL_HOUR + 1, LABEL_HOUR + 1)];

    assert_eq!(
        hour_label_mismatched_bracket_count(&brackets, LABEL_HOUR, &BTreeSet::new()),
        (1, 1),
        "premise: with nothing already published, this bracket does cost a \
         domain-hour"
    );
    assert_eq!(
        hour_label_mismatched_bracket_count(&brackets, LABEL_HOUR, &BTreeSet::from([DOMAIN_ID])),
        (0, 0),
        "a domain that already published a gap costs nothing to refuse, AND is \
         not offered as a domain that could have"
    );
}

#[test]
fn the_denominator_offers_only_domains_the_numerator_could_count() {
    const OTHER_DOMAIN_ID: u16 = DOMAIN_ID + 1;
    let brackets = [
        bracket(DOMAIN_ID, LABEL_HOUR + 1, LABEL_HOUR + 1),
        bracket(OTHER_DOMAIN_ID, LABEL_HOUR + 1, LABEL_HOUR + 1),
    ];

    assert_eq!(
        hour_label_mismatched_bracket_count(&brackets, LABEL_HOUR, &BTreeSet::new()),
        (2, 2),
        "premise: with nothing already published both mismatches are charged \
         against both brackets"
    );
    assert_eq!(
        hour_label_mismatched_bracket_count(
            &brackets,
            LABEL_HOUR,
            &BTreeSet::from([OTHER_DOMAIN_ID])
        ),
        (1, 1),
        "the denominator must offer only domains the numerator could count"
    );
}

#[test]
fn the_already_gapped_set_is_read_from_the_frames_own_records() {
    let mut with_data = Vec::new();
    append_compact_record(
        &mut with_data,
        1,
        LABEL_HOUR,
        DOMAIN_ID,
        compact_contract::COMPACT_RECORD_TYPE_DOMAIN_DATA,
        0,
        &[7u8; 8],
    );
    assert_eq!(
        compact_domains_already_carrying_a_gap(&with_data),
        BTreeSet::new(),
        "only a published GAP suppresses the charge; a DATA record is exactly \
         the thing the refusal would take away"
    );

    let mut with_gap = Vec::new();
    append_compact_record(
        &mut with_gap,
        1,
        LABEL_HOUR,
        DOMAIN_ID,
        compact_contract::COMPACT_RECORD_TYPE_DOMAIN_GAP,
        0,
        &DomainGapRecordV1 {
            part: domain_gap_part(DOMAIN_ID),
            reason: DomainGapReason::SourceUnavailable,
            target_hour_raw: LABEL_HOUR,
            range_start_hour_raw: LABEL_HOUR,
            range_end_hour_raw: LABEL_HOUR,
            tick_before: u64::try_from(LABEL_HOUR).unwrap(),
            tick_after: u64::try_from(LABEL_HOUR).unwrap(),
            queue_capacity: 0,
            queue_depth: 0,
            queue_dropped: 0,
            outcome_ordinal: 0,
        }
        .encode()
        .expect("staged gap"),
    );
    assert_eq!(
        compact_domains_already_carrying_a_gap(&with_gap),
        BTreeSet::from([DOMAIN_ID])
    );

    let truncated = &with_gap[..with_gap.len() - 4];
    assert_eq!(
        compact_domains_already_carrying_a_gap(truncated),
        BTreeSet::new()
    );
}

#[test]
fn the_shipped_policy_refuses_mislabelled_hours_since_t20_priced_them() {
    assert_eq!(
        HOUR_LABEL_MISMATCH_POLICY,
        HourLabelMismatchPolicy::Refuse,
        "t20 measured the refusal at 3.35% of bracketed domain-reads (574 of \
         17,136) and the owner's hour-misattribution ruling makes keeping \
         mislabelled hours a stop condition; reverting to CountOnly re-admits \
         them and needs an owner decision, not an edit"
    );
}

fn stamped(tag: u16, opening: (i32, u64, u64), closing: (i32, u64, u64)) -> DomainClockBracket {
    DomainClockBracket {
        tag,
        opening: Some(ClockObservation {
            hour: opening.0,
            ordinal: opening.1,
            at_us: opening.2,
        }),
        closing: Some(ClockObservation {
            hour: closing.0,
            ordinal: closing.1,
            at_us: closing.2,
        }),
        worker_panicked: false,
    }
}

/// changed to `max_by_key`, suite run in `--release`. OBSERVED:
#[test]
fn the_boundary_position_is_the_first_post_label_observation() {
    let brackets = [
        stamped(10, (LABEL_HOUR, 1, 5_000), (LABEL_HOUR, 2, 20_000)),
        stamped(11, (LABEL_HOUR, 3, 21_000), (LABEL_HOUR, 4, 40_000)),
        stamped(12, (LABEL_HOUR + 1, 5, 61_000), (LABEL_HOUR + 1, 6, 80_000)),
        stamped(13, (LABEL_HOUR + 1, 7, 81_000), (LABEL_HOUR + 1, 8, 90_000)),
    ];
    assert_eq!(
        frame_boundary_position(&brackets, LABEL_HOUR),
        Some(FrameBoundaryPosition {
            at_us: 61_000,
            brackets_closed_before: 2,
        }),
        "the boundary is witnessed by the earliest read that saw a later hour, \
         and two brackets had closed by then"
    );
}

#[test]
fn no_boundary_is_absent_and_a_boundary_at_the_origin_is_zero() {
    let clean = [
        stamped(10, (LABEL_HOUR, 1, 5_000), (LABEL_HOUR, 2, 20_000)),
        stamped(11, (LABEL_HOUR, 3, 21_000), (LABEL_HOUR, 4, 40_000)),
    ];
    assert_eq!(
        frame_boundary_position(&clean, LABEL_HOUR),
        None,
        "a sweep that never saw a later hour witnessed no boundary"
    );

    let from_the_origin = [stamped(
        10,
        (LABEL_HOUR + 1, 1, 0),
        (LABEL_HOUR + 1, 2, 20_000),
    )];
    assert_eq!(
        frame_boundary_position(&from_the_origin, LABEL_HOUR),
        Some(FrameBoundaryPosition {
            at_us: 0,
            brackets_closed_before: 0,
        }),
        "a build that began in the next hour witnessed the boundary at its own \
         origin, which is a measurement and not an absence"
    );
}

#[test]
fn a_frame_that_fell_two_hours_behind_still_reports_its_boundary() {
    let brackets = [
        stamped(10, (LABEL_HOUR, 1, 5_000), (LABEL_HOUR, 2, 20_000)),
        stamped(11, (LABEL_HOUR + 2, 3, 45_000), (LABEL_HOUR + 2, 4, 60_000)),
    ];
    assert_eq!(
        frame_boundary_position(&brackets, LABEL_HOUR),
        Some(FrameBoundaryPosition {
            at_us: 45_000,
            brackets_closed_before: 1,
        })
    );
}

#[test]
fn a_label_the_calendar_refuses_has_no_boundary_position() {
    let brackets = [stamped(10, (LABEL_HOUR, 1, 5_000), (LABEL_HOUR, 2, 20_000))];
    assert!(!frame_label_is_a_real_hour(0), "0 is the NO_CLOCK sentinel");
    assert_eq!(frame_boundary_position(&brackets, 0), None);
    assert!(frame_boundary_position(&brackets, LABEL_HOUR - 1).is_some());
}

#[test]
fn a_panicked_bracket_witnesses_no_boundary_and_closes_nothing() {
    let panicked = [
        stamped(10, (LABEL_HOUR, 1, 5_000), (LABEL_HOUR, 2, 20_000)),
        DomainClockBracket {
            worker_panicked: true,
            ..stamped(11, (LABEL_HOUR + 1, 3, 30_000), (LABEL_HOUR + 1, 4, 40_000))
        },
    ];
    assert_eq!(
        frame_boundary_position(&panicked, LABEL_HOUR),
        None,
        "the only witness unwound, and the panic is already its own refusal"
    );

    let mixed = [
        DomainClockBracket {
            worker_panicked: true,
            ..stamped(10, (LABEL_HOUR, 1, 5_000), (LABEL_HOUR, 2, 20_000))
        },
        stamped(11, (LABEL_HOUR, 3, 21_000), (LABEL_HOUR, 4, 40_000)),
        stamped(12, (LABEL_HOUR + 1, 5, 61_000), (LABEL_HOUR + 1, 6, 80_000)),
    ];
    assert_eq!(
        frame_boundary_position(&mixed, LABEL_HOUR),
        Some(FrameBoundaryPosition {
            at_us: 61_000,
            brackets_closed_before: 1,
        }),
        "one trustworthy bracket had closed; the panicked one is not counted"
    );
}

#[test]
fn an_unreadable_close_is_not_counted_as_closed() {
    let brackets = [
        stamped(10, (LABEL_HOUR, 1, 5_000), (LABEL_HOUR, 2, 20_000)),
        DomainClockBracket {
            tag: 11,
            opening: Some(clock_obs(LABEL_HOUR, 3)),
            closing: None,
            worker_panicked: false,
        },
        stamped(12, (LABEL_HOUR + 1, 5, 61_000), (LABEL_HOUR + 1, 6, 80_000)),
    ];
    assert_eq!(
        frame_boundary_position(&brackets, LABEL_HOUR),
        Some(FrameBoundaryPosition {
            at_us: 61_000,
            brackets_closed_before: 1,
        })
    );
}

#[test]
fn the_ordinal_orders_the_reads_and_the_stamp_only_reports_them() {
    let brackets = [
        stamped(10, (LABEL_HOUR, 1, 5_000), (LABEL_HOUR, 2, 20_000)),
        stamped(12, (LABEL_HOUR + 1, 5, 52_000), (LABEL_HOUR + 1, 7, 80_000)),
        stamped(11, (LABEL_HOUR, 3, 21_000), (LABEL_HOUR + 1, 6, 50_000)),
    ];
    assert_eq!(
        frame_boundary_position(&brackets, LABEL_HOUR),
        Some(FrameBoundaryPosition {
            at_us: 52_000,
            brackets_closed_before: 1,
        }),
        "ordinal 5 is the first witness even though its stamp is later, and \
         only bracket 10 had closed before it"
    );
}

#[test]
fn the_boundary_position_reaches_the_probe_record_and_absence_omits_it() {
    let summary = summarize_lane_spans(&[]);
    let witnessed = [
        stamped(10, (LABEL_HOUR, 1, 5_000), (LABEL_HOUR, 2, 20_000)),
        stamped(11, (LABEL_HOUR + 1, 3, 61_000), (LABEL_HOUR + 1, 4, 80_000)),
    ];
    let mut out = Vec::new();
    append_compact_lane_wall_probe_record(
        &mut out,
        7,
        LABEL_HOUR,
        5,
        900,
        125,
        Some(0),
        FrameHourAge::default(),
        &witnessed,
        &BTreeSet::new(),
        MapTopologyShareCounts::default(),
        130,
        700,
        23_641,
        &OutsideBuildSpans::default(),
        92_000,
        &FrameTailSegments::default(),
        &summary,
        FrameCpuSplit::default(),
    );
    let units = compact_perf_diagnostic_units(&out[compact_contract::COMPACT_RECORD_HEADER_LEN..]);
    let value = |name: &str| {
        units
            .iter()
            .find(|unit| unit.name == name)
            .map(|unit| unit.value)
    };
    assert_eq!(value("lw_next_hour_at_us"), Some(61_000));
    assert_eq!(value("lw_next_hour_brackets_closed"), Some(1));
    assert_eq!(value("lw_next_hour_missing"), Some(0));
    assert_eq!(
        value("lw_fanout_origin_us"),
        Some(23_641),
        "the distance between the two origins this record mixes must be on the \
         wire, or a build-relative offset cannot be placed against a \
         fan-out-relative span"
    );

    let clean = [stamped(10, (LABEL_HOUR, 1, 5_000), (LABEL_HOUR, 2, 20_000))];
    let mut out = Vec::new();
    append_compact_lane_wall_probe_record(
        &mut out,
        7,
        LABEL_HOUR,
        5,
        900,
        125,
        Some(0),
        FrameHourAge::default(),
        &clean,
        &BTreeSet::new(),
        MapTopologyShareCounts::default(),
        130,
        700,
        23_641,
        &OutsideBuildSpans::default(),
        92_000,
        &FrameTailSegments::default(),
        &summary,
        FrameCpuSplit::default(),
    );
    let units = compact_perf_diagnostic_units(&out[compact_contract::COMPACT_RECORD_HEADER_LEN..]);
    let value = |name: &str| {
        units
            .iter()
            .find(|unit| unit.name == name)
            .map(|unit| unit.value)
    };
    assert_eq!(
        value("lw_next_hour_at_us"),
        None,
        "an absent boundary must publish NO position; a zero would read as a \
         build that began in the next hour"
    );
    assert_eq!(value("lw_next_hour_brackets_closed"), None);
    assert_eq!(value("lw_next_hour_missing"), Some(1));
}

#[test]
fn only_a_poll_that_saw_a_new_hour_publishes_an_observation() {
    const HOUR: i32 = 60_832_190;
    assert_eq!(
        frontier_observation_to_publish(Some(HOUR - 1), Some((HOUR, 7))),
        Some(HOUR),
        "the poll that first sees an hour is the one that knows when it began"
    );
    assert_eq!(
        frontier_observation_to_publish(None, Some((HOUR, 7))),
        Some(HOUR),
        "the very first readable poll has nothing to compare against and stamps"
    );
    assert_eq!(
        frontier_observation_to_publish(Some(HOUR), Some((HOUR, 8))),
        None,
        "re-reading the hour already stamped observes no boundary; stamping it \
         would reset the age of an hour that began long ago and report every \
         build as having started at the boundary"
    );
    assert_eq!(
        frontier_observation_to_publish(Some(HOUR), None),
        None,
        "a clock-unreadable poll knows no hour to stamp"
    );
    assert_eq!(frontier_observation_to_publish(None, None), None);
    assert_eq!(
        frontier_observation_to_publish(Some(HOUR), Some((HOUR - 5, 9))),
        Some(HOUR - 5),
        "a backwards clock is a different hour, and a different hour is stamped"
    );
}

#[test]
fn the_build_start_lateness_is_measured_or_absent_and_never_negative() {
    let observed_at_us = 1_000_000u64;
    let seen = Some(observed_at_us);

    let late = FrameHourAge::measure(
        seen,
        Some(LABEL_HOUR),
        observed_at_us + 66_500,
        LABEL_HOUR,
        true,
    );
    assert_eq!(late.age_us, Some(66_500));
    assert_eq!(
        late.frontier_ahead, 0,
        "the engine had not moved past this frame's label"
    );

    let prompt = FrameHourAge::measure(seen, Some(LABEL_HOUR), observed_at_us, LABEL_HOUR, true);
    assert_eq!(prompt.age_us, Some(0));

    let behind = FrameHourAge::measure(
        seen,
        Some(LABEL_HOUR + 1),
        observed_at_us + 112_000,
        LABEL_HOUR,
        true,
    );
    assert_eq!(behind.age_us, Some(112_000));
    assert_eq!(behind.frontier_ahead, 1);

    assert_eq!(
        FrameHourAge::measure(
            None,
            Some(LABEL_HOUR),
            observed_at_us + 40_000,
            LABEL_HOUR,
            true
        )
        .age_us,
        None
    );

    assert_eq!(
        FrameHourAge::measure(seen, Some(LABEL_HOUR), observed_at_us + 40_000, 0, false).age_us,
        None
    );

    assert_eq!(
        FrameHourAge::measure(
            seen,
            Some(LABEL_HOUR),
            observed_at_us - 3_000,
            LABEL_HOUR,
            true
        )
        .age_us,
        None
    );
}

#[test]
fn the_build_start_lateness_reaches_the_probe_record_and_absence_omits_it() {
    let summary = summarize_lane_spans(&[]);
    let probe = |hour_age: FrameHourAge| {
        let mut out = Vec::new();
        append_compact_lane_wall_probe_record(
            &mut out,
            7,
            LABEL_HOUR,
            5,
            900,
            125,
            Some(0),
            hour_age,
            &[],
            &BTreeSet::new(),
            MapTopologyShareCounts::default(),
            130,
            700,
            23_641,
            &OutsideBuildSpans::default(),
            92_000,
            &FrameTailSegments::default(),
            &summary,
            FrameCpuSplit::default(),
        );
        compact_perf_diagnostic_units(&out[compact_contract::COMPACT_RECORD_HEADER_LEN..])
    };
    let value = |units: &[compact_contract::diagnostic_channel::DiagUnit], name: &str| {
        units
            .iter()
            .find(|unit| unit.name == name)
            .map(|unit| unit.value)
    };

    let measured = probe(FrameHourAge {
        age_us: Some(66_500),
        frontier_ahead: 0,
    });
    assert_eq!(value(&measured, "lw_hour_age_us"), Some(66_500));
    assert_eq!(value(&measured, "lw_hour_frontier_ahead"), Some(0));
    assert_eq!(value(&measured, "lw_hour_age_missing"), Some(0));

    let behind = probe(FrameHourAge {
        age_us: Some(12_000),
        frontier_ahead: 1,
    });
    assert_eq!(value(&behind, "lw_hour_frontier_ahead"), Some(1));

    let absent = probe(FrameHourAge::default());
    assert_eq!(
        value(&absent, "lw_hour_age_us"),
        None,
        "an unmeasured phase must publish NO value; a zero would read as a \
         build that began exactly at the boundary, which is the CLEANEST \
         possible frame"
    );
    assert_eq!(value(&absent, "lw_hour_frontier_ahead"), None);
    assert_eq!(value(&absent, "lw_hour_age_missing"), Some(1));
}

#[test]
fn a_build_reads_the_stamp_for_its_own_hour_not_the_newest_one() {
    hour_frontier_observation::reset_for_test();
    assert_eq!(
        hour_frontier_observation::observation_for(LABEL_HOUR).0,
        None,
        "before the observer has seen a single advance there is nothing to report"
    );
    assert_eq!(
        hour_frontier_observation::observation_for(LABEL_HOUR).1,
        None
    );

    let at = Instant::now();
    hour_frontier_observation::record(LABEL_HOUR, at);
    assert_eq!(
        hour_frontier_observation::observation_for(LABEL_HOUR).0,
        Some(hour_frontier_observation::micros_since_origin(at))
    );
    assert_eq!(
        hour_frontier_observation::observation_for(LABEL_HOUR).1,
        Some(LABEL_HOUR)
    );

    let later = Instant::now();
    hour_frontier_observation::record(LABEL_HOUR + 1, later);
    assert_eq!(
        hour_frontier_observation::observation_for(LABEL_HOUR).1,
        Some(LABEL_HOUR + 1)
    );
    assert_eq!(
        hour_frontier_observation::observation_for(LABEL_HOUR).0,
        Some(hour_frontier_observation::micros_since_origin(at)),
        "the older hour's stamp must survive the newer one, or a frame that is \
         a whole hour behind reports an age about an hour it is not labelled with"
    );
    assert_eq!(
        hour_frontier_observation::observation_for(LABEL_HOUR + 1).0,
        Some(hour_frontier_observation::micros_since_origin(later))
    );

    assert_eq!(
        hour_frontier_observation::observation_for(LABEL_HOUR + 7).0,
        None
    );
    hour_frontier_observation::reset_for_test();

    the_frontier_ring_keeps_recent_hours_and_evicts_the_oldest();
}

fn the_frontier_ring_keeps_recent_hours_and_evicts_the_oldest() {
    let depth = i32::try_from(hour_frontier_observation::DEPTH).expect("a sane ring depth");
    hour_frontier_observation::reset_for_test();
    let mut stamps = Vec::new();
    for step in 0..=depth {
        let at = Instant::now();
        hour_frontier_observation::record(LABEL_HOUR + step, at);
        stamps.push(at);
    }
    assert_eq!(
        hour_frontier_observation::observation_for(LABEL_HOUR).0,
        None,
        "the oldest hour is the one evicted"
    );
    for step in 1..=depth {
        assert_eq!(
            hour_frontier_observation::observation_for(LABEL_HOUR + step).0,
            Some(hour_frontier_observation::micros_since_origin(
                stamps[step as usize]
            )),
            "hour +{step} must still be readable"
        );
    }

    let held = depth / 2;
    let again = Instant::now();
    hour_frontier_observation::record(LABEL_HOUR + held, again);
    assert_eq!(
        hour_frontier_observation::observation_for(LABEL_HOUR + held).0,
        Some(hour_frontier_observation::micros_since_origin(again))
    );
    for step in 1..=depth {
        if step == held {
            continue;
        }
        assert!(
            hour_frontier_observation::observation_for(LABEL_HOUR + step)
                .0
                .is_some(),
            "re-observing hour +{held} must not have evicted hour +{step}"
        );
    }
    hour_frontier_observation::reset_for_test();
}
