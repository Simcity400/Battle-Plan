#[cfg(test)]
use dashboard_text::template_facts::{country, hq, template};
pub(super) use dashboard_text::template_facts::{country_with, hq_with, template_with};
#[cfg(test)]
use content_reader_localisation::localisation::Localisation;
#[cfg(test)]
use replay_frame::frame::DecodedFrame;

#[cfg(test)]
pub(super) fn fixture() -> DecodedFrame {
    use replay_data::rows::*;
    use viewer_foundation::test_support::{test_frame, test_put as put};
    let mut frame = test_frame(77);
    put(
        &mut frame,
        TableRows::CountryLinks(vec![CountryLinkRow {
            id: 11,
            canonical_slot: 1,
        }]),
    );
    put(
        &mut frame,
        TableRows::Countries(vec![CountryRow {
            id: 11,
            canonical_slot: 1,
            tag: b"AAA".to_vec(),
            ..Default::default()
        }]),
    );
    put(
        &mut frame,
        TableRows::Divisions(vec![DivisionRow {
            id: 1,
            roster_owner: 11,
            template: 52,
            orders_group: 7,
            ..Default::default()
        }]),
    );
    put(
        &mut frame,
        TableRows::LexicalTokens(vec![LexicalTokenRow {
            token: 7,
            literal: b"ModInfantry".to_vec(),
        }]),
    );
    put(
        &mut frame,
        TableRows::DivisionTemplates(vec![DivisionTemplateRow {
            template: 52,
            owner: 11,
            name: b"Example".to_vec(),
            ..Default::default()
        }]),
    );
    put(
        &mut frame,
        TableRows::DivisionTemplateIcons(vec![DivisionTemplateIconRow {
            template: 52,
            country: 11,
            foreign_country: -1,
            settings: Recorded::Value(Some(
                TemplateSettings {
                    priority: -3,
                    names_group: Some(b"MOD_NAMES".to_vec()),
                    forbidden_equipment: Some(vec![7]),
                    ..Default::default()
                }
                .into(),
            )),
            display_name: Recorded::Value(Some(TemplateDisplayName {
                sample_hour: PacketHour::Host,
                value: TemplateNameValue::SameAsDivisionTemplates,
            })),
            layout: Recorded::Value(Some(
                TemplateLayout {
                    regiments: TemplateGrid {
                        columns: 1,
                        rows: 2,
                        cells: vec![
                            TemplateCell::default(),
                            TemplateCell {
                                token: 7,
                                local: 1,
                                occupied: true,
                            },
                        ],
                    },
                    ..Default::default()
                }
                .into(),
            )),
            ..Default::default()
        }]),
    );
    put(
        &mut frame,
        TableRows::CountryTemplateState(vec![CountryTemplateStateRow {
            country: 11,
            selected_staff: 52,
            garrison_priority: Recorded::Value(Some(-7)),
            garrisons: Recorded::Value(Some(
                vec![
                    GarrisonTemplateSelection::default(),
                    GarrisonTemplateSelection {
                        occupied: Some(21),
                        state: Some(0),
                        template: Some(52),
                    },
                ]
                .into(),
            )),
            ..Default::default()
        }]),
    );
    put(
        &mut frame,
        TableRows::OrdersGroupCounterInputs(vec![OrdersGroupCounterInputRow {
            group: 7,
            hq_nearest_front: Recorded::Value(Some(12)),
            template_context: Recorded::Value(Some(
                TemplateHqContext {
                    target: 52,
                    queued: 1,
                    deployment: Some(HqDeployment {
                        country: 11,
                        priority: 2,
                        assembled_manpower: 5,
                        requested_manpower: 9,
                        order: 31,
                        ..Default::default()
                    }),
                    equipment: vec![HqEquipment {
                        variant: 70,
                        amount: 125_000,
                    }],
                    required: vec![HqRequirement {
                        token: 7,
                        amount: 200_000,
                    }],
                    ..Default::default()
                }
                .into(),
            )),
            ..Default::default()
        }]),
    );
    frame
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn existing_division_sections_show_layout_restrictions_garrisons_and_hq_pools() {
        let frame = fixture();
        let text = template(&frame, 52, &Localisation::default()).unwrap().text;
        for fact in [
            "Historical naming group: \"MOD_NAMES\"",
            "Priority: -3",
            "Forbidden equipment: ModInfantry",
            "(0, 0): Empty",
            "(0, 1): ModInfantry (country local)",
        ] {
            assert!(text.contains(fact), "{text}");
        }
        let text = country(&frame, 1, &Localisation::default(), |_| Ok(1))
            .unwrap()
            .text;
        for fact in [
            "Garrison reinforcement priority: -7",
            "Military-staff template: 52",
            "Country default: Inherit",
            "state 0: 52",
        ] {
            assert!(text.contains(fact), "{text}");
        }
        let text = hq(&frame, 7, &Localisation::default()).unwrap().text;
        for fact in [
            "Queued: true",
            "Manpower: 5 / 9",
            "Order: 31",
            "Assembled variant 70: 1.25",
            "Required ModInfantry: 2",
        ] {
            assert!(text.contains(fact), "{text}");
        }
    }

    #[test]
    fn missing_layout_keeps_settings_and_refuses_only_layout() {
        use replay_data::rows::*;
        use viewer_foundation::test_support::test_put as put;
        for layout in [Recorded::Value(None), Recorded::NotRecorded] {
            let mut frame = fixture();
            let mut rows = frame
                .complete::<tables::DivisionTemplateIcons>()
                .unwrap()
                .to_vec();
            rows[0].layout = layout;
            put(&mut frame, TableRows::DivisionTemplateIcons(rows));
            let rendered = template(&frame, 52, &Localisation::default()).unwrap();
            for fact in [
                "MOD_NAMES",
                "Priority: -3",
                "Forbidden equipment: ModInfantry",
                "Locked: false",
                "Role: None",
                "Cached manpower: 0",
                "Template layout: Unavailable",
            ] {
                assert!(rendered.text.contains(fact), "{}", rendered.text);
            }
            assert_eq!(
                rendered
                    .refused
                    .iter()
                    .map(|(field, _)| field.as_str())
                    .collect::<Vec<_>>(),
                ["Template layout"]
            );
        }
    }

    #[test]
    fn missing_settings_keeps_layout_and_model_facts() {
        use replay_data::rows::*;
        use viewer_foundation::test_support::test_put as put;
        for settings in [Recorded::NotRecorded, Recorded::Value(None)] {
            let mut frame = fixture();
            let mut rows = frame
                .complete::<tables::DivisionTemplateIcons>()
                .unwrap()
                .to_vec();
            rows[0].settings = settings;
            put(&mut frame, TableRows::DivisionTemplateIcons(rows));
            let rendered = template(&frame, 52, &Localisation::default()).unwrap();
            assert!(
                rendered
                    .text
                    .contains("(0, 1): ModInfantry (country local)")
            );
            assert!(rendered.text.contains("Composition country: 11"));
            assert_eq!(
                rendered
                    .refused
                    .iter()
                    .map(|(field, _)| field.as_str())
                    .collect::<Vec<_>>(),
                ["Template settings"]
            );
        }
    }

    #[test]
    fn country_fields_refuse_independently_and_keep_staff_configuration() {
        use replay_data::rows::*;
        use viewer_foundation::test_support::test_put as put;
        for garrisons in [Recorded::Value(None), Recorded::NotRecorded] {
            let mut frame = fixture();
            let mut rows = frame
                .complete::<tables::CountryTemplateState>()
                .unwrap()
                .to_vec();
            rows[0].garrisons = garrisons;
            rows[0].garrison_priority = Recorded::Value(None);
            rows[0].lock_all = 1;
            rows[0].next_hq_order = 23;
            put(&mut frame, TableRows::CountryTemplateState(rows));
            let rendered = country(&frame, 1, &Localisation::default(), |_| Ok(1)).unwrap();
            for fact in [
                "Military-staff template: 52",
                "All templates locked: true",
                "Next HQ deployment order: 23",
            ] {
                assert!(rendered.text.contains(fact), "{}", rendered.text);
            }
            assert_eq!(
                rendered
                    .refused
                    .iter()
                    .map(|(field, _)| field.as_str())
                    .collect::<Vec<_>>(),
                ["Garrison reinforcement priority", "Garrison templates"]
            );
        }
        let mut frame = fixture();
        let mut rows = frame
            .complete::<tables::CountryTemplateState>()
            .unwrap()
            .to_vec();
        rows[0].garrison_priority = Recorded::NotRecorded;
        put(&mut frame, TableRows::CountryTemplateState(rows));
        let rendered = country(&frame, 1, &Localisation::default(), |_| Ok(1)).unwrap();
        assert!(rendered.text.contains("state 0: 52"));
        assert_eq!(rendered.refused.len(), 1);
        assert_eq!(rendered.refused[0].0, "Garrison reinforcement priority");
    }

    #[test]
    fn missing_role_or_restriction_spelling_keeps_known_siblings() {
        use replay_data::rows::*;
        use viewer_foundation::test_support::test_put as put;
        let mut frame = fixture();
        let mut rows = frame
            .complete::<tables::DivisionTemplateIcons>()
            .unwrap()
            .to_vec();
        let mut settings = rows[0]
            .settings
            .require("settings")
            .unwrap()
            .as_ref()
            .unwrap()
            .as_ref()
            .clone();
        settings.role = Some(99);
        settings.ai_role = Some(7);
        settings.forbidden_equipment = Some(vec![99, 7]);
        rows[0].settings = Recorded::Value(Some(settings.into()));
        put(&mut frame, TableRows::DivisionTemplateIcons(rows));
        let rendered = template(&frame, 52, &Localisation::default()).unwrap();
        for fact in [
            "Role: Unavailable",
            "AI role: ModInfantry",
            "Forbidden equipment: Unavailable, ModInfantry",
            "Cached manpower: 0",
            "(0, 0): Empty",
        ] {
            assert!(rendered.text.contains(fact), "{}", rendered.text);
        }
        assert_eq!(
            rendered
                .refused
                .iter()
                .map(|(field, _)| field.as_str())
                .collect::<Vec<_>>(),
            ["Role", "Forbidden equipment 0"]
        );
    }

    #[test]
    fn missing_nearest_front_keeps_hq_context_and_pools() {
        use replay_data::rows::*;
        use viewer_foundation::test_support::test_put as put;
        for front in [Recorded::NotRecorded, Recorded::Value(None)] {
            let mut frame = fixture();
            let mut rows = frame
                .complete::<tables::OrdersGroupCounterInputs>()
                .unwrap()
                .to_vec();
            rows[0].hq_nearest_front = front;
            put(&mut frame, TableRows::OrdersGroupCounterInputs(rows));
            let rendered = hq(&frame, 7, &Localisation::default()).unwrap();
            for fact in [
                "HQ target template: 52",
                "Manpower: 5 / 9",
                "Assembled variant 70: 1.25",
                "Required ModInfantry: 2",
                "Nearest front province: Unavailable",
            ] {
                assert!(rendered.text.contains(fact), "{}", rendered.text);
            }
            assert!(!rendered.text.contains("Nearest front province: None"));
            assert_eq!(
                rendered
                    .refused
                    .iter()
                    .map(|(field, _)| field.as_str())
                    .collect::<Vec<_>>(),
                ["Nearest front province"]
            );
        }
    }

    #[test]
    fn model_read_failure_refuses_choice_and_exclusions_without_hiding_other_inputs() {
        use replay_data::rows::*;
        use viewer_foundation::test_support::test_put as put;
        let mut frame = fixture();
        let mut rows = frame
            .complete::<tables::DivisionTemplateIcons>()
            .unwrap()
            .to_vec();
        rows[0].model_unreadable = 1;
        rows[0].model_entity.clear();
        rows[0].excluded_variants.clear();
        rows[0].model_category = Some(23);
        put(&mut frame, TableRows::DivisionTemplateIcons(rows));
        let rendered = template(&frame, 52, &Localisation::default()).unwrap();
        for fact in [
            "Model choice: Unavailable",
            "Excluded variants: Unavailable",
            "Model category: Some(23)",
            "Composition country: 11",
            "Foreign country: -1",
            "MOD_NAMES",
            "Priority: -3",
        ] {
            assert!(rendered.text.contains(fact), "{}", rendered.text);
        }
        assert!(!rendered.text.contains("Excluded variants: []"));
        assert_eq!(
            rendered
                .refused
                .iter()
                .map(|(field, _)| field.as_str())
                .collect::<Vec<_>>(),
            ["Excluded variants", "Model choice"]
        );
    }

    #[test]
    fn text_validation_refuses_individual_names_definitions_and_choices() {
        use replay_data::rows::*;
        use viewer_foundation::test_support::test_put as put;
        use viewer_foundation::tooltip_text::{NoIcons, validate};
        let mut frame = fixture();
        let mut rows = frame
            .complete::<tables::DivisionTemplateIcons>()
            .unwrap()
            .to_vec();
        let mut settings = rows[0]
            .settings
            .require("settings")
            .unwrap()
            .as_ref()
            .unwrap()
            .as_ref()
            .clone();
        settings.names_group = Some(b"\x12GER".to_vec());
        settings.role = Some(7);
        rows[0].settings = Recorded::Value(Some(settings.into()));
        rows[0].model_entity = b"\x12GER".to_vec();
        put(&mut frame, TableRows::DivisionTemplateIcons(rows));
        put(
            &mut frame,
            TableRows::LexicalTokens(vec![LexicalTokenRow {
                token: 7,
                literal: b"\x12GER".to_vec(),
            }]),
        );
        let rendered = template_with(&frame, 52, &Localisation::default(), &|text| {
            validate(&NoIcons, text)
        })
        .unwrap();
        for field in [
            "Historical naming group",
            "Role",
            "Forbidden equipment 0",
            "Model choice",
            "Regiments (0, 1)",
        ] {
            assert!(
                rendered.refused.iter().any(|(name, _)| name == field),
                "{:?}",
                rendered.refused
            );
        }
        for fact in [
            "Priority: -3",
            "Cached manpower: 0",
            "(0, 0): Empty",
            "Composition country: 11",
        ] {
            assert!(rendered.text.contains(fact), "{}", rendered.text);
        }
        assert!(!rendered.text.contains("\u{12}GER"));
        let rendered = hq_with(&frame, 7, &Localisation::default(), &|text| {
            validate(&NoIcons, text)
        })
        .unwrap();
        assert!(
            rendered
                .refused
                .iter()
                .any(|(name, _)| name == "HQ requirement 0")
        );
        assert!(rendered.text.contains("Required Unavailable: 2"));
        assert!(rendered.text.contains("Manpower: 5 / 9"));
    }
}
