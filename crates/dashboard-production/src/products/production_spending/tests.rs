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
    let repairs = RepairRead {
        total: vec![(0, 9, Some((Some(2_400), true))), (10, 19, Some((Some(2_400), false))), (20, 24, Some((None, false))), (25, 29, None)],
        parts: BTreeMap::from([(Some(5), vec![(0, 4, None), (5, 29, Some(Some(1_000)))]), (None, vec![(0, 29, Some(Some(1_400)))])]),
    };
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
    let repairs = RepairRead {
        total: vec![(0, 23, Some((Some(100_000), true))), (24, 100, Some((Some(0), true)))],
        parts: BTreeMap::from([(Some(3), vec![(0, 23, Some(Some(100_000))), (24, 100, Some(Some(0)))]), (None, vec![(0, 100, Some(Some(0)))])]),
    };
    let span_read = SpanRead {
        span: span(0, 100, false),
        lines: vec![(0, 100, Context::Known)],
        rocket: vec![(0, 100, Context::Known)],
        country: vec![(0, 100, Some((Some(1_050_000), Some(1_000_000))))],
        rows,
        archetypes: BTreeMap::new(),
        archetype_since: BTreeMap::from([(2, 0)]),
        equipment_observations: BTreeSet::new(),
        repairs: Some(repairs),
    };
    let design = |type_key: &str, name: &str, domain: Domain, label: &str, first_hour: i32| Design {
        type_key: type_key.into(),
        name: Some(name.into()),
        domain: Some(domain),
        names: vec![StockVariantName { first_hour, name: Some(label.into()), known: true, design: None }],
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
    let repairs = RepairRead {
        total: vec![(0, 23, Some((Some(21_100_000), true))), (24, 50, Some((Some(0), true)))],
        parts: BTreeMap::from([(None, vec![(0, 23, Some(Some(21_100_000))), (24, 50, Some(Some(0)))])]),
    };
    let span_read = SpanRead {
        span: span(0, 50, false),
        lines: vec![(0, 50, Context::NotShown)],
        rocket: vec![(0, 50, Context::Unknown(Why::RocketTableUnavailable))],
        country: vec![(0, 50, None)],
        rows: BTreeMap::new(),
        archetypes: BTreeMap::new(),
        archetype_since: BTreeMap::new(),
        equipment_observations: BTreeSet::new(),
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
        repairs: None,
    };
    let design = |type_key: &str, domain: Domain| Design {
        type_key: type_key.into(),
        name: Some(type_key.into()),
        domain: Some(domain),
        names: vec![StockVariantName { first_hour: 60, name: Some(format!("{type_key} I")), known: true, design: None }],
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
    let docks = index.at(50, ProductionSpendingScope::Dockyards, false);
    assert_eq!(docks.breakdown.total.amount, Some(BreakdownAmount { numerator: "2".into(), denominator: "1".into() }));
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
        repairs: None,
    };
    let design = Design { type_key: "infantry_equipment".into(), name: Some("Infantry".into()), domain: Some(Domain::Land), names: vec![StockVariantName { first_hour: 0, name: Some("Rifle".into()), known: true, design: None }] };
    let index = |spans: Vec<IndexedSpan>| SpendingIndex { from: 0, to: 100, country_label: "GER".into(), unknown: Vec::new(), unresolved: Vec::new(), spans, rocket_source: None, warnings: Vec::new() };
    let whole = index(vec![IndexedSpan::new(span_read(0, 100), HashMap::from([(1, design.clone())]), Vec::new())]);
    let ranged = whole.between(50, 80, ProductionSpendingScope::Military, false);
    assert_eq!((ranged.from, ranged.breakdown.total.amount), (50, Some(BreakdownAmount { numerator: "20".into(), denominator: "1".into() })), "100 IC before the range and 20 inside it show 20");
    let restarted = index(vec![IndexedSpan::new(span_read(0, 39), HashMap::from([(1, design.clone())]), Vec::new()), IndexedSpan::new(span_read(40, 100), HashMap::from([(1, design)]), Vec::new())]);
    let across = restarted.between(30, 80, ProductionSpendingScope::Military, false);
    assert_eq!(across.breakdown.total.state, BreakdownState::NotRecorded, "a range whose baseline is in an earlier observation is not a difference of two counters");
}
