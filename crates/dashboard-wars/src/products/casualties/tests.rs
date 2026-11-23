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
fn a_pair_sums_its_lifetimes_cuts_where_a_tag_changes_and_leaves_out_what_was_not_recorded() {
    let (old, new) = (key(ONE, TWO, 90), key(ONE, TWO, 120));
    let lifetimes = [(old, 100, 199), (new, 130, 199)];
    let reads = WindowReads {
        taken_a: HashMap::from([(old, vec![(100, 199, Some(5))]), (new, vec![(130, 149, Some(2)), (150, 159, None), (160, 199, Some(3))])]),
        taken_b: HashMap::from([(old, vec![(100, 199, Some(7))]), (new, vec![(130, 199, Some(1))])]),
        partial: HashMap::from([(holder_key(TWO), vec![(100, 169, Some(0)), (170, 199, Some(1))])]),
        observed: vec![(100, 179), (185, 199)],
    };
    let names = tags(&[(ONE, "ONE", 100, 199), (TWO, "TWO", 100, 139), (TWO, "NEW", 140, 199)]);
    let pairs = fold_pairs(&lifetimes, &reads, &names, 100, 199, |_, _| true);
    let runs = |first: &str, second: &str| -> Vec<(i32, i32, String, String, bool)> {
        pairs
            .iter()
            .find(|pair| pair.first_country == first && pair.second_country == second)
            .map(|pair| pair.runs.iter().map(|run| (run.first_hour, run.last_hour, run.first_taken.clone(), run.second_taken.clone(), run.incomplete)).collect())
            .unwrap_or_default()
    };
    assert_eq!(pairs.len(), 2, "one series per pair of tags: {pairs:?}");
    assert_eq!(pairs[0].identity, pairs[1].identity, "both are one pair of identities");
    let s = |text: &str| text.to_owned();
    assert_eq!(runs("ONE", "TWO"), vec![(100, 129, s("5"), s("7"), false), (130, 139, s("7"), s("8"), false)]);
    assert_eq!(
        runs("ONE", "NEW"),
        vec![
            (140, 149, s("7"), s("8"), false),
            (160, 169, s("8"), s("8"), false),
            (170, 179, s("8"), s("8"), true),
            (185, 199, s("8"), s("8"), true),
        ]
    );
}

#[test]
fn an_ended_lifetime_keeps_its_last_counters_in_the_pairs_sum() {
    let (before, inside) = (key(ONE, TWO, 10), key(ONE, TWO, 60));
    let lifetimes = [(before, 10, 50), (inside, 60, 129)];
    let reads = WindowReads {
        taken_a: HashMap::from([(before, vec![(50, 50, Some(4))]), (inside, vec![(100, 119, Some(1)), (120, 129, Some(2))])]),
        taken_b: HashMap::from([(before, vec![(50, 50, Some(6))]), (inside, vec![(100, 129, Some(3))])]),
        partial: HashMap::new(),
        observed: vec![(100, 199)],
    };
    let names = tags(&[(ONE, "ONE", 0, 199), (TWO, "TWO", 0, 199)]);
    let pairs = fold_pairs(&lifetimes, &reads, &names, 100, 199, |_, _| true);
    let runs: Vec<(i32, i32, String, String)> = pairs[0].runs.iter().map(|run| (run.first_hour, run.last_hour, run.first_taken.clone(), run.second_taken.clone())).collect();
    let s = |text: &str| text.to_owned();
    assert_eq!(runs, vec![(100, 119, s("5"), s("9")), (120, 199, s("6"), s("9"))], "an ended lifetime's last counters hold after it");
}

#[test]
fn between_keeps_only_pairs_whose_two_tags_are_chosen() {
    let (first, second) = (key(ONE, TWO, 0), key(ONE, THREE, 0));
    let lifetimes = [(first, 0, 9), (second, 0, 9)];
    let full = |key| (key, vec![(0, 9, Some(1))]);
    let reads = WindowReads {
        taken_a: HashMap::from([full(first), full(second)]),
        taken_b: HashMap::from([full(first), full(second)]),
        partial: HashMap::new(),
        observed: vec![(0, 9)],
    };
    let names = tags(&[(ONE, "ONE", 0, 9), (TWO, "TWO", 0, 9), (THREE, "THREE", 0, 9)]);
    let chosen = ["ONE", "TWO"];
    let between = fold_pairs(&lifetimes, &reads, &names, 0, 9, |a, b| chosen.contains(&a.1) && chosen.contains(&b.1));
    assert_eq!(between.iter().map(|pair| pair.second_country.as_str()).collect::<Vec<_>>(), ["TWO"]);
    let either = fold_pairs(&lifetimes, &reads, &names, 0, 9, |a, b| chosen.contains(&a.1) || chosen.contains(&b.1));
    assert_eq!(either.len(), 2);
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
    let pairs = |window| CasualtyPairs::load(&source, &CasualtyPairsParams { branch_id: "1".into(), countries: Vec::new(), between: false, window }, &load, &cancelled).unwrap();
    let manifest = pairs(None).windows;
    assert!(!manifest.is_empty());
    let shown = pairs(Some(0));
    assert_eq!((shown.windows, shown.window), (manifest.clone(), Some(0)));
    let breakdown = |window| CasualtyBreakdown::load(&source, &CasualtyBreakdownParams { branch_id: "1".into(), country: "GER".into(), window }, &load, &cancelled).unwrap();
    assert_eq!(breakdown(None).windows, manifest);
    let shown = breakdown(Some(0));
    assert_eq!((shown.windows, shown.window), (manifest, Some(0)));
}
