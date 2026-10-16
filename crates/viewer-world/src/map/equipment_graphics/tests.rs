use super::test_support::*;
use super::*;
use std::time::Instant;
use std::{collections::BTreeSet, io};

fn country_frame(count: u32, cosmetic: &str) -> replay::frame::DecodedFrame {
    use replay::{
        frame::TableView,
        rows::{
            CountryAppearanceRow, CountryCounterInputRow, CountryLinkRow, CountryRow,
            PoliticalPartyRow, TableId, TableRows,
        },
    };
    let mut frame = empty_frame();
    let tag = |i: u32| {
        vec![
            b'A' + (i / 676) as u8,
            b'A' + (i / 26 % 26) as u8,
            b'A' + (i % 26) as u8,
        ]
    };
    for (id, rows) in [
        (
            TableId::Countries,
            TableRows::Countries(
                (0..count)
                    .map(|i| CountryRow {
                        id: i + 1,
                        canonical_slot: i + 1,
                        tag: tag(i),
                        ..Default::default()
                    })
                    .collect(),
            ),
        ),
        (
            TableId::CountryLinks,
            TableRows::CountryLinks(
                (0..count)
                    .map(|i| CountryLinkRow {
                        id: i + 1,
                        canonical_slot: i + 1,
                    })
                    .collect(),
            ),
        ),
        (
            TableId::CountryCounterInputs,
            TableRows::CountryCounterInputs(
                (0..count)
                    .map(|i| CountryCounterInputRow {
                        forward_visibility: replay::rows::Recorded::NotRecorded,
                        country: i + 1,
                        owned_state_count: 1,
                        ..Default::default()
                    })
                    .collect(),
            ),
        ),
        (
            TableId::CountryAppearance,
            TableRows::CountryAppearance(
                (0..count)
                    .map(|i| CountryAppearanceRow {
                        country: i + 1,
                        name_tag: tag(i),
                        cosmetic_tag: cosmetic.as_bytes().to_vec(),
                        ..Default::default()
                    })
                    .collect(),
            ),
        ),
        (
            TableId::PoliticalParties,
            TableRows::PoliticalParties(
                (0..count)
                    .map(|i| PoliticalPartyRow {
                        country: i + 1,
                        token: b"government".to_vec(),
                        is_ruling: 1,
                        ..Default::default()
                    })
                    .collect(),
            ),
        ),
    ] {
        *frame.tables[id.index()] = TableView::Complete(rows);
    }
    frame
}

#[test]
fn pool_choice_reads_this_hours_world_and_keeps_unknown_named() {
    engine::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
    let pools = content(br#"default = { tank = {
        pool = { weight = 1 models = { fallback } }
        pool = { weight = 2 limit = { AAA = { has_cosmetic_tag = costume has_government = government } } models = { changed } }
    }}"#).unwrap();
    for (cosmetic, expected) in [("costume", "changed"), ("other", "fallback")] {
        with_world(&country_frame(5_000, cosmetic), &[], |limits| {
            for _ in 0..100 {
                assert_eq!(
                    pools
                        .lookup_with_limits(&context(None, &[]), &limits, |_| true)
                        .unwrap()[0]
                        .entity,
                    expected
                );
            }
            // One result per pool, independent of unit count.
            assert_eq!(limits.values.0.borrow().len(), 2);
        });
    }
    let mut missing = country_frame(1, "costume");
    *missing.tables[replay::rows::TableId::CountryAppearance.index()] =
        replay::frame::TableView::Unavailable {
            reason: "not sampled".into(),
        };
    with_world(&missing, &[], |limits| {
        let why = pools
            .lookup_with_limits(&context(None, &[]), &limits, |_| true)
            .unwrap_err();
        assert!(why.contains("CountryAppearance"), "{why}");
        assert!(why.contains("in fixture:"), "{why}");
    });
}

#[test]
fn pool_check_starts_a_fresh_variable_context() {
    engine::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
    let pools = content(
        br#"default = { tank = {
        pool = { limit = { check_variable = { global.pool_ready = 1 } } models = { ready } }
    }}"#,
    )
    .unwrap();
    let mut frame = empty_frame();
    use replay::{
        frame::TableView,
        rows::{ScriptVariableEntry, ScriptVariablesRow, TableRows, tables},
    };
    *frame.tables[replay::rows::TableId::ScriptVariables.index()] =
        TableView::Complete(TableRows::from_rows::<tables::ScriptVariables>(vec![
            ScriptVariablesRow {
                scope_kind: 0,
                scope_id: 0,
                seed: replay::rows::Recorded::Value(None),
                entries: vec![ScriptVariableEntry {
                    name: b"pool_ready".to_vec(),
                    value_raw: 100_000,
                }],
            },
        ]));
    with_world(&frame, &[], |limits| {
        // Evaluation clears prior temporary values, so this
        // operand reaches the recorded global variable store.
        assert_eq!(
            pools
                .lookup_with_limits(&context(None, &[]), &limits, |_| true)
                .unwrap()[0]
                .entity,
            "ready"
        );
    });
    *frame.tables[replay::rows::TableId::ScriptVariables.index()] =
        replay::frame::TableView::Unavailable {
            reason: "not sampled".into(),
        };
    with_world(&frame, &[], |limits| {
        let why = pools
            .lookup_with_limits(&context(None, &[]), &limits, |_| true)
            .unwrap_err();
        assert!(why.contains("ScriptVariables"), "{why}");
    });
}

#[test]
#[ignore = "release timing; run without concurrent builds"]
fn pool_limits_builder_budget() {
    engine::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
    let mut text = String::from("default = { tank = {");
    for i in 0..510 {
        text.push_str(&format!("pool = {{ limit = {{ AAA = {{ has_cosmetic_tag = costume has_government = government }} }} models = {{ m{i} }} }}"));
    }
    text.push_str("} }");
    let content = content(text.as_bytes()).unwrap();
    let pool_count = 510;
    for countries in [1_500, 5_000] {
        let frame = country_frame(countries, "costume");
        let start = Instant::now();
        let hours = 128;
        for _ in 0..hours {
            with_world(&frame, &[], |limits| {
                let candidates = content
                    .lookup_with_limits(&context(None, &[]), &limits, |_| true)
                    .unwrap();
                assert_eq!(std::hint::black_box(candidates).len(), pool_count);
                assert_eq!(limits.values.0.borrow().len(), pool_count);
            });
        }
        println!(
            "POOL_LIMITS countries={countries} pools={} hours={hours} ms_per_hour={:.3}",
            pool_count,
            start.elapsed().as_secs_f64() * 1000.0 / f64::from(hours)
        );
    }
}

fn names() -> Names {
    Names {
        countries: ["AAA", "GER", "EN2"].map(str::to_owned).into(),
        continents: ["europe"].map(str::to_owned).into(),
    }
}

fn content(text: &[u8]) -> io::Result<Content> {
    Content::from_texts(&[text], &names())
}

fn context<'a>(country: Option<&'a str>, factions: &'a [String]) -> Context<'a> {
    Context {
        equipment: "tank_1",
        archetype: Some("tank"),
        family: None,
        subunit: Some("light_armor"),
        ideology: Some("fascism"),
        culture: Some("western_european_gfx"),
        country,
        faction_tags: factions,
        capital_continent: Some("Europe"),
    }
}

#[test]
fn filters_and_dlc_limits_decide_pools_and_unknown_predicates_fail() {
    engine::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
    let pools = content(
        br#"
        default = { tank = {
            pool = { sub_units = { medium_armor } models = { wrong_unit } }
            pool = { ideologies = { FASCISM } cultures = { western_european_gfx } models = { right } }
            pool = { limit = { has_dlc = "No Step Back" } models = { dlc } }
            pool = { limit = { NOT = { has_dlc = "No Step Back" } } models = { no_dlc } }
            pool = { limit = { OR = { has_dlc = a has_dlc = "No Step Back" } } models = { either } }
        } }
    "#,
    )
    .unwrap();
    let found = with_world(&empty_frame(), &["No Step Back"], |limits| {
        pools.lookup_with_limits(&context(None, &[]), &limits, |_| true)
    })
    .unwrap();
    let entities: BTreeSet<_> = found.iter().map(|c| c.entity.as_str()).collect();
    assert_eq!(entities, BTreeSet::from(["right", "dlc", "either"]));
    // Scope-masked predicates are false before they read any input.
    let scoped =
        content(b"default = { tank = { pool = { limit = { tag = GER } models = { x } } } }")
            .unwrap();
    assert!(
        with_world(&empty_frame(), &[], |limits| {
            scoped.lookup_with_limits(&context(None, &[]), &limits, |_| true)
        })
        .unwrap()
        .is_empty()
    );
}
