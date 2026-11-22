use super::test_support::*;
use super::*;
use std::{collections::BTreeSet, io};

fn country_frame(count: u32, cosmetic: &str) -> replay_frame::frame::DecodedFrame {
    use replay_data::{
        rows::CountryAppearanceRow, rows::CountryCounterInputRow, rows::CountryLinkRow,
        rows::CountryRow, rows::PoliticalPartyRow, rows::TableId, rows::TableRows,
    };
    use replay_frame::frame::TableView;
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
                        forward_visibility: replay_data::rows::Recorded::NotRecorded,
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
    viewer_scene_base::test_game::enter();
    let pools = content(br#"default = { tank = {
        pool = { weight = 1 models = { fallback } }
        pool = { weight = 2 limit = { AAA = { has_cosmetic_tag = costume has_government = government } } models = { changed } }
    }}"#).unwrap();
    for (cosmetic, expected) in [("costume", "changed"), ("other", "fallback")] {
        with_world(&country_frame(12, cosmetic), &[], |limits| {
            for _ in 0..100 {
                assert_eq!(
                    pools
                        .lookup_with_limits(&context(None, &[]), &limits, |_| true)
                        .unwrap()[0]
                        .entity,
                    expected
                );
            }
            assert_eq!(limits.values.0.borrow().len(), 2);
        });
    }
    let mut missing = country_frame(1, "costume");
    *missing.tables[replay_data::rows::TableId::CountryAppearance.index()] =
        replay_frame::frame::TableView::Unavailable {
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
    viewer_scene_base::test_game::enter();
    let pools = content(
        br#"default = { tank = {
        pool = { limit = { check_variable = { var = global.pool_ready value = 1 compare = equals } } models = { ready } }
    }}"#,
    )
    .unwrap();
    let mut frame = empty_frame();
    use replay_data::{
        rows::ScriptVariableEntry, rows::ScriptVariablesRow, rows::TableRows, rows::tables,
    };
    use replay_frame::frame::TableView;
    *frame.tables[replay_data::rows::TableId::ScriptVariables.index()] =
        TableView::Complete(TableRows::from_rows::<tables::ScriptVariables>(vec![
            ScriptVariablesRow {
                scope_kind: 0,
                scope_id: 0,
                seed: replay_data::rows::Recorded::Value(None),
                entries: vec![ScriptVariableEntry {
                    name: b"pool_ready".to_vec(),
                    value_raw: 100_000,
                }],
            },
        ]));
    with_world(&frame, &[], |limits| {
        assert_eq!(
            pools
                .lookup_with_limits(&context(None, &[]), &limits, |_| true)
                .unwrap()[0]
                .entity,
            "ready"
        );
    });
    *frame.tables[replay_data::rows::TableId::ScriptVariables.index()] =
        replay_frame::frame::TableView::Unavailable {
            reason: "not sampled".into(),
        };
    with_world(&frame, &[], |limits| {
        let why = pools
            .lookup_with_limits(&context(None, &[]), &limits, |_| true)
            .unwrap_err();
        assert!(why.contains("ScriptVariables"), "{why}");
    });
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
        capital_continent: Ok(Some("Europe")),
    }
}

#[test]
fn filters_and_dlc_limits_decide_pools_and_unknown_predicates_fail() {
    viewer_scene_base::test_game::enter();
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
