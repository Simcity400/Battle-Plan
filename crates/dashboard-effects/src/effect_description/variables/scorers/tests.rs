use super::*;
use crate::effect_description::tests::{Fixture, fresh};
use content_reader_politics::country_scorers::Database;
use content_reader_script::key_names::KeyNames;

fn make_database(body: &[u8]) -> (KeyNames, Database) {
    let mut names = KeyNames::default();
    names.countries = [b"AAA".to_vec(), b"BBB".to_vec(), b"CCC".to_vec()].into();
    let mut database = Database::default();
    database.read_file(body, &names);
    assert!(database.notes.is_empty(), "{:?}", database.notes);
    (names, database)
}

#[test]
fn scorer_highest_keeps_first_tie_sorted_replaces_later_score_and_uses_ties() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLEPLAN_GAME_DIR must name the supported installation for runtime table tests");
    let fixture = Fixture::new();
    let (names, database) = make_database(b"sample={targets={targets={BBB AAA CCC} score={base=10}} targets={targets={BBB} score={base=3}}} ties={targets={targets={CCC BBB AAA} score={base=5}}}");
    fixture.with_renderer(&mut fresh(), Content { key_names: Some(&names), country_scorers: Some(&database), ..Default::default() }, |renderer| {
        let scopes = renderer.evaluator().unwrap().event_scopes(0).unwrap();
        let effects = content_reader_script::effect_keys::read_body(b"get_highest_scored_country_temp={var=chosen scorer=sample} get_sorted_scored_countries_temp={array=ordered scores=points scorer=sample} get_sorted_scored_countries_temp={array=tied scores=tied_points scorer=ties}", &names).unwrap();
        assert!(renderer.group(&effects, scopes, 0).is_empty());
        assert!(renderer.out.refusals.is_empty(), "{:?}", renderer.out.refusals);
        let evaluator = renderer.evaluator().unwrap();
        assert_eq!(evaluator.scoped_country(b"chosen", scopes).unwrap(), Some(1));
        assert_eq!(evaluator.scoped_value(b"ordered^num", scopes).unwrap(), 300_000);
        assert_eq!(evaluator.scoped_country(b"ordered^2", scopes).unwrap(), Some(2));
        assert_eq!(evaluator.scoped_value(b"points^2", scopes).unwrap(), 300_000);
        assert_eq!(evaluator.scoped_country(b"tied^0", scopes).unwrap(), Some(2));
        assert_eq!(evaluator.scoped_country(b"tied^1", scopes).unwrap(), Some(3));
        assert_eq!(evaluator.scoped_country(b"tied^2", scopes).unwrap(), Some(1));
    });
}

#[test]
fn scorer_filters_reverse_from_for_score_and_reads_shared_temporary_array() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLEPLAN_GAME_DIR must name the supported installation for runtime table tests");
    let fixture = Fixture::new();
    let (names, database) = make_database(b"sample={targets={target_array=candidates targets={AAA BBB} target_root_trigger={tag=AAA} target_trigger={FROM={tag=BBB}} score={base=1 modifier={FROM={tag=AAA} add=2}}}} changing={targets={target_array=candidates target_trigger={set_temp_variable={var=candidates^1 value=CCC.id}} score={base=1 modifier={tag=CCC add=2}}}}");
    fixture.with_renderer(&mut fresh(), Content { key_names: Some(&names), country_scorers: Some(&database), ..Default::default() }, |renderer| {
        let scopes = renderer.evaluator().unwrap().event_scopes(0).unwrap();
        let default = renderer.evaluator().unwrap().fresh_country_scope(None, None).unwrap();
        assert_eq!(default.this, Scope::Country(0));
        assert_eq!(default.root, Scope::Country(0));
        let effects = content_reader_script::effect_keys::read_body(b"set_temp_variable={var=candidates^num value=1} set_temp_variable={var=candidates^0 value=BBB.id} get_highest_scored_country_temp={scorer=sample} get_sorted_scored_countries_temp={scorer=sample}", &names).unwrap();
        assert!(renderer.group(&effects, scopes, 0).is_empty());
        assert!(renderer.out.refusals.is_empty(), "{:?}", renderer.out.refusals);
        let evaluator = renderer.evaluator().unwrap();
        assert_eq!(evaluator.scoped_country(b"highest_scored_country", scopes).unwrap(), Some(2));
        assert_eq!(evaluator.scoped_value(b"country_list_scores^0", scopes).unwrap(), 300_000);
        assert_eq!(evaluator.scoped_value(b"sorted_country_list^num", scopes).unwrap(), 100_000);
        let effects = content_reader_script::effect_keys::read_body(b"set_temp_variable={var=candidates^num value=2} set_temp_variable={var=candidates^0 value=AAA.id} set_temp_variable={var=candidates^1 value=BBB.id} get_highest_scored_country_temp={var=changed scorer=changing}", &names).unwrap();
        assert!(renderer.group(&effects, scopes, 0).is_empty());
        assert!(renderer.out.refusals.is_empty(), "{:?}", renderer.out.refusals);
        assert_eq!(renderer.evaluator().unwrap().scoped_country(b"changed", scopes).unwrap(), Some(3));
    });
}

#[test]
fn invalid_scorer_is_noop_and_unknown_target_form_is_named_once() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLEPLAN_GAME_DIR must name the supported installation for runtime table tests");
    let fixture = Fixture::new();
    let (names, database) = make_database(b"sample={targets={state_target=yes}}");
    let out = fixture.render_native(
        "get_highest_scored_country_temp={scorer=missing} army_experience=2",
        &mut fresh(),
        Content {
            key_names: Some(&names),
            country_scorers: Some(&database),
            ..Default::default()
        },
    );
    assert_eq!(out.text, "XP2\n");
    assert!(out.refusals.is_empty(), "{:?}", out.refusals);
    let out = fixture.render_native(
        "get_highest_scored_country_temp={scorer=sample}",
        &mut fresh(),
        Content {
            key_names: Some(&names),
            country_scorers: Some(&database),
            ..Default::default()
        },
    );
    assert_eq!(out.refusals.values().sum::<usize>(), 1);
    assert!(
        out.refusals
            .keys()
            .any(|why| why.contains("state_target") && why.contains("selection mode")),
        "{:?}",
        out.refusals
    );
    let (_, usable) = make_database(b"sample={targets={targets={AAA}}}");
    let out = fixture.render_native(
        "get_highest_scored_country_temp={scorer=sample var=chosen var=other}",
        &mut fresh(),
        Content {
            key_names: Some(&names),
            country_scorers: Some(&usable),
            ..Default::default()
        },
    );
    assert_eq!(out.refusals.values().sum::<usize>(), 1);
    assert!(
        out.refusals
            .keys()
            .any(|why| why.contains("repeated variable operand other")
                && why.contains("keeps fields from its earlier read")),
        "{:?}",
        out.refusals
    );
}

#[test]
fn score_map_rehashes_and_preserves_distinct_values() {
    let mut scores = ScoreMap::default();
    for country in 1..=5_000 {
        scores.insert(country, i64::from(country));
    }
    scores.insert(100, 6_000);
    let sorted = scores.sorted();
    assert_eq!(sorted.len(), 5_000);
    assert_eq!(sorted[0], (100, 6_000));
    assert_eq!(sorted[1], (5_000, 5_000));
}
