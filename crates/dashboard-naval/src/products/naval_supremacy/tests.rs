use super::*;
use dashboard_query::timeline::{Join, Stretch, Timeline};

#[test]
fn each_day_is_read_at_its_last_held_hour_and_a_day_split_by_runs_takes_the_later() {
    let timeline = Timeline {
        id: 1,
        first: 30,
        last: 100,
        stretches: vec![Stretch { run: 0, first: 30, last: 50, join: Join::Start }, Stretch { run: 1, first: 60, last: 100, join: Join::Break }],
        unheld: Vec::new(),
    };
    let hours = day_hours(&timeline, 30, 100);
    assert_eq!(hours.get(&0), Some(&(0, 47)));
    assert_eq!(hours.get(&1), Some(&(1, 71)), "hours 48-50 and 60-71: the later run's last hour");
    assert_eq!(hours.get(&2), Some(&(1, 95)));
    assert_eq!(hours.get(&3), Some(&(1, 100)), "the window's end cuts the last day");
}

#[test]
fn equal_days_make_one_run_and_an_unknown_day_ends_it() {
    let contested = Values { control: Some(0), friendly: Some(5), enemy: Some(5), share: Some(50_000), own: Some(5) };
    let friendly = Values { control: Some(1), ..contested };
    let refused = Values { control: None, ..contested };
    let piece = |day: u32, values: &Values| (day, day, values.clone());
    let runs = runs(BTreeMap::from([(9, vec![piece(3, &contested), piece(4, &friendly), piece(5, &refused), (0, 1, contested.clone())])]));
    let summary: Vec<(u32, u32, Option<i8>)> = runs[&9].iter().map(|run| (run.first_day, run.last_day, run.control)).collect();
    assert_eq!(summary, [(0, 1, Some(0)), (3, 3, Some(0)), (4, 4, Some(1)), (5, 5, None)], "day 2 is not known: no run spans it");
    assert_eq!(runs[&9][0].share.as_deref(), Some("50000"));
}

#[test]
fn a_viewers_row_comes_first_then_the_every_viewer_row_then_zeros_and_a_refusal_is_no_value() {
    let viewer = 7_u32;
    let key = |holder: u32, region: u32| u128::from(holder) << 32 | u128::from(region);
    let n = Cell::Number;
    let row = |friendly, enemy, own, refusal: u32| Some(Box::from([n(friendly), n(enemy), n(friendly), n(enemy), n(own), Cell::Text(refusal), Cell::Text(0)]));
    let step = |hour, cells| lanes::Step { hour, cells };
    let history = lanes::History {
        columns: COLUMNS.to_vec(),
        to: 200,
        texts: vec![Box::from(&b""[..]), Box::from(&b"Naval current owner identity is unresolved"[..])],
        keys: vec![
            (key(viewer, 1), vec![step(100, row(300, 100, 50, 0))]),
            (key(viewer, 4), vec![step(100, row(1, 1, 1, 1))]),
            (key(NavalRegionControlRow::EVERY_VIEWER, 1), vec![step(100, row(9, 9, 9, 0))]),
            (key(NavalRegionControlRow::EVERY_VIEWER, 2), vec![step(100, row(0, 200, 0, 0)), step(150, None)]),
        ],
    };
    let sea = SeaContent {
        sea: Ok(BTreeSet::from([1, 2, 3, 4])),
        minima: (1..=4).map(|region| (region, Ok(1_000))).collect(),
        ratio: Ok(200_000),
        notes: Vec::new(),
    };
    let mut notes = Refusals::default();
    let mut pieces = BTreeMap::new();
    region_runs(&history, viewer, &[(0, 120), (1, 160), (2, 170)], &BTreeSet::from([3]), &sea, &mut notes, &mut pieces);
    let at = |region: u32, day: u32| pieces[&region].iter().find(|run: &&(u32, u32, Values)| run.0 <= day && day <= run.1).unwrap().2.clone();
    let first = BTreeMap::from([1, 2, 3, 4].map(|region| (region, at(region, 0))));
    assert_eq!((first[&1].friendly, first[&1].enemy, first[&1].own), (Some(300), Some(100), Some(50)), "the viewer's own row, not the every-viewer row");
    assert_eq!((first[&2].enemy, first[&2].own), (Some(200), Some(0)), "the every-viewer row");
    assert_eq!((first[&3].friendly, first[&3].own, first[&3].share), (Some(0), Some(0), Some(0)), "no row: zeros");
    assert_eq!(first[&4].control, None, "a recorded refusal is no control");
    assert_eq!(at(2, 1).enemy, Some(0), "the every-viewer row gone: zeros");
    assert_eq!(pieces[&1], vec![(0, 2, at(1, 0))], "a region whose rows do not change is one run");
    assert_eq!(pieces[&2].len(), 2, "the every-viewer row's end starts a run");
}
