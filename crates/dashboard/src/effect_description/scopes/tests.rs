use super::*;
use crate::effect_description::tests::{Fixture, fresh};
use replay::{frame::TableView, rows::*};

#[test]
fn industrial_organisation_iterator_uses_recorded_order_native_scopes_and_visibility() {
    let mut fixture = Fixture::new();
    *fixture.frame.tables[TableId::IndustrialOrganisations.index()] =
        TableView::Complete(TableRows::IndustrialOrganisations(vec![
            IndustrialOrganisationRow {
                organisation: 79 | 2 << 32,
                country: 100,
                owner: 100,
                ordinal: 1,
                token: 11,
                name: b"TINY".to_vec(),
                ..Default::default()
            },
            IndustrialOrganisationRow {
                organisation: 79 | 1 << 32,
                country: 100,
                owner: 100,
                ordinal: 0,
                token: 10,
                name: b"REFERENCE".to_vec(),
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
    let mut database = installed_content::industrial_organisations::Database::default();
    database
        .read_file(
            b"base={visible={FROM={tag=AAA}}} one={include=base} two={visible={always=no}}",
            &installed_content::key_names::KeyNames::default(),
        )
        .unwrap();
    let content = Content {
        industrial_organisations: Some(&database),
        ..Default::default()
    };
    let out = fixture.render(
        "every_military_industrial_organization={limit={OWNER={tag=AAA}} army_experience=2}",
        &mut fresh(),
        content,
    );
    assert_eq!(out.text, "\u{11}YAAA\u{11}!: \n   XP2\n");
    assert!(out.refusals.is_empty(), "{:?}", out.refusals);
    let out = fixture.render(
        "every_military_industrial_organization={include_invisible=yes army_experience=2}",
        &mut fresh(),
        Content::default(),
    );
    assert_eq!(out.text, "(\u{11}YAAA\u{11}!, \u{11}Yx\u{11}!):\n   XP2\n");
    assert!(out.refusals.is_empty(), "{:?}", out.refusals);
    let mut graph = fresh();
    let out = fixture.render(
        "random_military_industrial_organization={army_experience=2}",
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
    let out = fixture.render("THIS={army_experience=2}", &mut graph, content);
    assert_eq!(out.text, "AAA \u{12}AAA XP2\n");
    assert!(out.refusals.is_empty(), "{:?}", out.refusals);
    database = installed_content::industrial_organisations::Database::default();
    database
        .read_file(
            b"base={visible={always=no}} one={visible={always=no}} two={delete_included_values={visible} include=base}",
            &installed_content::key_names::KeyNames::default(),
        )
        .unwrap();
    let out = fixture.render(
        "every_military_industrial_organization={army_experience=2}",
        &mut fresh(),
        Content {
            industrial_organisations: Some(&database),
            ..Default::default()
        },
    );
    assert_eq!(out.text, "\u{11}Yx\u{11}!: \n   XP2\n");
    assert!(out.refusals.is_empty(), "{:?}", out.refusals);
    let out = fixture.render("every_military_industrial_organization={include_invisible=yes limit={check_variable={var=unrecorded value=1}} army_experience=2}", &mut fresh(), Content::default());
    assert!(out.text.is_empty());
    assert_eq!(out.refusals.values().sum::<usize>(), 1);
    assert!(
        out.refusals
            .keys()
            .any(|why| why.contains("CVariables +0x1C0")),
        "{:?}",
        out.refusals
    );
}

#[test]
fn shared_multiple_iterator_truncates_and_keeps_native_layout_options() {
    let fixture = Fixture::new();
    for (limit, expected) in [("NOT={state=5}", "Four"), ("always=yes", "2 others")] {
        let out = fixture.render(
            &format!("every_state={{limit={{{limit}}} army_experience=2}}"),
            &mut fresh(),
            Content::default(),
        );
        assert!(out.refusals.is_empty(), "{:?}", out.refusals);
        assert!(out.text.contains(expected), "{}", out.text);
        assert_eq!(out.text.matches("XP2").count(), 1);
    }
    let out = fixture.render("every_state={limit={state=1} display_individual_scopes=yes army_experience=2} every_state={limit={always=no} visible_when_empty=yes tooltip=TIP army_experience=3}", &mut fresh(), Content::default());
    assert_eq!(out.text, "(\u{11}YOne\u{11}!):\n   XP2\ntip:\n   XP3\n");
    assert!(out.refusals.is_empty(), "{:?}", out.refusals);
    // Multiple-target vf11 is true even with one leaf (1401802A0).
    let out = fixture.render(
        "BBB={every_state={limit={state=1} army_experience=2}}",
        &mut fresh(),
        Content::default(),
    );
    assert_eq!(
        out.text,
        "\u{12}BBBBeta:\n \u{11}YOne\u{11}!: \n       XP2\n"
    );
}

#[test]
fn random_iterator_advances_singletons_and_subset_uses_swap_removal() {
    let fixture = Fixture::new();
    let mut graph = fresh();
    let out = fixture.render(
        "random_state={limit={state=1} army_experience=2}",
        &mut graph,
        Content::default(),
    );
    assert_eq!(out.text, "(\u{11}YOne\u{11}!): XP2\n");
    assert_eq!(graph.scopes[0].random_seed, 2);
    assert!(out.refusals.is_empty(), "{:?}", out.refusals);
    for (preferred, limit, name) in [("5 2", "NOT={state=5}", "Two"), ("5", "state=1", "One")] {
        let out = fixture.render(
            &format!(
                "random_state={{prioritize={{{preferred}}} limit={{{limit}}} army_experience=2}}"
            ),
            &mut fresh(),
            Content::default(),
        );
        assert_eq!(out.text, format!("(\u{11}Y{name}\u{11}!): XP2\n"));
        assert!(out.refusals.is_empty(), "{:?}", out.refusals);
    }
    let mut graph = fresh();
    let out = fixture.render(
        "every_state={random_select_amount=2 army_experience=2}",
        &mut graph,
        Content::default(),
    );
    let mut ids = vec![1, 2, 3, 4, 5];
    for seed in 1..4 {
        ids.swap_remove(
            crate::scripted_text::random::scoped_draw(seed, graph.scopes[0].random_counter)
                as usize
                % ids.len(),
        );
    }
    assert_eq!(graph.scopes[0].random_seed, 4);
    let names = ["One", "Two", "Three", "Four", "Five"];
    assert_eq!(
        out.text,
        format!(
            "(\u{11}Y{}\u{11}!, \u{11}Y{}\u{11}!):\n   XP2\n",
            names[ids[0] - 1],
            names[ids[1] - 1]
        )
    );
}

#[test]
fn unrecorded_subjects_refuse_the_subject_roster() {
    let mut fixture = Fixture::new();
    let TableView::Complete(TableRows::CountryAppearance(rows)) =
        &mut *fixture.frame.tables[TableId::CountryAppearance.index()]
    else {
        panic!("the fixture records CountryAppearance");
    };
    rows[0].subjects = Recorded::NotRecorded;
    let out = fixture.render(
        "every_subject_country={army_experience=2}",
        &mut fresh(),
        Content::default(),
    );
    assert!(
        out.refusals
            .keys()
            .any(|reason| reason.contains("CountryAppearance.subjects not recorded")),
        "{:?}",
        out.refusals
    );
}

#[test]
fn recorded_country_subject_and_character_rosters_select_natively() {
    let mut fixture = Fixture::new();
    let out = fixture.render(
        "every_other_country={army_experience=2}",
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
        "every_subject_country={army_experience=2}",
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
        "every_character={army_experience=2}",
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
                "random_scientist={army_experience=2}",
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
        "every_allied_country={army_experience=2}",
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
    // A row is an active matrix relation; its date is not the native predicate.
    let out = fixture.render(
        "every_enemy_country={army_experience=2}",
        &mut fresh(),
        Content::default(),
    );
    assert_eq!(out.text, "\u{12}BBBBeta: \n   XP2\n");
    fixture.neighbors = Some([(1, vec![2, 4])].into());
    let mut graph =
        crate::scripted_text::EventScopes::new(vec![crate::scripted_text::EventScope::of(
            crate::scripted_text::EventObject::State(1),
        )]);
    let out = fixture.render("every_neighbor_state={army_experience=2} random_neighbor_state={prioritize={4} army_experience=3}", &mut graph, Content::default());
    assert_eq!(
        out.text,
        "(\u{11}YTwo\u{11}!, \u{11}YFour\u{11}!):\n   XP2\n(\u{11}YFour\u{11}!): XP3\n"
    );
    assert!(out.refusals.is_empty(), "{:?}", out.refusals);
}

#[test]
fn character_sections_sort_all_roles_and_refuse_missing_command_assignments() {
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
        "character_list_tooltip={}",
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
        "character_list_tooltip={}",
        &mut fresh(),
        Content::default(),
    );
    assert!(out.text.is_empty());
    assert!(
        out.refusals
            .keys()
            .any(|why| why.contains("CArmyLeader +0x1048"))
    );
}

#[test]
fn scope_chains_and_character_headers_keep_native_graph_selection() {
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
        "FROM.FROM={ROOT={army_experience=2}} event_target:advisor={army_experience=3}",
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
fn country_iterator_sizes_from_five_thousand_live_rows() {
    let mut fixture = Fixture::new();
    let alphabet = b"ABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789";
    let countries = (1..=5_000)
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
        "every_country={tooltip=TIP army_experience=2}",
        &mut graph,
        Content::default(),
    );
    assert_eq!(out.text, "tip:\n   XP2\n");
    assert!(out.refusals.is_empty(), "{:?}", out.refusals);
}

#[test]
fn operand_gaps_and_zero_reach_are_named_once_by_native_class() {
    let mut fixture = Fixture::new();
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
            "every_owned_state={army_experience=2}",
            "CEveryOwnedStateEffect",
            "+0x478",
        ),
        (
            "random_army_leader={army_experience=2}",
            "CRandomArmyLeaderEffect",
            "army_leader_order",
        ),
        (
            "every_active_scientist={army_experience=2}",
            "CEveryActiveScientistEffect",
            "not ported",
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
        "set_politics={elections_allowed=yes election_frequency=48 last_election=1936.1.1}",
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
    let mut graph = fresh();
    let out = fixture.render(
        "party_leader={limit={has_ideology=ideology} army_experience=2}",
        &mut graph,
        Content::default(),
    );
    assert_eq!(out.refusals.values().sum::<usize>(), 1);
    assert!(
        out.refusals
            .keys()
            .any(|why| why.contains("CCountryLeader current ideology pointer +0x140"))
    );
    assert_eq!(graph.scopes[0].random_seed, 1);
    let out = fixture.render(
        "set_politics={ruling_party=ideology long_name=TIP}",
        &mut fresh(),
        Content::default(),
    );
    assert_eq!(out.text, "Politics\nideology=tip\nParty tip\n");
    assert!(out.refusals.is_empty(), "{:?}", out.refusals);
}

#[test]
fn random_selectors_and_character_list_use_native_multiple_virtual() {
    let fixture = Fixture::new();
    fixture.with_renderer(&mut fresh(), Content::default(), |renderer| {
        for (_, name) in installed_content::effect_keys::REGISTERED_EFFECTS {
            let item = installed_content::parser::parse(format!("{name}={{}}").as_bytes())
                .unwrap()
                .items
                .remove(0);
            let class = renderer.effect_class(&item).unwrap();
            if iterator_kind(class.vtable).is_some() || class.vtable == 0x14272B088 {
                assert!(renderer.iterator_multiple(&item), "{name}");
            }
        }
    });
    let out = fixture.render(
        "BBB={random_state={limit={state=1} army_experience=2}}",
        &mut fresh(),
        Content::default(),
    );
    assert_eq!(out.text, "\u{12}BBBBeta:\n (\u{11}YOne\u{11}!): XP2\n");
    assert!(out.refusals.is_empty(), "{:?}", out.refusals);
}

#[test]
fn individual_targets_share_one_scope_seed_and_probe_refusals_count_once() {
    let fixture = Fixture::new();
    let mut graph = fresh();
    let out = fixture.render("every_state={limit={OR={state=1 state=2}} display_individual_scopes=yes random_state={army_experience=2}}", &mut graph, Content::default());
    assert!(out.refusals.is_empty(), "{:?}", out.refusals);
    assert_eq!(out.text.matches("XP2").count(), 2);
    // Both descriptions advance the same iterator copy; the caller is unchanged.
    assert_eq!(graph.scopes[0].random_seed, 1);
    assert!(graph.scopes.iter().any(|scope| scope.this
        == crate::scripted_text::EventObject::State(2)
        && scope.random_seed == 3));
    let out = fixture.render(
        "random_state={army_experience=2 add_political_power=3 unported_effect=1}",
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
fn state_and_leader_order_gaps_only_refuse_multiple_passing_targets() {
    let mut fixture = Fixture::new();
    *fixture.frame.tables[TableId::StateCores.index()] =
        TableView::Complete(TableRows::StateCores(
            (1..=5)
                .map(|state| StateCoreRow {
                    state,
                    cores: vec![100],
                })
                .collect(),
        ));
    for (key, reason) in [
        ("every_owned_state", "owned_state_order"),
        ("random_owned_state", "owned_state_order"),
        ("every_controlled_state", "controlled_state_order"),
        ("random_controlled_state", "controlled_state_order"),
        ("every_core_state", "core_state_order"),
        ("random_core_state", "core_state_order"),
    ] {
        for (limit, count) in [
            ("always=no", 0),
            ("state=1", 1),
            ("OR={state=1 state=2}", 2),
        ] {
            let out = fixture.render(
                &format!("{key}={{limit={{{limit}}} army_experience=2}}"),
                &mut fresh(),
                Content::default(),
            );
            assert_eq!(
                out.refusals.values().sum::<usize>(),
                usize::from(count == 2),
                "{key}/{limit}: {:?}",
                out.refusals
            );
            if count == 2 {
                assert!(out.refusals.keys().any(|why| why.contains(reason)));
            } else {
                assert_eq!(out.text.matches("XP2").count(), count);
            }
        }
        let mut graph =
            crate::scripted_text::EventScopes::new(vec![crate::scripted_text::EventScope::of(
                crate::scripted_text::EventObject::State(1),
            )]);
        let out = fixture.render(
            &format!("{key}={{army_experience=2}}"),
            &mut graph,
            Content::default(),
        );
        assert!(out.text.is_empty() && out.refusals.is_empty());
    }
    *fixture.frame.tables[TableId::UnitLeaders.index()] =
        TableView::Complete(TableRows::UnitLeaders(
            (1..=6)
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
                    inherited_text_a: b"Leader".to_vec(),
                    ..Default::default()
                })
                .collect(),
        ));
    *fixture.frame.tables[TableId::OperativeMemberships.index()] =
        TableView::Complete(TableRows::OperativeMemberships(
            [5, 6]
                .map(|id| OperativeMembershipRow {
                    id: id << 32,
                    roster_country: 100,
                    roster_kind: 0,
                    ..Default::default()
                })
                .to_vec(),
        ));
    for (key, reason) in [
        ("random_army_leader", "army_leader_order"),
        ("random_navy_leader", "navy_leader_order"),
        ("random_operative", "operative_order"),
    ] {
        for (limit, multiple) in [("always=no", false), ("always=yes", true)] {
            let out = fixture.render(
                &format!("{key}={{limit={{{limit}}} army_experience=2}}"),
                &mut fresh(),
                Content::default(),
            );
            assert_eq!(
                out.refusals.values().sum::<usize>(),
                usize::from(multiple),
                "{key}: {:?}",
                out.refusals
            );
            if multiple {
                assert!(out.refusals.keys().any(|why| why.contains(reason)));
            }
        }
    }
    if let TableView::Complete(TableRows::UnitLeaders(rows)) =
        &mut *fixture.frame.tables[TableId::UnitLeaders.index()]
    {
        rows.retain(|row| row.id % 2 == 1);
    }
    for key in [
        "random_army_leader",
        "random_navy_leader",
        "random_operative",
    ] {
        let mut graph = fresh();
        let out = fixture.render(
            &format!("{key}={{army_experience=2}}"),
            &mut graph,
            Content::default(),
        );
        assert_eq!(
            out.text.matches("XP2").count(),
            1,
            "{key}: {} / {:?}",
            out.text,
            out.refusals
        );
        assert!(out.refusals.is_empty());
        assert_eq!(graph.scopes[0].random_seed, 2);
    }
    if let TableView::Complete(TableRows::CountryCounterInputs(rows)) =
        &mut *fixture.frame.tables[TableId::CountryCounterInputs.index()]
    {
        rows[0].owned_state_count = 6;
    }
    let out = fixture.render(
        "every_owned_state={limit={always=no} army_experience=2}",
        &mut fresh(),
        Content::default(),
    );
    assert!(
        out.refusals
            .keys()
            .any(|why| why.contains("membership differs"))
    );
}

#[test]
fn party_leader_without_mandatory_ideology_is_empty_even_with_two_leaders() {
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
        "party_leader={army_experience=2}",
        &mut graph,
        Content::default(),
    );
    assert!(out.text.is_empty() && out.refusals.is_empty());
    assert_eq!(graph.scopes[0].random_seed, 1);
}

#[test]
fn fresh_country_state_environment_does_not_inherit_caller_links() {
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
    let out = fixture.render("every_collection_element={input={input=game:scope operators={faction_members limit={tag=BBB}}} if={limit={tag=BBB} army_experience=2}}", &mut fresh(), Content::default());
    assert_eq!(out.text, "Faction members:\n   XP2\n");
    assert!(out.refusals.is_empty(), "{:?}", out.refusals);
    let out = fixture.render("every_collection_element={input={input=game:scope name=REFERENCE operators={faction_members}} army_experience=2}", &mut fresh(), Content::default());
    assert_eq!(out.text, "AAA:\n   XP2\n"); // Header uses caller, not selected BBB.
    assert!(out.refusals.is_empty());
    let out = fixture.render("every_collection_element={input={input=game:scope operators={faction_members limit={always=no}}} army_experience=2}", &mut fresh(), Content::default());
    assert!(out.text.is_empty() && out.refusals.is_empty());
}
