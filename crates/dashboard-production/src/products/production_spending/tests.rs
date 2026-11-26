use super::{
    Amount, Context, Design, Domain, ProductionSpendingScope, ProductionSpendingUnknown as Why, RepairRead, Row, Span, SpanRead, StockVariantName,
    breakdown::{IndexedSpan, SpendingIndex},
};
use dashboard_base::products::breakdown::{BreakdownAmount, BreakdownState};
use replay_data::rows::OutputPurpose;
use std::collections::{BTreeMap, BTreeSet, HashMap};

fn span(first: i32, last: i32, breaks: bool) -> Span {
    Span { run: 0, first, last, id: 7, breaks }
}

fn row(spent: Option<i64>) -> Option<Row> {
    Some(Row { spent, reason: 3, unproven: None })
}

fn shifted<T>(pieces: Vec<(i32, i32, T)>, by: i32) -> Vec<(i32, i32, T)> {
    pieces.into_iter().map(|(first, last, value)| (first + by, last + by, value)).collect()
}

fn read(span: Span) -> SpanRead {
    let by = span.first;
    let mut rows = BTreeMap::new();
    rows.insert((false, 11, Some(OutputPurpose::New.raw())), shifted(vec![(0, 4, None), (5, 19, row(Some(100_000))), (20, 29, row(None))], by));
    rows.insert((true, 12, None), shifted(vec![(0, 7, None), (8, 29, row(Some(40_000)))], by));
    rows.insert((true, 13, None), shifted(vec![(0, 15, None), (16, 29, row(Some(2_000)))], by));
    SpanRead {
        span,
        lines: shifted(vec![(0, 9, Context::Known), (10, 14, Context::NotShown), (15, 29, Context::Known)], by),
        rocket: shifted(vec![(0, 19, Context::Known), (20, 24, Context::Unknown(Why::RocketTableUnavailable)), (25, 29, Context::Known)], by),
        country: shifted(vec![(0, 9, Some((Some(100_000), Some(90_000)))), (10, 14, None), (15, 29, Some((Some(150_000), None)))], by),
        rows,
        archetypes: BTreeMap::new(),
        archetype_since: BTreeMap::new(),
        equipment_observations: BTreeSet::new(),
        rocket_observations: BTreeSet::new(),
        repairs: None,
    }
}

#[test]
fn a_design_is_zero_without_a_row_and_rocket_and_the_total_sum_their_parts_or_say_why_not() {
    let read = read(span(0, 29, false));
    assert_eq!(read.amounts(&(false, 11, Some(OutputPurpose::New.raw())), |row| row.spent), vec![
        (0, 4, Amount::Known(0)),
        (5, 9, Amount::Known(100_000)),
        (10, 14, Amount::Unknown(Why::NotRecorded)),
        (15, 19, Amount::Known(100_000)),
        (20, 29, Amount::Unknown(Why::LostOutput)),
    ]);
    assert_eq!(read.amounts(&(false, 99, Some(OutputPurpose::New.raw())), |row| row.spent)[0], (0, 9, Amount::Known(0)));
    assert_eq!(read.rocket_total(), vec![
        (0, 7, Amount::Known(0)),
        (8, 15, Amount::Known(40_000)),
        (16, 19, Amount::Known(42_000)),
        (20, 24, Amount::Unknown(Why::RocketTableUnavailable)),
        (25, 29, Amount::Known(42_000)),
    ]);
    assert_eq!(read.equipment(false), vec![(0, 9, Amount::Known(90_000)), (10, 14, Amount::Unknown(Why::NotRecorded)), (15, 29, Amount::Unknown(Why::SpendingUnavailable))]);
    let total = super::zip(&read.equipment(true), &read.rocket_total(), |a, b| a.plus(*b));
    assert_eq!(total[0], (0, 7, Amount::Known(100_000)));
    assert_eq!(total.last(), Some(&(25, 29, Amount::Known(192_000))));
}

#[test]
fn a_repaired_design_is_zero_where_the_split_is_known_and_unknown_where_it_is_not() {
    let repairs = RepairRead::new(
        vec![(0, 9, Some((Some(2_400), true))), (10, 19, Some((Some(2_400), false))), (20, 24, Some((None, false))), (25, 29, None)],
        BTreeMap::from([(Some(5), vec![(0, 4, None), (5, 29, Some(Some(1_000)))]), (None, vec![(0, 29, Some(Some(1_400)))])]),
    );
    assert_eq!(repairs.total()[0], (0, 19, Amount::Known(2_400)));
    assert_eq!(repairs.part(Some(5)), vec![
        (0, 4, Amount::Known(0)),
        (5, 9, Amount::Known(1_000)),
        (10, 24, Amount::Unknown(Why::SpendingUnavailable)),
        (25, 29, Amount::Unknown(Why::NotRecorded)),
    ]);
    assert_eq!(repairs.part(Some(6))[0], (0, 9, Amount::Known(0)));
    assert_eq!(repairs.part(None)[0], (0, 9, Amount::Known(1_400)));
}

#[test]
fn the_rocket_source_is_the_one_building_content_classifies_and_ambiguity_names_none() {
    use content_reader_key_names::buildings::{Buildings, FactoryFlags};
    let mut buildings = Buildings {
        declaration_order: vec!["mod_launch_works".into()],
        factory_flags: BTreeMap::from([("mod_launch_works".into(), FactoryFlags { rocket_production: 7, ..Default::default() })]),
        ..Default::default()
    };
    assert_eq!(super::rocket_name(&buildings, None).unwrap(), ("mod_launch_works".into(), None));
    buildings.ambiguous_factory_types.insert("mod_launch_works".into());
    assert!(super::rocket_name(&buildings, None).unwrap_err().contains("ambiguous"));
    buildings.ambiguous_factory_types.clear();
    buildings.declaration_order.push("mod_other_launch".into());
    buildings.factory_flags.insert("mod_other_launch".into(), FactoryFlags { rocket_production: 1, ..Default::default() });
    assert!(super::rocket_name(&buildings, None).unwrap_err().contains("2 content building definitions"));
}

#[test]
fn breakdown_scopes_answer_at_hour_with_exact_fractions_and_quiet_through_bounds() {
    let mut rows = BTreeMap::new();
    rows.insert((false, 1, Some(OutputPurpose::New.raw())), vec![(0, 9, row(Some(250_000))), (10, 100, row(Some(500_000)))]);
    rows.insert((false, 2, Some(OutputPurpose::Converted.raw())), vec![(0, 100, row(Some(100_000)))]);
    rows.insert((false, 3, Some(OutputPurpose::New.raw())), vec![(0, 100, row(Some(400_000)))]);
    rows.insert((false, 3, Some(OutputPurpose::Refit.raw())), vec![(0, 100, row(Some(50_000)))]);
    let repairs = RepairRead::new(
        vec![(0, 23, Some((Some(100_000), true))), (24, 100, Some((Some(0), true)))],
        BTreeMap::from([(Some(3), vec![(0, 23, Some(Some(100_000))), (24, 100, Some(Some(0)))]), (None, vec![(0, 100, Some(Some(0)))])]),
    );
    let span_read = SpanRead {
        span: span(0, 100, false),
        lines: vec![(0, 100, Context::Known)],
        rocket: vec![(0, 100, Context::Known)],
        country: vec![(0, 100, Some((Some(1_050_000), Some(1_000_000))))],
        rows,
        archetypes: BTreeMap::new(),
        archetype_since: BTreeMap::from([(2, 0)]),
        equipment_observations: BTreeSet::new(),
        rocket_observations: BTreeSet::new(),
        repairs: Some(repairs),
    };
    let design = |type_key: &str, name: &str, domain: Domain, label: &str, first_hour: i32| Design {
        type_key: type_key.into(),
        name: Some(name.into()),
        domain: Some(domain),
        names: vec![StockVariantName { first_hour, name: Some(label.into()), known: true, design: None }],
        typed_since: Some(first_hour),
    };
    let designs = HashMap::from([
        (1, design("infantry_equipment", "Infantry Equipment", Domain::Land, "Rifle I", 0)),
        (2, design("fighter", "Fighter", Domain::Air, "Fighter I", 50)),
        (3, design("destroyer", "Destroyer", Domain::Naval, "DD I", 0)),
    ]);
    let index = SpendingIndex {
        from: 0,
        to: 100,
        country_label: "GER".into(),
        unknown: Vec::new(),
        unresolved: Vec::new(),
        spans: vec![IndexedSpan::new(span_read, designs, Vec::new())],
        rocket_source: None,
        warnings: Vec::new(),
    };

    let mil = index.at(20, ProductionSpendingScope::Military, false);
    assert_eq!(mil.through, 49, "quiet stretch holds until Fighter I's naming boundary at 50");
    assert_eq!(mil.breakdown.total.state, BreakdownState::Known);
    assert_eq!(mil.breakdown.total.amount, Some(BreakdownAmount { numerator: "6".into(), denominator: "1".into() }));
    let air = mil.breakdown.parts.iter().find(|part| part.key == "air").expect("air source");
    assert_eq!(air.children[0].children[0].name, "Design 2", "future design name at hour 50 never leaks at hour 20");
    assert!(air.children[0].children[0].name_missing.is_some());

    let mil_late = index.at(50, ProductionSpendingScope::Military, false);
    assert_eq!(mil_late.through, 100, "quiet stretch spans all remaining unchanged hours");
    let air_late = mil_late.breakdown.parts.iter().find(|part| part.key == "air").expect("air source");
    assert_eq!(air_late.children[0].children[0].name, "Fighter I");

    let dock_no_refits = index.at(50, ProductionSpendingScope::Dockyards, false);
    assert_eq!(dock_no_refits.breakdown.total.amount, Some(BreakdownAmount { numerator: "4".into(), denominator: "1".into() }));
    let dock_with_refits = index.at(50, ProductionSpendingScope::Dockyards, true);
    assert_eq!(dock_with_refits.breakdown.total.amount, Some(BreakdownAmount { numerator: "9".into(), denominator: "2".into() }), "4.5 IC with refits included");

    let dock_active_repair = index.at(10, ProductionSpendingScope::DockyardPurposes, true);
    assert_eq!(dock_active_repair.through, 10, "active non-zero repair rate advances cumulative total next hour");
    let dock_quiet_repair = index.at(30, ProductionSpendingScope::DockyardPurposes, true);
    assert_eq!(dock_quiet_repair.through, 100, "zero repair rate allows full quiet-stretch proof");
    assert_eq!(dock_quiet_repair.breakdown.total.amount, Some(BreakdownAmount { numerator: "11".into(), denominator: "2".into() }), "4 new + 0.5 refit + 1.0 repair (24h * 100_000 / 2_400_000)");
    let repair_part = dock_quiet_repair.breakdown.parts.iter().find(|part| part.key == "repair").expect("repairs");
    assert_eq!(repair_part.amount, Some(BreakdownAmount { numerator: "1".into(), denominator: "1".into() }));

    let air_purposes = index.at(60, ProductionSpendingScope::AirPurposes, true);
    assert_eq!(air_purposes.breakdown.parts.len(), 1);
    assert_eq!(air_purposes.breakdown.parts[0].key, "converted");
    assert_eq!(air_purposes.breakdown.total.amount, Some(BreakdownAmount { numerator: "1".into(), denominator: "1".into() }));
}

#[test]
fn dockyard_purposes_keeps_known_repairs_when_equipment_is_not_recorded_and_withholds_shares() {
    let repairs = RepairRead::new(
        vec![(0, 23, Some((Some(21_100_000), true))), (24, 50, Some((Some(0), true)))],
        BTreeMap::from([(None, vec![(0, 23, Some(Some(21_100_000))), (24, 50, Some(Some(0)))])]),
    );
    let span_read = SpanRead {
        span: span(0, 50, false),
        lines: vec![(0, 50, Context::NotShown)],
        rocket: vec![(0, 50, Context::Unknown(Why::RocketTableUnavailable))],
        country: vec![(0, 50, None)],
        rows: BTreeMap::new(),
        archetypes: BTreeMap::new(),
        archetype_since: BTreeMap::new(),
        equipment_observations: BTreeSet::new(),
        rocket_observations: BTreeSet::new(),
        repairs: Some(repairs),
    };
    let index = SpendingIndex {
        from: 0,
        to: 50,
        country_label: "GER".into(),
        unknown: Vec::new(),
        unresolved: Vec::new(),
        spans: vec![IndexedSpan::new(span_read, HashMap::new(), Vec::new())],
        rocket_source: None,
        warnings: Vec::new(),
    };
    let dock = index.at(30, ProductionSpendingScope::DockyardPurposes, true);
    assert_eq!(dock.breakdown.total.state, BreakdownState::AtLeast);
    assert_eq!(dock.breakdown.total.amount, Some(BreakdownAmount { numerator: "211".into(), denominator: "1".into() }));
    assert!(dock.breakdown.parts.iter().all(|part| part.share.is_none()), "shares are withheld when New ships and Refits are not recorded");
    assert_eq!(dock.breakdown.parts.iter().map(|part| (part.key.as_str(), part.state)).collect::<Vec<_>>(), vec![
        ("repair", BreakdownState::Known),
        ("new", BreakdownState::NotRecorded),
        ("refit", BreakdownState::NotRecorded),
    ]);

    let air = index.at(30, ProductionSpendingScope::AirPurposes, true);
    assert_eq!(air.breakdown.total.state, BreakdownState::NotRecorded);
    assert!(air.breakdown.parts.is_empty());
}

#[test]
fn a_design_counts_under_its_type_only_from_the_hour_its_classification_is_recorded_and_unassigned_spending_leaves_the_factory_total_unknown() {
    let mut rows = BTreeMap::new();
    rows.insert((false, 1, Some(OutputPurpose::New.raw())), vec![(0, 100, row(Some(100_000)))]);
    rows.insert((false, 2, Some(OutputPurpose::New.raw())), vec![(0, 100, row(Some(200_000)))]);
    let span_read = SpanRead {
        span: span(0, 100, false),
        lines: vec![(0, 100, Context::Known)],
        rocket: vec![(0, 100, Context::Known)],
        country: vec![(0, 100, Some((Some(300_000), Some(300_000))))],
        rows,
        archetypes: BTreeMap::new(),
        archetype_since: BTreeMap::from([(1, 0), (2, 40)]),
        equipment_observations: BTreeSet::new(),
        rocket_observations: BTreeSet::new(),
        repairs: None,
    };
    let design = |type_key: &str, domain: Domain| Design {
        type_key: type_key.into(),
        name: Some(type_key.into()),
        domain: Some(domain),
        names: vec![StockVariantName { first_hour: 60, name: Some(format!("{type_key} I")), known: true, design: None }],
        typed_since: None,
    };
    let designs = HashMap::from([(1, design("infantry_equipment", Domain::Land)), (2, design("destroyer", Domain::Naval))]);
    let index = SpendingIndex {
        from: 0,
        to: 100,
        country_label: "GER".into(),
        unknown: Vec::new(),
        unresolved: Vec::new(),
        spans: vec![IndexedSpan::new(span_read, designs, Vec::new())],
        rocket_source: None,
        warnings: Vec::new(),
    };
    let early = index.at(20, ProductionSpendingScope::Military, false);
    assert_eq!(early.through, 39, "the reply holds until the destroyer's classification is recorded");
    assert_eq!((early.breakdown.total.state, early.breakdown.total.amount.clone()), (BreakdownState::NotRecorded, None), "an unclassified design's IC may not be the military factories'");
    let unresolved = early.breakdown.parts.iter().find(|part| part.key == "~unresolved").expect("the unclassified design is kept");
    assert_eq!(unresolved.amount, Some(BreakdownAmount { numerator: "2".into(), denominator: "1".into() }), "its witnessed 2 IC stays, unassigned");
    assert_eq!(unresolved.share, None);
    let late = index.at(50, ProductionSpendingScope::Military, false);
    assert_eq!((late.breakdown.total.state, late.breakdown.total.amount), (BreakdownState::Known, Some(BreakdownAmount { numerator: "1".into(), denominator: "1".into() })), "once it is a destroyer, the military factories spent exactly 1");
    let land = index.at(20, ProductionSpendingScope::LandPurposes, false);
    assert_eq!(land.breakdown.total.state, BreakdownState::NotRecorded, "the unclassified design may be land");
    let new = land.breakdown.parts.iter().find(|part| part.key == "new").expect("land's own new production");
    assert_eq!(new.amount, Some(BreakdownAmount { numerator: "1".into(), denominator: "1".into() }), "the unclassified design is not counted in land's New");
    let docks = index.at(50, ProductionSpendingScope::Dockyards, false);
    assert_eq!(docks.breakdown.total.amount, Some(BreakdownAmount { numerator: "2".into(), denominator: "1".into() }));
}

#[test]
fn a_design_is_typed_from_its_first_archetype_and_named_only_from_its_last_rename() {
    use replay_lanes::lanes::{Cell, History, Step};
    let history = History { texts: vec![b"Old".to_vec().into_boxed_slice(), b"New".to_vec().into_boxed_slice(), b"New".to_vec().into_boxed_slice()], ..History::default() };
    let cells = |name: u32, archetype: i64| Some(vec![Cell::Number(1), Cell::Number(0), Cell::Number(0), Cell::Number(0), Cell::Text(name), Cell::Text(name), Cell::Text(name), Cell::Number(archetype)].into_boxed_slice());
    let steps = [Step { hour: 10, cells: cells(0, 0) }, Step { hour: 40, cells: cells(0, 7) }, Step { hour: 60, cells: cells(1, 7) }, Step { hour: 70, cells: cells(2, 7) }];
    let dates = super::definition_dates(&history, &steps, 0).unwrap();
    assert_eq!((dates.since, dates.typed), (10, Some(40)), "typed when the archetype is first read");
    assert_eq!(
        dates.names.iter().map(|(hour, fields)| (*hour, fields.as_ref().map(|fields| fields.display_name.clone()))).collect::<Vec<_>>(),
        [(10, Some(b"Old".to_vec())), (60, Some(b"New".to_vec()))],
        "each recorded name holds from its own hour, and an equal spelling is no rename"
    );
}

#[test]
fn a_design_is_named_at_each_hour_its_recorded_name_changed_and_not_known_once_its_row_is_gone() {
    use dashboard_base::{Source, test_fixtures::{CAMPAIGN, records, source as base_frame}};
    use replay::campaign::Replay;
    use replay_data::rows::{TableId, TableRows, VariantDefinitionRow};
    use replay_frame::frame::TableView;
    use std::sync::{Arc, atomic::AtomicBool};
    const VARIANT: u64 = 77;
    const TAGGED: u64 = 78;
    const ALIASED: u64 = 79;
    const GENERIC: u64 = 80;
    let frames: Vec<_> = (100..=110)
        .map(|hour| {
            let mut frame = base_frame(1, hour, Some(1));
            if hour >= 103
                && let TableView::Complete(TableRows::Countries(rows)) = &mut *frame.tables[TableId::Countries.index()]
            {
                rows[0].tag = b"DEU".to_vec();
                if hour == 110 {
                    rows.clear();
                }
            }
            if hour == 110
                && let TableView::Complete(TableRows::CountryLinks(rows)) = &mut *frame.tables[TableId::CountryLinks.index()]
            {
                rows.retain(|row| row.id != 9001);
            }
            if hour == 100 {
                *frame.tables[TableId::CountryLinks.index()] = TableView::Unavailable { reason: "links read failed".into() };
            }
            *frame.tables[TableId::CountryAppearance.index()] = if hour == 106 {
                TableView::Unavailable { reason: "appearance read failed".into() }
            } else {
                TableView::Complete(TableRows::CountryAppearance(vec![replay_data::rows::CountryAppearanceRow { country: 9001, name_tag: if hour < 108 { b"GER".to_vec() } else { b"DEU".to_vec() }, ..Default::default() }]))
            };
            frame.metrics.table_ns[TableId::CountryAppearance.index()] = 1;
            let mut rows = vec![VariantDefinitionRow { id: TAGGED, creator_link: 9001, ..VariantDefinitionRow::default() }, VariantDefinitionRow { id: ALIASED, creator_link: 11, ..VariantDefinitionRow::default() }, VariantDefinitionRow { id: GENERIC, ..VariantDefinitionRow::default() }];
            if hour < 109 {
                rows.insert(0, VariantDefinitionRow { id: VARIANT, creator_link: 9001, display_name: if hour < 105 { b"Old".to_vec() } else { b"New".to_vec() }, ..VariantDefinitionRow::default() });
            }
            *frame.tables[TableId::VariantDefinitions.index()] = TableView::Complete(TableRows::VariantDefinitions(rows));
            frame.metrics.table_ns[TableId::VariantDefinitions.index()] = 1;
            frame
        })
        .collect();
    let replay = Replay::from_records(recorder_derive::writers(), CAMPAIGN.into(), &[records(1, frames)]).unwrap().into_timelines(&Default::default()).unwrap();
    let source = Source::from_replay(dashboard_base::SourceKey { source_id: "design-names".into(), revision: "1".into() }, "synthetic.battleplan".into(), Arc::new(replay), &AtomicBool::new(false)).unwrap();
    let query = dashboard_base::queries::query(&source, 1).unwrap();
    let run = query.timeline.stretches[0].run;
    let content = super::Content { catalogue: None, units: None };
    let mut notes = BTreeSet::new();
    let named = super::designs(&query, &Span { run, first: 100, last: 110, id: 9001, breaks: false }, &BTreeMap::from([(VARIANT, 0), (TAGGED, 0), (ALIASED, 0), (GENERIC, 0)]), &content, &mut notes).unwrap();
    let tagged: Vec<i32> = named[&TAGGED].names.iter().map(|entry| entry.first_hour).collect();
    assert!(tagged.contains(&103), "a name taken from the creator's tag is dated again when that country changes tag: {tagged:?}");
    assert!(tagged.contains(&106) && tagged.contains(&107), "a gap in the creator's appearance dates the name before, during and after it: {tagged:?}");
    let aliased: Vec<i32> = named[&ALIASED].names.iter().map(|entry| entry.first_hour).collect();
    assert_eq!(named[&GENERIC].names.iter().map(|entry| (entry.first_hour, entry.known)).collect::<Vec<_>>(), [(100, true)], "a design named by no country is named though country links are unread at its first hour, and never dated by country changes");
    assert!(aliased.contains(&108), "a creator link whose slot's appearance row is another country id is dated when that row's name tag changes: {aliased:?}");
    let names: Vec<(i32, bool)> = named[&VARIANT].names.iter().map(|entry| (entry.first_hour, entry.known)).collect();
    assert_eq!(names, [(100, true), (105, true), (109, false)], "a name per recorded change, not known once the definition is gone");
}

#[test]
fn repairs_are_summed_from_indexed_rate_segments_clipped_to_the_asked_hours() {
    let repairs = RepairRead::new(
        vec![(0, 9, Some((Some(100), true))), (10, 19, Some((Some(50), false))), (20, 24, Some((None, false))), (25, 29, Some((Some(10), true)))],
        BTreeMap::from([(Some(5), vec![(0, 29, Some(Some(100)))]), (None, vec![(0, 9, Some(Some(0))), (10, 29, Some(Some(10)))])]),
    );
    assert_eq!(repairs.index.sum(5, 14), (BTreeMap::from([(5, 500)]), 250, None, true), "five hours split to the design, five unsplit");
    assert_eq!(repairs.index.sum(0, 9), (BTreeMap::from([(5, 1_000)]), 0, None, false));
    assert_eq!(repairs.index.sum(12, 22).2, Some(Why::SpendingUnavailable), "an unread rate inside the hours leaves them unknown");
    let (designs, unattributed, gap, _) = repairs.index.sum(26, 40);
    assert_eq!((designs, unattributed, gap), (BTreeMap::from([(5, 400)]), 40, Some(Why::NotRecorded)), "hours after the last rate are not recorded");
}

#[test]
fn a_range_counts_only_what_was_spent_inside_it_and_refuses_a_baseline_across_a_restart() {
    let mut rows = BTreeMap::new();
    rows.insert((false, 1, Some(OutputPurpose::New.raw())), vec![(0, 49, row(Some(10_000_000))), (50, 100, row(Some(12_000_000)))]);
    let span_read = |first: i32, last: i32| SpanRead {
        span: span(first, last, false),
        lines: vec![(first, last, Context::Known)],
        rocket: vec![(first, last, Context::Known)],
        country: vec![(first, last, Some((Some(12_000_000), Some(12_000_000))))],
        rows: rows.iter().map(|(key, pieces)| (*key, pieces.iter().filter(|piece| piece.1 >= first && piece.0 <= last).map(|&(a, b, row)| (a.max(first), b.min(last), row)).collect())).collect(),
        archetypes: BTreeMap::new(),
        archetype_since: BTreeMap::from([(1, 0)]),
        equipment_observations: BTreeSet::new(),
        rocket_observations: BTreeSet::new(),
        repairs: None,
    };
    let design = Design { type_key: "infantry_equipment".into(), name: Some("Infantry".into()), domain: Some(Domain::Land), names: vec![StockVariantName { first_hour: 0, name: Some("Rifle".into()), known: true, design: None }], typed_since: None };
    let index = |spans: Vec<IndexedSpan>| SpendingIndex { from: 0, to: 100, country_label: "GER".into(), unknown: Vec::new(), unresolved: Vec::new(), spans, rocket_source: None, warnings: Vec::new() };
    let whole = index(vec![IndexedSpan::new(span_read(0, 100), HashMap::from([(1, design.clone())]), Vec::new())]);
    let ranged = whole.between(50, 80, ProductionSpendingScope::Military, false);
    assert_eq!((ranged.from, ranged.breakdown.total.amount), (50, Some(BreakdownAmount { numerator: "20".into(), denominator: "1".into() })), "100 IC before the range and 20 inside it show 20");
    let restarted = index(vec![IndexedSpan::new(span_read(0, 39), HashMap::from([(1, design.clone())]), Vec::new()), IndexedSpan::new(span_read(40, 100), HashMap::from([(1, design)]), Vec::new())]);
    let across = restarted.between(30, 80, ProductionSpendingScope::Military, false);
    assert_eq!(across.breakdown.total.state, BreakdownState::NotRecorded, "a range whose baseline is in an earlier observation is not a difference of two counters");
}

#[test]
fn a_counter_restarted_inside_a_holding_dates_itself_from_the_restart_and_a_range_never_subtracts_across_it() {
    let mut rows = BTreeMap::new();
    rows.insert((false, 1, Some(OutputPurpose::New.raw())), vec![(0, 49, row(Some(40_000_000))), (50, 100, row(Some(1_000_000)))]);
    rows.insert((false, 2, Some(OutputPurpose::New.raw())), vec![(0, 100, row(Some(1_000_000)))]);
    let read = SpanRead {
        span: span(0, 100, false),
        lines: vec![(0, 100, Context::Known)],
        rocket: vec![(0, 100, Context::Known)],
        country: vec![(0, 100, Some((Some(1_000_000), Some(1_000_000))))],
        rows,
        archetypes: BTreeMap::new(),
        archetype_since: BTreeMap::from([(1, 0), (2, 0)]),
        equipment_observations: BTreeSet::from([0, 50]),
        rocket_observations: BTreeSet::new(),
        repairs: None,
    };
    let design = |key: &str, domain: Domain| Design { type_key: key.into(), name: Some(key.into()), domain: Some(domain), names: vec![StockVariantName { first_hour: 0, name: Some(key.into()), known: true, design: None }], typed_since: None };
    let index = SpendingIndex {
        from: 0,
        to: 100,
        country_label: "GER".into(),
        unknown: Vec::new(),
        unresolved: Vec::new(),
        spans: vec![IndexedSpan::new(read, HashMap::from([(1, design("infantry_equipment", Domain::Land)), (2, design("fighter", Domain::Air))]), Vec::new())],
        rocket_source: None,
        warnings: Vec::new(),
    };
    assert_eq!(index.at(80, ProductionSpendingScope::Military, false).from, 50, "a counter restarted at 50 counts from 50, not the holding's start");
    assert_eq!(index.at(20, ProductionSpendingScope::Military, false).through, 49);
    assert_eq!(index.between(30, 80, ProductionSpendingScope::Military, false).breakdown.total.state, BreakdownState::NotRecorded, "400 then 10 is no difference of one counter");
    assert_eq!(index.between(0, 80, ProductionSpendingScope::Military, false).breakdown.total.state, BreakdownState::NotRecorded, "a range from the branch start across the restart is not the new counter alone");
    let at_restart = index.between(50, 80, ProductionSpendingScope::Military, false);
    assert_eq!((at_restart.from, at_restart.breakdown.total.amount), (50, Some(BreakdownAmount { numerator: "20".into(), denominator: "1".into() })), "a range starting at the restart is the new counter");
    let mut rockets = BTreeMap::new();
    rockets.insert((false, 2, Some(OutputPurpose::New.raw())), vec![(0, 100, row(Some(1_000_000)))]);
    rockets.insert((true, 9, None), vec![(0, 49, row(Some(40_000_000))), (50, 100, row(Some(1_000_000)))]);
    let rocket_read = SpanRead {
        span: span(0, 100, false),
        lines: vec![(0, 100, Context::Known)],
        rocket: vec![(0, 100, Context::Known)],
        country: vec![(0, 100, Some((Some(1_000_000), Some(1_000_000))))],
        rows: rockets,
        archetypes: BTreeMap::new(),
        archetype_since: BTreeMap::from([(2, 0)]),
        equipment_observations: BTreeSet::new(),
        rocket_observations: BTreeSet::from([0, 50]),
        repairs: None,
    };
    let mut steady = BTreeMap::new();
    steady.insert((false, 1, Some(OutputPurpose::New.raw())), vec![(0, 49, row(Some(40_000_000))), (50, 100, row(Some(1_000_000)))]);
    steady.insert((true, 9, None), vec![(0, 100, row(Some(10_000_000)))]);
    let steady_read = SpanRead {
        span: span(0, 100, false),
        lines: vec![(0, 100, Context::Known)],
        rocket: vec![(0, 100, Context::Known)],
        country: vec![(0, 100, Some((Some(1_000_000), Some(1_000_000))))],
        rows: steady,
        archetypes: BTreeMap::new(),
        archetype_since: BTreeMap::from([(1, 0)]),
        equipment_observations: BTreeSet::from([0, 50]),
        rocket_observations: BTreeSet::new(),
        repairs: None,
    };
    let steady_index = SpendingIndex { spans: vec![IndexedSpan::new(steady_read, HashMap::from([(1, design("infantry_equipment", Domain::Land))]), Vec::new())], from: 0, to: 100, country_label: "GER".into(), unknown: Vec::new(), unresolved: Vec::new(), rocket_source: None, warnings: Vec::new() };
    let steady_range = steady_index.between(50, 80, ProductionSpendingScope::Military, false);
    assert_eq!(steady_range.breakdown.total.amount, Some(BreakdownAmount { numerator: "10".into(), denominator: "1".into() }), "equipment counts from its restart and the rocket counter, running on, subtracts its own baseline");
    let steady_at = steady_index.at(80, ProductionSpendingScope::Military, false);
    assert_eq!((steady_at.from, steady_at.breakdown.total.amount), (50, Some(BreakdownAmount { numerator: "10".into(), denominator: "1".into() })), "the plain reply counts every counter from the date it publishes");
    let rocket_index = SpendingIndex { spans: vec![IndexedSpan::new(rocket_read, HashMap::from([(2, design("fighter", Domain::Air))]), Vec::new())], ..index };
    assert_eq!(rocket_index.between(30, 80, ProductionSpendingScope::Military, false).breakdown.total.state, BreakdownState::NotRecorded, "a rocket counter restarted inside the range is no difference either");
    assert_eq!(rocket_index.at(20, ProductionSpendingScope::Military, false).through, 49, "the answer stands no later than the rocket restart");
    let mut zeroed = BTreeMap::new();
    zeroed.insert((false, 2, Some(OutputPurpose::New.raw())), vec![(0, 100, row(Some(1_000_000)))]);
    zeroed.insert((true, 9, None), vec![(0, 49, row(Some(500_000))), (50, 100, row(Some(0)))]);
    let zeroed_read = SpanRead {
        span: span(0, 100, false),
        lines: vec![(0, 100, Context::Known)],
        rocket: vec![(0, 100, Context::Known)],
        country: vec![(0, 100, Some((Some(1_000_000), Some(1_000_000))))],
        rows: zeroed,
        archetypes: BTreeMap::new(),
        archetype_since: BTreeMap::from([(2, 0)]),
        equipment_observations: BTreeSet::new(),
        rocket_observations: BTreeSet::from([0, 50]),
        repairs: None,
    };
    let zeroed_index = SpendingIndex { spans: vec![IndexedSpan::new(zeroed_read, HashMap::from([(2, design("fighter", Domain::Air))]), Vec::new())], from: 0, to: 100, country_label: "GER".into(), unknown: Vec::new(), unresolved: Vec::new(), rocket_source: None, warnings: Vec::new() };
    assert_eq!(zeroed_index.between(30, 80, ProductionSpendingScope::Military, false).breakdown.total.state, BreakdownState::NotRecorded, "a rocket restart is checked even when the rocket counter reads zero at the end");
    let mut unread_start = BTreeMap::new();
    unread_start.insert((false, 1, Some(OutputPurpose::New.raw())), vec![(0, 100, row(Some(1_000_000)))]);
    unread_start.insert((true, 9, None), vec![(0, 100, row(Some(0)))]);
    let unread_read = SpanRead {
        span: span(0, 100, false),
        lines: vec![(0, 49, Context::Unknown(Why::SpendingUnavailable)), (50, 100, Context::Known)],
        rocket: vec![(0, 100, Context::Known)],
        country: vec![(0, 49, None), (50, 100, Some((Some(1_000_000), Some(1_000_000))))],
        rows: unread_start,
        archetypes: BTreeMap::new(),
        archetype_since: BTreeMap::from([(1, 0)]),
        equipment_observations: BTreeSet::from([50]),
        rocket_observations: BTreeSet::from([0]),
        repairs: None,
    };
    let unread_index = SpendingIndex { spans: vec![IndexedSpan::new(unread_read, HashMap::from([(1, design("infantry_equipment", Domain::Land))]), Vec::new())], from: 0, to: 100, country_label: "GER".into(), unknown: Vec::new(), unresolved: Vec::new(), rocket_source: None, warnings: Vec::new() };
    let unread_at = unread_index.at(80, ProductionSpendingScope::Military, false);
    assert_eq!((unread_at.from, unread_at.breakdown.total.state), (50, BreakdownState::Known), "only the rocket baseline is needed, and a recorded zero is one, though equipment was unread before its restart");
    let index = rocket_index;
    let quiet = index.between(60, 80, ProductionSpendingScope::Military, false);
    assert_eq!((quiet.breakdown.total.state, quiet.breakdown.total.amount), (BreakdownState::Known, Some(BreakdownAmount { numerator: "0".into(), denominator: "1".into() })), "nothing spent inside the range is exactly zero");
}

#[test]
fn per_ship_repairs_are_partial_when_repair_hours_are_unsplit_or_unrecorded() {
    let design = Design {
        type_key: "destroyer".into(),
        name: Some("Destroyer".into()),
        domain: Some(Domain::Naval),
        names: vec![StockVariantName { first_hour: 0, name: Some("DD I".into()), known: true, design: None }],
        typed_since: Some(0),
    };
    let make_index = |repairs: RepairRead| {
        let mut rows = BTreeMap::new();
        rows.insert((false, 3, Some(OutputPurpose::New.raw())), vec![(0, 50, row(Some(200_000)))]);
        let span_read = SpanRead {
            span: span(0, 50, false),
            lines: vec![(0, 50, Context::Known)],
            rocket: vec![(0, 50, Context::Known)],
            country: vec![(0, 50, Some((Some(200_000), Some(200_000))))],
            rows,
            archetypes: BTreeMap::new(),
            archetype_since: BTreeMap::from([(3, 0)]),
            equipment_observations: BTreeSet::new(),
            rocket_observations: BTreeSet::new(),
            repairs: Some(repairs),
        };
        SpendingIndex {
            from: 0,
            to: 50,
            country_label: "GER".into(),
            unknown: Vec::new(),
            unresolved: Vec::new(),
            spans: vec![IndexedSpan::new(span_read, HashMap::from([(3, design.clone())]), Vec::new())],
            rocket_source: None,
            warnings: Vec::new(),
        }
    };

    let unsplit = make_index(RepairRead::new(
        vec![(0, 23, Some((Some(200_000), true))), (24, 47, Some((Some(100_000), false))), (48, 50, Some((Some(0), true)))],
        BTreeMap::from([
            (Some(3), vec![(0, 23, Some(Some(100_000))), (24, 50, Some(Some(0)))]),
            (Some(4), vec![(0, 23, Some(Some(100_000))), (24, 50, Some(Some(0)))]),
        ]),
    ));
    let unsplit_reply = unsplit.at(50, ProductionSpendingScope::DockyardPurposes, true);
    assert_eq!(unsplit_reply.breakdown.total.state, BreakdownState::Known, "known total repair rate keeps dockyard total exact");
    assert_eq!(unsplit_reply.breakdown.total.amount, Some(BreakdownAmount { numerator: "5".into(), denominator: "1".into() }));
    let repairs = unsplit_reply.breakdown.parts.iter().find(|part| part.key == "repair").expect("repairs");
    assert_eq!(repairs.state, BreakdownState::Known);
    assert_eq!(repairs.share, Some(0.6), "exact repair total keeps its share of dockyard spending");
    let destroyer = repairs.children.iter().find(|part| part.key == "destroyer").expect("destroyer type");
    assert_eq!(destroyer.state, BreakdownState::Partial, "per-type repair is partial when some repair hours were unsplit");
    assert_eq!(destroyer.parent_share, None, "unsplit repair hours withhold per-type share of repairs");
    assert_eq!(destroyer.share, None);
    let dd = &destroyer.children[0];
    assert_eq!(dd.state, BreakdownState::Partial, "per-ship repair is partial when some repair hours were unsplit");
    assert_eq!(dd.parent_share, None);
    assert_eq!(dd.share, None);
    let unresolved = repairs.children.iter().find(|part| part.key == ":variant/4").expect("unresolved ship type");
    assert_eq!(unresolved.state, BreakdownState::Partial, "unresolved ship type is partial under unsplit hours");
    assert_eq!(unresolved.parent_share, None);
    assert_eq!(unresolved.children[0].state, BreakdownState::Partial, "ship child under unresolved type is also partial");
    assert_eq!(unresolved.children[0].parent_share, None);

    let gapped = make_index(RepairRead::new(
        vec![(0, 23, Some((Some(100_000), true))), (24, 47, None), (48, 50, Some((Some(0), true)))],
        BTreeMap::from([(Some(3), vec![(0, 23, Some(Some(100_000))), (24, 50, Some(Some(0)))])]),
    ));
    let gapped_reply = gapped.at(50, ProductionSpendingScope::DockyardPurposes, true);
    assert_eq!(gapped_reply.breakdown.total.state, BreakdownState::AtLeast);
    assert_eq!(gapped_reply.breakdown.total.reason.as_deref(), Some("Spending was not recorded"), "gap reason is not duplicated");
    let repairs = gapped_reply.breakdown.parts.iter().find(|part| part.key == "repair").expect("repairs");
    assert_eq!(repairs.state, BreakdownState::AtLeast);
    assert_eq!(repairs.reason.as_deref(), Some("Spending was not recorded"), "repair aggregate reports gap reason once");
    let destroyer = repairs.children.iter().find(|part| part.key == "destroyer").expect("destroyer type");
    assert_eq!(destroyer.state, BreakdownState::Partial, "per-ship repair is partial when some repair hours were not recorded");
    assert_eq!(destroyer.children[0].state, BreakdownState::Partial);
}
