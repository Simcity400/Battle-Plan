use super::*;
use crate::effect_description::tests::{Fixture, fresh};
use replay_data::rows::*;
use replay_frame::frame::TableView;
use crate::fixtures::game;

#[test]
fn format99_country_rosters_preserve_stored_order() {
    game::enter();
    let mut fixture = Fixture::new();
    let countries = crate::effect_description::tests::synthetic_countries(24);
    *fixture.frame.tables[TableId::CountryLinks.index()] =
        TableView::Complete(TableRows::CountryLinks(
            countries
                .iter()
                .map(|row| CountryLinkRow {
                    id: row.id,
                    canonical_slot: row.canonical_slot,
                })
                .collect(),
        ));
    *fixture.frame.tables[TableId::Countries.index()] =
        TableView::Complete(TableRows::Countries(countries));
    let references: Vec<_> = (1..=24).rev().map(|slot| slot * 100).collect();
    *fixture.frame.tables[TableId::CountryCounterInputs.index()] =
        TableView::Complete(TableRows::CountryCounterInputs(vec![
            CountryCounterInputRow {
                country: 100,
                neighbors: Recorded::Value(Some(references.clone())),
                ..Default::default()
            },
        ]));
    *fixture.frame.tables[TableId::OccupationSettings.index()] =
        TableView::Complete(TableRows::OccupationSettings(vec![OccupationSettingsRow {
            country: 100,
            occupied: Recorded::Value(Some(references)),
            ..Default::default()
        }]));
    let item = content_reader_parser::parser::parse(b"fixture_list_neighbor_country={}")
        .unwrap()
        .items
        .remove(0);
    let roster = |fixture: &Fixture, source| {
        fixture.with_renderer(&mut fresh(), Content::default(), |renderer| {
            renderer.roster(source, &item, Scopes::of(Scope::Country(1)))
        })
    };
    let expected: Vec<_> = (1..=24).rev().map(Scope::Country).collect();
    for source in [Roster::CountryNeighbors, Roster::OccupiedCountries] {
        assert_eq!(roster(&fixture, source).unwrap(), expected);
    }
    let TableView::Complete(TableRows::CountryCounterInputs(rows)) =
        &mut *fixture.frame.tables[TableId::CountryCounterInputs.index()]
    else {
        unreachable!()
    };
    rows[0].neighbors = Recorded::Value(None);
    assert!(
        roster(&fixture, Roster::CountryNeighbors)
            .unwrap_err()
            .contains("neighbors of country counter inputs capture refused")
    );
    let TableView::Complete(TableRows::OccupationSettings(rows)) =
        &mut *fixture.frame.tables[TableId::OccupationSettings.index()]
    else {
        unreachable!()
    };
    rows[0].occupied = Recorded::NotRecorded;
    assert!(
        roster(&fixture, Roster::OccupiedCountries)
            .unwrap_err()
            .contains("occupied of occupation settings not recorded")
    );

    *fixture.frame.tables[TableId::Divisions.index()] = TableView::Complete(TableRows::Divisions(
        (0_u32..3)
            .rev()
            .map(|ordinal| DivisionRow {
                id: 39 | u64::from(ordinal + 1) << 32,
                roster_owner: 100,
                ..Default::default()
            })
            .collect(),
    ));
    *fixture.frame.tables[TableId::DivisionStatus.index()] =
        TableView::Complete(TableRows::DivisionStatus(
            (0_u32..3)
                .map(|ordinal| DivisionStatusRow {
                    division: 39 | u64::from(ordinal + 1) << 32,
                    country_ordinal: Recorded::Value(Some(ordinal)),
                    ..Default::default()
                })
                .collect(),
        ));
    assert_eq!(
        roster(&fixture, Roster::CountryDivisions).unwrap(),
        (0_u32..3)
            .map(|ordinal| Scope::Unit(39 | u64::from(ordinal + 1) << 32))
            .collect::<Vec<_>>()
    );
    let TableView::Complete(TableRows::DivisionStatus(rows)) =
        &mut *fixture.frame.tables[TableId::DivisionStatus.index()]
    else {
        unreachable!()
    };
    rows[2].country_ordinal = Recorded::Value(Some(0));
    assert!(
        roster(&fixture, Roster::CountryDivisions)
            .unwrap_err()
            .contains("not a complete country roster")
    );
}

#[test]
fn state_divisions_follow_native_province_and_mixed_unit_order() {
    game::enter();
    use crate::triggers::ContentFacts;
    let mut fixture = Fixture::new();
    let first = 39 | 1_u64 << 32;
    let second = 4713 | 1_u64 << 32;
    let third = 39 | 2_u64 << 32;
    let fleet = 4713 | 2_u64 << 32;
    let gun = 4713 | 3_u64 << 32;
    let provinces = BTreeMap::from([(7, vec![4, 2, 1])]);
    *fixture.frame.tables[TableId::States.index()] =
        TableView::Complete(TableRows::States(vec![StateRow {
            id: 7,
            ..Default::default()
        }]));
    *fixture.frame.tables[TableId::Provinces.index()] = TableView::Complete(TableRows::Provinces(
        [1, 2, 4]
            .map(|id| ProvinceRow {
                id,
                state: 7,
                ..Default::default()
            })
            .to_vec(),
    ));
    *fixture.frame.tables[TableId::Divisions.index()] = TableView::Complete(TableRows::Divisions(
        [(first, 4), (second, 4), (third, 2)]
            .map(|(id, province)| DivisionRow {
                id,
                province,
                ..Default::default()
            })
            .to_vec(),
    ));
    *fixture.frame.tables[TableId::TaskForces.index()] =
        TableView::Complete(TableRows::TaskForces(vec![TaskForceRow {
            id: fleet,
            province: 4,
            ..Default::default()
        }]));
    *fixture.frame.tables[TableId::RailwayGunCounterInputs.index()] =
        TableView::Complete(TableRows::RailwayGunCounterInputs(vec![
            RailwayGunCounterInputRow {
                gun,
                province: 4,
                ..Default::default()
            },
        ]));
    *fixture.frame.tables[TableId::ProvinceUnitOrder.index()] =
        TableView::Complete(TableRows::ProvinceUnitOrder(vec![
            ProvinceUnitOrderRow {
                province: 1,
                ..Default::default()
            },
            ProvinceUnitOrderRow {
                province: 2,
                mixed: vec![third],
                ..Default::default()
            },
            ProvinceUnitOrderRow {
                province: 4,
                mixed: vec![fleet, second, gun, first],
                ..Default::default()
            },
        ]));
    let roster = |fixture: &Fixture, scope| {
        let scripted = fixture.catalogue.scripted_content().unwrap();
        let text = ScriptedText::new(&fixture.frame, &fixture.catalogue.localisation, scripted, 1);
        let dlcs = std::collections::BTreeSet::new();
        let mut facts = ContentFacts::content_only(&dlcs, &fixture.triggers, &fixture.aliases);
        facts.state_provinces = &provinces;
        facts.state_content = Ok(());
        text.set_synthetic_world(World::read(&fixture.frame, facts));
        let context = Context::new(
            &text,
            &fixture.catalogue.localisation,
            1,
            "synthetic",
            Content::default(),
        );
        let renderer = Renderer {
            context: &context,
            graph: Rc::new(RefCell::new(fresh())),
            evaluator: OnceCell::new(),
            out: Description::default(),
            scripted_stack: Vec::new(),
            concise: None,
        };
        let item = content_reader_parser::parser::parse(b"fixture_list_state_division={}")
            .unwrap()
            .items
            .remove(0);
        renderer.roster(Roster::StateDivisions, &item, Scopes::of(scope))
    };
    assert_eq!(
        roster(&fixture, Scope::State(7)).unwrap(),
        vec![Scope::Unit(second), Scope::Unit(first), Scope::Unit(third)]
    );
    for scope in [Scope::State(0), Scope::State(8), Scope::Country(1)] {
        assert!(roster(&fixture, scope).unwrap().is_empty());
    }
    let TableView::Complete(TableRows::ProvinceUnitOrder(rows)) =
        &mut *fixture.frame.tables[TableId::ProvinceUnitOrder.index()]
    else {
        unreachable!()
    };
    rows[2].mixed.push(39 | 99_u64 << 32);
    assert!(
        roster(&fixture, Scope::State(7))
            .unwrap_err()
            .contains("no recorded class census")
    );
    let TableView::Complete(TableRows::ProvinceUnitOrder(rows)) =
        &mut *fixture.frame.tables[TableId::ProvinceUnitOrder.index()]
    else {
        unreachable!()
    };
    rows[2].mixed.pop();
    rows.remove(0);
    assert!(
        roster(&fixture, Scope::State(7))
            .unwrap_err()
            .contains("province unit order lacks state 7 province 1")
    );
    let TableView::Complete(TableRows::Provinces(rows)) =
        &mut *fixture.frame.tables[TableId::Provinces.index()]
    else {
        unreachable!()
    };
    rows[0].state = 8;
    assert!(
        roster(&fixture, Scope::State(7))
            .unwrap_err()
            .contains("native order is unknown")
    );
}

#[test]
fn format99_active_scientists_use_raw_nonzero_byte_and_native_order() {
    game::enter();
    let mut fixture = Fixture::new();
    *fixture.frame.tables[TableId::Scientists.index()] =
        TableView::Complete(TableRows::Scientists(
            [(2, 7, 255), (0, 8, 1), (1, 9, 0)]
                .into_iter()
                .map(|(ordinal, character, active)| ScientistRow {
                    country: 100,
                    ordinal,
                    character,
                    active: Recorded::Value(Some(active)),
                    ..Default::default()
                })
                .collect(),
        ));
    fixture.with_renderer(&mut fresh(), Content::default(), |renderer| {
        let item = content_reader_parser::parser::parse(b"fixture_list_active_scientist={}")
            .unwrap()
            .items
            .remove(0);
        let scopes = Scopes::of(Scope::Country(1));
        let roster = renderer
            .roster(Roster::Scientists { active: true }, &item, scopes)
            .unwrap();
        assert_eq!(
            renderer
                .targets(roster, children(&item), scopes, true)
                .unwrap(),
            vec![Scope::Character(8), Scope::Character(7)]
        );
    });
    for unavailable in [Recorded::NotRecorded, Recorded::Value(None)] {
        let TableView::Complete(TableRows::Scientists(rows)) =
            &mut *fixture.frame.tables[TableId::Scientists.index()]
        else {
            unreachable!()
        };
        rows[0].active = unavailable;
        fixture.with_renderer(&mut fresh(), Content::default(), |renderer| {
            let item = content_reader_parser::parser::parse(b"fixture_select_active_scientist={}")
                .unwrap()
                .items
                .remove(0);
            let scopes = Scopes::of(Scope::Country(1));
            let roster = renderer
                .roster(Roster::Scientists { active: true }, &item, scopes)
                .unwrap();
            assert!(
                renderer
                    .targets(roster, children(&item), scopes, true)
                    .unwrap_err()
                    .contains("active of scientists")
            );
            assert_eq!(
                renderer
                    .roster(Roster::Scientists { active: false }, &item, scopes)
                    .unwrap(),
                vec![
                    Scope::Character(8),
                    Scope::Character(9),
                    Scope::Character(7)
                ]
            );
        });
        assert_eq!(
            fixture.render(
                "fixture_list_active_scientist={limit={always=no} fixture_points=2}",
                &mut fresh(),
                Content::default()
            ),
            Description::default()
        );
    }
}

#[test]
fn inactive_scientists_still_consume_limit_variable_randomness() {
    game::enter();
    let mut fixture = Fixture::new();
    *fixture.frame.tables[TableId::Scientists.index()] =
        TableView::Complete(TableRows::Scientists(vec![ScientistRow {
            country: 100,
            character: 9,
            active: Recorded::Value(Some(0)),
            ..Default::default()
        }]));
    *fixture.frame.tables[TableId::Characters.index()] =
        TableView::Complete(TableRows::Characters(vec![CharacterRow {
            id: 9,
            country: 100,
            ..Default::default()
        }]));
    *fixture.frame.tables[TableId::ScriptVariables.index()] =
        TableView::Complete(TableRows::ScriptVariables(vec![ScriptVariablesRow {
            scope_kind: 1,
            scope_id: 100,
            seed: Recorded::Value(Some(37)),
            ..Default::default()
        }]));
    fixture.with_renderer(&mut fresh(), Content::default(), |renderer| {
        let scopes = renderer.evaluator().unwrap().event_scopes(0).unwrap();
        let evaluator = renderer.evaluator().unwrap();
        assert_eq!(evaluator.scoped_value(b"random_seed", scopes), Ok(967));
        let item = content_reader_parser::parser::parse(
            b"fixture_list_active_scientist={limit={check_variable={random < 1}}}",
        )
        .unwrap()
        .items
        .remove(0);
        let roster = renderer
            .roster(Roster::Scientists { active: true }, &item, scopes)
            .unwrap();
        assert!(
            renderer
                .targets(roster, children(&item), scopes, true)
                .unwrap()
                .is_empty()
        );
        assert_eq!(
            renderer
                .evaluator()
                .unwrap()
                .scoped_value(b"random_seed", scopes),
            Ok(1_514_732_982_084_589_112)
        );
        assert_eq!(renderer.graph.borrow().scopes[0].random_seed, 1);
    });
}

#[test]
fn format99_party_leader_selector_uses_current_ideology_and_refuses_capture_failure() {
    game::enter();
    let mut fixture = Fixture::new();
    fixture.triggers.counts = Some(content_reader_script::trigger_keys::CountTriggerNames {
        ideology_types: [b"synthetic_type".to_vec()].into(),
        ..Default::default()
    });
    *fixture.frame.tables[TableId::LexicalTokens.index()] =
        TableView::Complete(TableRows::LexicalTokens(vec![LexicalTokenRow {
            token: 11,
            literal: b"synthetic_type".to_vec(),
        }]));
    *fixture.frame.tables[TableId::Characters.index()] =
        TableView::Complete(TableRows::Characters(vec![CharacterRow {
            id: 9,
            country: 100,
            name: b"Nine".to_vec(),
            ..Default::default()
        }]));
    *fixture.frame.tables[TableId::PoliticalParties.index()] =
        TableView::Complete(TableRows::PoliticalParties(vec![PoliticalPartyRow {
            country: 100,
            id: 2,
            token: b"synthetic_group".to_vec(),
            ..Default::default()
        }]));
    *fixture.frame.tables[TableId::PartyLeaders.index()] =
        TableView::Complete(TableRows::PartyLeaders(vec![PartyLeaderRow {
            country: 100,
            party: 2,
            character: Some(9),
            role: Some(-7),
            ..Default::default()
        }]));
    *fixture.frame.tables[TableId::CountryLeaderRoles.index()] =
        TableView::Complete(TableRows::CountryLeaderRoles(vec![CountryLeaderRoleRow {
            country: 100,
            character: 9,
            ideology: -7,
            current_ideology: Recorded::Value(Some(Some(11))),
        }]));
    let body = "fixture_party_leader={limit={has_ideology=synthetic_type} fixture_points=2}";
    let output = fixture.render(body, &mut fresh(), Content::default());
    assert!(output.text.contains("Nine") && output.text.contains("XP2"));
    assert!(output.refusals.is_empty(), "{:?}", output.refusals);
    let TableView::Complete(TableRows::CountryLeaderRoles(rows)) =
        &mut *fixture.frame.tables[TableId::CountryLeaderRoles.index()]
    else {
        unreachable!()
    };
    rows[0].current_ideology = Recorded::Value(Some(None));
    let output = fixture.render(body, &mut fresh(), Content::default());
    assert!(output.text.is_empty());
    assert!(output.refusals.is_empty(), "{:?}", output.refusals);
    let TableView::Complete(TableRows::CountryLeaderRoles(rows)) =
        &mut *fixture.frame.tables[TableId::CountryLeaderRoles.index()]
    else {
        unreachable!()
    };
    rows[0].current_ideology = Recorded::Value(None);
    let output = fixture.render(body, &mut fresh(), Content::default());
    assert!(output.text.is_empty());
    assert_eq!(output.refusals.values().sum::<usize>(), 1);
    assert!(
        output
            .refusals
            .keys()
            .any(|why| why.contains("country leader roles, current ideology of character")
                && why.contains("not captured"))
    );
}

#[test]
fn industrial_organisation_iterator_uses_recorded_order_native_scopes_and_visibility() {
    game::enter();
    let mut fixture = Fixture::new();
    *fixture.frame.tables[TableId::IndustrialOrganisations.index()] =
        TableView::Complete(TableRows::IndustrialOrganisations(vec![
            IndustrialOrganisationRow {
                organisation: 79 | 1 << 32,
                country: 100,
                owner: 100,
                ordinal: 0,
                token: 10,
                name: b"REFERENCE".to_vec(),
                ..Default::default()
            },
            IndustrialOrganisationRow {
                organisation: 79 | 2 << 32,
                country: 100,
                owner: 100,
                ordinal: 1,
                token: 11,
                name: b"TINY".to_vec(),
                ..Default::default()
            },
        ]));
    *fixture.frame.tables[TableId::LexicalTokens.index()] =
        TableView::Complete(TableRows::LexicalTokens(vec![
            LexicalTokenRow {
                token: 10,
                literal: b"one".to_vec(),
            },
            LexicalTokenRow {
                token: 11,
                literal: b"two".to_vec(),
            },
        ]));
    let mut database = content_reader_military::industrial_organisations::Database::default();
    database
        .read_file(
            b"base={visible={FROM={tag=AAA}}} one={include=base} two={visible={always=no}}",
            &content_reader_script::key_names::KeyNames::default(),
        )
        .unwrap();
    let content = Content {
        industrial_organisations: Some(&database),
        ..Default::default()
    };
    let out = fixture.render(
        "fixture_list_organisation={limit={OWNER={tag=AAA}} fixture_points=2}",
        &mut fresh(),
        content,
    );
    assert_eq!(out.text, "\u{11}YAAA\u{11}!: \n   XP2\n");
    assert!(out.refusals.is_empty(), "{:?}", out.refusals);
    let out = fixture.render(
        "fixture_list_organisation={include_invisible=yes fixture_points=2}",
        &mut fresh(),
        Content::default(),
    );
    assert_eq!(out.text, "(\u{11}YAAA\u{11}!, \u{11}Yx\u{11}!):\n   XP2\n");
    assert!(out.refusals.is_empty(), "{:?}", out.refusals);
    let mut graph = fresh();
    let out = fixture.render(
        "fixture_select_organisation={fixture_points=2}",
        &mut graph,
        content,
    );
    assert_eq!(out.text, "(\u{11}YAAA\u{11}!): XP2\n");
    assert!(out.refusals.is_empty(), "{:?}", out.refusals);
    assert_eq!(graph.scopes[0].random_seed, 2);
    use crate::scripted_text::{EventObject, EventScope, EventScopes, Link};
    let mut graph = EventScopes::new(vec![
        EventScope {
            from: Link::Scope(1),
            ..EventScope::of(EventObject::IndustrialOrganisation(79 | 1 << 32))
        },
        EventScope::of(EventObject::Country(1)),
    ]);
    let out = fixture.render("THIS={fixture_points=2}", &mut graph, content);
    assert_eq!(out.text, "AAA \u{12}AAA XP2\n");
    assert!(out.refusals.is_empty(), "{:?}", out.refusals);
    database = content_reader_military::industrial_organisations::Database::default();
    database
        .read_file(
            b"base={visible={always=no}} one={visible={always=no}} two={delete_included_values={visible} include=base}",
            &content_reader_script::key_names::KeyNames::default(),
        )
        .unwrap();
    let out = fixture.render(
        "fixture_list_organisation={fixture_points=2}",
        &mut fresh(),
        Content {
            industrial_organisations: Some(&database),
            ..Default::default()
        },
    );
    assert_eq!(out.text, "\u{11}Yx\u{11}!: \n   XP2\n");
    assert!(out.refusals.is_empty(), "{:?}", out.refusals);
    let out = fixture.render("fixture_list_organisation={include_invisible=yes limit={check_variable={var=unrecorded value=1}} fixture_points=2}", &mut fresh(), Content::default());
    assert!(out.text.is_empty());
    assert_eq!(out.refusals.values().sum::<usize>(), 1);
    assert!(
        out.refusals
            .keys()
            .any(|why| why.contains("industrial organisation details")),
        "{:?}",
        out.refusals
    );
}

#[test]
fn shared_multiple_iterator_truncates_and_keeps_native_layout_options() {
    game::enter();
    let fixture = Fixture::new();
    for (limit, expected) in [("NOT={state=5}", "Four"), ("always=yes", "2 others")] {
        let out = fixture.render(
            &format!("fixture_all_states={{limit={{{limit}}} fixture_points=2}}"),
            &mut fresh(),
            Content::default(),
        );
        assert!(out.refusals.is_empty(), "{:?}", out.refusals);
        assert!(out.text.contains(expected), "{}", out.text);
        assert_eq!(out.text.matches("XP2").count(), 1);
    }
    let out = fixture.render("fixture_all_states={limit={state=1} display_individual_scopes=yes fixture_points=2} fixture_all_states={limit={always=no} visible_when_empty=yes tooltip=TIP fixture_points=3}", &mut fresh(), Content::default());
    assert_eq!(out.text, "(\u{11}YOne\u{11}!):\n   XP2\ntip:\n   XP3\n");
    assert!(out.refusals.is_empty(), "{:?}", out.refusals);
    let out = fixture.render(
        "BBB={fixture_all_states={limit={state=1} fixture_points=2}}",
        &mut fresh(),
        Content::default(),
    );
    assert_eq!(
        out.text,
        "\u{12}BBBBeta:\n \u{11}YOne\u{11}!: \n       XP2\n"
    );
}

#[test]
fn random_country_labels_share_native_marked_definite_names() {
    game::enter();
    let fixture = Fixture::new();
    for (tag, name) in [("AAA", "Alpha"), ("BBB", "Beta")] {
        let out = fixture.render(
            &format!("fixture_select_country={{limit={{tag={tag}}} fixture_points=2}}"),
            &mut fresh(),
            Content::default(),
        );
        assert_eq!(
            out.text,
            format!("(\u{11}Y\u{12}{tag} {name}\u{11}!): XP2\n")
        );
        assert!(out.refusals.is_empty(), "{:?}", out.refusals);
    }
}

#[test]
fn unrecorded_subjects_refuse_the_subject_roster() {
    game::enter();
    let mut fixture = Fixture::new();
    let TableView::Complete(TableRows::CountryAppearance(rows)) =
        &mut *fixture.frame.tables[TableId::CountryAppearance.index()]
    else {
        panic!("the fixture records country appearance");
    };
    rows[0].subjects = Recorded::NotRecorded;
    let out = fixture.render(
        "fixture_list_subject_country={fixture_points=2}",
        &mut fresh(),
        Content::default(),
    );
    assert!(
        out.refusals
            .keys()
            .any(|reason| reason.contains("subjects of country appearance not recorded")),
        "{:?}",
        out.refusals
    );
}

#[test]
fn subject_roster_preserves_alias_duplicates_after_native_filters() {
    game::enter();
    let mut fixture = Fixture::new();
    crate::test_country_links::extend(
        &mut fixture.frame,
        [
            CountryLinkRow {
                id: 101,
                canonical_slot: 1,
            },
            CountryLinkRow {
                id: 301,
                canonical_slot: 3,
            },
        ],
    );
    let TableView::Complete(TableRows::CountryAppearance(rows)) =
        &mut *fixture.frame.tables[TableId::CountryAppearance.index()]
    else {
        unreachable!()
    };
    rows[0].subjects = vec![-1, 0, 100, 101, 301, 200, 300, 301].into();
    let TableView::Complete(TableRows::CountryCounterInputs(rows)) =
        &mut *fixture.frame.tables[TableId::CountryCounterInputs.index()]
    else {
        unreachable!()
    };
    rows[1].owned_state_count = 0;
    let out = fixture.render(
        "fixture_list_subject_country={fixture_points=2}",
        &mut fresh(),
        Content::default(),
    );
    assert_eq!(
        out.text,
        "(\u{12}CCCGamma, \u{12}CCCGamma, \u{12}CCCGamma):\n   XP2\n"
    );
    assert!(out.refusals.is_empty(), "{:?}", out.refusals);
}

#[test]
fn subject_roster_reads_existence_before_excluding_caller_alias() {
    game::enter();
    let mut fixture = Fixture::new();
    let TableView::Complete(TableRows::CountryAppearance(rows)) =
        &mut *fixture.frame.tables[TableId::CountryAppearance.index()]
    else {
        unreachable!()
    };
    rows[0].subjects = vec![101].into();
    crate::test_country_links::extend(
        &mut fixture.frame,
        [CountryLinkRow {
            id: 101,
            canonical_slot: 1,
        }],
    );
    let TableView::Complete(TableRows::CountryCounterInputs(rows)) =
        &mut *fixture.frame.tables[TableId::CountryCounterInputs.index()]
    else {
        unreachable!()
    };
    rows.retain(|row| row.country != 100);
    let out = fixture.render(
        "fixture_list_subject_country={fixture_points=2} fixture_select_subject_country={fixture_points=3}",
        &mut fresh(),
        Content::default(),
    );
    assert!(out.text.is_empty());
    assert_eq!(
        out.refusals.values().sum::<usize>(),
        2,
        "{:?}",
        out.refusals
    );
    assert!(
        out.refusals
            .keys()
            .all(|why| why.contains("country 100 has no country counter inputs row"))
    );
}

#[test]
fn recorded_country_subject_and_character_rosters_select_natively() {
    game::enter();
    let mut fixture = Fixture::new();
    let out = fixture.render(
        "fixture_list_other_country={fixture_points=2}",
        &mut fresh(),
        Content::default(),
    );
    assert_eq!(out.text, "(\u{12}BBBBeta, \u{12}CCCGamma):\n   XP2\n");
    if let TableView::Complete(TableRows::CountryAppearance(rows)) =
        &mut *fixture.frame.tables[TableId::CountryAppearance.index()]
    {
        rows[0].subjects = vec![300, 200].into();
    }
    let out = fixture.render(
        "fixture_list_subject_country={fixture_points=2}",
        &mut fresh(),
        Content::default(),
    );
    assert!(out.refusals.is_empty(), "{:?}", out.refusals);
    assert_eq!(out.text, "(\u{12}CCCGamma, \u{12}BBBBeta):\n   XP2\n");
    *fixture.frame.tables[TableId::CountryCharacters.index()] =
        TableView::Complete(TableRows::CountryCharacters(vec![CountryCharacterRow {
            country: 100,
            ordinal: 0,
            character: Some(9),
        }]));
    *fixture.frame.tables[TableId::Characters.index()] =
        TableView::Complete(TableRows::Characters(vec![CharacterRow {
            id: 9,
            country: 100,
            name: b"Scientist".to_vec(),
            ..Default::default()
        }]));
    let out = fixture.render(
        "fixture_list_character={fixture_points=2}",
        &mut fresh(),
        Content::default(),
    );
    assert_eq!(out.text, "\u{11}YScientist\u{11}!: \n   XP2\n");
    assert!(out.refusals.is_empty(), "{:?}", out.refusals);
    *fixture.frame.tables[TableId::Scientists.index()] =
        TableView::Complete(TableRows::Scientists(vec![ScientistRow {
            country: 100,
            ordinal: 0,
            character: 9,
            ..Default::default()
        }]));
    assert_eq!(
        fixture
            .render(
                "fixture_select_scientist={fixture_points=2}",
                &mut fresh(),
                Content::default()
            )
            .text,
        "(\u{11}YScientist\u{11}!): XP2\n"
    );
    *fixture.frame.tables[TableId::FactionMembers.index()] =
        TableView::Complete(TableRows::FactionMembers(vec![
            FactionMemberRow {
                faction: 4,
                country: 100,
                ordinal: 2,
            },
            FactionMemberRow {
                faction: 4,
                country: 200,
                ordinal: 1,
            },
            FactionMemberRow {
                faction: 4,
                country: 300,
                ordinal: 0,
            },
        ]));
    let out = fixture.render(
        "fixture_list_allied_country={fixture_points=2}",
        &mut fresh(),
        Content::default(),
    );
    assert_eq!(out.text, "(\u{12}CCCGamma, \u{12}BBBBeta):\n   XP2\n");
    assert!(out.refusals.is_empty(), "{:?}", out.refusals);
    *fixture.frame.tables[TableId::WarRelations.index()] =
        TableView::Complete(TableRows::WarRelations(vec![WarRelationRow {
            first_country: 100,
            second_country: 200,
            end_hour: -10,
            ..Default::default()
        }]));
    let out = fixture.render(
        "fixture_list_enemy_country={fixture_points=2}",
        &mut fresh(),
        Content::default(),
    );
    assert_eq!(out.text, "\u{12}BBBBeta: \n   XP2\n");
    fixture.neighbors = Some([(1, vec![2, 4])].into());
    let mut graph =
        crate::scripted_text::EventScopes::new(vec![crate::scripted_text::EventScope::of(
            crate::scripted_text::EventObject::State(1),
        )]);
    let out = fixture.render("fixture_list_neighbor_state={fixture_points=2} fixture_select_neighbor_state={prioritize={4} fixture_points=3}", &mut graph, Content::default());
    assert_eq!(
        out.text,
        "(\u{11}YTwo\u{11}!, \u{11}YFour\u{11}!):\n   XP2\n(\u{11}YFour\u{11}!): XP3\n"
    );
    assert!(out.refusals.is_empty(), "{:?}", out.refusals);
}

#[test]
fn character_sections_sort_all_roles_and_refuse_missing_command_assignments() {
    game::enter();
    let mut fixture = Fixture::new();
    *fixture.frame.tables[TableId::CountryCharacters.index()] =
        TableView::Complete(TableRows::CountryCharacters(
            (9..=10)
                .enumerate()
                .map(|(ordinal, id)| CountryCharacterRow {
                    country: 100,
                    ordinal: ordinal as u32,
                    character: Some(id),
                })
                .collect(),
        ));
    *fixture.frame.tables[TableId::Characters.index()] =
        TableView::Complete(TableRows::Characters(
            (9..=10)
                .map(|id| CharacterRow {
                    id,
                    country: 100,
                    name: id.to_string().into_bytes(),
                    ..Default::default()
                })
                .collect(),
        ));
    *fixture.frame.tables[TableId::AdvisorRoles.index()] =
        TableView::Complete(TableRows::AdvisorRoles(vec![
            AdvisorRoleRow {
                character: 9,
                country: 100,
                ordinal: 0,
                slot: b"high_command".to_vec(),
                ..Default::default()
            },
            AdvisorRoleRow {
                character: 9,
                country: 100,
                ordinal: 1,
                slot: b"theorist".to_vec(),
                ..Default::default()
            },
            AdvisorRoleRow {
                character: 10,
                country: 100,
                ordinal: 0,
                slot: b"high_command".to_vec(),
                ..Default::default()
            },
        ]));
    *fixture.frame.tables[TableId::CharacterRoleTraits.index()] =
        TableView::Complete(TableRows::CharacterRoleTraits(vec![
            CharacterRoleTraitRow {
                character: 9,
                kind: CharacterRoleTraitRow::ADVISOR,
                role: 0,
                tokens: vec![1],
                ..Default::default()
            },
            CharacterRoleTraitRow {
                character: 9,
                kind: CharacterRoleTraitRow::ADVISOR,
                role: 1,
                tokens: vec![5],
                ..Default::default()
            },
            CharacterRoleTraitRow {
                character: 10,
                kind: CharacterRoleTraitRow::ADVISOR,
                role: 0,
                tokens: vec![2],
                ..Default::default()
            },
        ]));
    *fixture.frame.tables[TableId::LexicalTokens.index()] =
        TableView::Complete(TableRows::LexicalTokens(
            [1, 2, 5]
                .map(|token| LexicalTokenRow {
                    token,
                    literal: format!("trait_{token}").into_bytes(),
                })
                .to_vec(),
        ));
    *fixture.frame.tables[TableId::HiredAdvisors.index()] =
        TableView::Complete(TableRows::HiredAdvisors(vec![HiredAdvisorRow {
            country: 100,
            character: 9,
            slot: b"high_command".to_vec(),
            ..Default::default()
        }]));
    let out = fixture.render(
        "fixture_character_roster={}",
        &mut fresh(),
        Content::default(),
    );
    assert_eq!(
        out.text,
        "\nMilitary advisors\n\u{11}Y9\u{11}! (trait_1, in service)\n\u{11}Y10\u{11}! (trait_2)\nTheorists\n\u{11}Y9\u{11}! (trait_5)\n\n"
    );
    assert!(out.refusals.is_empty(), "{:?}", out.refusals);
    *fixture.frame.tables[TableId::UnitLeaders.index()] =
        TableView::Complete(TableRows::UnitLeaders(vec![UnitLeaderRow {
            id: 7,
            character_id: 9,
            category: 0,
            ..Default::default()
        }]));
    let out = fixture.render(
        "fixture_character_roster={}",
        &mut fresh(),
        Content::default(),
    );
    assert!(out.text.is_empty());
    assert!(
        out.refusals
            .keys()
            .any(|why| why.contains("army/navy leader command-size inputs"))
    );
}

#[test]
fn scope_chains_and_character_headers_keep_native_graph_selection() {
    game::enter();
    use crate::scripted_text::{EventObject, EventScope, Link};
    let mut fixture = Fixture::new();
    *fixture.frame.tables[TableId::Characters.index()] =
        TableView::Complete(TableRows::Characters(vec![CharacterRow {
            id: 9,
            country: 200,
            name: b"Nine".to_vec(),
            ..Default::default()
        }]));
    let mut graph = fresh();
    graph.scopes[0].from = Link::Scope(1);
    let mut from = EventScope::of(EventObject::Country(2));
    from.from = Link::Scope(2);
    graph.scopes.push(from);
    graph.scopes.push(EventScope::of(EventObject::Country(3)));
    graph
        .saved_targets
        .insert(2, vec![(b"advisor".to_vec(), EventObject::Character(9))]);
    let out = fixture.render(
        "FROM.FROM={ROOT={fixture_points=2}} event_target:advisor={fixture_points=3}",
        &mut graph,
        Content::default(),
    );
    assert!(
        out.text.contains("XP2") && out.text.contains("\u{12}BBBNine XP3"),
        "{} / {:?}",
        out.text,
        out.refusals
    );
    assert!(out.refusals.is_empty(), "{:?}", out.refusals);
}

#[test]
fn country_iterator_sizes_from_live_rows() {
    game::enter();
    let mut fixture = Fixture::new();
    let alphabet = b"ABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789";
    let countries = (1..=24)
        .map(|id| CountryRow {
            id,
            canonical_slot: id,
            tag: vec![
                alphabet[id as usize / 1296],
                alphabet[id as usize / 36 % 36],
                alphabet[id as usize % 36],
            ],
            ..Default::default()
        })
        .collect::<Vec<_>>();
    *fixture.frame.tables[TableId::CountryLinks.index()] =
        TableView::Complete(TableRows::CountryLinks(
            countries
                .iter()
                .map(|row| CountryLinkRow {
                    id: row.id,
                    canonical_slot: row.canonical_slot,
                })
                .collect(),
        ));
    *fixture.frame.tables[TableId::CountryCounterInputs.index()] =
        TableView::Complete(TableRows::CountryCounterInputs(
            countries
                .iter()
                .map(|row| CountryCounterInputRow {
                    country: row.id,
                    owned_state_count: 1,
                    ..Default::default()
                })
                .collect(),
        ));
    *fixture.frame.tables[TableId::Countries.index()] =
        TableView::Complete(TableRows::Countries(countries));
    let mut graph = fresh();
    let out = fixture.render(
        "fixture_list_country={tooltip=TIP fixture_points=2}",
        &mut graph,
        Content::default(),
    );
    assert_eq!(out.text, "tip:\n   XP2\n");
    assert!(out.refusals.is_empty(), "{:?}", out.refusals);
}

#[test]
fn operand_gaps_and_zero_reach_are_named_once_by_native_class() {
    game::enter();
    let mut fixture = Fixture::new();
    if let TableView::Complete(TableRows::CountryCounterInputs(rows)) =
        &mut *fixture.frame.tables[TableId::CountryCounterInputs.index()]
    {
        rows[0].owned_states = Recorded::NotRecorded;
        rows[0].army_leaders = Recorded::NotRecorded;
    }
    *fixture.frame.tables[TableId::Scientists.index()] =
        TableView::Complete(TableRows::Scientists(vec![ScientistRow {
            country: 100,
            character: 9,
            active: Recorded::NotRecorded,
            ..Default::default()
        }]));
    *fixture.frame.tables[TableId::UnitLeaders.index()] =
        TableView::Complete(TableRows::UnitLeaders(
            [7, 8]
                .map(|id| UnitLeaderRow {
                    id,
                    owner: 100,
                    category: 0,
                    ..Default::default()
                })
                .to_vec(),
        ));
    for (script, class, why) in [
        (
            "fixture_list_owned_state={fixture_points=2}",
            "fixture_list_owned_state",
            "owned states of country counter inputs",
        ),
        (
            "fixture_select_army_leader={fixture_points=2}",
            "fixture_select_army_leader",
            "army leaders of country counter inputs not recorded",
        ),
        (
            "fixture_list_active_scientist={fixture_points=2}",
            "fixture_list_active_scientist",
            "active of scientists",
        ),
    ] {
        let out = fixture.render(script, &mut fresh(), Content::default());
        assert!(out.text.is_empty());
        assert_eq!(out.refusals.values().sum::<usize>(), 1);
        assert!(
            out.refusals
                .keys()
                .any(|reason| reason.contains(class) && reason.contains(why)),
            "{:?}",
            out.refusals
        );
    }
    let out = fixture.render(
        "fixture_government_detail={elections_allowed=yes election_frequency=48 last_election=1936.1.1}",
        &mut fresh(),
        Content::default(),
    );
    assert_eq!(out.text, "Politics\nElections\nEvery 4\n");
    assert!(out.refusals.is_empty(), "{:?}", out.refusals);
    *fixture.frame.tables[TableId::PoliticalParties.index()] =
        TableView::Complete(TableRows::PoliticalParties(vec![PoliticalPartyRow {
            country: 100,
            id: 2,
            token: b"ideology".to_vec(),
            name: b"TIP".to_vec(),
            long_name: b"VARIABLE_TIP".to_vec(),
            ..Default::default()
        }]));
    *fixture.frame.tables[TableId::PartyLeaders.index()] =
        TableView::Complete(TableRows::PartyLeaders(vec![PartyLeaderRow {
            country: 100,
            party: 2,
            character: Some(9),
            role: Some(1),
            ..Default::default()
        }]));
    let mut names = content_reader_script::key_names::KeyNames::default();
    names.counts.ideologies.insert(b"ideology".to_vec());
    if let TableView::Complete(TableRows::Politics(rows)) =
        &mut *fixture.frame.tables[TableId::Politics.index()]
    {
        rows.iter_mut()
            .find(|row| row.country == 100)
            .unwrap()
            .ruling_party = Some(2);
    }
    for (body, country, expected) in [
        (
            "ruling_party=ideology long_name=TIP",
            1,
            "Politics\nideology=tip\nParty tip\n",
        ),
        (
            "ruling_party=var:2 long_name=TIP",
            1,
            "Politics\nideology=tip\nParty tip\n",
        ),
        (
            "ruling_party=ROOT long_name=TIP",
            1,
            "Politics\nideology=tip\nParty tip\n",
        ),
        ("ruling_party=ideology", 1, "Politics\nParty tip\n"),
        (
            "ruling_party=ideology long_name=TIP",
            2,
            "Politics\nideology=tip\nParty tip\n",
        ),
    ] {
        let mut scopes =
            crate::scripted_text::EventScopes::new(vec![crate::scripted_text::EventScope::of(
                crate::scripted_text::EventObject::Country(country),
            )]);
        let out = fixture.render(
            &format!("fixture_government_detail={{{body}}}"),
            &mut scopes,
            Content {
                key_names: Some(&names),
                ..Default::default()
            },
        );
        assert_eq!(out.text, expected, "{body}: {:?}", out.refusals);
        assert!(out.refusals.is_empty(), "{body}: {:?}", out.refusals);
    }
}

#[test]
fn random_selectors_and_character_list_use_native_multiple_virtual() {
    game::enter();
    let fixture = Fixture::new();
    fixture.with_renderer(&mut fresh(), Content::default(), |renderer| {
        for &(_, name) in content_reader_script::effect_keys::REGISTERED_EFFECTS.iter() {
            let item = content_reader_parser::parser::parse(format!("{name}={{}}").as_bytes())
                .unwrap()
                .items
                .remove(0);
            let class = renderer.effect_class(&item).unwrap();
            if iterator_kind(class.vtable).is_some() || class.vtable == effect_identity::CHARACTER_ROSTER {
                assert!(renderer.iterator_multiple(&item), "{name}");
            }
        }
    });
    let out = fixture.render(
        "BBB={fixture_pick_state={limit={state=1} fixture_points=2}}",
        &mut fresh(),
        Content::default(),
    );
    assert_eq!(out.text, "\u{12}BBBBeta:\n (\u{11}YOne\u{11}!): XP2\n");
    assert!(out.refusals.is_empty(), "{:?}", out.refusals);
}

#[test]
fn individual_targets_share_one_scope_seed_and_probe_refusals_count_once() {
    game::enter();
    let fixture = Fixture::new();
    let mut graph = fresh();
    let out = fixture.render("fixture_all_states={limit={OR={state=1 state=2}} display_individual_scopes=yes fixture_pick_state={fixture_points=2}}", &mut graph, Content::default());
    assert!(out.refusals.is_empty(), "{:?}", out.refusals);
    assert_eq!(out.text.matches("XP2").count(), 2);
    assert_eq!(graph.scopes[0].random_seed, 1);
    assert!(graph.scopes.iter().any(|scope| scope.this
        == crate::scripted_text::EventObject::State(2)
        && scope.random_seed == 3));
    let out = fixture.render(
        "fixture_pick_state={fixture_points=2 add_political_power=3 unported_effect=1}",
        &mut fresh(),
        Content::default(),
    );
    assert_eq!(
        out.refusals.values().sum::<usize>(),
        1,
        "{:?}",
        out.refusals
    );
    assert!(out.text.contains("XP2") && out.text.contains("PP3"));
}

#[test]
fn format98_state_iterators_list_and_select_in_recorded_order() {
    game::enter();
    let mut fixture = Fixture::new();
    let TableView::Complete(TableRows::CountryCounterInputs(rows)) =
        &mut *fixture.frame.tables[TableId::CountryCounterInputs.index()]
    else {
        panic!("complete counter inputs");
    };
    rows[0].owned_states = Recorded::Value(Some(vec![3, 1, 2]));
    rows[0].controlled_states = Recorded::Value(Some(vec![2, 3, 1]));
    rows[0].core_states = Recorded::Value(Some(vec![1, 3, 2]));
    for (kind, names, picked) in [
        ("owned", ["Three", "One", "Two"], "Two"),
        ("controlled", ["Two", "Three", "One"], "One"),
        ("core", ["One", "Three", "Two"], "Two"),
    ] {
        let out = fixture.render(
            &format!("fixture_list_{kind}_state={{fixture_points=2}}"),
            &mut fresh(),
            Content::default(),
        );
        assert_eq!(
            out.text,
            format!(
                "(\u{11}Y{}\u{11}!, \u{11}Y{}\u{11}!, \u{11}Y{}\u{11}!):\n   XP2\n",
                names[0], names[1], names[2]
            )
        );
        assert!(out.refusals.is_empty(), "{:?}", out.refusals);
        let mut graph = fresh();
        let out = fixture.render(
            &format!("fixture_pick_{kind}_state={{fixture_points=2}}"),
            &mut graph,
            Content::default(),
        );
        assert_eq!(out.text, format!("(\u{11}Y{picked}\u{11}!): XP2\n"));
        assert!(out.refusals.is_empty(), "{:?}", out.refusals);
        assert_eq!(graph.scopes[0].random_seed, 2);
    }
    let out = fixture.render(
        "fixture_pick_owned_state={prioritize={1 3} fixture_points=2}",
        &mut fresh(),
        Content::default(),
    );
    assert_eq!(out.text, "(\u{11}YThree\u{11}!): XP2\n");
    assert!(out.refusals.is_empty(), "{:?}", out.refusals);
    let TableView::Complete(TableRows::States(rows)) =
        &mut *fixture.frame.tables[TableId::States.index()]
    else {
        panic!("complete states");
    };
    rows[2].controller = 200;
    let out = fixture.render(
        "fixture_select_owned_controlled_state={fixture_points=2}",
        &mut fresh(),
        Content::default(),
    );
    assert_eq!(out.text, "(\u{11}YOne\u{11}!): XP2\n");
    assert!(out.refusals.is_empty(), "{:?}", out.refusals);
}

#[test]
fn format98_leaders_use_army_then_navy_order_and_native_global_repetition() {
    game::enter();
    let mut fixture = Fixture::new();
    *fixture.frame.tables[TableId::UnitLeaders.index()] =
        TableView::Complete(TableRows::UnitLeaders(
            (1..=5)
                .map(|id| UnitLeaderRow {
                    id,
                    owner: 100,
                    category: if id < 3 {
                        0
                    } else if id < 5 {
                        2
                    } else {
                        3
                    },
                    inherited_text_a: format!("Leader{id}").into_bytes(),
                    ..Default::default()
                })
                .collect(),
        ));
    let TableView::Complete(TableRows::CountryCounterInputs(rows)) =
        &mut *fixture.frame.tables[TableId::CountryCounterInputs.index()]
    else {
        panic!("complete counter inputs");
    };
    rows[0].army_leaders = Recorded::Value(Some(vec![2 << 32, 1 << 32]));
    rows[0].navy_leaders = Recorded::Value(Some(vec![4 << 32, 3 << 32]));
    for (key, ids) in [
        ("fixture_list_army_leader", vec![2, 1]),
        ("fixture_list_navy_leader", vec![4, 3]),
        ("fixture_list_unit_leader", vec![2, 1, 4, 3]),
        ("fixture_list_global_army", vec![2, 1, 2, 1]),
    ] {
        let out = fixture.render(
            &format!("{key}={{fixture_points=2}}"),
            &mut fresh(),
            Content::default(),
        );
        let labels = ids
            .iter()
            .map(|id| format!("\u{11}YLeader{id}\u{11}!"))
            .collect::<Vec<_>>()
            .join(", ");
        assert_eq!(out.text, format!("({labels}):\n   XP2\n"), "{key}");
        assert!(out.refusals.is_empty(), "{key}: {:?}", out.refusals);
    }
    for (key, picked) in [
        ("fixture_select_army_leader", 2),
        ("fixture_select_navy_leader", 4),
        ("fixture_select_unit_leader", 4),
    ] {
        let out = fixture.render(
            &format!("{key}={{fixture_points=2}}"),
            &mut fresh(),
            Content::default(),
        );
        assert_eq!(
            out.text,
            format!("(\u{11}YLeader{picked}\u{11}!): XP2\n"),
            "{key}"
        );
        assert!(out.refusals.is_empty(), "{key}: {:?}", out.refusals);
    }
}

#[test]
fn format98_orders_require_recorded_fields_even_for_single_passing_targets() {
    game::enter();
    let mut fixture = Fixture::new();
    for unavailable in [Recorded::NotRecorded, Recorded::Value(None)] {
        let TableView::Complete(TableRows::CountryCounterInputs(rows)) =
            &mut *fixture.frame.tables[TableId::CountryCounterInputs.index()]
        else {
            panic!("complete counter inputs");
        };
        rows[0].owned_states = unavailable.clone();
        rows[0].controlled_states = unavailable.clone();
        rows[0].core_states = unavailable.clone();
        rows[0].army_leaders = match &unavailable {
            Recorded::NotRecorded => Recorded::NotRecorded,
            _ => Recorded::Value(None),
        };
        rows[0].navy_leaders = rows[0].army_leaders.clone();
        for (key, name) in [
            ("fixture_list_owned_state", "owned states of country counter inputs"),
            ("fixture_list_controlled_state", "controlled states of country counter inputs"),
            ("fixture_list_core_state", "core states of country counter inputs"),
            ("fixture_list_army_leader", "army leaders of country counter inputs"),
            ("fixture_list_navy_leader", "navy leaders of country counter inputs"),
        ] {
            let out = fixture.render(
                &format!("{key}={{limit={{state=1}} fixture_points=2}}"),
                &mut fresh(),
                Content::default(),
            );
            assert_eq!(
                out.refusals.values().sum::<usize>(),
                1,
                "{key}: {:?}",
                out.refusals
            );
            assert!(out.text.is_empty());
            assert!(out.refusals.keys().any(|why| why.contains(name)));
        }
    }
}

#[test]
fn format98_operative_memberships_supply_native_order_and_full_handles() {
    game::enter();
    let mut fixture = Fixture::new();
    *fixture.frame.tables[TableId::UnitLeaders.index()] =
        TableView::Complete(TableRows::UnitLeaders(
            [(5, 101, "Five"), (5, 102, "WrongHandle"), (6, 101, "Six")]
                .map(|(id, reference_type, name)| UnitLeaderRow {
                    id,
                    reference_type,
                    owner: 100,
                    category: 3,
                    inherited_text_a: name.as_bytes().to_vec(),
                    ..Default::default()
                })
                .to_vec(),
        ));
    *fixture.frame.tables[TableId::OperativeMemberships.index()] =
        TableView::Complete(TableRows::OperativeMemberships(
            [
                (5, 100, 0, 1),
                (5, 100, 1, 0),
                (5, 200, 0, 0),
                (6, 100, 0, 0),
            ]
            .map(
                |(id, roster_country, roster_kind, ordinal)| OperativeMembershipRow {
                    id: id << 32 | 101,
                    roster_country,
                    roster_kind,
                    ordinal: Recorded::Value(ordinal),
                    ..Default::default()
                },
            )
            .to_vec(),
        ));
    let out = fixture.render(
        "fixture_list_operative={fixture_points=2}",
        &mut fresh(),
        Content::default(),
    );
    assert_eq!(
        out.text,
        "(\u{11}YSix\u{11}!, \u{11}YFive\u{11}!):\n   XP2\n"
    );
    assert!(out.refusals.is_empty(), "{:?}", out.refusals);
    let out = fixture.render(
        "fixture_select_operative={fixture_points=2}",
        &mut fresh(),
        Content::default(),
    );
    assert_eq!(out.text, "(\u{11}YSix\u{11}!): XP2\n");
    assert!(out.refusals.is_empty(), "{:?}", out.refusals);
    let TableView::Complete(TableRows::UnitLeaders(rows)) =
        &mut *fixture.frame.tables[TableId::UnitLeaders.index()]
    else {
        unreachable!()
    };
    rows.remove(0);
    let out = fixture.render(
        "fixture_list_operative={fixture_points=2}",
        &mut fresh(),
        Content::default(),
    );
    assert!(out.text.is_empty());
    assert!(
        out.refusals
            .keys()
            .any(|why| why.contains("iterator operative") && why.contains("not recorded"))
    );
}

fn unrecorded_operative_fixture(name: &[u8]) -> Fixture {
    let mut fixture = Fixture::new();
    *fixture.frame.tables[TableId::UnitLeaders.index()] =
        TableView::Complete(TableRows::UnitLeaders(
            [5, 6]
                .map(|id| UnitLeaderRow {
                    id,
                    owner: 100,
                    category: 3,
                    inherited_text_a: name.to_vec(),
                    ..Default::default()
                })
                .to_vec(),
        ));
    *fixture.frame.tables[TableId::OperativeMemberships.index()] =
        TableView::Complete(TableRows::OperativeMemberships(
            [5, 6]
                .map(|id| OperativeMembershipRow {
                    id: id << 32,
                    roster_country: 100,
                    roster_kind: 0,
                    ordinal: Recorded::NotRecorded,
                    ..Default::default()
                })
                .to_vec(),
        ));
    fixture
}

#[test]
fn unrecorded_operative_order_allows_pure_empty_and_singleton_selections() {
    game::enter();
    let mut fixture = unrecorded_operative_fixture(b"Leader");
    let out = fixture.render(
        "fixture_select_operative={limit={always=no} fixture_points=2}",
        &mut fresh(),
        Content::default(),
    );
    assert_eq!(out, Description::default());
    let out = fixture.render(
        "fixture_select_operative={fixture_points=2}",
        &mut fresh(),
        Content::default(),
    );
    assert_eq!(out.refusals.values().sum::<usize>(), 1);
    assert!(out.refusals.keys().any(|why| why.contains("ordinal of operative memberships") && why.contains("not recorded")));
    let TableView::Complete(TableRows::OperativeMemberships(rows)) =
        &mut *fixture.frame.tables[TableId::OperativeMemberships.index()]
    else {
        panic!("complete operative memberships");
    };
    rows.pop();
    let out = fixture.render(
        "fixture_select_operative={fixture_points=2}",
        &mut fresh(),
        Content::default(),
    );
    assert_eq!(out.text, "(\u{11}YLeader\u{11}!): XP2\n");
    assert!(out.refusals.is_empty(), "{:?}", out.refusals);
}

#[test]
fn unrecorded_operative_order_refuses_reached_limit_writes_and_rng_even_for_no_matches() {
    game::enter();
    let mut fixture = unrecorded_operative_fixture(b"");
    *fixture.frame.tables[TableId::ScriptVariables.index()] =
        TableView::Complete(TableRows::ScriptVariables(vec![ScriptVariablesRow {
            scope_kind: 1,
            scope_id: 100,
            seed: Recorded::Value(Some(37)),
            ..Default::default()
        }]));
    for limit in [
        "set_temp_variable={stable=1} always=no",
        "set_temp_variable={stable=2} set_temp_variable={stable=1} always=no",
        "check_variable={random > 2}",
    ] {
        fixture.with_renderer(&mut fresh(), Content::default(), |renderer| {
            let scopes = renderer.evaluator().unwrap().event_scopes(0).unwrap();
            renderer
                .evaluator()
                .unwrap()
                .store_description_temp(b"stable".to_vec(), 100_000);
            assert_eq!(renderer.scoped_value(b"random_seed", scopes), Ok(967));
            let item = content_reader_parser::parser::parse(
                format!("fixture_select_operative={{limit={{{limit}}}}}").as_bytes(),
            )
            .unwrap()
            .items
            .remove(0);
            let result = renderer.iterator(
                &item,
                scopes,
                0,
                IteratorKind {
                    roster: Roster::Leaders {
                        army: false,
                        navy: false,
                        operative: true,
                        global: false,
                    },
                    random: true,
                },
            );
            assert!(
                result.unwrap_err().contains("ordinal of operative memberships"),
                "{limit}"
            );
            assert_eq!(renderer.scoped_value(b"stable", scopes), Ok(100_000));
            assert_eq!(renderer.scoped_value(b"random_seed", scopes), Ok(967));
            assert_eq!(renderer.graph.borrow().scopes[0].random_seed, 1);
        });
    }
    fixture.with_renderer(&mut fresh(), Content::default(), |renderer| {
        let scopes = renderer.evaluator().unwrap().event_scopes(0).unwrap();
        let evaluator = renderer.evaluator().unwrap();
        evaluator.store_description_temp(b"stable".to_vec(), 100_000);
        let effects = evaluator.variable_effects();
        let item = content_reader_parser::parser::parse(
            b"fixture_select_operative={limit={divide_temp_variable={stable=0} always=no}}",
        )
        .unwrap()
        .items
        .remove(0);
        assert_eq!(
            renderer.iterator(
                &item,
                scopes,
                0,
                IteratorKind {
                    roster: Roster::Leaders {
                        army: false,
                        navy: false,
                        operative: true,
                        global: false
                    },
                    random: true,
                }
            ),
            Ok(String::new())
        );
        assert_eq!(renderer.evaluator().unwrap().variable_effects(), effects);
        assert_eq!(renderer.scoped_value(b"stable", scopes), Ok(100_000));
    });
}

#[test]
fn party_leader_without_mandatory_ideology_is_empty_even_with_two_leaders() {
    game::enter();
    let mut fixture = Fixture::new();
    *fixture.frame.tables[TableId::PoliticalParties.index()] =
        TableView::Complete(TableRows::PoliticalParties(
            [1, 2]
                .map(|id| PoliticalPartyRow {
                    country: 100,
                    id,
                    ..Default::default()
                })
                .to_vec(),
        ));
    *fixture.frame.tables[TableId::PartyLeaders.index()] =
        TableView::Complete(TableRows::PartyLeaders(
            [1, 2]
                .map(|party| PartyLeaderRow {
                    country: 100,
                    party,
                    character: Some(party),
                    ..Default::default()
                })
                .to_vec(),
        ));
    let mut graph = fresh();
    let out = fixture.render(
        "fixture_party_leader={fixture_points=2}",
        &mut graph,
        Content::default(),
    );
    assert!(out.text.is_empty() && out.refusals.is_empty());
    assert_eq!(graph.scopes[0].random_seed, 1);
}

#[test]
fn fresh_country_state_environment_does_not_inherit_caller_links() {
    game::enter();
    let fixture = Fixture::new();
    let mut graph = fresh();
    graph.scopes[0].root = crate::scripted_text::Link::Unknown;
    graph.scopes[0].from = crate::scripted_text::Link::Unknown;
    graph.scopes[0].prev = crate::scripted_text::Link::Unknown;
    fixture.with_renderer(&mut graph, Content::default(), |renderer| {
        let scope = renderer.fresh_country_state_scope(2, 3).unwrap();
        assert_eq!(
            renderer
                .expanded(
                    "[State.GetName]/[Country.GetTag]/[ROOT.GetTag]/[FROM.GetTag]/[PREV.GetTag]",
                    scope
                )
                .unwrap(),
            "Three/BBB/BBB/BBB/BBB"
        );
        let fresh = renderer.graph.borrow().scopes[scope.cursor.unwrap()];
        assert_eq!(
            (fresh.root, fresh.from, fresh.prev),
            (
                crate::scripted_text::Link::Itself,
                crate::scripted_text::Link::Itself,
                crate::scripted_text::Link::Itself
            )
        );
    });
}

#[test]
fn collection_builder_renders_header_in_caller_and_body_in_representative() {
    game::enter();
    let mut fixture = Fixture::new();
    *fixture.frame.tables[TableId::FactionMembers.index()] =
        TableView::Complete(TableRows::FactionMembers(vec![
            FactionMemberRow {
                faction: 8,
                country: 200,
                ordinal: 0,
            },
            FactionMemberRow {
                faction: 8,
                country: 100,
                ordinal: 1,
            },
        ]));
    let out = fixture.render("fixture_collection_members={input={input=game:scope operators={faction_members limit={tag=BBB}}} if={limit={tag=BBB} fixture_points=2}}", &mut fresh(), Content::default());
    assert_eq!(out.text, "Faction roster:\n   XP2\n");
    assert!(out.refusals.is_empty(), "{:?}", out.refusals);
    let out = fixture.render("fixture_collection_members={input={input=game:scope name=REFERENCE operators={faction_members}} fixture_points=2}", &mut fresh(), Content::default());
    assert_eq!(out.text, "AAA:\n   XP2\n");
    assert!(out.refusals.is_empty());
    let out = fixture.render("fixture_collection_members={input={input=game:scope operators={faction_members limit={always=no}}} fixture_points=2}", &mut fresh(), Content::default());
    assert!(out.text.is_empty() && out.refusals.is_empty());
}

#[test]
fn ability_context_header_uses_country_while_character_getters_keep_leader() {
    game::enter();
    use crate::scripted_text::{EventObject, EventScope, EventScopes};
    let mut fixture = Fixture::new();
    let leader = 9_u64 << 32 | 3;
    *fixture.frame.tables[TableId::UnitLeaders.index()] =
        TableView::Complete(TableRows::UnitLeaders(vec![UnitLeaderRow {
            id: 9,
            reference_type: 3,
            owner: 300,
            character_id: 44,
            character_type: engine_native::ccharacter::REFERENCE_TYPE,
            ..Default::default()
        }]));
    *fixture.frame.tables[TableId::Characters.index()] =
        TableView::Complete(TableRows::Characters(vec![CharacterRow {
            id: 44,
            country: 300,
            name: b"Ability Leader".to_vec(),
            ..Default::default()
        }]));
    let object = EventObject::CountryUnitLeader { country: 2, leader };
    let mut graph = EventScopes::new(vec![EventScope::of(object)]);
    fixture.with_renderer(&mut graph, Content::default(), |renderer| {
        let scopes = renderer.evaluator().unwrap().event_scopes(0).unwrap();
        assert_eq!(renderer.current_country(scopes), Ok(2));
        assert_eq!(
            renderer.effect_character(scopes.this).unwrap().unwrap().id,
            44
        );
    });
    let out = fixture.render("ROOT={fixture_points=2}", &mut graph, Content::default());
    assert!(out.refusals.is_empty(), "{:?}", out.refusals);
    assert!(
        out.text.contains("Beta") && out.text.contains("XP2"),
        "{}",
        out.text
    );
    assert!(!out.text.contains("Ability Leader"), "{}", out.text);
    assert_eq!(graph.scopes[0].this, object);
}

#[test]
fn state_event_uses_current_character_role_or_refuses_ambiguous_association() {
    game::enter();
    use crate::scripted_text::{EventObject, EventScope, EventScopes};
    let mut fixture = Fixture::new();
    let leader = 9_u64 << 32 | 3;
    *fixture.frame.tables[TableId::UnitLeaders.index()] =
        TableView::Complete(TableRows::UnitLeaders(vec![UnitLeaderRow {
            id: 9,
            reference_type: 3,
            owner: 300,
            character_id: 44,
            character_type: engine_native::ccharacter::REFERENCE_TYPE,
            ..Default::default()
        }]));
    *fixture.frame.tables[TableId::Characters.index()] =
        TableView::Complete(TableRows::Characters(vec![CharacterRow {
            id: 44,
            country: 200,
            ..Default::default()
        }]));
    let lookup = |_: &[u8]| {
        Ok(std::sync::Arc::new(content_reader_events::events::Event {
            titles: content_reader_parser::parser::parse(b"title=TIP").unwrap().items,
            ..Default::default()
        }))
    };
    let content = Content {
        events: Some(&lookup),
        ..Default::default()
    };
    let object = EventObject::CountryUnitLeader { country: 2, leader };
    let mut graph = EventScopes::new(vec![EventScope::of(object)]);
    let out = fixture.render("fixture_notify_state={id=synthetic.1}", &mut graph, content);
    assert!(out.refusals.is_empty(), "{:?}", out.refusals);
    assert!(
        graph
            .scopes
            .iter()
            .skip(1)
            .any(|scope| scope.this == EventObject::CountryUnitLeader { country: 3, leader })
    );
    let TableView::Complete(TableRows::UnitLeaders(leaders)) =
        &mut *fixture.frame.tables[TableId::UnitLeaders.index()]
    else {
        panic!("leaders");
    };
    leaders.push(UnitLeaderRow {
        id: 10,
        reference_type: 3,
        owner: 200,
        character_id: 44,
        character_type: engine_native::ccharacter::REFERENCE_TYPE,
        ..Default::default()
    });
    let mut graph = EventScopes::new(vec![EventScope::of(object)]);
    let out = fixture.render("fixture_notify_state={id=synthetic.1}", &mut graph, content);
    assert!(
        out.refusals
            .keys()
            .any(|why| why.contains("more than one unit leaders row")),
        "{:?}",
        out.refusals
    );
    let mut graph = EventScopes::new(vec![EventScope::of(EventObject::Character(44))]);
    let out = fixture.render("fixture_notify_state={id=synthetic.1}", &mut graph, content);
    assert!(
        out.refusals
            .keys()
            .any(|why| why.contains("current character role")),
        "{:?}",
        out.refusals
    );
}

#[test]
fn original_tag_effect_rosters_keep_native_order_and_skip_nonpositive_raw_before_lookup() {
    game::enter();
    let mut fixture = Fixture::new();
    if let TableView::Complete(TableRows::CountryAppearance(rows)) =
        &mut *fixture.frame.tables[TableId::CountryAppearance.index()]
    {
        rows[0].original_tag_reference = Recorded::Value(Some(0));
        rows[0].original_tag = Recorded::NotRecorded;
    }
    *fixture.frame.tables[TableId::ReservedCountryNaming.index()] =
        TableView::Complete(TableRows::ReservedCountryNaming(vec![
            ReservedCountryNamingRow {
                tag: b"ZZ0".to_vec(),
                original_tag_reference: Recorded::Value(Some(-7)),
                original_tag_buckets: Recorded::Value(Some(vec![
                    vec![],
                    vec![0, u32::MAX, 300, 200, 300],
                    vec![],
                    vec![],
                ])),
                ..Default::default()
            },
        ]));
    let item =
        content_reader_parser::parser::parse(b"fixture_list_country_with_original_tag={original_tag=THIS}")
            .unwrap()
            .items
            .remove(0);
    let roster = |fixture: &Fixture, slot| {
        fixture.with_renderer(&mut fresh(), Content::default(), |renderer| {
            renderer.roster(
                Roster::OriginalTags,
                &item,
                Scopes::of(Scope::Country(slot)),
            )
        })
    };
    assert_eq!(
        roster(&fixture, 1),
        Ok(vec![
            Scope::Country(3),
            Scope::Country(2),
            Scope::Country(3),
            Scope::Country(1)
        ])
    );
    if let TableView::Complete(TableRows::ReservedCountryNaming(rows)) =
        &mut *fixture.frame.tables[TableId::ReservedCountryNaming.index()]
    {
        rows[0].original_tag_buckets =
            Recorded::Value(Some(vec![vec![], vec![999], vec![], vec![]]));
    }
    assert!(
        roster(&fixture, 1)
            .unwrap_err()
            .contains("country raw reference 999")
    );
    if let TableView::Complete(TableRows::ReservedCountryNaming(rows)) =
        &mut *fixture.frame.tables[TableId::ReservedCountryNaming.index()]
    {
        rows[0].original_tag_buckets = Recorded::NotRecorded;
    }
    assert!(
        roster(&fixture, 1)
            .unwrap_err()
            .contains("original tag buckets of reserved country naming not recorded")
    );
    assert_eq!(roster(&fixture, 0), Ok(Vec::new()));
}
