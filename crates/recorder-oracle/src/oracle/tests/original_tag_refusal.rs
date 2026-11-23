use super::*;
use replay_data::rows::CountryAppearanceRow;

fn appearance_report(
    frame: &DecodedFrame,
    saved: &Save,
    membership: &save::PartialMapMembership,
    equipment: &content_reader_key_names::equipment::Database,
) -> Report {
    let mut report = compare(frame, saved, membership, equipment).unwrap();
    report.attempt_table(TableId::CountryAppearance, |report| {
        map_inputs::countries(frame, saved, report)
    });
    report
}

#[test]
fn full_report_cannot_certify_original_tag_with_current_reference_or_byte_faults() {
    let (mut frame, mut saved, membership, equipment) = fixture();
    let country = saved.countries.get_mut(b"GER".as_slice()).unwrap();
    country.original_tag = Some(b"OWN".to_vec());
    country.cosmetic_tag = Some(Vec::new());
    let mut row = CountryAppearanceRow {
        country: 101,
        original_tag_reference: Recorded::Value(Some(7)),
        original_tag: Recorded::Value(Some(b"OWN".to_vec())),
        ..Default::default()
    };
    *frame.tables[TableId::CountryAppearance.index()] =
        TableView::Complete(TableRows::CountryAppearance(vec![row.clone()]));
    let baseline = appearance_report(&frame, &saved, &membership, &equipment);
    assert!(
        baseline.outcome().is_ok(),
        "{:?} {:?}",
        baseline.differences,
        baseline.failed_comparisons
    );
    for reference_fault in [false, true] {
        row.original_tag_reference = Recorded::Value(if reference_fault { None } else { Some(7) });
        row.original_tag = Recorded::Value(None);
        *frame.tables[TableId::CountryAppearance.index()] =
            TableView::Complete(TableRows::CountryAppearance(vec![row.clone()]));
        for expected in [None, Some(b"OTHER".to_vec()), Some(b"OWN".to_vec())] {
            saved
                .countries
                .get_mut(b"GER".as_slice())
                .unwrap()
                .original_tag = expected;
            let report = appearance_report(&frame, &saved, &membership, &equipment);
            assert!(report.outcome().is_err());
            assert!(
                report
                    .differences
                    .iter()
                    .any(|field| field.contains("original_tag"))
            );
            assert!(
                report.failed_comparisons.is_empty(),
                "{:?}",
                report.failed_comparisons
            );
        }
    }
}

#[test]
fn nonpositive_original_reference_does_not_demand_unused_byte_capture() {
    for bytes in [Recorded::NotRecorded, Recorded::Value(None)] {
        let row = CountryAppearanceRow {
            original_tag_reference: Recorded::Value(Some(-1)),
            original_tag: bytes,
            ..Default::default()
        };
        let mut report = Report::default();
        let (mut frame, mut saved, _, _) = fixture();
        saved
            .countries
            .get_mut(b"GER".as_slice())
            .unwrap()
            .cosmetic_tag = Some(Vec::new());
        *frame.tables[TableId::CountryAppearance.index()] =
            TableView::Complete(TableRows::CountryAppearance(vec![CountryAppearanceRow {
                country: 101,
                ..row
            }]));
        map_inputs::countries(&frame, &saved, &mut report).unwrap();
        assert!(report.outcome().is_ok());
        assert!(
            !report
                .limitations
                .contains_key("CountryAppearance.original_tag not recorded")
        );
    }
}
