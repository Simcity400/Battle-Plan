use super::*;
use replay_data::{
    rows::DivisionTemplateIconRow, rows::DivisionTemplateRow, rows::TemplateDisplayName,
    rows::TemplateNameValue,
};

fn fixture_name(value: TemplateNameValue) -> (DecodedFrame, Save) {
    let (mut frame, mut saved, _, _) = division_fixture();
    frame.hour = 77;
    saved.divisions.clear();
    let template = (7000_u64 << 32) | 52;
    saved.division_templates.clear();
    saved.division_template_owners.clear();
    saved.division_templates.insert(
        template,
        save::DivisionTemplate {
            origin_type: None,
            template,
            country: b"GER".to_vec(),
            name: b"New".to_vec(),
            name_key: b"KEY".to_vec(),
            name_present: true,
            name_key_present: true,
            custom_icon: 0,
            custom_icon_nato: false,
            foreign_template_tag: Some(b"---".to_vec()),
            composition: BTreeMap::new(),
            names_group: None,
            settings: save::DivisionTemplateSettings {
                priority: Some(0),
                allow_new_equipment: Some(false),
                allow_foreign_equipment: Some(false),
                origin_type: Some(b"master_host".to_vec()),
                ..Default::default()
            },
            ..Default::default()
        },
    );
    saved
        .division_template_owners
        .insert(template, b"GER".to_vec());
    *frame.tables[TableId::DivisionTemplates.index()] =
        TableView::Complete(TableRows::DivisionTemplates(vec![DivisionTemplateRow {
            template,
            owner: 102,
            name: b"Old".to_vec(),
            name_key: Recorded::Value(b"OLD_KEY".to_vec()),
            ..Default::default()
        }]));
    *frame.tables[TableId::DivisionTemplateIcons.index()] =
        TableView::Complete(TableRows::DivisionTemplateIcons(vec![
            DivisionTemplateIconRow {
                template,
                country: 102,
                layout: Recorded::Value(Some(replay_data::rows::TemplateLayout::default().into())),
                settings: Recorded::Value(Some(
                    replay_data::rows::TemplateSettings {
                        forbidden_equipment: Some(Vec::new()),
                        ..Default::default()
                    }
                    .into(),
                )),
                display_name: Recorded::Value(Some(TemplateDisplayName {
                    sample_hour: replay_data::rows::PacketHour::Host,
                    value,
                })),
                ..Default::default()
            },
        ]));
    (frame, saved)
}
fn report_name(frame: &DecodedFrame, saved: &Save) -> Report {
    let countries = country_context(frame).unwrap();
    let mut report = Report::default();
    compare_division_template_icons(frame, saved, &countries, &BTreeMap::new(), &mut report)
        .unwrap();
    report
}

fn saved_name_fields(saved: &mut Save, fields: &str) {
    saved.division_templates = save::parse(format!(
        "HOI4txt\ndate=1936.1.1.1 game_unique_id=c version=b game_unique_seed=0 \
         debug_current_ref_id=1 unit=2 order_index=3 front_index=4 theatre_index=5 \
         theater_group_index=6 military_deployment_line_index=7 \
         military_deployment_conveyor_index=8 equipment_variant_index=9 \
         country_leader_index=10 navy_id=11 land_combat_id=12 \
         multiplayer_random_count=-13 countries={{}} states={{}} provinces={{}} \
         division_templates={{division_template={{id={{id=7000 type=52}} country=GER \
         foreign_template_tag=\"---\" template_counter=0 ingame_set_template_counter=no \
         priority=0 allow_new_equipment=no allow_foreign_equipment=no origin_type=master_host {fields}}}}}"
    ).as_bytes()).unwrap().division_templates;
}

fn report_daily_name(frame: &DecodedFrame, saved: &Save) -> Report {
    let countries = country_context(frame).unwrap();
    let mut report = Report::default();
    report
        .read_now.insert(TableId::DivisionTemplates);
    compare_division_templates(frame, saved, &countries, &mut report).unwrap();
    report
}

#[test]
fn fresh_hourly_override_compares_same_day_rename_without_daily_refresh_claim() {
    let (frame, mut saved) = fixture_name(TemplateNameValue::Changed {
        literal: b"New".to_vec(),
        fallback_key: None,
    });
    assert!(report_name(&frame, &saved).differences.is_empty());
    saved.division_templates.values_mut().next().unwrap().name = b"Wrong".to_vec();
    assert!(
        report_name(&frame, &saved)
            .differences
            .iter()
            .any(|difference| difference.contains("hourly name"))
    );
}

#[test]
fn same_frame_certificate_is_compared_to_save_and_never_waives_difference() {
    let (frame, saved) = fixture_name(TemplateNameValue::SameAsDivisionTemplates);
    assert!(
        report_name(&frame, &saved)
            .differences
            .iter()
            .any(|difference| difference.contains("hourly name"))
    );
}

#[test]
fn active_fallback_key_is_compared_and_save_nul_limit_is_named() {
    let (frame, mut saved) = fixture_name(TemplateNameValue::Changed {
        literal: Vec::new(),
        fallback_key: Some(b"KEY\0tail".to_vec()),
    });
    let template = saved.division_templates.values_mut().next().unwrap();
    template.name.clear();
    template.name_present = false;
    let report = report_name(&frame, &saved);
    assert!(report.differences.is_empty());
    assert!(
        report
            .limitations
            .values()
            .flatten()
            .any(|subject| subject.contains("hourly name_key"))
    );
    saved
        .division_templates
        .values_mut()
        .next()
        .unwrap()
        .name_key = b"DIFFERENT".to_vec();
    assert!(
        report_name(&frame, &saved)
            .differences
            .iter()
            .any(|difference| difference.contains("hourly name_key"))
    );
}

#[test]
fn current_name_refusal_fails_saved_operand_acceptance() {
    let (mut frame, saved) = fixture_name(TemplateNameValue::Changed {
        literal: b"New".to_vec(),
        fallback_key: None,
    });
    let mut row = frame.complete::<tables::DivisionTemplateIcons>().unwrap()[0].clone();
    row.display_name = Recorded::Value(None);
    *frame.tables[TableId::DivisionTemplateIcons.index()] =
        TableView::Complete(TableRows::DivisionTemplateIcons(vec![row]));
    let report = report_name(&frame, &saved);
    assert!(report.outcome().is_err());
    assert!(
        report
            .failed_comparisons
            .keys()
            .any(|field| field.contains("display_name"))
    );
}

#[test]
fn missing_same_frame_certificate_and_malformed_demand_fail_saved_comparison() {
    let (mut frame, saved) = fixture_name(TemplateNameValue::SameAsDivisionTemplates);
    *frame.tables[TableId::DivisionTemplates.index()] = TableView::Unavailable {
        reason: "missing dictionary".into(),
    };
    assert!(report_name(&frame, &saved).outcome().is_err());
    let mut row = frame.complete::<tables::DivisionTemplateIcons>().unwrap()[0].clone();
    row.display_name = Recorded::Value(Some(TemplateDisplayName {
        sample_hour: replay_data::rows::PacketHour::Host,
        value: TemplateNameValue::Changed {
            literal: b"New".to_vec(),
            fallback_key: Some(b"inactive".to_vec()),
        },
    }));
    *frame.tables[TableId::DivisionTemplateIcons.index()] =
        TableView::Complete(TableRows::DivisionTemplateIcons(vec![row]));
    assert!(report_name(&frame, &saved).outcome().is_err());
}

#[test]
fn historical_name_absence_keeps_named_limit_without_fresh_data_claim() {
    let (mut frame, saved) = fixture_name(TemplateNameValue::Changed {
        literal: b"New".to_vec(),
        fallback_key: None,
    });
    let mut row = frame.complete::<tables::DivisionTemplateIcons>().unwrap()[0].clone();
    row.display_name = Recorded::NotRecorded;
    *frame.tables[TableId::DivisionTemplateIcons.index()] =
        TableView::Complete(TableRows::DivisionTemplateIcons(vec![row]));
    let report = report_name(&frame, &saved);
    assert!(report.outcome().is_ok());
    assert!(
        report
            .limitations
            .values()
            .flatten()
            .any(|field| field.contains("display_name"))
    );
}

#[test]
fn daily_and_hourly_names_compare_native_nul_prefix_without_changing_demand() {
    for (stored, visible) in [
        (b"New\0suffix".as_slice(), b"New".as_slice()),
        (b"\0suffix".as_slice(), b"".as_slice()),
    ] {
        let (mut frame, mut saved) = fixture_name(TemplateNameValue::Changed {
            literal: stored.to_vec(),
            fallback_key: None,
        });
        saved.division_templates.values_mut().next().unwrap().name = visible.to_vec();
        let TableView::Complete(TableRows::DivisionTemplates(rows)) =
            &mut *frame.tables[TableId::DivisionTemplates.index()]
        else {
            unreachable!()
        };
        rows[0].name = stored.to_vec();
        rows[0].name_key = Recorded::Value(b"KEY".to_vec());
        let context = country_context(&frame).unwrap();
        let mut report = report_name(&frame, &saved);
        report
            .read_now.insert(TableId::DivisionTemplates);
        compare_division_templates(&frame, &saved, &context, &mut report).unwrap();
        assert!(report.differences.is_empty(), "{:?}", report.differences);
        assert!(
            report
                .limitations
                .values()
                .flatten()
                .any(|field| field.contains("hourly name"))
        );
        assert!(
            report
                .limitations
                .values()
                .flatten()
                .any(|field| field.ends_with(".name"))
        );
        assert!(report.failed_comparisons.is_empty());
        assert!(report.outcome().is_ok());
        saved.division_templates.values_mut().next().unwrap().name = b"Different".to_vec();
        let mut report = report_name(&frame, &saved);
        report
            .read_now.insert(TableId::DivisionTemplates);
        compare_division_templates(&frame, &saved, &context, &mut report).unwrap();
        assert!(
            report
                .differences
                .iter()
                .any(|field| field.contains("hourly name"))
        );
        assert!(
            report
                .differences
                .iter()
                .any(|field| field.contains(".name:"))
        );
    }
}

#[test]
fn daily_and_hourly_leading_nul_keys_compare_emitted_empty_presence() {
    let (mut frame, mut saved) = fixture_name(TemplateNameValue::Changed {
        literal: Vec::new(),
        fallback_key: Some(b"\0suffix".to_vec()),
    });
    let template = saved.division_templates.values_mut().next().unwrap();
    template.name.clear();
    template.name_present = false;
    template.name_key.clear();
    let TableView::Complete(TableRows::DivisionTemplates(rows)) =
        &mut *frame.tables[TableId::DivisionTemplates.index()]
    else {
        unreachable!()
    };
    rows[0].name.clear();
    rows[0].name_key = Recorded::Value(b"\0suffix".to_vec());
    let context = country_context(&frame).unwrap();
    let mut report = report_name(&frame, &saved);
    report
        .read_now.insert(TableId::DivisionTemplates);
    compare_division_templates(&frame, &saved, &context, &mut report).unwrap();
    assert!(report.differences.is_empty());
    assert!(report.failed_comparisons.is_empty());
    assert!(report.outcome().is_ok());
}

#[test]
fn parsed_template_name_presence_compares_daily_and_both_hourly_sources() {
    for (literal, key, matching, changed, field) in [
        (
            b"\0literal".as_slice(),
            b"KEY".as_slice(),
            "name=\"\" localization_key=KEY",
            "localization_key=KEY",
            "name",
        ),
        (
            b"".as_slice(),
            b"\0key".as_slice(),
            "localization_key=\"\"",
            "",
            "name_key",
        ),
        (b"".as_slice(), b"".as_slice(), "", "name=\"\"", "name"),
        (
            b"".as_slice(),
            b"".as_slice(),
            "",
            "localization_key=\"\"",
            "name_key",
        ),
        (
            b"New".as_slice(),
            b"KEY".as_slice(),
            "name=New localization_key=KEY",
            "localization_key=KEY",
            "name",
        ),
        (
            b"".as_slice(),
            b"KEY".as_slice(),
            "localization_key=KEY",
            "",
            "name_key",
        ),
    ] {
        for inherited in [false, true] {
            let value = if inherited {
                TemplateNameValue::SameAsDivisionTemplates
            } else {
                TemplateNameValue::Changed {
                    literal: literal.to_vec(),
                    fallback_key: literal.is_empty().then(|| key.to_vec()),
                }
            };
            let (mut frame, mut saved) = fixture_name(value);
            let TableView::Complete(TableRows::DivisionTemplates(rows)) =
                &mut *frame.tables[TableId::DivisionTemplates.index()]
            else {
                unreachable!()
            };
            rows[0].name = literal.to_vec();
            rows[0].name_key = Recorded::Value(key.to_vec());
            let template = rows[0].template;
            saved_name_fields(&mut saved, matching);
            for report in [
                report_daily_name(&frame, &saved),
                report_name(&frame, &saved),
            ] {
                assert!(
                    report.differences.is_empty(),
                    "{matching}: {:?}",
                    report.differences
                );
                assert!(
                    report.failed_comparisons.is_empty(),
                    "{:?}",
                    report.failed_comparisons
                );
                assert!(report.outcome().is_ok());
            }
            saved_name_fields(&mut saved, changed);
            let daily = report_daily_name(&frame, &saved);
            let hourly = report_name(&frame, &saved);
            assert!(daily.differences.iter().any(|difference|
                difference.contains(&format!("template {template}.{field}:"))), "{:?}", daily.differences);
            assert!(
                hourly.differences.iter().any(|difference| difference
                    .contains(&format!("template {template}.hourly {field}:"))),
                "{:?}",
                hourly.differences
            );
            for report in [daily, hourly] {
                assert!(report.failed_comparisons.is_empty());
                assert!(report.outcome().is_err());
            }
        }
    }
}

#[test]
fn daily_key_presence_is_independent_of_inactive_hourly_fallback() {
    let (mut frame, mut saved) = fixture_name(TemplateNameValue::Changed {
        literal: b"\0literal".to_vec(),
        fallback_key: None,
    });
    let TableView::Complete(TableRows::DivisionTemplates(rows)) =
        &mut *frame.tables[TableId::DivisionTemplates.index()]
    else {
        unreachable!()
    };
    rows[0].name = b"\0literal".to_vec();
    rows[0].name_key = Recorded::Value(b"\0key".to_vec());
    saved_name_fields(&mut saved, "name=\"\" localization_key=\"\"");
    assert!(report_daily_name(&frame, &saved).outcome().is_ok());
    assert!(report_name(&frame, &saved).outcome().is_ok());

    saved_name_fields(&mut saved, "name=\"\"");
    let daily = report_daily_name(&frame, &saved);
    assert!(
        daily
            .differences
            .iter()
            .any(|difference| difference.contains(".name_key:"))
    );
    assert!(daily.outcome().is_err());
    let hourly = report_name(&frame, &saved);
    assert!(hourly.differences.is_empty());
    assert!(hourly.failed_comparisons.is_empty());
    assert!(hourly.outcome().is_ok());
}

#[test]
fn template_speed_keys_refuse_a_zero_key_and_name_their_selection() {
    let (mut frame, saved) = fixture_name(TemplateNameValue::Changed {
        literal: b"New".to_vec(),
        fallback_key: None,
    });
    for (keys, differs) in [
        (vec![7, 9, 9], false),
        (Vec::new(), false),
        (vec![7, 0], true),
    ] {
        let TableView::Complete(TableRows::DivisionTemplateIcons(rows)) =
            &mut *frame.tables[TableId::DivisionTemplateIcons.index()]
        else {
            unreachable!()
        };
        rows[0].speed_keys = Recorded::Value(Some(keys));
        let report = report_name(&frame, &saved);
        assert_eq!(
            report
                .differences
                .iter()
                .any(|difference| difference.contains(".speed_keys")),
            differs,
            "{:?}",
            report.differences
        );
        assert!(report.limitations.iter().any(|(reason, subjects)| {
            reason.contains("absent from text saves")
                && subjects
                    .iter()
                    .any(|subject| subject == "DivisionTemplateIcons.speed_keys")
        }));
    }
}
