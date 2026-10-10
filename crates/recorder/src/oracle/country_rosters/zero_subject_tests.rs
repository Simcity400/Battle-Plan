use super::*;
use replay::rows::{CountryCounterInputRow, Recorded, ZeroSubjectInputs};

#[test]
fn zero_subject_reports_history_capture_refusal_and_unsaved_native_branches() {
    let (_, mut save, _, _) = super::super::tests::fixture();
    // The runtime limitation must survive a missing positive save-country join.
    save.countries.remove(b"GER".as_slice());
    let tags = [(101, b"GER".as_slice())].into();
    for (value, expected) in [
        (Recorded::NotRecorded, "not recorded"),
        (Recorded::Value(None), "unavailable at capture"),
        (
            Recorded::Value(Some(ZeroSubjectInputs::default())),
            "runtime-only",
        ),
        (
            Recorded::Value(Some(ZeroSubjectInputs {
                relation_type: Some(0x380a),
                parties: None,
            })),
            "runtime-only",
        ),
        (
            Recorded::Value(Some(ZeroSubjectInputs {
                relation_type: Some(0x30d1),
                parties: Some([Some(0), None]),
            })),
            "runtime-only",
        ),
        (
            Recorded::Value(Some(ZeroSubjectInputs {
                relation_type: Some(0x30d1),
                parties: Some([None, Some(0)]),
            })),
            "runtime-only",
        ),
    ] {
        let mut report = Report::default();
        compare_lists(
            &[CountryCounterInputRow {
                forward_visibility: replay::rows::Recorded::NotRecorded,
                country: 101,
                zero_subject: value,
                ..Default::default()
            }],
            &save,
            &tags,
            &mut report,
        )
        .unwrap();
        if expected == "not recorded" {
            // Historical field identity is in the reason; the subject names the row.
            assert_eq!(
                report.limitations["CountryCounterInputs.zero_subject not recorded"],
                vec!["CountryCounterInputs GER".to_string()],
            );
        } else {
            assert!(report.limitations.iter().any(|(reason, labels)| {
                reason.contains(expected)
                    && labels.iter().any(|label| label.contains("zero_subject"))
            }));
        }
        assert_eq!(report.differences.len(), 1);
        assert_eq!(report.scalar_checks, 0);
    }
}
