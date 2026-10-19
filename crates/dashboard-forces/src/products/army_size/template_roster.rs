//! Reached-hour template and configuration sections in the Army breakdown.
//! Roster ownership, never deployed membership, selects definitions and groups.
use super::*;
use dashboard_text::template_facts;

pub(super) const TABLES: &[TableId] = &[
    TableId::Countries,
    TableId::CountryLinks,
    TableId::DivisionTemplates,
    TableId::DivisionTemplateIcons,
    TableId::CountryTemplateState,
    TableId::OrdersGroups,
    TableId::OrdersGroupCounterInputs,
    TableId::LexicalTokens,
];

pub(super) fn query(
    source: &Source,
    params: &ArmySizeParams,
    context: &LoadContext,
    cancelled: &AtomicBool,
) -> io::Result<ArmySizeReply> {
    let (tag, hour) = params
        .template_at
        .as_ref()
        .ok_or_else(|| dashboard_core::invalid("Template roster country/hour missing"))?;
    let branch = params
        .branch_id
        .parse()
        .map_err(|_| dashboard_core::invalid("Invalid branch"))?;
    let frame = dashboard_base::prepared::query::progressive::frame_at(
        source, branch, *hour, TABLES, cancelled,
    )?
    .ok_or_else(|| dashboard_core::invalid("Template roster hour is not recorded"))?;
    let countries = frame.complete::<tables::Countries>()?;
    let links = frame.complete::<tables::CountryLinks>()?;
    let country = dashboard_text::metadata::tagged_country(countries, links, tag);
    let TaggedCountry::Found(_, slot) = country else {
        return Err(dashboard_core::invalid(
            "Template roster country is unavailable at this hour",
        ));
    };
    let content = context.content.as_ref().ok_or_else(|| {
        dashboard_core::invalid("Template roster needs the recorded content stack")
    })?;
    let loc = dashboard_base::content::localisation(source, content)?;
    let canonical =
        |raw| link_slot(links, raw).ok_or_else(|| format!("CountryLinks country {raw} is absent"));
    let mut sections = Vec::new();
    let mut warnings = Vec::new();
    let mut append = |label: &str, value: Result<template_facts::Rendered, String>| match value {
        Ok(rendered) => {
            warnings.extend(
                rendered
                    .refused
                    .into_iter()
                    .map(|(field, reason)| format!("{label}/{field}: {reason}")),
            );
            if !rendered.text.is_empty() {
                sections.push(rendered.text);
            }
        }
        Err(reason) => {
            sections.push(format!("{label}\nUnavailable"));
            warnings.push(format!("{label}: {reason}"));
        }
    };
    append(
        "Garrison and staff templates",
        template_facts::country(&frame, slot, &loc, canonical),
    );
    let roster = frame.complete::<tables::DivisionTemplates>();
    match roster {
        Err(error) => append("Template roster", Err(error.to_string())),
        Ok(rows) => {
            let mut selected = Vec::new();
            for row in rows {
                match canonical(row.owner) {
                    Ok(owner) if owner == slot => selected.push(row.template),
                    Ok(_) => (),
                    Err(reason) => append("Template roster owner", Err(reason)),
                }
            }
            if selected.is_empty() {
                append(
                    "Template roster",
                    Ok(template_facts::Rendered::text("No division templates.")),
                );
            }
            let icons = frame.complete::<tables::DivisionTemplateIcons>();
            selected.sort_by_key(|handle| {
                icons
                    .as_ref()
                    .ok()
                    .and_then(|rows| rows.iter().find(|r| r.template == *handle))
                    .and_then(|r| r.settings.require("template settings").ok())
                    .and_then(Option::as_ref)
                    .map(|s| s.roster_ordinal)
            });
            for handle in selected {
                dashboard_text::metadata::check_cancel(cancelled)?;
                append(
                    "Template settings and layout",
                    template_facts::template(&frame, handle, &loc),
                );
            }
        }
    }
    match frame.complete::<tables::OrdersGroups>() {
        Err(error) => append("HQ groups", Err(error.to_string())),
        Ok(groups) => {
            for group in groups {
                match canonical(group.country) {
                    Ok(owner) if owner == slot => append(
                        "HQ deployment",
                        template_facts::hq(&frame, group.handle, &loc),
                    ),
                    Ok(_) => (),
                    Err(reason) => append("HQ group owner", Err(reason)),
                }
            }
        }
    }
    let reply = ArmySizeReply {
        template_sections: Some(sections),
        branch_id: params.branch_id.clone(),
        windows: Vec::new(),
        window: None,
        interval: None,
        available: true,
        measures: Vec::new(),
        types: Vec::new(),
        series: Vec::new(),
        warnings,
    };
    dashboard_core::transport::json_len(&reply, dashboard_base::products::MAX_PRODUCT_REPLY_BYTES)
        .ok_or_else(reply_bound)?;
    Ok(reply)
}

#[cfg(test)]
mod tests {
    use super::*;
    use replay_data::rows::*;
    use replay_frame::frame::TableView;

    #[test]
    fn production_army_query_shows_unused_templates_and_empty_country_configuration_without_division_inputs()
     {
        engine_executable::executable_tables::initialize_for_tests().expect(
            "template roster content requires BATTLE_PLAN_GAME_DIR with the supported installation",
        );
        let mut frame = crate::products::army_size::test_fixtures::frame(100);
        *frame.tables[TableId::Divisions.index()] = TableView::Unavailable {
            reason: "not a roster dependency".into(),
        };
        let templates = [(52, "Unused with holes"), (53, "Unused HQ")];
        *frame.tables[TableId::DivisionTemplates.index()] =
            TableView::Complete(TableRows::DivisionTemplates(
                templates
                    .iter()
                    .map(|&(template, name)| DivisionTemplateRow {
                        template,
                        owner: 30,
                        name: name.as_bytes().to_vec(),
                        ..Default::default()
                    })
                    .collect(),
            ));
        *frame.tables[TableId::DivisionTemplateIcons.index()] =
            TableView::Complete(TableRows::DivisionTemplateIcons(
                templates
                    .iter()
                    .enumerate()
                    .map(|(index, &(template, _))| DivisionTemplateIconRow {
                        template,
                        country: 30,
                        foreign_country: -1,
                        display_name: Recorded::Value(Some(TemplateDisplayName {
                            sample_hour: PacketHour::Host,
                            value: TemplateNameValue::SameAsDivisionTemplates,
                        })),
                        settings: Recorded::Value(Some(
                            TemplateSettings {
                                roster_ordinal: index as u32,
                                names_group: Some(b"MOD_UNUSED".to_vec()),
                                priority: -3,
                                role: Some(7),
                                forbidden_equipment: Some(vec![7]),
                                total_manpower: 17,
                                ..Default::default()
                            }
                            .into(),
                        )),
                        layout: Recorded::Value(Some(
                            TemplateLayout {
                                regiments: TemplateGrid {
                                    columns: 1,
                                    rows: 1,
                                    cells: vec![TemplateCell::default()],
                                },
                                ..Default::default()
                            }
                            .into(),
                        )),
                        ..Default::default()
                    })
                    .collect(),
            ));
        *frame.tables[TableId::LexicalTokens.index()] =
            TableView::Complete(TableRows::LexicalTokens(vec![LexicalTokenRow {
                token: 7,
                literal: b"ModRole".to_vec(),
            }]));
        *frame.tables[TableId::CountryTemplateState.index()] =
            TableView::Complete(TableRows::CountryTemplateState(vec![
                CountryTemplateStateRow {
                    country: 30,
                    selected_staff: 52,
                    garrison_priority: Recorded::Value(Some(-7)),
                    garrisons: Recorded::Value(Some(
                        vec![GarrisonTemplateSelection {
                            occupied: Some(20),
                            state: Some(0),
                            template: Some(52),
                        }]
                        .into(),
                    )),
                    ..Default::default()
                },
            ]));
        *frame.tables[TableId::OrdersGroups.index()] =
            TableView::Complete(TableRows::OrdersGroups(vec![OrdersGroupRow {
                handle: 99,
                country: 30,
                ..Default::default()
            }]));
        *frame.tables[TableId::OrdersGroupCounterInputs.index()] =
            TableView::Complete(TableRows::OrdersGroupCounterInputs(vec![
                OrdersGroupCounterInputRow {
                    group: 99,
                    hq_nearest_front: Recorded::Value(Some(12)),
                    template_context: Recorded::Value(Some(
                        TemplateHqContext {
                            target: 53,
                            queued: 1,
                            deployment: Some(HqDeployment {
                                country: 30,
                                assembled_manpower: 2,
                                requested_manpower: 10,
                                ..Default::default()
                            }),
                            ..Default::default()
                        }
                        .into(),
                    )),
                    ..Default::default()
                },
            ]));
        let mut partial = frame.clone();
        partial.hour = 102;
        let mut icons = partial
            .complete::<tables::DivisionTemplateIcons>()
            .unwrap()
            .to_vec();
        icons[0].model_unreadable = 1;
        icons[0].model_category = Some(23);
        icons[0].layout = Recorded::Value(None);
        // Current captures encode guarded failures as None; the formatter's
        // direct tests also cover fields absent from a recorded schema.
        icons[1].layout = Recorded::Value(None);
        *partial.tables[TableId::DivisionTemplateIcons.index()] =
            TableView::Complete(TableRows::DivisionTemplateIcons(icons));
        let mut groups = partial
            .complete::<tables::OrdersGroupCounterInputs>()
            .unwrap()
            .to_vec();
        groups[0].hq_nearest_front = Recorded::Value(None);
        *partial.tables[TableId::OrdersGroupCounterInputs.index()] =
            TableView::Complete(TableRows::OrdersGroupCounterInputs(groups));
        let mut configuration = partial
            .complete::<tables::CountryTemplateState>()
            .unwrap()
            .to_vec();
        configuration[0].garrisons = Recorded::Value(None);
        configuration[0].garrison_priority = Recorded::Value(None);
        configuration[0].lock_all = 1;
        configuration[0].next_hq_order = 23;
        *partial.tables[TableId::CountryTemplateState.index()] =
            TableView::Complete(TableRows::CountryTemplateState(configuration));
        let mut empty = frame.clone();
        empty.hour = 101;
        *empty.tables[TableId::DivisionTemplates.index()] =
            TableView::Complete(TableRows::DivisionTemplates(Vec::new()));
        *empty.tables[TableId::DivisionTemplateIcons.index()] =
            TableView::Complete(TableRows::DivisionTemplateIcons(Vec::new()));
        let mut refused_settings = frame.clone();
        refused_settings.hour = 103;
        let mut icons = refused_settings
            .complete::<tables::DivisionTemplateIcons>()
            .unwrap()
            .to_vec();
        for icon in &mut icons {
            icon.settings = Recorded::Value(None);
        }
        *refused_settings.tables[TableId::DivisionTemplateIcons.index()] =
            TableView::Complete(TableRows::DivisionTemplateIcons(icons));
        let root =
            std::env::temp_dir().join(format!("replay-template-roster-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&root).unwrap();
        let (source, context) = dashboard_base::test_fixtures::source_with_content(
            vec![frame, empty, partial, refused_settings],
            &root,
        );
        let mut params = ArmySizeParams {
            template_at: Some(("empty".into(), 100)),
            branch_id: "1".into(),
            tags: Vec::new(),
            top: None,
            measure: ArmyMeasure::Divisions,
            count_for: ArmyAttribution::Owner,
            by_type: false,
            window: None,
        };
        let cancel = AtomicBool::new(false);
        let reply = prepared::query(&source, &params, &context, &cancel)
            .unwrap()
            .unwrap();
        assert!(reply.warnings.is_empty(), "{:?}", reply.warnings);
        let text = reply.template_sections.unwrap().join("\n");
        for fact in [
            "Unused with holes",
            "Unused HQ",
            "MOD_UNUSED",
            "(0, 0): Empty",
            "Role: ModRole",
            "Forbidden equipment: ModRole",
            "Cached manpower: 17",
            "Garrison reinforcement priority: -7",
            "Military-staff template: 52",
            "state 0: 52",
            "HQ target template: 53",
            "Manpower: 2 / 10",
        ] {
            assert!(text.contains(fact), "missing {fact}: {text}");
        }
        params.template_at = Some(("empty".into(), 101));
        let reply = prepared::query(&source, &params, &context, &cancel)
            .unwrap()
            .unwrap();
        let text = reply.template_sections.unwrap().join("\n");
        for fact in [
            "No division templates.",
            "Garrison reinforcement priority: -7",
            "HQ target template: 53",
            "Manpower: 2 / 10",
        ] {
            assert!(text.contains(fact), "missing {fact}: {text}");
        }
        params.template_at = Some(("empty".into(), 102));
        let reply = prepared::query(&source, &params, &context, &cancel)
            .unwrap()
            .unwrap();
        assert_eq!(reply.warnings.len(), 7, "{:?}", reply.warnings);
        for field in [
            "Template settings and layout/Template layout:",
            "Template settings and layout/Model choice:",
            "Template settings and layout/Excluded variants:",
            "HQ deployment/Nearest front province:",
            "Garrison and staff templates/Garrison templates:",
            "Garrison and staff templates/Garrison reinforcement priority:",
        ] {
            assert!(
                reply
                    .warnings
                    .iter()
                    .any(|warning| warning.starts_with(field)),
                "{:?}",
                reply.warnings
            );
        }
        let text = reply.template_sections.unwrap().join("\n");
        for fact in [
            "Unused with holes",
            "Unused HQ",
            "MOD_UNUSED",
            "Priority: -3",
            "Role: ModRole",
            "Forbidden equipment: ModRole",
            "Cached manpower: 17",
            "Military-staff template: 52",
            "All templates locked: true",
            "Next HQ deployment order: 23",
            "HQ target template: 53",
            "Template layout: Unavailable",
            "Model choice: Unavailable",
            "Excluded variants: Unavailable",
            "Model category: Some(23)",
            "Composition country: 30",
            "Nearest front province: Unavailable",
            "Garrison templates: Unavailable",
        ] {
            assert!(text.contains(fact), "missing {fact}: {text}");
        }
        params.template_at = Some(("empty".into(), 103));
        let reply = prepared::query(&source, &params, &context, &cancel)
            .unwrap()
            .unwrap();
        assert_eq!(reply.warnings.len(), 2, "{:?}", reply.warnings);
        assert!(reply.warnings.iter().all(|warning| warning.starts_with(
            "Template settings and layout/Template settings: template settings acquisition refused"
        )));
        let text = reply.template_sections.unwrap().join("\n");
        for fact in [
            "Unused with holes",
            "Unused HQ",
            "(0, 0): Empty",
            "Template settings: Unavailable",
            "Composition country: 30",
            "Military-staff template: 52",
            "HQ target template: 53",
            "Manpower: 2 / 10",
        ] {
            assert!(text.contains(fact), "missing {fact}: {text}");
        }
        drop(source);
        std::fs::remove_dir_all(root).unwrap();
    }
}
