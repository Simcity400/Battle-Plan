use super::*;
use replay_data::rows::*;
use replay_frame::frame::TableView;

pub(super) const DOCUMENT: &[u8] = br#"
sub_units = {
    line_unit = { type = infantry need = { widgets_1 = 1.250009 crates = 2 widgets = 3 } }
    helper_unit = { type = support need = { widgets = 0.5 } }
}
equipments = {
    widgets = { is_archetype = yes type = infantry }
    crates = { is_archetype = yes type = support }
    widgets_1 = { archetype = widgets }
}
"#;

fn frame() -> DecodedFrame {
    let mut frame = dashboard_base::test_fixtures::source(1, 60_000_002, None);
    *frame.tables[TableId::LexicalTokens.index()] =
        TableView::Complete(TableRows::LexicalTokens(vec![
            LexicalTokenRow {
                token: 1,
                literal: b"line_unit".to_vec(),
            },
            LexicalTokenRow {
                token: 2,
                literal: b"helper_unit".to_vec(),
            },
            LexicalTokenRow {
                token: 3,
                literal: b"widgets".to_vec(),
            },
            LexicalTokenRow {
                token: 4,
                literal: b"crates".to_vec(),
            },
        ]));
    *frame.tables[TableId::CountryLocalSubunits.index()] =
        TableView::Complete(TableRows::CountryLocalSubunits(vec![]));
    *frame.tables[TableId::DivisionTemplateIcons.index()] =
        TableView::Complete(TableRows::DivisionTemplateIcons(vec![
            DivisionTemplateIconRow {
                template: 5,
                country: 9001,
                // An unrecorded visual representative is irrelevant to equipment need.
                representative: 99,
                subunits: vec![
                    TemplateSubunit {
                        token: 1,
                        local: 0,
                        count: 2,
                    },
                    TemplateSubunit {
                        token: 2,
                        local: 0,
                        count: 3,
                    },
                ],
                ..Default::default()
            },
        ]));
    frame
}

#[test]
fn grouped_needs_use_canonical_first_occurrence_order_and_checked_fixed5() {
    let frame = frame();
    let needs = Needs::from_documents(&[DOCUMENT], &frame);
    assert_eq!(
        needs.template(&frame, 5).unwrap(),
        [("widgets".into(), 750_000), ("crates".into(), 400_000)]
    );
}

#[test]
fn recorded_local_clone_replaces_authored_needs_and_complete_absence_falls_back() {
    let mut frame = frame();
    if let TableView::Complete(TableRows::DivisionTemplateIcons(rows)) =
        &mut *frame.tables[TableId::DivisionTemplateIcons.index()]
    {
        rows[0].subunits[0].local = 1;
    }
    *frame.tables[TableId::CountryLocalSubunits.index()] =
        TableView::Complete(TableRows::CountryLocalSubunits(vec![
            CountryLocalSubunitRow {
                country: 11,
                token: 1,
                requirements: vec![LocalSubunitRequirement {
                    archetype_token: 4,
                    amount_raw: -25_000,
                }],
            },
        ]));
    let needs = Needs::from_documents(&[DOCUMENT], &frame);
    assert_eq!(
        needs.template(&frame, 5).unwrap(),
        [("crates".into(), -50_000), ("widgets".into(), 150_000)]
    );
    *frame.tables[TableId::CountryLocalSubunits.index()] = TableView::Truncated {
        rows: TableRows::CountryLocalSubunits(vec![]),
        missing: 1,
    };
    let needs = Needs::from_documents(&[DOCUMENT], &frame);
    assert!(
        needs
            .template(&frame, 5)
            .unwrap_err()
            .contains("incomplete census")
    );
    *frame.tables[TableId::CountryLocalSubunits.index()] =
        TableView::Complete(TableRows::CountryLocalSubunits(vec![]));
    let needs = Needs::from_documents(&[DOCUMENT], &frame);
    assert_eq!(needs.template(&frame, 5).unwrap()[0].1, 750_000);
}

#[test]
fn unreadable_composition_and_overflow_refuse_instead_of_returning_zero() {
    let mut frame = frame();
    if let TableView::Complete(TableRows::DivisionTemplateIcons(rows)) =
        &mut *frame.tables[TableId::DivisionTemplateIcons.index()]
    {
        rows[0].model_unreadable = 1;
    }
    assert!(
        Needs::from_documents(&[DOCUMENT], &frame)
            .template(&frame, 5)
            .unwrap_err()
            .contains("not readable")
    );
    if let TableView::Complete(TableRows::DivisionTemplateIcons(rows)) =
        &mut *frame.tables[TableId::DivisionTemplateIcons.index()]
    {
        rows[0].model_unreadable = 0;
        rows[0].subunits[0].local = 1;
    }
    *frame.tables[TableId::CountryLocalSubunits.index()] =
        TableView::Complete(TableRows::CountryLocalSubunits(vec![
            CountryLocalSubunitRow {
                country: 11,
                token: 1,
                requirements: vec![LocalSubunitRequirement {
                    archetype_token: 3,
                    amount_raw: i64::MAX,
                }],
            },
        ]));
    assert!(
        Needs::from_documents(&[DOCUMENT], &frame)
            .template(&frame, 5)
            .unwrap_err()
            .contains("overflows")
    );
}

#[test]
fn content_load_is_shared_by_the_bound_stack_not_by_the_reached_frame() {
    let (cat, _folder) = dashboard_text::catalogue::tests::folder_catalogue(&[
        (
            "common/units/equipment/test.txt",
            std::str::from_utf8(DOCUMENT).unwrap(),
        ),
        (
            "common/units/test.txt",
            std::str::from_utf8(DOCUMENT).unwrap(),
        ),
    ]);
    let cat = Arc::new(cat);
    let frame = frame();
    let _synthetic = Needs::from_documents(&[DOCUMENT], &frame);
    let first = Needs::read(Some(&cat), &frame);
    let second = Needs::read(Some(&cat), &frame);
    assert_eq!(first.template(&frame, 5), second.template(&frame, 5));
    assert!(Arc::ptr_eq(
        first.content.as_ref().unwrap(),
        second.content.as_ref().unwrap()
    ));
    assert_eq!(
        cat.content
            .layers
            .derived::<CachedResult<Content>>()
            .loads(),
        1
    );
    let equipment = dashboard_base::products::equipment::units_of(&cat).unwrap();
    assert!(Arc::ptr_eq(
        &first.content.as_ref().unwrap().equipment,
        &equipment
    ));
}

#[test]
fn native_dropped_requirement_names_are_reported_without_refusing_surviving_needs() {
    let frame = frame();
    let needs = Needs::from_documents(
        &[
            DOCUMENT,
            b"sub_units = { line_unit = { need = { no_equipment = 2 widgets = 4 } } }",
        ],
        &frame,
    );
    assert_eq!(
        needs.template(&frame, 5).unwrap(),
        [("widgets".into(), 950_000)]
    );
    assert!(
        needs
            .warnings()
            .any(|warning| warning.contains("line_unit needs no_equipment"))
    );
}
