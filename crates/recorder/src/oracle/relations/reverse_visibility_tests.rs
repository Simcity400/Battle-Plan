use super::*;
use replay::{
    frame::TableView,
    rows::{
        Recorded, ReservedCountryNamingRow, ReservedReverseVisibilityInputs,
        ReverseZeroVisibilityInputs, TableRows, ZeroSubjectInputs,
    },
};

fn fixture(value: Recorded<Option<ReservedReverseVisibilityInputs>>) -> DecodedFrame {
    let (mut frame, _, _, _) = super::super::super::tests::fixture();
    *frame.tables[TableId::ReservedCountryNaming.index()] =
        TableView::Complete(TableRows::ReservedCountryNaming(vec![
            ReservedCountryNamingRow {
                reverse_visibility: value,
                ..Default::default()
            },
        ]));
    frame
}

fn limits(report: &Report, fragment: &str) -> bool {
    report
        .limitations
        .keys()
        .any(|reason| reason.contains(fragment))
}

#[test]
fn historical_refused_and_known_empty_reverse_matrix_remain_distinct() {
    let mut report = Report::default();
    compare(&fixture(Recorded::NotRecorded), &mut report).unwrap();
    assert!(limits(&report, "not recorded"));
    assert!(!limits(&report, "diplomacy matrix count/order"));
    assert!(report.differences.is_empty());
    assert!(report.failed_comparisons.is_empty());
    assert_eq!(report.scalar_checks, 0);
    assert!(!report.compared_tables.contains_key(LABEL));

    let mut report = Report::default();
    compare(&fixture(Recorded::Value(None)), &mut report).unwrap();
    assert!(limits(&report, "unavailable at capture"));
    assert!(!limits(&report, "diplomacy matrix count/order"));
    assert!(report.differences.is_empty());
    assert!(report.failed_comparisons.is_empty());
    assert_eq!(report.scalar_checks, 0);
    assert!(!report.compared_tables.contains_key(LABEL));

    let mut frame = fixture(Recorded::Value(Some(
        ReservedReverseVisibilityInputs::default(),
    )));
    let TableView::Complete(TableRows::ReservedCountryNaming(rows)) =
        &mut *frame.tables[TableId::ReservedCountryNaming.index()]
    else {
        unreachable!()
    };
    rows[0].reverse_visibility = Recorded::Value(Some(ReservedReverseVisibilityInputs {
        sample_hour: frame.hour,
        cells: Vec::new(),
    }));
    let mut report = Report::default();
    compare(&frame, &mut report).unwrap();
    assert!(limits(&report, "diplomacy matrix count/order"));
    // This is a present native count0, not a refusal or a saved comparison.
    assert!(report.limitations.iter().any(|(reason, subjects)| {
        reason.contains("diplomacy matrix count/order")
            && reason.contains("no direct text-save counterpart")
            && reason.contains("matched same-hour native witness is required")
            && subjects
                .iter()
                .any(|subject| subject == &format!("{LABEL}: 0 native matrix ordinals"))
    }));
    assert!(report.failed_comparisons.is_empty());
    assert_eq!(report.compared_tables.get(LABEL), Some(&0));
    assert!(!limits(&report, "unavailable at capture"));
    assert!(report.differences.is_empty());
    assert_eq!(report.scalar_checks, 0);
}

#[test]
fn reverse_cells_preserve_missing_raw_zero_alias_and_retired_subject_inputs() {
    let mut frame = fixture(Recorded::Value(None));
    let TableView::Complete(TableRows::ReservedCountryNaming(rows)) =
        &mut *frame.tables[TableId::ReservedCountryNaming.index()]
    else {
        unreachable!()
    };
    rows[0].reverse_visibility = Recorded::Value(Some(ReservedReverseVisibilityInputs {
        sample_hour: frame.hour,
        cells: vec![
            Some(ReverseZeroVisibilityInputs {
                target: Some(0),
                war: Some([1, 0]),
                subject: None,
                military_access: None,
            }),
            None,
            Some(ReverseZeroVisibilityInputs {
                target: Some(u32::MAX),
                war: Some([1, 255]),
                subject: Some(ZeroSubjectInputs {
                    relation_type: Some(0x30d1),
                    parties: Some([None, Some(u32::MAX)]),
                }),
                military_access: Some(1),
            }),
        ],
    }));
    let mut report = Report::default();
    compare(&frame, &mut report).unwrap();
    assert!(limits(&report, "raw +0x50 target association"));
    assert!(limits(&report, "war-pointer presence"));
    assert!(limits(&report, "first/second-party OR gates"));
    assert!(limits(&report, "+0x2A0 military-access pointer presence"));
    let missing: Vec<_> = report
        .limitations
        .iter()
        .filter(|(reason, _)| reason.contains("a reverse receiver cell was unavailable"))
        .flat_map(|(_, labels)| labels)
        .collect();
    assert_eq!(missing.len(), 1);
    assert_eq!(missing[0], &format!("{LABEL}.cells[1]"));
    assert!(report.differences.is_empty());
}

#[test]
fn reverse_packet_requires_its_own_current_hour_and_complete_naming_prefix() {
    let mut frame = fixture(Recorded::Value(Some(ReservedReverseVisibilityInputs {
        sample_hour: i32::MIN,
        cells: Vec::new(),
    })));
    assert!(compare(&frame, &mut Report::default()).is_err());
    *frame.tables[TableId::ReservedCountryNaming.index()] = TableView::Unavailable {
        reason: "required naming prefix refused".into(),
    };
    assert!(compare(&frame, &mut Report::default()).is_err());
}

#[test]
fn reverse_military_presence_preserves_null_refusal_and_invalid_admission() {
    for presence in [None, Some(0), Some(1), Some(2), Some(u8::MAX)] {
        let mut frame = fixture(Recorded::Value(None));
        let TableView::Complete(TableRows::ReservedCountryNaming(rows)) =
            &mut *frame.tables[TableId::ReservedCountryNaming.index()]
        else {
            unreachable!()
        };
        rows[0].reverse_visibility = Recorded::Value(Some(ReservedReverseVisibilityInputs {
            sample_hour: frame.hour,
            cells: vec![Some(ReverseZeroVisibilityInputs {
                target: Some(0),
                war: Some([0, 0]),
                military_access: presence,
                ..Default::default()
            })],
        }));
        let mut report = Report::default();
        if presence.is_some_and(|presence| presence > 1) {
            let error = compare(&frame, &mut report).unwrap_err();
            assert!(error.to_string().contains("not a native pointer presence"));
        } else {
            compare(&frame, &mut report).unwrap();
            assert!(limits(&report, "+0x2A0 military-access pointer presence"));
            assert_eq!(
                limits(
                    &report,
                    "military-access pointer was unreached or unavailable"
                ),
                presence.is_none(),
            );
            assert!(report.differences.is_empty());
        }
    }
}
