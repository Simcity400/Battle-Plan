use super::{Amount, Context, ProductionSpendingUnknown as Why, RepairRead, Row, Span, SpanRead, runs_of};
use std::collections::{BTreeMap, BTreeSet};

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
    rows.insert((false, 11, Some(1)), shifted(vec![(0, 4, None), (5, 19, row(Some(100))), (20, 29, row(None))], by));
    rows.insert((true, 12, None), shifted(vec![(0, 7, None), (8, 29, row(Some(40)))], by));
    rows.insert((true, 13, None), shifted(vec![(0, 15, None), (16, 29, row(Some(2)))], by));
    SpanRead {
        span,
        lines: shifted(vec![(0, 9, Context::Known), (10, 14, Context::NotShown), (15, 29, Context::Known)], by),
        rocket: shifted(vec![(0, 19, Context::Known), (20, 24, Context::Unknown(Why::RocketTableUnavailable)), (25, 29, Context::Known)], by),
        country: shifted(vec![(0, 9, Some((Some(100), Some(90)))), (10, 14, None), (15, 29, Some((Some(150), None)))], by),
        rows,
        archetypes: BTreeMap::new(),
        equipment_reads: vec![by, by + 24],
        rocket_reads: vec![by, by + 24],
        equipment_observations: BTreeSet::new(),
        rocket_observations: BTreeSet::new(),
        repairs: None,
    }
}

#[test]
fn a_design_is_zero_without_a_row_and_rocket_and_the_total_sum_their_parts_or_say_why_not() {
    let read = read(span(0, 29, false));
    assert_eq!(read.amounts(&(false, 11, Some(1)), |row| row.spent), vec![
        (0, 4, Amount::Known(0)),
        (5, 9, Amount::Known(100)),
        (10, 14, Amount::Unknown(Why::NotRecorded)),
        (15, 19, Amount::Known(100)),
        (20, 29, Amount::Unknown(Why::LostOutput)),
    ]);
    assert_eq!(read.amounts(&(false, 99, Some(1)), |row| row.spent)[0], (0, 9, Amount::Known(0)));
    assert_eq!(read.rocket_total(), vec![
        (0, 7, Amount::Known(0)),
        (8, 15, Amount::Known(40)),
        (16, 19, Amount::Known(42)),
        (20, 24, Amount::Unknown(Why::RocketTableUnavailable)),
        (25, 29, Amount::Known(42)),
    ]);
    assert_eq!(read.equipment(false), vec![(0, 9, Amount::Known(90)), (10, 14, Amount::Unknown(Why::NotRecorded)), (15, 29, Amount::Unknown(Why::SpendingUnavailable))]);
    let total = super::zip(&read.equipment(true), &read.rocket_total(), |a, b| a.plus(*b));
    assert_eq!(total[0], (0, 7, Amount::Known(100)));
    assert_eq!(total.last(), Some(&(25, 29, Amount::Known(192))));
}

#[test]
fn presence_needs_a_shown_country_row_and_an_empty_one_is_dated_by_its_read() {
    let mut read = read(span(0, 29, false));
    read.rows.retain(|key, _| !key.0);
    let presence = read.presence();
    assert_eq!(presence.iter().map(|run| (run.first_hour, run.last_hour, run.present, run.empty_census_hour)).collect::<Vec<_>>(), vec![
        (0, 4, Some(false), Some(0)),
        (5, 9, Some(true), None),
        (10, 14, None, None),
        (15, 29, Some(true), None),
    ]);
}

#[test]
fn a_new_holder_breaks_the_runs_and_equal_neighbours_are_one_run() {
    let spans = [read(span(0, 29, false)), read(span(30, 59, true))];
    let runs = runs_of(&spans, |span| span.amounts(&(false, 11, Some(1)), |row| row.spent));
    let second = runs.iter().position(|run| run.first_hour == 30).expect("the second holder's first run");
    assert_eq!(runs[second].break_before, Some(true));
    assert!(runs.iter().filter(|run| run.break_before.is_some()).count() == 1);
    assert_eq!((runs[0].first_hour, runs[0].last_hour, runs[0].total_raw.as_deref()), (0, 4, Some("0")));
    assert_eq!(runs[second].total_raw.as_deref(), Some("0"));
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
