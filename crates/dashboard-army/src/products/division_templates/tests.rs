use super::*;
use replay_data::rows::*;
use replay_frame::frame::TableView;

fn settings(ordinal: u32, obsolete: bool) -> Recorded<Option<Arc<TemplateSettings>>> {
    Recorded::Value(Some(Arc::new(TemplateSettings {
        roster_ordinal: ordinal,
        priority: 2,
        locked: 1,
        obsolete: u8::from(obsolete),
        obsolete_change_date: if obsolete { 60_000_000 } else { engine_native::cgame_date::NULL_RAW_HOUR },
        total_manpower: 10_000,
        training_time: 90,
        ..Default::default()
    })))
}

/// GER (raw 11) owns templates 3 (unused), 5 and 9; another country (raw 77) owns 4.
pub fn frame() -> DecodedFrame {
    let mut f = dashboard_base::test_fixtures::source(1, 60_000_001, None);
    *f.tables[TableId::CountryLinks.index()] = TableView::Complete(TableRows::CountryLinks(vec![
        CountryLinkRow { id: 11, canonical_slot: 7 },
        CountryLinkRow { id: 77, canonical_slot: 8 },
        CountryLinkRow { id: 9001, canonical_slot: 7 },
    ]));
    *f.tables[TableId::DivisionTemplates.index()] = TableView::Complete(TableRows::DivisionTemplates(vec![
        DivisionTemplateRow { template: 3, owner: 11, ..Default::default() },
        DivisionTemplateRow { template: 4, owner: 77, ..Default::default() },
        DivisionTemplateRow { template: 5, owner: 9001, ..Default::default() },
        DivisionTemplateRow { template: 9, owner: 11, ..Default::default() },
    ]));
    *f.tables[TableId::DivisionTemplateIcons.index()] = TableView::Complete(TableRows::DivisionTemplateIcons(vec![
        DivisionTemplateIconRow {
            template: 3,
            settings: settings(2, false),
            base_width: Recorded::Value(Some(1_800_000)),
            grids: Recorded::Value(TemplateGrids {
                regiments: vec![TemplateSubunit { token: 21, local: 0, count: 9 }],
                support: vec![TemplateSubunit { token: 22, local: 1, count: 1 }],
                regimental_support: vec![],
            }),
            ..Default::default()
        },
        DivisionTemplateIconRow { template: 5, settings: settings(0, false), ..Default::default() },
        DivisionTemplateIconRow { template: 9, settings: Recorded::Value(None), ..Default::default() },
    ]));
    *f.tables[TableId::LexicalTokens.index()] = TableView::Complete(TableRows::LexicalTokens(vec![
        LexicalTokenRow { token: 21, literal: b"line_unit".to_vec() },
        LexicalTokenRow { token: 22, literal: b"support_unit".to_vec() },
    ]));
    // Two divisions of template 5 on GER's roster, one hosted for another country,
    // one intelligence copy that never counts.
    *f.tables[TableId::Divisions.index()] = TableView::Complete(TableRows::Divisions(vec![
        DivisionRow { id: 1, roster_owner: 11, template: 5, ..Default::default() },
        DivisionRow { id: 2, roster_owner: 11, expeditionary_owner: 77, template: 5, ..Default::default() },
        DivisionRow { id: 3, roster_owner: 11, template: 5, reveal_skip: 1, ..Default::default() },
    ]));
    f
}

fn reply(frame: &DecodedFrame, count_for: ArmyAttribution) -> DivisionTemplatesReply {
    let mut refusals = Refusals::default();
    let mut reply = build(frame, "GER", count_for, None, &|_| Err("no artwork".into()), &mut refusals, &AtomicBool::new(false)).unwrap();
    reply.warnings = refusals.lines();
    reply
}

#[test]
fn every_owned_template_lists_in_roster_order_with_unused_ones_at_zero() {
    let r = reply(&frame(), ArmyAttribution::Owner);
    assert_eq!(r.templates.iter().map(|t| t.template.as_str()).collect::<Vec<_>>(), ["5", "3", "9"],
        "roster order, a template without settings after the rest");
    assert_eq!(r.templates[0].fielded, Some(1), "the owner rule counts the expeditionary division for its owner");
    assert_eq!(r.templates[1].fielded, Some(0), "an unused template is listed with none fielded");
    let host = reply(&frame(), ArmyAttribution::Host);
    assert_eq!(host.templates[0].fielded, Some(2), "the host rule counts both, never the intelligence copy");
}

#[test]
fn composition_settings_and_width_keep_recorded_values_and_named_gaps() {
    let r = reply(&frame(), ArmyAttribution::Owner);
    let unused = &r.templates[1];
    assert_eq!(unused.base_width_raw.as_deref(), Some("1800000"));
    assert_eq!(unused.units.len(), 2);
    assert_eq!((unused.units[0].key.as_deref(), unused.units[0].count, unused.units[0].support), (Some("line_unit"), 9, false));
    assert!(unused.units[1].support && unused.units[1].local);
    assert!(unused.units.iter().all(|u| u.name.is_none() && u.name_missing.is_some()), "names need the content stack");
    assert_eq!((unused.priority, unused.locked, unused.obsolete, unused.obsolete_hour), (Some(2), Some(true), Some(false), None));
    assert!(unused.priority_name.is_none() && unused.priority_missing.is_some(), "a stored priority is never shown as its number");
    let refused = &r.templates[2];
    assert!(refused.settings_missing.is_some() && refused.priority.is_none(), "a refused settings read is never default settings");
    assert!(refused.name_missing.is_some(), "the name is missing on its own");
    assert!(r.warnings.iter().any(|w| w.contains("refused at capture")));
}

#[test]
fn an_unread_census_leaves_fielded_counts_unknown_and_an_absent_country_is_named() {
    let mut f = frame();
    *f.tables[TableId::Divisions.index()] = TableView::Unavailable { reason: "attachment interval".into() };
    let r = reply(&f, ArmyAttribution::Owner);
    assert!(r.fielded_missing.is_some());
    assert!(r.templates.iter().all(|t| t.fielded.is_none()), "unknown, never zero");
    let mut absent = frame();
    *absent.tables[TableId::Countries.index()] = TableView::Complete(TableRows::Countries(vec![]));
    let mut refusals = Refusals::default();
    assert!(build(&absent, "GER", ArmyAttribution::Owner, None, &|_| Err(String::new()), &mut refusals, &AtomicBool::new(false)).unwrap().absent);
}


#[test]
fn a_composition_whose_model_read_failed_is_unknown_never_empty() {
    let mut f = frame();
    let TableView::Complete(TableRows::DivisionTemplateIcons(icons)) = &mut *f.tables[TableId::DivisionTemplateIcons.index()] else {
        panic!()
    };
    // As capture keeps a failed model read: empty grids, no width, the flag set.
    icons[0].model_unreadable = 1;
    icons[0].grids = Recorded::Value(TemplateGrids::default());
    icons[0].base_width = Recorded::Value(None);
    let r = reply(&f, ArmyAttribution::Owner);
    let unread = &r.templates[1];
    assert_eq!(unread.template, "3");
    assert!(unread.units.is_empty());
    assert_eq!(unread.units_missing.as_deref(), Some(MODEL_UNREADABLE));
    assert_eq!(unread.base_width_raw, None);
    assert_eq!((unread.priority, unread.locked), (Some(2), Some(true)), "settings read apart stand");
    assert!(r.warnings.iter().any(|w| w.contains(MODEL_UNREADABLE)));
}

#[test]
fn an_unlinked_country_leaves_its_divisions_templates_unknown_and_its_templates_out() {
    let mut f = frame();
    let TableView::Complete(TableRows::Divisions(divisions)) = &mut *f.tables[TableId::Divisions.index()] else {
        panic!()
    };
    divisions.push(DivisionRow { id: 4, roster_owner: 11, expeditionary_owner: 555, template: 5, ..Default::default() });
    let TableView::Complete(TableRows::DivisionTemplates(templates)) = &mut *f.tables[TableId::DivisionTemplates.index()] else {
        panic!()
    };
    templates.insert(3, DivisionTemplateRow { template: 6, owner: 555, ..Default::default() });
    let r = reply(&f, ArmyAttribution::Owner);
    assert_eq!(r.templates.iter().map(|t| t.template.as_str()).collect::<Vec<_>>(), ["5", "3", "9"]);
    assert_eq!(r.templates[0].fielded, None, "the division may count for GER");
    assert_eq!(r.templates[1].fielded, Some(0), "no such division is of template 3");
    assert!(r.fielded_missing.is_none());
    assert!(r.warnings.iter().any(|w| w.contains(UNLINKED_DIVISION)));
    assert!(r.warnings.iter().any(|w| w.contains(UNLINKED_TEMPLATE)));
    let host = reply(&f, ArmyAttribution::Host);
    assert_eq!(host.templates[0].fielded, Some(3), "its roster is linked");
    assert!(!host.warnings.iter().any(|w| w.contains(UNLINKED_DIVISION)));
}
