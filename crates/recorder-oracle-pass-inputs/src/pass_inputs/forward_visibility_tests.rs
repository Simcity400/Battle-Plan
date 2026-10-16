use super::*;
use replay::{
    frame::TableView,
    rows::{
        CountryCounterInputRow, CountryRow, Recorded, ReverseZeroVisibilityInputs, TableId,
        TableRows, ZeroSubjectInputs,
    },
};

fn fixture(
    value: Recorded<Option<ReverseZeroVisibilityInputs>>,
) -> (DecodedFrame, super::super::Save) {
    let (mut frame, mut save, _, _) = super::super::super::fixtures::fixture();
    *frame.tables[TableId::CountryCounterInputs.index()] =
        TableView::Complete(TableRows::CountryCounterInputs(vec![
            CountryCounterInputRow {
                country: 101,
                owned_state_count: 1,
                controlled_state_count: 1,
                forward_visibility: value,
                ..Default::default()
            },
        ]));
    save.counter_inputs.prides.insert(b"GER".to_vec(), None);
    (frame, save)
}

fn null_packet() -> ReverseZeroVisibilityInputs {
    ReverseZeroVisibilityInputs {
        target: Some(0),
        war: Some([0, 0]),
        subject: Some(ZeroSubjectInputs::default()),
        military_access: Some(0),
    }
}

fn limits(report: &Report, fragment: &str) -> bool {
    report
        .limitations
        .keys()
        .any(|reason| reason.contains(fragment))
}

fn run(value: Recorded<Option<ReverseZeroVisibilityInputs>>) -> Report {
    let (frame, _) = fixture(value);
    let mut report = Report::default();
    compare(&frame, &mut report).unwrap();
    report
}

#[test]
fn forward_history_whole_refusal_missing_members_and_native_null_are_distinct() {
    let old = run(Recorded::NotRecorded);
    assert!(limits(&old, "not recorded"));
    assert!(!limits(&old, "packet was unavailable"));
    let refused = run(Recorded::Value(None));
    assert!(!limits(&refused, "not recorded"));
    assert!(limits(&refused, "packet was unavailable"));
    assert!(!limits(&refused, "no proved text-save counterpart"));
    let missing = run(Recorded::Value(
        Some(ReverseZeroVisibilityInputs::default()),
    ));
    for fragment in [
        "raw target was unreached",
        "war-pointer presence/retired byte was unreached",
        "military-access pointer was unreached",
        "subject operands were unreached",
    ] {
        assert!(
            limits(&missing, fragment),
            "{fragment}: {:?}",
            missing.limitations
        );
    }
    let null = run(Recorded::Value(Some(null_packet())));
    assert!(!limits(&null, "unreached or unavailable"));
    assert!(!limits(&null, "packet was unavailable"));
    for report in [&old, &refused, &missing, &null] {
        assert!(
            report.failed_comparisons.is_empty(),
            "{:?}",
            report.failed_comparisons
        );
        assert!(report.differences.is_empty());
        assert_eq!(report.scalar_checks, 0);
        assert!(report.compared_tables.is_empty());
    }
}

#[test]
fn forward_historical_field_is_counted_once_in_whole_frame_reports() {
    let (frame, _) = fixture(Recorded::NotRecorded);
    let mut report = Report::default();
    report.record_tables(&frame);
    compare(&frame, &mut report).unwrap();
    assert_eq!(
        report.limitations[&format!("{LABEL} not recorded")].len(),
        1
    );
}

#[test]
fn forward_war_self_check_keeps_raw_retirement_and_rejects_impossible_null() {
    for (war, valid) in [
        ([0, 0], true),
        ([1, 0], true),
        ([1, 1], true),
        ([1, 2], true),
        ([1, u8::MAX], true),
        ([0, 1], false),
        ([0, u8::MAX], false),
        ([2, 0], false),
        ([u8::MAX, 255], false),
    ] {
        let report = run(Recorded::Value(Some(ReverseZeroVisibilityInputs {
            war: Some(war),
            ..null_packet()
        })));
        assert_eq!(
            report
                .failed_comparisons
                .contains_key("CountryCounterInputs 101.forward_visibility.war"),
            !valid,
            "{war:?}"
        );
        assert!(limits(&report, "pointed raw retired byte"));
        // Even corrupt war does not suppress the independently known MIL field.
        assert!(limits(&report, "exact eight-byte pointer presence"));
        assert_eq!(report.scalar_checks, 0);
    }
}

#[test]
fn forward_military_presence_is_binary_without_demanding_later_target_or_subject() {
    for military_access in [None, Some(0), Some(1), Some(2), Some(u8::MAX)] {
        let report = run(Recorded::Value(Some(ReverseZeroVisibilityInputs {
            war: Some([0, 0]),
            military_access,
            ..Default::default()
        })));
        assert_eq!(
            report
                .failed_comparisons
                .contains_key("CountryCounterInputs 101.forward_visibility.military_access"),
            military_access.is_some_and(|presence| presence > 1)
        );
        assert_eq!(
            limits(&report, "military-access pointer was unreached"),
            military_access.is_none()
        );
        assert!(limits(&report, "raw target was unreached"));
        assert!(limits(&report, "subject operands were unreached"));
        assert_eq!(report.scalar_checks, 0);
    }
    let active_war = run(Recorded::Value(Some(ReverseZeroVisibilityInputs {
        war: Some([1, 0]),
        ..Default::default()
    })));
    assert!(active_war.failed_comparisons.is_empty());
}

#[test]
fn forward_raw_targets_and_parties_remain_native_witnesses_without_link_coercion() {
    for raw in [0, 73, u32::MAX] {
        let (mut frame, _) = fixture(Recorded::Value(Some(ReverseZeroVisibilityInputs {
            target: Some(raw),
            subject: Some(ZeroSubjectInputs {
                relation_type: Some(0x30d1),
                parties: Some([Some(raw), Some(u32::MAX)]),
            }),
            ..null_packet()
        })));
        // This structural oracle does not resolve even positive target aliases.
        // Native target/link equality still needs the explicit matched witness.
        *frame.tables[TableId::CountryLinks.index()] = TableView::Unavailable {
            reason: "target alias witness intentionally absent".into(),
        };
        let mut report = Report::default();
        compare(&frame, &mut report).unwrap();
        assert!(report.failed_comparisons.is_empty());
        assert!(limits(
            &report,
            "literal0 from exact positive aliases to canonical0"
        ));
        assert!(limits(&report, "no target or party alias is resolved here"));
        assert!(report.limitations.values().flatten().any(|label| label
            == &format!("CountryCounterInputs 101.forward_visibility.target raw {raw}")));
        assert_eq!(report.scalar_checks, 0);
        assert!(report.compared_tables.is_empty());
    }
}

#[test]
fn forward_subject_null_unreached_and_partial_ordered_party_inputs_remain_distinct() {
    for (subject, missing_party) in [
        (ZeroSubjectInputs::default(), None),
        (
            ZeroSubjectInputs {
                relation_type: Some(0x380a),
                parties: None,
            },
            None,
        ),
        (
            ZeroSubjectInputs {
                relation_type: Some(0x30d1),
                parties: Some([Some(0), None]),
            },
            Some(1),
        ),
        (
            ZeroSubjectInputs {
                relation_type: Some(0x30d1),
                parties: Some([None, Some(u32::MAX)]),
            },
            Some(0),
        ),
    ] {
        let report = run(Recorded::Value(Some(ReverseZeroVisibilityInputs {
            subject: Some(subject),
            ..null_packet()
        })));
        assert!(report.failed_comparisons.is_empty());
        assert!(!limits(&report, "subject operands were unreached"));
        assert_eq!(
            limits(&report, "subject parties were unreached"),
            subject.relation_type.is_some() && subject.parties.is_none()
        );
        for ordinal in 0..2 {
            let label =
                format!("CountryCounterInputs 101.forward_visibility.subject.parties[{ordinal}]");
            assert_eq!(
                report
                    .limitations
                    .iter()
                    .filter(|(reason, _)| reason.contains("subject party was unreached"))
                    .flat_map(|(_, labels)| labels)
                    .any(|item| item == &label),
                missing_party == Some(ordinal)
            );
        }
    }
    let corrupt = run(Recorded::Value(Some(ReverseZeroVisibilityInputs {
        subject: Some(ZeroSubjectInputs {
            relation_type: None,
            parties: Some([Some(0), None]),
        }),
        ..null_packet()
    })));
    assert!(
        corrupt
            .failed_comparisons
            .contains_key("CountryCounterInputs 101.forward_visibility.subject")
    );
}

#[test]
fn forward_receiver_admission_never_grants_actual_zero_or_unknown_owner_support() {
    for (raw, slot, duplicate) in [
        (0, 7, false),
        (101, 0, false),
        (102, 7, false),
        (101, 7, true),
    ] {
        let (mut frame, _) = fixture(Recorded::Value(Some(null_packet())));
        let TableView::Complete(TableRows::CountryCounterInputs(rows)) =
            &mut *frame.tables[TableId::CountryCounterInputs.index()]
        else {
            unreachable!()
        };
        rows[0].country = raw;
        let TableView::Complete(TableRows::Countries(countries)) =
            &mut *frame.tables[TableId::Countries.index()]
        else {
            unreachable!()
        };
        countries[0].canonical_slot = slot;
        if duplicate {
            countries.push(countries[0].clone());
        }
        let mut report = Report::default();
        compare(&frame, &mut report).unwrap();
        assert!(report.failed_comparisons.contains_key(&format!(
            "CountryCounterInputs {raw}.forward_visibility.receiver"
        )));
        assert!(limits(&report, "exact eight-byte pointer presence"));
    }
}

#[test]
fn forward_corruption_preserves_saved_prefix_differences_and_comparison_count() {
    for value in [
        Recorded::NotRecorded,
        Recorded::Value(None),
        Recorded::Value(Some(ReverseZeroVisibilityInputs {
            military_access: Some(2),
            ..null_packet()
        })),
    ] {
        let corrupt = matches!(&value, Recorded::Value(Some(_)));
        let (mut frame, save) = fixture(value);
        let TableView::Complete(TableRows::CountryCounterInputs(rows)) =
            &mut *frame.tables[TableId::CountryCounterInputs.index()]
        else {
            unreachable!()
        };
        rows[0].owned_state_count = 99;
        let mut report = Report::default();
        super::super::compare(&frame, &save, &mut report);
        assert_eq!(report.compared_tables["CountryCounterInputs"], 1);
        assert!(report.differences.iter().any(|difference| difference.contains("CountryCounterInputs GER.owned_state_count")));
        assert_eq!(
            report
                .failed_comparisons
                .contains_key("CountryCounterInputs 101.forward_visibility.military_access"),
            corrupt
        );
        assert!(
            !report
                .failed_comparisons
                .contains_key("CountryCounterInputs")
        );
    }
}

#[test]
fn forward_bad_member_or_receiver_does_not_suppress_other_rows_or_fields() {
    let (mut frame, _) = fixture(Recorded::Value(Some(ReverseZeroVisibilityInputs {
        war: Some([0, 255]),
        military_access: Some(2),
        ..null_packet()
    })));
    let TableView::Complete(TableRows::Countries(countries)) =
        &mut *frame.tables[TableId::Countries.index()]
    else {
        unreachable!()
    };
    countries.push(CountryRow {
        id: 103,
        canonical_slot: 9,
        ..Default::default()
    });
    let TableView::Complete(TableRows::CountryCounterInputs(rows)) =
        &mut *frame.tables[TableId::CountryCounterInputs.index()]
    else {
        unreachable!()
    };
    rows.push(CountryCounterInputRow {
        country: 103,
        forward_visibility: Recorded::Value(Some(null_packet())),
        ..Default::default()
    });
    let mut report = Report::default();
    compare(&frame, &mut report).unwrap();
    assert!(
        report
            .failed_comparisons
            .contains_key("CountryCounterInputs 101.forward_visibility.war")
    );
    assert!(
        report
            .failed_comparisons
            .contains_key("CountryCounterInputs 101.forward_visibility.military_access")
    );
    assert!(
        report
            .limitations
            .values()
            .flatten()
            .any(|label| label == "CountryCounterInputs 103.forward_visibility.military_access")
    );
    *frame.tables[TableId::Countries.index()] = TableView::Unavailable {
        reason: "positive receiver unavailable".into(),
    };
    let mut report = Report::default();
    compare(&frame, &mut report).unwrap();
    assert!(
        report
            .failed_comparisons
            .contains_key("CountryCounterInputs 103.forward_visibility.receiver")
    );
    assert!(
        report
            .failed_comparisons
            .contains_key("CountryCounterInputs 101.forward_visibility.military_access")
    );
}
