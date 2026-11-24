use super::*;
use replay_data::rows::CasualtyTotalRow;

fn key(a: Identity, b: Identity, start: i32) -> u128 {
    CasualtyLifetimeRow { slot_a: a.0, id_a: a.1, slot_b: b.0, id_b: b.1, start_hour: start, ..Default::default() }.key()
}

fn tags(runs: &[(Identity, &str, i32, i32)]) -> HolderTags {
    let mut by: HashMap<Identity, Vec<(String, i32, i32)>> = HashMap::new();
    for &(identity, tag, first, last) in runs {
        by.entry(identity).or_default().push((tag.to_owned(), first, last));
    }
    HolderTags::from_runs(by)
}

const ONE: Identity = (1, 11);
const TWO: Identity = (2, 22);
const THREE: Identity = (3, 33);

#[test]
fn a_lifetime_key_and_a_holder_key_are_the_row_codecs() {
    let packed = key((0xff_fffe, 7), (3, 0xab_cdef), -5);
    assert_eq!(Lifetime::of(packed), Lifetime { a: (0xff_fffe, 7), b: (3, 0xab_cdef), start: -5 });
    assert_eq!(holder_key((9, 4)), CasualtyTotalRow { slot: 9, id: 4, ..Default::default() }.key());
}

#[test]
fn a_breakdown_lists_what_the_country_took_in_each_lifetime_by_enemy_under_no_war() {
    let (west, east) = (key(ONE, TWO, 50), key(THREE, (4, 44), 60));
    let mine = key((0, 5), THREE, 70);
    let lifetimes = [(west, 0, 19), (east, 0, 19), (mine, 10, 19)];
    let reads = WindowReads {
        taken_a: HashMap::from([(west, vec![(0, 19, Some(4))]), (east, vec![(0, 19, Some(9))]), (mine, vec![(10, 19, Some(1))])]),
        taken_b: HashMap::from([(west, vec![(0, 19, Some(6))]), (east, vec![(0, 19, Some(2))]), (mine, vec![(10, 14, Some(3)), (15, 19, Some(8))])]),
        partial: HashMap::from([(holder_key(THREE), vec![(0, 19, Some(0))])]),
        observed: vec![(0, 19)],
    };
    let names = tags(&[(ONE, "ONE", 0, 19), (TWO, "TWO", 0, 19), (THREE, "THREE", 0, 19), ((4, 44), "FOUR", 0, 9), ((4, 44), "FIVE", 10, 19), ((0, 5), "ZERO", 0, 19)]);
    let (countries, series) = fold_breakdown(&"THREE".into(), &lifetimes, &reads, &names, 0, 19, &HashMap::new());
    assert_eq!(countries, vec![CasualtyBreakdownCountryRun { first_hour: 0, last_hour: 19, identity: "3:33".into(), incomplete: false }]);
    let shown: Vec<(&str, Vec<(i32, i32, &str)>)> = series
        .iter()
        .map(|series| (series.enemy_tag.as_str(), series.runs.iter().map(|run| (run.first_hour, run.last_hour, run.value.as_str())).collect()))
        .collect();
    assert_eq!(
        shown,
        vec![
            ("ZERO", vec![(10, 14, "3"), (15, 19, "8")]),
            ("FIVE", vec![(10, 19, "9")]),
            ("FOUR", vec![(0, 9, "9")]),
        ],
        "the war ONE fought with TWO is not THREE's"
    );
    assert!(series.iter().flat_map(|series| &series.runs).all(|run| run.war_id.is_none() && run.reason.as_deref() == Some(NO_WAR)));
}

#[test]
fn a_breakdown_groups_each_lifetime_by_the_war_holding_it_and_an_ended_one_by_its_last_war() {
    use crate::products::wars::{WarRelationKey, WarRelationMembership};
    let (west, east) = (key(ONE, THREE, 0), key(TWO, THREE, 0));
    let lifetimes = [(west, 0, 19), (east, 0, 9)];
    let reads = WindowReads {
        taken_a: HashMap::new(),
        taken_b: HashMap::from([(west, vec![(0, 19, Some(5))]), (east, vec![(0, 9, Some(7))])]),
        partial: HashMap::new(),
        observed: vec![(0, 19)],
    };
    let names = tags(&[(ONE, "ONE", 0, 19), (TWO, "TWO", 0, 19), (THREE, "THREE", 0, 19)]);
    let held = |a: Identity, b: Identity, first_hour: i32, last_hour: i32| WarRelationMembership { key: WarRelationKey::new(a, b, 0), oriented: false, first_hour, last_hour };
    let first = [held(ONE, THREE, 0, 9), held(TWO, THREE, 0, 9)];
    let second = [held(ONE, THREE, 10, 19)];
    let overlap = [held(ONE, THREE, 15, 16)];
    let wars = [("first", first.as_slice()), ("second", second.as_slice()), ("overlap", overlap.as_slice())];
    let groupings = lifetime_groupings(wars, &HashSet::from([west, east]));
    let (_, series) = fold_breakdown(&"THREE".into(), &lifetimes, &reads, &names, 0, 19, &groupings);
    let shown = |enemy: &str| -> Vec<(i32, i32, Option<&str>, Option<&str>, i32, bool)> {
        series
            .iter()
            .find(|series| series.enemy_tag == enemy)
            .map(|series| series.runs.iter().map(|run| (run.first_hour, run.last_hour, run.war_id.as_deref(), run.reason.as_deref(), run.grouping_hour, run.grouping_hourly)).collect())
            .unwrap_or_default()
    };
    assert_eq!(
        shown("ONE"),
        vec![
            (0, 9, Some("first"), None, 0, true),
            (10, 14, Some("second"), None, 10, true),
            (15, 16, None, Some(SEVERAL_WARS), 15, true),
            (17, 19, Some("second"), None, 17, true),
        ]
    );
    assert_eq!(shown("TWO"), vec![(0, 9, Some("first"), None, 0, true), (10, 19, Some("first"), None, 9, false)], "an ended lifetime stays in its last war");
    for series in &series {
        let value = if series.enemy_tag == "TWO" { "7" } else { "5" };
        assert!(series.runs.iter().all(|run| run.value == value), "{series:?}");
    }
}

#[test]
fn a_window_reply_carries_the_windows_its_index_names() {
    use dashboard_base::test_fixtures::{CAMPAIGN, records, source};
    use replay::campaign::Replay;
    use std::sync::Arc;
    let frames = (1_000..=1_100).map(|hour| source(1, hour, Some(1))).collect();
    let replay = Replay::from_records(recorder_derive::writers(), CAMPAIGN.into(), &[records(1, frames)]).unwrap().into_timelines(&Default::default()).unwrap();
    let source = Source::from_replay(dashboard_base::SourceKey { source_id: "casualties".into(), revision: "1".into() }, "synthetic.battleplan".into(), Arc::new(replay), &AtomicBool::new(false)).unwrap();
    let load = LoadContext { content: None };
    let cancelled = AtomicBool::new(false);
    let query = dashboard_base::queries::query(&source, 1).unwrap();
    let breakdown = |window| window_reply(&source, &query, 1, &"AAA".into(), window, &load, &cancelled).unwrap();
    let manifest = breakdown(None).windows;
    assert!(!manifest.is_empty());
    let shown = breakdown(Some(0));
    assert_eq!((shown.windows, shown.window), (manifest, Some(0)));
    let asked = CasualtyBreakdown::load(&source, &CasualtyBreakdownParams { branch_id: "1".into(), country: "AAA".into(), hour: 1_050 }, &load, &cancelled).unwrap();
    assert_eq!(asked.hour, 1_050);
    assert!(asked.through >= asked.hour);
}

fn run(first: i32, last: i32, value: i64, war: Option<&str>, hourly: bool) -> CasualtyBreakdownRun {
    CasualtyBreakdownRun {
        first_hour: first,
        last_hour: last,
        read_hour: first,
        hourly,
        grouping_hour: first,
        grouping_hourly: hourly,
        enemy_hour: if hourly { first } else { first - 1 },
        enemy_hourly: hourly,
        value: value.to_string(),
        war_id: war.map(Into::into),
        reason: war.is_none().then(|| NO_WAR.to_owned()),
        incomplete: false,
    }
}

fn named(text: &str, first: i32, last: i32, known: bool) -> CasualtyWarNameRun {
    CasualtyWarNameRun { first_hour: first, last_hour: last, name: CasualtyWarName { text: text.into(), known, reason: (!known).then(|| "Its name was not localised".into()), key: (!known).then(|| text.into()) } }
}

#[test]
fn the_breakdown_at_an_hour_is_the_holder_then_by_war_and_enemy_and_stands_to_the_next_change() {
    use dashboard_base::products::breakdown::BreakdownState;
    let series = |id: &str, identity: &str, enemy: &str, runs: Vec<CasualtyBreakdownRun>| CasualtyBreakdownSeries { id: id.into(), country_identity: identity.into(), enemy_tag: enemy.into(), enemy_identity: "9:99".into(), runs };
    let window = CasualtyWindow {
        windows: Vec::new(),
        window: Some(0),
        observed: vec![HourSpan { first_hour: 0, last_hour: 49 }, HourSpan { first_hour: 60, last_hour: 99 }],
        countries: vec![
            CasualtyBreakdownCountryRun { first_hour: 0, last_hour: 49, identity: "1:11".into(), incomplete: false },
            CasualtyBreakdownCountryRun { first_hour: 60, last_hour: 99, identity: "2:22".into(), incomplete: false },
        ],
        series: vec![
            series("a", "1:11", "BBB", vec![run(5, 19, 40, Some("w1"), true), run(20, 49, 70, Some("w1"), false)]),
            series("b", "1:11", "CCC", vec![run(10, 49, 15, None, true)]),
            series("c", "2:22", "BBB", vec![run(60, 99, 500, Some("w1"), true)]),
        ],
        wars: vec![CasualtyBreakdownWar { id: "w1".into(), first_hour: 0, last_hour: 99, started_by: true, ended_by: false, names: vec![named("First war", 0, 29, true), named("war_key", 30, 99, false)] }],
        warnings: Vec::new(),
    };
    let held = index(window, 0, 99);
    let at = held.at(12);
    let parts = &at.breakdown.parts;
    assert_eq!(parts.iter().map(|part| part.name.as_str()).collect::<Vec<_>>(), ["First war", "War not recorded"]);
    assert_eq!(parts[0].children[0].key, "BBB");
    assert_eq!(parts[1].name_missing.as_deref(), Some(NO_WAR), "a lifetime with no recorded war says why");
    assert_eq!(at.through, 19, "the next change to anything shown, a new run at 20");
    let later = held.at(35);
    assert_eq!(later.breakdown.total.raw.as_deref(), Some("85"));
    let war = &later.breakdown.parts[0];
    assert_eq!((war.name.as_str(), war.name_missing.is_some()), ("First war", false), "an ended relation keeps the war's name at its own grouping hour");
    assert_eq!(war.children[0].last, Some(19), "an ended relation is dated at its read, not the displayed hour");
    assert_eq!(held.at(55).breakdown.total.state, BreakdownState::NotRecorded, "an hour not observed is not a zero");
    let changed = held.at(70);
    assert_eq!(changed.breakdown.total.raw.as_deref(), Some("500"), "after the holder changes, only its own losses");
    let war = &changed.breakdown.parts[0];
    assert_eq!((war.name.as_str(), war.name_missing.is_some()), ("war_key", true), "a war name not localised is its key with the reason");
    for hour in [12, 35, 55, 70] {
        assert_eq!(held.at(hour), held.at(hour), "a lookup");
    }
}
