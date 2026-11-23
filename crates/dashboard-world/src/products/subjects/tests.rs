use super::*;
use dashboard_query::timeline::Join;

fn cell(start: i32, end: i32) -> Option<i64> {
    Some(i64::from(start) << 32 | i64::from(end as u32))
}

#[test]
fn the_schema_numbers_the_cells_a_relation_and_a_level_are_read_from() {
    let field = |table, name| dashboard_declare::field(table, name).unwrap();
    assert_eq!(field(TableId::Subjects, "start_hour"), START);
    assert_eq!(field(TableId::Subjects, "end_hour"), END);
    assert_eq!(field(TableId::Subjects, "cancel"), CANCEL);
    assert_eq!(field(TableId::Autonomy, "state"), STATE);
    assert_eq!(split(cell(-5, 300).unwrap()), (-5, 300));
}

#[test]
fn a_switch_between_levels_with_equal_bounds_is_a_new_level_and_an_unshown_row_is_not_recorded() {
    use replay_lanes::points::Cell;
    let step = |hour, cell: Option<Cell>| lanes::Step { hour, cells: cell.map(|cell| Box::from([cell])) };
    let history = lanes::History {
        columns: vec![STATE],
        to: 199,
        texts: vec![Box::from(&b"puppet"[..]), Box::from(&b"reichskommissariat"[..])],
        keys: vec![
            (2, vec![step(100, Some(Cell::Text(0))), step(120, Some(Cell::Text(1))), step(140, None), step(150, Some(Cell::Text(1))), step(160, Some(Cell::Unrecorded))]),
            (3, vec![step(100, Some(Cell::Text(0)))]),
        ],
    };
    let levels = level_runs(&history);
    let named = |text: &str| Some(text.to_owned());
    assert_eq!(levels[&2], vec![(100, 119, named("puppet")), (120, 139, named("reichskommissariat")), (140, 149, None), (150, 159, named("reichskommissariat")), (160, 199, None)]);
    assert_eq!(levels[&3], vec![(100, 199, named("puppet"))]);
}

#[test]
fn relations_end_at_their_recorded_end_break_on_unknown_countries_and_join_across_a_load() {
    let first = RunSubjects {
        relations: vec![
            (1, 2, vec![(100, 129, cell(50, 0)), (130, 134, None), (135, 149, cell(50, 0)), (150, 189, cell(50, 180)), (190, 199, cell(190, 0))]),
            (1, 3, vec![(100, 199, cell(60, 0))]),
        ],
        levels: HashMap::from([(2, vec![(100, 139, Some("colony".to_owned())), (140, 199, Some("dominion".to_owned()))]), (3, vec![(100, 119, None), (120, 199, Some("puppet".to_owned()))])]),
    };
    let second = RunSubjects { relations: vec![(1, 2, vec![(200, 249, cell(190, 0))])], levels: HashMap::from([(2, vec![(200, 249, Some("dominion".to_owned()))])]) };
    let links = |pairs: &[(u32, u32)], first: i32, last: i32| RunLinks { slots: pairs.iter().map(|&(raw, slot)| (raw, vec![(first, last, Some(i64::from(slot)))])).collect() };
    let held = HashMap::from([
        (0, (Arc::new(first), Arc::new(links(&[(1, 10), (2, 20), (3, 30)], 100, 199)))),
        (1, (Arc::new(second), Arc::new(links(&[(1, 10), (2, 20)], 200, 249)))),
    ]);
    let names = Names {
        slots: HashMap::from([
            (10, vec![("OVR".to_owned(), 100, 249)]),
            (20, vec![("SUB".to_owned(), 100, 249)]),
            (30, vec![("THR".to_owned(), 100, 159)]),
        ]),
    };
    let stretches = [Stretch { run: 0, first: 100, last: 199, join: Join::Start }, Stretch { run: 1, first: 200, last: 249, join: Join::Load { parent: 199 } }];
    let (runs, unobserved) = fold(&stretches, &held, &names);
    let shown: Vec<_> = runs
        .iter()
        .map(|run| (run.subject.as_str(), run.start_hour, run.first_hour, run.last_hour, run.steps.iter().map(|step| (step.first_hour, step.last_hour, step.level.clone())).collect::<Vec<_>>()))
        .collect();
    let level = |text: &str| Some(text.to_owned());
    assert_eq!(
        shown,
        vec![
            ("SUB", 50, 100, 129, vec![(100, 129, level("colony"))]),
            ("THR", 60, 100, 159, vec![(100, 119, None), (120, 159, level("puppet"))]),
            ("SUB", 50, 135, 179, vec![(135, 139, level("colony")), (140, 179, level("dominion"))]),
            ("SUB", 190, 190, 249, vec![(190, 249, level("dominion"))]),
        ]
    );
    assert!(runs.iter().all(|run| run.overlord == "OVR"));
    assert_eq!(settle(unobserved), vec![HourSpan { first_hour: 130, last_hour: 134 }, HourSpan { first_hour: 160, last_hour: 199 }]);
}
