use super::*;
use crate::effect_description::tests::{Fixture, synthetic_countries};
use crate::scripted_text::{EventObject, EventScope};
use EventObject::{Country as C, State as S};
use replay_data::rows::*;
use replay_frame::frame::TableView;
use crate::fixtures::game;

#[test]
fn faction_preview_keeps_known_empty_names_in_named_removal_and_join_branches() {
    game::enter();
    let mut fixture = Fixture::new();
    let (catalogue, _folder) = crate::fixtures::folder_catalogue(&[
        (
            "common/defines/test.lua",
            "NDefines={NGame={MAX_SCRIPTED_LOC_RECURSION=30}}",
        ),
        (
            "common/factions/templates/test.txt",
            "unbound={} localised_empty={name=EMPTY_NAME}",
        ),
        (
            "localisation/test_l_english.yml",
            concat!(
                "\u{feff}l_english:\n BBB:0 \"Beta\"\n CCC:0 \"Gamma\"\n EMPTY_NAME:0 \"\"\n",
                " EFFECT_REMOVE_FROM_FACTION_NAME:0 \"Leave $COUNTRY$/$FACTION$\"\n",
                " EFFECT_ADD_TO_FACTION_NAME:0 \"Join $COUNTRY$/$FACTION$\"\n",
            ),
        ),
    ]);
    fixture.catalogue = catalogue;
    fixture
        .catalogue
        .scripted_content()
        .unwrap()
        .set_synthetic_environment(Default::default());
    *fixture.frame.tables[TableId::FactionMembers.index()] =
        TableView::Complete(TableRows::FactionMembers(vec![
            FactionMemberRow {
                country: 100,
                faction: 4,
                ordinal: 0,
            },
            FactionMemberRow {
                country: 200,
                faction: 5,
                ordinal: 0,
            },
        ]));
    let TableView::Complete(TableRows::CountryAppearance(rows)) =
        &mut *fixture.frame.tables[TableId::CountryAppearance.index()]
    else {
        unreachable!()
    };
    rows[1].subjects = vec![300].into();
    *fixture.frame.tables[TableId::LexicalTokens.index()] =
        TableView::Complete(TableRows::LexicalTokens(vec![
            LexicalTokenRow {
                token: 17,
                literal: b"unbound".to_vec(),
            },
            LexicalTokenRow {
                token: 18,
                literal: b"localised_empty".to_vec(),
            },
        ]));
    for template_token in [None, Some(17), Some(18)] {
        *fixture.frame.tables[TableId::Factions.index()] =
            TableView::Complete(TableRows::Factions(
                [4, 5]
                    .into_iter()
                    .map(|id| FactionRow {
                        id,
                        template_token,
                        ..Default::default()
                    })
                    .collect(),
            ));
        let mut graph = EventScopes::new(vec![EventScope::of(C(1))]);
        let out = fixture.render("fixture_invite_country=BBB", &mut graph, Content::default());
        assert_eq!(
            out.text, "Leave Beta/\nLeave Gamma/\nJoin Beta/\nJoin Gamma/\n",
            "{template_token:?}: {:?}",
            out.refusals,
        );
        assert!(out.refusals.is_empty(), "{:?}", out.refusals);
    }
}

#[test]
fn same_faction_nonleader_preview_reads_only_subject_operands() {
    game::enter();
    let mut fixture = Fixture::new();
    *fixture.frame.tables[TableId::FactionMembers.index()] =
        TableView::Complete(TableRows::FactionMembers(vec![
            FactionMemberRow {
                country: 200,
                faction: 4,
                ordinal: 0,
            },
            FactionMemberRow {
                country: 100,
                faction: 4,
                ordinal: 1,
            },
        ]));
    let TableView::Complete(TableRows::CountryAppearance(rows)) =
        &mut *fixture.frame.tables[TableId::CountryAppearance.index()]
    else {
        unreachable!()
    };
    rows[1].subjects = Recorded::NotRecorded;
    let mut graph = EventScopes::new(vec![EventScope::of(C(1))]);
    let out = fixture.render("fixture_invite_country=BBB", &mut graph, Content::default());
    assert_eq!(out.text, "");
    assert!(out.refusals.is_empty(), "{:?}", out.refusals);

    let TableView::Complete(TableRows::CountryAppearance(rows)) =
        &mut *fixture.frame.tables[TableId::CountryAppearance.index()]
    else {
        unreachable!()
    };
    rows[1].overlord = 300;
    let out = fixture.render("fixture_invite_country=BBB", &mut graph, Content::default());
    assert_eq!(out.text, "Free Beta/gamma\n");
    assert!(out.refusals.is_empty(), "{:?}", out.refusals);
}

#[test]
fn reached_faction_subject_loops_keep_order_aliases_and_refusals() {
    game::enter();
    let mut fixture = Fixture::new();
    let TableView::Complete(TableRows::CountryAppearance(rows)) =
        &mut *fixture.frame.tables[TableId::CountryAppearance.index()]
    else {
        unreachable!()
    };
    rows[1].subjects = vec![301, 100, 300, 301].into();
    let TableView::Complete(TableRows::CountryLinks(links)) =
        &mut *fixture.frame.tables[TableId::CountryLinks.index()]
    else {
        unreachable!()
    };
    links.push(CountryLinkRow {
        id: 301,
        canonical_slot: 3,
    });
    let mut graph = EventScopes::new(vec![EventScope::of(C(1))]);
    let out = fixture.render("fixture_invite_country=BBB", &mut graph, Content::default());
    assert_eq!(
        out.text,
        "Join Beta\nJoin Gamma\nJoin Alpha\nJoin Gamma\nJoin Gamma\n"
    );
    assert!(out.refusals.is_empty(), "{:?}", out.refusals);

    *fixture.frame.tables[TableId::Factions.index()] =
        TableView::Complete(TableRows::Factions(vec![
            FactionRow {
                id: 4,
                name: b"Union".to_vec(),
                ..Default::default()
            },
            FactionRow {
                id: 5,
                name: b"Other".to_vec(),
                ..Default::default()
            },
        ]));
    *fixture.frame.tables[TableId::FactionMembers.index()] =
        TableView::Complete(TableRows::FactionMembers(vec![
            FactionMemberRow {
                country: 300,
                faction: 4,
                ordinal: 0,
            },
            FactionMemberRow {
                country: 100,
                faction: 4,
                ordinal: 1,
            },
            FactionMemberRow {
                country: 200,
                faction: 5,
                ordinal: 0,
            },
        ]));
    let TableView::Complete(TableRows::CountryAppearance(rows)) =
        &mut *fixture.frame.tables[TableId::CountryAppearance.index()]
    else {
        unreachable!()
    };
    rows[1].subjects = Recorded::NotRecorded;
    let out = fixture.render("fixture_invite_country=BBB", &mut graph, Content::default());
    assert!(out.text.is_empty());
    assert_eq!(
        out.refusals.values().sum::<usize>(),
        1,
        "{:?}",
        out.refusals
    );
    assert!(
        out.refusals
            .keys()
            .all(|why| why.contains("subjects of country appearance not recorded"))
    );
}

#[test]
fn create_wargoal_refuses_without_complete_inventory_tables() {
    game::enter();
    let mut fixture = Fixture::new();
    let (catalogue, _folder) = crate::fixtures::folder_catalogue(&[
        (
            "common/defines/test.lua",
            "NDefines={NGame={MAX_SCRIPTED_LOC_RECURSION=30}}",
        ),
        (
            "localisation/test_l_english.yml",
            concat!(
                "\u{feff}l_english:\n AAA:0 \"Alpha\"\n BBB:0 \"Beta\"\n",
                " EFFECT_CREATE_WARGOAL:0 \"Goal $COUNTRY$/$WARGOAL$/$TARGET$\"\n",
            ),
        ),
    ]);
    fixture.catalogue = catalogue;
    fixture
        .catalogue
        .scripted_content()
        .unwrap()
        .set_synthetic_environment(Default::default());
    let names = content_reader_script::key_names::KeyNames::default();
    let database =
        content_reader_wargoals::wargoals::parse(b"wargoal_types={SynGoal={}}", &names).unwrap();
    let content = Content {
        key_names: Some(&names),
        wargoals: Some(&database),
        ..Default::default()
    };
    for table in [TableId::CountryWargoals, TableId::WargoalTargets] {
        *fixture.frame.tables[table.index()] = TableView::Unavailable {
            reason: "synthetic inventory not recorded".into(),
        };
    }
    for (body, refused) in [
        (
            "fixture_war_objective={target=BBB type=SynGoal generator={1}}",
            true,
        ),
        ("fixture_war_objective={target=BBB type=SynGoal}", true),
        ("fixture_war_objective={target=BBB type=unknown}", false),
        ("fixture_war_objective={target=BBB type=\"SynGoal\"}", false),
        ("fixture_war_objective={target=BBB}", false),
    ] {
        let out = fixture.render_native(
            body,
            &mut EventScopes::new(vec![EventScope::of(C(1))]),
            content,
        );
        assert_eq!(out.text, "", "{body}");
        if refused {
            assert_eq!(out.refusals.values().sum::<usize>(), 1, "{body}");
            assert!(
                out.refusals
                    .keys()
                    .any(|key| key.contains(CREATE_WARGOAL_UNRECORDED)),
                "{body}: {:?}",
                out.refusals
            );
        } else {
            assert!(out.refusals.is_empty(), "{body}: {:?}", out.refusals);
        }
    }
}

#[test]
fn purchase_contract_uses_seller_roster_and_quantity_pool() {
    game::enter();
    let mut fixture = Fixture::new();
    let (catalogue, _folder) = crate::fixtures::folder_catalogue(&[
        (
            "common/defines/test.lua",
            "NDefines={NGame={MAX_SCRIPTED_LOC_RECURSION=30}}",
        ),
        (
            "localisation/test_l_english.yml",
            concat!(
                "\u{feff}l_english:\n AAA:0 \"Alpha\"\n BBB:0 \"Beta\"\n CCC:0 \"Gamma\"\n",
                " AAA_DEF:0 \"alpha\"\n BBB_DEF:0 \"beta\"\n CCC_DEF:0 \"gamma\"\n",
                " AAA_syn_a_short:0 \"ViewA\"\n syn_a_short:0 \"GenericA\"\n",
                " AAA_syn_b_short:0 \"ViewB\"\n AAA_syn_other_short:0 \"MakerB\"\n",
                " INTERNATIONAL_MARKET_EQUIPMENT_AMOUNT:0 \"$AMOUNT$ $EQUIPMENT$ ($TYPE$)\"\n",
                " EFFECT_MARKET_CREATE_PURCHASE_CONTRACT:0 \"$SELLER$->$BUYER$/$CIVS$/$EQUIPMENT$ [This.GetTag]/[Root.GetTag]\"\n",
            ),
        ),
    ]);
    fixture.catalogue = catalogue;
    fixture
        .catalogue
        .scripted_content()
        .unwrap()
        .set_synthetic_environment(Default::default());
    let equipment = content_reader_key_names::equipment::fixture::load(&[(
        "common/units/equipment/synthetic.txt",
        "equipments={syn_a={is_archetype=yes type=infantry} syn_one={archetype=syn_a} syn_two={parent=syn_one archetype=syn_a} syn_b={is_archetype=yes type=artillery} syn_other={archetype=syn_b}}",
    )])
    .0;
    *fixture.frame.tables[TableId::LexicalTokens.index()] =
        TableView::Complete(TableRows::LexicalTokens(
            [
                (10, "syn_a"),
                (11, "syn_one"),
                (12, "syn_two"),
                (20, "syn_b"),
                (21, "syn_other"),
            ]
            .into_iter()
            .map(|(token, literal)| LexicalTokenRow {
                token,
                literal: literal.as_bytes().to_vec(),
            })
            .collect(),
        ));
    let handles = [10_u64, 20, 30, 40].map(|id| (id << 32) | 1);
    *fixture.frame.tables[TableId::VariantDefinitions.index()] =
        TableView::Complete(TableRows::VariantDefinitions(vec![
            VariantDefinitionRow {
                id: handles[0],
                equipment_token: 11,
                creator_link: 100,
                display_name: b"Older".to_vec(),
                ..Default::default()
            },
            VariantDefinitionRow {
                id: handles[1],
                equipment_token: 11,
                creator_link: 200,
                display_name: b"New [From.GetTag]".to_vec(),
                append_variant_name: 1,
                variant_name: b"ignored".to_vec(),
                ..Default::default()
            },
            VariantDefinitionRow {
                id: handles[2],
                equipment_token: 12,
                creator_link: 200,
                display_name: b"Deep".to_vec(),
                ..Default::default()
            },
            VariantDefinitionRow {
                id: handles[3],
                equipment_token: 21,
                creator_link: 100,
                ..Default::default()
            },
        ]));
    *fixture.frame.tables[TableId::AvailableEquipmentVariants.index()] =
        TableView::Complete(TableRows::AvailableEquipmentVariants(
            handles
                .iter()
                .enumerate()
                .map(|(ordinal, &variant)| AvailableEquipmentVariantRow {
                    country: 200,
                    ordinal: ordinal as u32,
                    variant,
                    ..Default::default()
                })
                .chain([AvailableEquipmentVariantRow {
                    country: 100,
                    variant: handles[0],
                    ..Default::default()
                }])
                .collect(),
        ));
    *fixture.frame.tables[TableId::ScriptVariables.index()] =
        TableView::Complete(TableRows::ScriptVariables(vec![ScriptVariablesRow {
            scope_kind: 1,
            scope_id: 100,
            entries: vec![
                ScriptVariableEntry {
                    name: b"syn_type".to_vec(),
                    value_raw: 1_100_000,
                },
                ScriptVariableEntry {
                    name: b"syn_amount".to_vec(),
                    value_raw: 249_999,
                },
            ],
            ..Default::default()
        }]));
    for (fields, expected, refusal) in [
        ("civilian_factories=-1.25", "-2/", None),
        ("civilian_factories=2147483648.75", "-2147483648/", None),
        (
            "civilian_factories=-92233720368547.75808",
            "-702313053/",
            None,
        ),
        (
            "civilian_factories=1 civilian_factories=var:syn_amount",
            "",
            Some("repeated purchase operand civilian_factories"),
        ),
        (
            "equipment={type=syn_one amount=2.99}",
            "0/2 New CCC (ViewA)",
            None,
        ),
        ("equipment={type=syn_a amount=5}", "0/5 Deep (ViewA)", None),
        (
            "equipment={type=syn_a producer=AAA amount=5}",
            "0/5 Older (ViewA)",
            None,
        ),
        (
            "equipment={type=syn_one producer=AAA variant_name=Older amount=1}",
            "0/1 Older (ViewA)",
            None,
        ),
        (
            "equipment={type=syn_other amount=1}",
            "0/1 MakerB (ViewB)",
            None,
        ),
        (
            "equipment={type=syn_b amount=2} equipment={type=syn_a amount=3}",
            "0/2 MakerB (ViewB), 3 Deep (ViewA)",
            None,
        ),
        (
            "equipment={type=syn_one amount=0.4} equipment={type=syn_one amount=0.6}",
            "0/1 New CCC (ViewA)",
            None,
        ),
        (
            "equipment={type=syn_one amount=92233720368547}",
            "0/922337202 New CCC (ViewA)",
            None,
        ),
        (
            "equipment={type=syn_one amount=1.4} equipment={type=syn_one amount=-0.5}",
            "0/",
            None,
        ),
        ("equipment={type=unknown amount=random}", "0/", None),
        (
            "equipment={type=syn_one producer=0 amount=random}",
            "0/",
            None,
        ),
        (
            "equipment={type=syn_one variant_name=unknown amount=random}",
            "0/",
            None,
        ),
        (
            "equipment={type=var:syn_type amount=var:syn_amount}",
            "0/2 New CCC (ViewA)",
            None,
        ),
        (
            "equipment={type=syn_one amount=2} equipment={type=syn_two amount=3}",
            "0/2 New CCC (ViewA), 3 Deep (ViewA)",
            None,
        ),
        (
            "equipment={type=syn_one producer=AAA producer=0 amount=1}",
            "",
            Some("repeated purchase operand producer"),
        ),
    ] {
        let mut scopes = EventScopes::new(vec![
            EventScope {
                root: crate::scripted_text::Link::Scope(1),
                from: crate::scripted_text::Link::Scope(2),
                ..EventScope::of(C(1))
            },
            EventScope::of(C(2)),
            EventScope::of(C(3)),
        ]);
        let body = format!("fixture_purchase_order={{seller=ROOT buyer=THIS {fields}}}");
        let out = fixture.render(
            &body,
            &mut scopes,
            Content {
                equipment: Some(&equipment),
                ..Default::default()
            },
        );
        if let Some(why) = refusal {
            assert!(out.text.is_empty(), "{fields}: {}", out.text);
            assert!(
                out.refusals.keys().any(|key| key.contains(why)),
                "{fields}: {:?}",
                out.refusals,
            );
        } else {
            assert_eq!(
                out.text,
                format!("\u{12}BBB Beta->\u{12}AAA Alpha/{expected} AAA/BBB\n"),
                "{fields}: {:?}",
                out.refusals,
            );
            assert!(out.refusals.is_empty(), "{fields}: {:?}", out.refusals);
        }
    }
    *fixture.frame.tables[TableId::FactionMembers.index()] =
        TableView::Complete(TableRows::FactionMembers(vec![
            FactionMemberRow {
                country: 100,
                faction: 5,
                ordinal: 1,
            },
            FactionMemberRow {
                country: 200,
                faction: 5,
                ordinal: 0,
            },
        ]));
    let mut scopes = EventScopes::new(vec![EventScope::of(C(1))]);
    let out = fixture.render(
        "fixture_purchase_order={seller=faction_leader buyer=ROOT civilian_factories=3 equipment={type=syn_a amount=2000} equipment={type=syn_b amount=200}}",
        &mut scopes,
        Content {
            equipment: Some(&equipment),
            ..Default::default()
        },
    );
    assert_eq!(
        out.text,
        "\u{12}BBB Beta->\u{12}AAA Alpha/3/2000 Deep (ViewA), 200 MakerB (ViewB) AAA/AAA\n",
        "{:?}",
        out.refusals,
    );
    assert!(out.refusals.is_empty(), "{:?}", out.refusals);
    *fixture.frame.tables[TableId::AvailableEquipmentVariants.index()] = TableView::Unavailable {
        reason: "synthetic gap".into(),
    };
    let mut scopes = EventScopes::new(vec![EventScope::of(C(1))]);
    let out = fixture.render(
        "fixture_purchase_order={seller=BBB buyer=AAA equipment={type=syn_one amount=1}}",
        &mut scopes,
        Content {
            equipment: Some(&equipment),
            ..Default::default()
        },
    );
    assert!(
        out.refusals
            .keys()
            .any(|why| why.contains("AvailableEquipmentVariants")),
        "{:?}",
        out.refusals,
    );
}

#[test]
fn civil_war_heading_and_selected_party_follow_separate_operands() {
    game::enter();
    let mut fixture = Fixture::new();
    let (catalogue, _folder) = crate::fixtures::folder_catalogue(&[
        (
            "common/defines/test.lua",
            "NDefines={NGame={MAX_SCRIPTED_LOC_RECURSION=30}}",
        ),
        (
            "common/ideologies/test.txt",
            "ideologies={syn_old={} syn_new={}}",
        ),
        (
            "localisation/test_l_english.yml",
            concat!(
                "\u{feff}l_english:\n",
                " syn_old:0 \"Old [This.GetTag]\"\n syn_new:0 \"New [This.GetTag]\"\n",
                " SYN_SHORT:0 \"Short [From.GetTag]\"\n SYN_LONG:0 \"Long\"\n",
                " AAA_syn_new_party:0 \"Generated [Root.GetTag]\"\n",
                " EFFECT_START_CIVIL_WAR:0 \"War $IDEOLOGY$ [From.GetTag]\"\n",
                " EFFECT_SET_POLITICS:0 \"Politics [This.GetTag]\"\n",
                " EFFECT_SET_POLITICS_RULING_PARTY:0 \"Party $PARTY$\"\n",
                " EFFECT_SET_POLITICS_NO_ELECTIONS:0 \"No elections\"\n",
            ),
        ),
    ]);
    fixture.catalogue = catalogue;
    let scripted = fixture.catalogue.scripted_content().unwrap();
    scripted.set_synthetic_environment(Default::default());
    let mut names = content_reader_script::key_names::KeyNames::default();
    names
        .counts
        .ideologies
        .extend([b"syn_old".to_vec(), b"syn_new".to_vec()]);
    *fixture.frame.tables[TableId::PoliticalParties.index()] =
        TableView::Complete(TableRows::PoliticalParties(vec![
            PoliticalPartyRow {
                country: 200,
                id: 20,
                token: b"syn_old".to_vec(),
                name: b"Foreign".to_vec(),
                ..Default::default()
            },
            PoliticalPartyRow {
                country: 100,
                tag: 100,
                id: 20,
                token: b"syn_old".to_vec(),
                name: b"SYN_SHORT".to_vec(),
                long_name: b"SYN_LONG".to_vec(),
                ..Default::default()
            },
            PoliticalPartyRow {
                country: 100,
                tag: 100,
                id: 30,
                token: b"syn_new".to_vec(),
                name_default: 1,
                ..Default::default()
            },
        ]));
    if let TableView::Complete(TableRows::Politics(rows)) =
        &mut *fixture.frame.tables[TableId::Politics.index()]
    {
        rows.iter_mut()
            .find(|row| row.country == 100)
            .unwrap()
            .ruling_party = Some(20);
    }
    *fixture.frame.tables[TableId::ScriptVariables.index()] =
        TableView::Complete(TableRows::ScriptVariables(vec![ScriptVariablesRow {
            scope_kind: 1,
            scope_id: 100,
            entries: vec![ScriptVariableEntry {
                name: b"syn_group".to_vec(),
                value_raw: 3_000_000,
            }],
            ..Default::default()
        }]));
    for (body, country, expected, refusal) in [
        (
            "fixture_civil_conflict={ideology=syn_new size=0.5}",
            1,
            "War New AAA BBB\nPolitics [This.GetTag]\nParty Short [From.GetTag]\nNo elections\n",
            None,
        ),
        (
            "fixture_civil_conflict={ideology=syn_new ruling_party=0}",
            1,
            "War New AAA BBB\nPolitics [This.GetTag]\nParty Short [From.GetTag]\nNo elections\n",
            None,
        ),
        (
            "fixture_civil_conflict={ideology=syn_new ruling_party=syn_new}",
            1,
            "War New AAA BBB\nPolitics [This.GetTag]\nParty Generated [Root.GetTag]\nNo elections\n",
            None,
        ),
        (
            "fixture_civil_conflict={ideology=syn_new ruling_party=var:20}",
            1,
            "War New AAA BBB\nPolitics [This.GetTag]\nParty Short [From.GetTag]\nNo elections\n",
            None,
        ),
        (
            "fixture_civil_conflict={ideology=var:syn_group ruling_party=syn_old}",
            1,
            "War New AAA BBB\nPolitics [This.GetTag]\nParty Short [From.GetTag]\nNo elections\n",
            None,
        ),
        (
            "fixture_civil_conflict={ideology=syn_old ideology=syn_new ruling_party=syn_new}",
            1,
            "War New AAA BBB\nPolitics [This.GetTag]\nParty Generated [Root.GetTag]\nNo elections\n",
            None,
        ),
        (
            "fixture_civil_conflict={ideology=syn_new ruling_party=syn_old size=0.9 keep_unit_leaders=yes}",
            3,
            "",
            Some("party selected by ideology"),
        ),
        (
            "fixture_civil_conflict={ideology=unresolved}",
            1,
            "",
            Some("null-object identity"),
        ),
    ] {
        let mut scopes = EventScopes::new(vec![
            EventScope {
                from: crate::scripted_text::Link::Scope(1),
                ..EventScope::of(C(country))
            },
            EventScope::of(C(2)),
        ]);
        let out = fixture.render(
            body,
            &mut scopes,
            Content {
                key_names: Some(&names),
                ..Default::default()
            },
        );
        assert_eq!(out.text, expected, "{body}: {:?}", out.refusals);
        match refusal {
            None => assert!(out.refusals.is_empty(), "{body}: {:?}", out.refusals),
            Some(why) => assert!(
                out.refusals.keys().any(|key| key.contains(why)),
                "{body}: {:?}",
                out.refusals
            ),
        }
    }
}

#[test]
fn faction_influence_environment_keeps_links() {
    game::enter();
    let count = 24;
    let mut fixture = Fixture::new();
    let (catalogue, _folder) = crate::fixtures::folder_catalogue(&[
        (
            "common/defines/test.lua",
            "NDefines={NGame={MAX_SCRIPTED_LOC_RECURSION=30}}",
        ),
        (
            "common/scripted_localisation/test.txt",
            "defined_text={name=SYN_LINK text={localization_key=SYN_LINK_TEXT}}",
        ),
        (
            "localisation/test_l_english.yml",
            concat!(
                "\u{feff}l_english:\n AAA:0 \"Alpha\"\n ZZZ:0 \"Last\"\n",
                " FACTION_INFLUENCE_RATIO:0 \"$VAL|%0$\"\n FACTION_INFLUENCE_VALUE:0 \"$VAL|2$\"\n",
                " SYN_FACTION:0 \"Union\"\n",
                " SYN_LINK_TEXT:0 \"[This.GetTag]/[Root.GetTag]/[From.GetTag]/[Prev.GetTag]\"\n",
                " ADD_FACTION_INFLUENCE_EFFECT:0 \"$COUNTRY$/$FACTION$/$VAL$ [Faction.GetName]/[Faction.Leader.GetTag] [This.GetTag]/[Root.GetTag]/[From.GetTag] [GetYear] [Faction.Leader.SYN_LINK]\"\n",
                " ADD_FACTION_INFLUENCE_EFFECT_NO_FACTION:0 \"$COUNTRY$/$VAL$ [Faction.GetName] [This.GetTag]/[Root.GetTag]/[From.GetTag]\"\n",
            ),
        ),
    ]);
    fixture.catalogue = catalogue;
    fixture
        .catalogue
        .scripted_content()
        .unwrap()
        .set_synthetic_environment(Default::default());
    fixture.set_countries(synthetic_countries(count));
    *fixture.frame.tables[TableId::Factions.index()] =
        TableView::Complete(TableRows::Factions(vec![FactionRow {
            id: 9,
            name: b"Union".to_vec(),
            ..Default::default()
        }]));
    *fixture.frame.tables[TableId::FactionMembers.index()] =
        TableView::Complete(TableRows::FactionMembers(vec![FactionMemberRow {
            country: count * 100,
            faction: 9,
            ordinal: 0,
        }]));
    *fixture.frame.tables[TableId::ScriptVariables.index()] =
        TableView::Complete(TableRows::ScriptVariables(vec![ScriptVariablesRow {
            scope_kind: 1,
            scope_id: count * 100,
            entries: vec![ScriptVariableEntry {
                name: b"syn_amount".to_vec(),
                value_raw: -12_500,
            }],
            ..Default::default()
        }]));
    for (body, country, expected) in [
        (
            "fixture_influence_ratio=var:syn_amount",
            count,
            "Last/Union/-12% Union/ZZZ ZZZ/AAB/AAA  ZZZ/ZZZ/ZZZ/ZZZ",
        ),
        (
            "fixture_influence_score=-1.25",
            count,
            "Last/Union/-1.25 Union/ZZZ ZZZ/AAB/AAA  ZZZ/ZZZ/ZZZ/ZZZ",
        ),
        (
            "fixture_influence_score=THIS",
            count,
            "Last/Union/0.00 Union/ZZZ ZZZ/AAB/AAA  ZZZ/ZZZ/ZZZ/ZZZ",
        ),
        (
            "fixture_influence_ratio=0",
            1,
            "Alpha/0% NULL AAA/AAB/AAA",
        ),
    ] {
        let mut scopes = EventScopes::new(vec![
            EventScope {
                root: crate::scripted_text::Link::Scope(1),
                from: crate::scripted_text::Link::Scope(2),
                ..EventScope::of(C(country))
            },
            EventScope::of(C(2)),
            EventScope::of(C(1)),
        ]);
        let out = fixture.render(body, &mut scopes, Content::default());
        assert_eq!(out.text, expected, "{count}/{body}: {:?}", out.refusals);
        assert!(
            out.refusals.is_empty(),
            "{count}/{body}: {:?}",
            out.refusals
        );
    }
    *fixture.frame.tables[TableId::FactionMembers.index()] = TableView::Unavailable {
        reason: "synthetic gap".into(),
    };
    let mut scopes = EventScopes::new(vec![EventScope::of(C(count))]);
    let out = fixture.render(
        "fixture_influence_score=1",
        &mut scopes,
        Content::default(),
    );
    assert!(
        out.refusals
            .keys()
            .any(|why| why.contains("FactionMembers")),
        "{:?}",
        out.refusals
    );
}

#[test]
fn diplomacy_output_branches() {
    game::enter();
    let mut fixture = Fixture::new();
    let (catalogue, _folder) = crate::fixtures::folder_catalogue(&[
        (
            "common/defines/test.lua",
            "NDefines={NGame={MAX_SCRIPTED_LOC_RECURSION=30}}",
        ),
        (
            "localisation/test_l_english.yml",
            concat!(
                "\u{feff}l_english:\n",
                " AAA:0 \"Alpha\"\n BBB:0 \"Beta\"\n CCC:0 \"Gamma\"\n",
                " AAA_DEF:0 \"alpha\"\n BBB_DEF:0 \"beta\"\n CCC_DEF:0 \"éclair\"\n",
                " ADD_AUTONOMY_SCORE_DESC:0 \"score $VALUE|0$/$COUNTRY|0$\"\n",
                " ADD_AUTONOMY_SCORE_DETAILED_DESC:0 \"detailed $VALUE|0$\"\n",
                " EFFECT_BECOME_EXILED_IN:0 \"host $HOSTNAMEDEF$ [Root.GetTag]\"\n",
                " EFFECT_END_EXILE:0 \"return $COUNTRY$\"\n",
                " EFFECT_END_PUPPET:0 \"free $NAME$/$COUNTRY$\"\n",
                " EFFECT_MAKE_PUPPET:0 \"subject $COUNTRY$/$TARGET$\"\n",
                " EFFECT_RELEASE_PUPPET:0 \"release $COUNTRY$\"\n",
                " synthetic:0 \"Level [Root.GetTag]\"\n synthetic_other:0 \"Other\"\n",
                " autonomy_free:0 \"Defined free\"\n",
                " EFFECT_RELEASE_AUTONOMY:0 \"Autonomy $COUNTRY$/$NAME$\"\n",
                " EFFECT_SET_AUTONOMY:0 \"Set $COUNTRY$/$TARGET$/$AUTONOMY$\"\n",
                " EFFECT_AUTONOMY_FREE:0 \"Free $COUNTRY$\"\n",
                " EFFECT_REMOVE_EXILE_TAG:0 \"exile [This.GetName]\"\n",
                " SYN_NAME:0 \"Charter [This.GetTag]\"\n ROOT:0 \"Root literal\"\n",
                " EFFECT_CREATE_FACTION:0 \"Create $NAME$\"\n",
                " EFFECT_DISMANTLE_FACTION:0 \"Dismantle $NAME$\"\n",
                " EFFECT_REMOVE_FROM_FACTION:0 \"Remove $COUNTRY$\"\n",
                " EFFECT_REMOVE_FROM_FACTION_NAME:0 \"Remove $COUNTRY$/$FACTION$\"\n",
                " EFFECT_SET_FACTION_NAME:0 \"Rename $OLDNAME$/$NEWNAME$\"\n",
                " EFFECT_SET_FACTION_SPYMASTER:0 \"Spy $NAME$\"\n",
                " EFFECT_MARKET_CREATE_PURCHASE_CONTRACT:0 \"Purchase $SELLER$/$BUYER$/$CIVS$/$EQUIPMENT$\"\n",
                " SHORT_TERM:0 \"Short\"\n MEDIUM_TERM:0 \"Medium\"\n LONG_TERM:0 \"Long\"\n",
                " FACTION_ADD_EXTRA_GOAL_SLOT:0 \"Slots $FACTION$/$CATEGORY$/$VAL$\"\n",
                " FACTION_ADD_EXTRA_GOAL_SLOT_NO_FACTION:0 \"SlotsOutside $CATEGORY$/$VAL$\"\n",
                " ADD_FACTION_INITIATIVE_EFFECT:0 \"Initiative $FACTION$/$AMOUNT|2$\"\n",
                " ADD_FACTION_INITIATIVE_EFFECT_NO_FACTION:0 \"InitiativeOutside $AMOUNT|2$\"\n",
                " FACTION_INFLUENCE_RATIO:0 \"$VAL|%0$\"\n FACTION_INFLUENCE_VALUE:0 \"$VAL|2$\"\n",
                " ADD_FACTION_INFLUENCE_EFFECT:0 \"Influence $COUNTRY$/$FACTION$/$VAL$\"\n",
                " ADD_FACTION_INFLUENCE_EFFECT_NO_FACTION:0 \"Outside $COUNTRY$/$VAL$\"\n",
                " FACTION_ACCESS_TAB_MILITARY:0 \"Military\"\n",
                " FACTION_ACCESS_TAB_UNLOCK:0 \"Unlock $NAME$\"\n FACTION_ACCESS_TAB_LOCK:0 \"Lock $NAME$\"\n",
                " EFFECT_ADD_CIVIL_WAR_TARGET:0 \"Civil $COUNTRY$\"\n",
                " EFFECT_REMOVE_CIVIL_WAR_TARGET:0 \"Uncivil $COUNTRY$\"\n",
                " EFFECT_GIVE_GUARANTEE:0 \"Guarantee $COUNTRY$\"\n",
                " EFFECT_GIVE_MILITARY_ACCESS:0 \"Access $COUNTRY$\"\n",
                " EFFECT_MARKET_GIVE_MARKET_ACCESS:0 \"Market $COUNTRY$\"\n",
                " EFFECT_REMOVE_WARGOAL:0 \"Goal $NAME$/$COUNTRY$\"\n",
                " EFFECT_ADD_TO_WAR:0 \"War $COUNTRY1$/$COUNTRY2$\"\n",
                " EFFECT_SET_WAR_SUPPORT:0 \"Support $VAL|0$\"\n",
                " EFFECT_ENACT_RELATION:0 \"Enact $NAME$/$WHO$\"\n",
                " EFFECT_CANCEL_RELATION:0 \"Cancel $NAME$/$WHO$\"\n",
                " DIPLOMACY_NAVAL_BLOCKADE_TITLE:0 \"Blockade\"\n",
                " DIPLOMACY_AIR_BASE_ACCESS:0 \"Air\"\n",
                " DIPLOMACY_GUARANTEE_TITLE:0 \"Guarantee\"\n",
                " DIPLOMACY_MILACC:0 \"Access\"\n",
                " DIPLOMACY_IMPROVERELATION:0 \"Improve\"\n",
                " DIPLOMACY_NONAGGRESSIONPACT:0 \"Pact\"\n",
                " DIPLOMACY_EMBARGO_TITLE:0 \"Embargo\"\n",
                " DIPLOMACY_INTERNATIONAL_MARKET_ACCESS_RIGHTS:0 \"Market\"\n",
                " DIPLOMACY_ATWAR:0 \"Atwar\"\n",
                " DIPLOMACY_SEND_ATTACHE:0 \"Attache\"\n",
                " DIPLOMACY_DOCKING_RIGHTS:0 \"Docking\"\n",
                " DIPLOMACY_LICACC:0 \"Licence\"\n",
                " EFFECT_BORDER_WAR:0 \"Border $STATE$ [This.GetName]\"\n",
                " EFFECT_END_BORDER_WAR:0 \"End $STATE$\"\n",
                " EFFECT_CANCEL_BORDER_WAR:0 \"Cancel $STATE1$/$STATE2$\"\n",
                " EFFECT_FINALIZE_BORDER_WAR:0 \"Final $STATE1$/$STATE2$\"\n",
                " EFFECT_UPDATE_BORDER_WAR:0 \"Update $STATE1$/$STATE2$\"\n",
                " EFFECT_UPDATE_BORDER_WAR_COMBAT_WIDTH:0 \"Width $NUM|2$\"\n",
                " EFFECT_UPDATE_BORDER_WAR_CHANGE_STATE:0 \"Change $VAL$\"\n",
                " EFFECT_UPDATE_BORDER_WAR_NUM_UNITS:0 \"Units $NUM|0$\"\n",
                " EFFECT_UPDATE_BORDER_WAR_MODIFIER:0 \"Modifier $NUM|2$\"\n",
                " YES:0 \"yes [Root.GetTag]\"\n NO:0 \"no\"\n",
            ),
        ),
    ]);
    fixture.catalogue = catalogue;
    fixture
        .catalogue
        .scripted_content()
        .unwrap()
        .set_synthetic_environment(Default::default());
    let mut autonomy_states = content_reader_politics::autonomy::AutonomyStates::new(
        ["synthetic", "synthetic_other"]
            .into_iter()
            .map(|id| content_reader_politics::autonomy::AutonomyState {
                id: id.into(),
                min_freedom_level: None,
            })
            .collect(),
    );
    *fixture.frame.tables[TableId::Autonomy.index()] =
        TableView::Complete(TableRows::Autonomy(vec![
            AutonomyRow {
                subject: 100,
                lower_score: 200_000,
                upper_score: 1_000_000,
                ..Default::default()
            },
            AutonomyRow {
                subject: 200,
                lower_score: -1,
                upper_score: i64::MAX,
                ..Default::default()
            },
        ]));
    *fixture.frame.tables[TableId::Characters.index()] =
        TableView::Complete(TableRows::Characters(
            (11..=15)
                .map(|id| CharacterRow {
                    id,
                    country: 100,
                    ..Default::default()
                })
                .collect(),
        ));
    let leader_type = engine_native::cunit_leader::REFERENCE_TYPE;
    *fixture.frame.tables[TableId::UnitLeaders.index()] =
        TableView::Complete(TableRows::UnitLeaders(
            [(7, 11, 200), (8, 12, 0), (9, 13, -1)]
                .into_iter()
                .map(|(id, character_id, exile_country)| UnitLeaderRow {
                    id,
                    reference_type: leader_type,
                    character_id,
                    exile_country: exile_country.into(),
                    ..Default::default()
                })
                .collect(),
        ));
    *fixture.frame.tables[TableId::CharacterOperatives.index()] =
        TableView::Complete(TableRows::CharacterOperatives(vec![
            CharacterOperativeRow {
                character: 14,
                reference: (20_u64 << 32) | u64::from(leader_type),
                resolved: true,
                category: 3,
                ..Default::default()
            },
        ]));
    let leader = EventObject::UnitLeader((7_u64 << 32) | u64::from(leader_type));
    *fixture.frame.tables[TableId::Factions.index()] =
        TableView::Complete(TableRows::Factions(vec![FactionRow {
            id: 11,
            name: b"Union".to_vec(),
            ..Default::default()
        }]));
    *fixture.frame.tables[TableId::FactionMembers.index()] =
        TableView::Complete(TableRows::FactionMembers(vec![
            FactionMemberRow {
                country: 100,
                faction: 11,
                ordinal: 0,
            },
            FactionMemberRow {
                country: 200,
                faction: 11,
                ordinal: 1,
            },
        ]));
    *fixture.frame.tables[TableId::ScriptVariables.index()] =
        TableView::Complete(TableRows::ScriptVariables(vec![ScriptVariablesRow {
            seed: replay_data::rows::Recorded::Value(None),
            scope_kind: 1,
            scope_id: 100,
            entries: vec![
                ScriptVariableEntry {
                    name: b"ratio".to_vec(),
                    value_raw: 50_000,
                },
                ScriptVariableEntry {
                    name: b"syn_slots".to_vec(),
                    value_raw: -150_000,
                },
                ScriptVariableEntry {
                    name: b"syn_initiative".to_vec(),
                    value_raw: 125_999,
                },
            ],
        }]));
    for (body, object, mask, expected, refusal) in [
        (
            "fixture_autonomy_points={value=9}",
            EventObject::Country(1),
            0,
            "",
            None,
        ),
        (
            "fixture_autonomy_points={value=9}",
            EventObject::Country(1),
            1,
            "",
            None,
        ),
        (
            "fixture_autonomy_points={value=-2 localization=unused}",
            EventObject::Country(1),
            2,
            "score -2/1\n",
            None,
        ),
        (
            "fixture_autonomy_points={value=3}",
            EventObject::Country(1),
            4,
            "score 3/1\n",
            None,
        ),
        (
            "fixture_autonomy_points={}",
            EventObject::Country(1),
            8,
            "score 0/1\n",
            None,
        ),
        (
            "fixture_autonomy_points={value=4}",
            EventObject::Country(0),
            2,
            "detailed 4\n",
            None,
        ),
        (
            "fixture_autonomy_points={value=4}",
            EventObject::State(1),
            2,
            "detailed 4\n",
            None,
        ),
        (
            "fixture_autonomy_points={value=4}",
            EventObject::Character(11),
            2,
            "detailed 4\n",
            None,
        ),
        (
            "fixture_autonomy_points={value=4}",
            leader,
            2,
            "detailed 4\n",
            None,
        ),
        (
            "fixture_autonomy_points={value=var:ratio}",
            EventObject::Country(1),
            2,
            "score 0/1\n",
            None,
        ),
        (
            "fixture_autonomy_points={value=3 value=var:ratio}",
            C(1),
            2,
            "score 3/1\n",
            None,
        ),
        (
            "fixture_autonomy_points={value=var:ratio value=-2}",
            C(1),
            2,
            "score -2/1\n",
            None,
        ),
        (
            "fixture_autonomy_ratio={value=var:missing}",
            EventObject::Country(1),
            0,
            "",
            None,
        ),
        (
            "fixture_autonomy_ratio={value=var:ratio}",
            EventObject::Country(1),
            2,
            "score 4/1\n",
            None,
        ),
        (
            "fixture_autonomy_ratio={value=-0.5}",
            EventObject::Country(1),
            2,
            "score -4/1\n",
            None,
        ),
        (
            "fixture_autonomy_ratio={}",
            EventObject::Country(1),
            2,
            "score 0/1\n",
            None,
        ),
        (
            "fixture_autonomy_ratio={value=0.5}",
            EventObject::Country(2),
            2,
            "score 0/1\n",
            None,
        ),
        (
            "fixture_autonomy_ratio={value=var:missing}",
            EventObject::Country(3),
            2,
            "",
            None,
        ),
        (
            "fixture_autonomy_ratio={value=var:missing}",
            EventObject::State(1),
            2,
            "",
            None,
        ),
        (
            "fixture_autonomy_ratio={value=var:missing}",
            EventObject::Character(11),
            2,
            "",
            None,
        ),
        (
            "fixture_exile_host={target=BBB legitimacy=var:missing}",
            EventObject::Country(1),
            2,
            "host beta AAA\n",
            None,
        ),
        (
            "fixture_exile_host={target=event_target:absent}",
            EventObject::Country(1),
            2,
            "",
            None,
        ),
        (
            "fixture_end_exile_status=BBB",
            EventObject::Country(1),
            2,
            "return Beta",
            None,
        ),
        (
            "fixture_end_exile_status=CCC",
            EventObject::Country(1),
            2,
            "return éclair",
            None,
        ),
        (
            "fixture_end_exile_status=event_target:absent",
            EventObject::Country(1),
            2,
            "",
            None,
        ),
        (
            "fixture_end_subject_status=BBB",
            EventObject::Country(1),
            2,
            "free Alpha/Beta\n",
            None,
        ),
        (
            "fixture_end_subject_status=event_target:absent",
            EventObject::Country(1),
            2,
            "",
            None,
        ),
        (
            "fixture_make_subject=BBB",
            EventObject::Country(1),
            2,
            "subject Alpha/Beta\n",
            None,
        ),
        (
            "fixture_make_subject={target=BBB end_wars=no end_civil_wars=yes}",
            EventObject::Country(1),
            2,
            "subject Alpha/Beta\n",
            None,
        ),
        (
            "fixture_make_subject={target=event_target:absent}",
            EventObject::Country(1),
            2,
            "",
            None,
        ),
        (
            "fixture_subject_release={target=BBB}",
            EventObject::Country(1),
            2,
            "",
            None,
        ),
        (
            "fixture_subject_release={target=event_target:absent autonomy_state=synthetic}",
            EventObject::Country(1),
            2,
            "",
            None,
        ),
        (
            "fixture_subject_release={target=BBB autonomy_state=synthetic freedom_level=var:missing}",
            EventObject::Country(1),
            2,
            "Autonomy Beta/Level AAA\n",
            None,
        ),
        (
            "fixture_subject_level={target=BBB autonomy_state=synthetic freedom_level=var:missing}",
            EventObject::Country(1),
            2,
            "Set Alpha/Beta/Level AAA\n",
            None,
        ),
        (
            "fixture_subject_level={target=event_target:absent autonomy_state=synthetic}",
            EventObject::Country(1),
            2,
            "",
            None,
        ),
        (
            "fixture_subject_release={target=BBB autonomous_state=synthetic_other}",
            C(1),
            2,
            "Autonomy Beta/Other\n",
            None,
        ),
        (
            "fixture_subject_release={target=BBB autonomy_state=synthetic_other autonomous_state=synthetic}",
            C(1),
            2,
            "Autonomy Beta/Level AAA\n",
            None,
        ),
        (
            "fixture_subject_release={target=BBB autonomous_state=synthetic autonomy_state=synthetic_other}",
            C(1),
            2,
            "Autonomy Beta/Other\n",
            None,
        ),
        (
            "fixture_subject_release={target=BBB autonomy_state=synthetic autonomy_state=missing}",
            C(1),
            2,
            "",
            Some("not defined"),
        ),
        (
            "fixture_subject_release={target=event_target:absent autonomy_state=missing}",
            C(1),
            2,
            "",
            Some("not defined"),
        ),
        (
            "fixture_subject_release={target=BBB autonomy_state=Synthetic}",
            C(1),
            2,
            "",
            Some("not defined"),
        ),
        (
            "fixture_subject_release={target=BBB autonomy_state=\"\"}",
            C(1),
            2,
            "",
            None,
        ),
        (
            "fixture_subject_level={target=BBB autonomous_state=synthetic_other autonomy_state=synthetic}",
            C(1),
            2,
            "Set Alpha/Beta/Level AAA\n",
            None,
        ),
        (
            "fixture_subject_level={target=BBB autonomy_state=synthetic autonomous_state=synthetic_other}",
            C(1),
            2,
            "Set Alpha/Beta/Other\n",
            None,
        ),
        (
            "fixture_subject_level={target=BBB autonomy_state=missing}",
            C(1),
            2,
            "",
            Some("not defined"),
        ),
        (
            "fixture_subject_level={target=event_target:absent autonomy_state=missing}",
            C(1),
            2,
            "",
            None,
        ),
        (
            "fixture_subject_level={target=BBB}",
            C(1),
            2,
            "Set Alpha/Beta/\n",
            None,
        ),
        (
            "fixture_subject_level={target=BBB autonomy_state=autonomy_free freedom_level=var:missing}",
            C(1),
            2,
            "Free beta\n",
            None,
        ),
        (
            "fixture_free_subject=BBB",
            EventObject::Country(1),
            2,
            "release Beta\n",
            None,
        ),
        (
            "fixture_free_subject=event_target:absent",
            EventObject::Country(1),
            2,
            "",
            None,
        ),
        (
            "fixture_free_controlled_subject=BBB",
            EventObject::Country(1),
            2,
            "release Beta\n",
            None,
        ),
        (
            "fixture_free_controlled_subject=event_target:absent",
            EventObject::Country(1),
            2,
            "",
            None,
        ),
        (
            "fixture_clear_exile_status=no",
            EventObject::Character(11),
            2,
            "exile [This.GetName]",
            None,
        ),
        (
            "fixture_clear_exile_status=yes",
            leader,
            2,
            "exile [This.GetName]",
            None,
        ),
        (
            "fixture_clear_exile_status=yes",
            EventObject::Character(12),
            2,
            "",
            None,
        ),
        (
            "fixture_clear_exile_status=yes",
            EventObject::Character(13),
            2,
            "",
            None,
        ),
        (
            "fixture_clear_exile_status=yes",
            EventObject::Character(15),
            2,
            "",
            None,
        ),
        (
            "fixture_clear_exile_status=yes",
            EventObject::Character(14),
            2,
            "",
            Some("exile_country"),
        ),
        (
            "fixture_new_alliance=SYN_NAME",
            C(1),
            2,
            "Create Charter AAA\n",
            None,
        ),
        (
            "fixture_new_alliance=ROOT",
            C(1),
            2,
            "Create Root literal\n",
            None,
        ),
        ("fixture_new_alliance=\"\"", C(1), 2, "", None),
        ("fixture_new_alliance=SYN_NAME", S(0), 2, "", None),
        ("fixture_end_alliance=no", C(1), 2, "Dismantle Union\n", None),
        ("fixture_end_alliance=yes", C(2), 2, "", None),
        ("fixture_end_alliance=yes", C(3), 2, "", None),
        ("fixture_exit_alliance=ROOT", C(1), 2, "Remove alpha/Union\n", None),
        ("fixture_exit_alliance=yes", C(3), 2, "", None),
        ("fixture_exit_alliance=yes", S(1), 2, "", None),
        (
            "fixture_expel_member=CCC",
            C(1),
            2,
            "Remove Gamma/Union\n",
            None,
        ),
        ("fixture_expel_member=AAA", C(3), 2, "Remove Alpha\n", None),
        (
            "fixture_expel_member=AAA",
            S(1),
            2,
            "",
            Some("current scope has no country"),
        ),
        ("fixture_expel_member=CCC", C(2), 2, "", None),
        ("fixture_expel_member=0", C(1), 2, "", None),
        (
            "fixture_alliance_title=SYN_NAME",
            C(1),
            2,
            "Rename Union/Charter AAA\n",
            None,
        ),
        ("fixture_alliance_title=ROOT", C(1), 2, "Rename Union/\n", None),
        ("fixture_alliance_title=SYN_NAME", C(3), 2, "", None),
        ("fixture_intelligence_lead=no", C(2), 2, "Spy Union\n", None),
        ("fixture_intelligence_lead=yes", C(3), 2, "", None),
        (
            "fixture_goal_slot={category=short_term value=1.5}",
            C(1),
            2,
            "Slots Union/Short/2\n",
            None,
        ),
        (
            "fixture_goal_slot={category=medium_term value=syn_slots}",
            C(1),
            2,
            "Slots Union/Medium/-2\n",
            None,
        ),
        (
            "fixture_goal_slot={category=long_term value=-1.49999}",
            C(1),
            2,
            "Slots Union/Long/-1\n",
            None,
        ),
        (
            "fixture_goal_slot={category=long_term value=2}",
            C(3),
            2,
            "SlotsOutside Long/2\n",
            None,
        ),
        (
            "fixture_goal_slot={category=short_term category=\"short_term\" value=3 value=4}",
            C(1),
            2,
            "Slots Union//4\n",
            None,
        ),
        (
            "fixture_goal_slot={}",
            S(0),
            2,
            "SlotsOutside /0\n",
            None,
        ),
        (
            "fixture_goal_slot={category=short_term value=2 fixture_guarantee_country=BBB}",
            C(1),
            2,
            "Slots Union/Short/2\nGuarantee Beta\n",
            None,
        ),
        (
            "fixture_alliance_initiative=syn_initiative",
            C(1),
            2,
            "Initiative Union/1.25",
            None,
        ),
        (
            "fixture_alliance_initiative=-2.5",
            C(3),
            2,
            "InitiativeOutside -2.50",
            None,
        ),
        (
            "fixture_alliance_initiative=ROOT",
            C(1),
            2,
            "Initiative Union/0.00",
            None,
        ),
        (
            "fixture_alliance_access=yes",
            S(0),
            2,
            "Unlock Military",
            None,
        ),
        (
            "fixture_alliance_access=no",
            C(1),
            2,
            "Lock Military",
            None,
        ),
        (
            "fixture_alliance_access=YES",
            C(1),
            2,
            "Lock Military",
            None,
        ),
        (
            "fixture_alliance_access=\"yes\"",
            C(1),
            2,
            "Unlock Military",
            None,
        ),
        (
            "fixture_alliance_initiative=2 fixture_alliance_access=yes",
            C(1),
            2,
            "Initiative Union/2.00\nUnlock Military",
            None,
        ),
        ("fixture_civil_target=BBB", C(1), 2, "Civil Beta\n", None),
        (
            "fixture_end_civil_target=CCC",
            C(1),
            2,
            "Uncivil Gamma\n",
            None,
        ),
        ("fixture_guarantee_country=BBB", C(1), 2, "Guarantee Beta\n", None),
        ("fixture_military_access=BBB", C(1), 2, "Access Beta\n", None),
        ("fixture_market_access=BBB", C(1), 2, "Market beta\n", None),
        (
            "fixture_revoke_objective={target=BBB type=all}",
            C(1),
            2,
            "Goal Alpha/Beta\n",
            None,
        ),
        (
            "fixture_revoke_objective={target=event_target:absent type=all}",
            C(1),
            2,
            "",
            None,
        ),
        (
            "fixture_revoke_objective={target=ROOT target=BBB}",
            C(1),
            2,
            "",
            Some("auxiliary reader replay is unavailable"),
        ),
        (
            "fixture_join_war={enemy=BBB targeted_alliance=CCC}",
            C(1),
            2,
            "War Beta/Gamma\n",
            None,
        ),
        (
            "fixture_join_war={enemy=ROOT.FROM targeted_alliance=PREV.ROOT}",
            C(1),
            2,
            "War Alpha/Alpha\n",
            None,
        ),
        (
            "fixture_join_war={enemy=\"root.from\" targeted_alliance=PREV}",
            S(1),
            2,
            "War Alpha/Alpha\n",
            None,
        ),
        (
            "fixture_join_war={enemy=ROOT enemy=FROM targeted_alliance=CCC}",
            C(1),
            2,
            "",
            Some("auxiliary reader replay is unavailable"),
        ),
        (
            "fixture_join_war={enemy=ROOT.owner targeted_alliance=CCC}",
            C(1),
            2,
            "",
            Some("invalid scope-link component"),
        ),
        (
            "fixture_join_war={enemy=var:ratio targeted_alliance=CCC}",
            C(1),
            2,
            "",
            Some("country specifier"),
        ),
        ("fixture_war_support=0.25", C(1), 2, "Support 0\n", None),
        (
            "fixture_border_status=yes",
            S(1),
            2,
            "Border One [This.GetName]\n",
            None,
        ),
        ("fixture_border_status=ROOT", S(1), 2, "End One\n", None),
        ("fixture_border_status=yes", C(1), 2, "", None),
        (
            "fixture_cancel_border={attacker=1 defender=2}",
            C(1),
            2,
            "Cancel One(\u{12}AAA)/Two(\u{12}AAA)\n",
            None,
        ),
        (
            "fixture_cancel_border={attacker=0 defender=2}",
            C(1),
            2,
            "",
            None,
        ),
        (
            "fixture_settle_border={attacker=1 defender=2}",
            C(1),
            2,
            "",
            None,
        ),
        (
            "fixture_settle_border={attacker=1 defender=2 attacker_win=yes defender_win=yes}",
            C(1),
            2,
            "Final One(\u{12}AAA)/Two(\u{12}AAA)\n",
            None,
        ),
        (
            "fixture_settle_border={attacker=1 defender=2 defender_win=yes}",
            C(1),
            2,
            "Final Two(\u{12}AAA)/One(\u{12}AAA)\n",
            None,
        ),
        (
            "fixture_settle_border={attacker=0 defender=2 attacker_win=yes}",
            C(1),
            2,
            "",
            Some("invalid state 0"),
        ),
        (
            "fixture_border_facts={attacker=1 defender=2 combat_width=2.5 change_state_after_war=yes max_attackers=3 attacker_modifier=-0.25 max_defenders=4 max_units=var:missing modifier=var:missing}",
            C(1),
            2,
            "Update One/Two\nWidth 2.50\nChange yes AAA\n\t\u{11}HOne\u{11}!:\nUnits 3\nModifier -0.25\n\t\u{11}HTwo\u{11}!:\nUnits 4\n",
            None,
        ),
        (
            "fixture_border_facts={attacker=0 defender=2 combat_width=var:missing}",
            C(1),
            2,
            "",
            None,
        ),
        (
            "fixture_bilateral_relation={country=BBB relation=military_access active=no}",
            C(1),
            2,
            "Cancel Access/beta\n",
            None,
        ),
        (
            "fixture_bilateral_relation={country=BBB relation=guarantee fixture_guarantee_country=CCC}",
            C(1),
            2,
            "Enact Guarantee/beta\nGuarantee Gamma\n",
            None,
        ),
        (
            "fixture_bilateral_relation={country=BBB relation=guarantee fixture_guarantee_country=CCC}",
            S(1),
            2,
            "Guarantee Gamma\n",
            None,
        ),
        (
            "fixture_bilateral_relation={country=\"BBB\" relation=guarantee relation=military_access active=no}",
            C(1),
            2,
            "Cancel Access/beta\n",
            None,
        ),
        (
            "fixture_join_war={enemy=\"BBB\" targeted_alliance=CCC}",
            C(1),
            2,
            "War Beta/Gamma\n",
            None,
        ),
        (
            "fixture_bilateral_relation={country=event_target:absent relation=guarantee}",
            C(1),
            2,
            "",
            None,
        ),
        (
            "fixture_bilateral_relation={country=BBB relation=guarantee}",
            S(1),
            2,
            "",
            None,
        ),
        (
            "fixture_bilateral_relation={country=BBB relation=\"guarantee\"}",
            C(1),
            2,
            "BROKEN DIPLOMATIC RELATION EFFECT",
            None,
        ),
        (
            "fixture_bilateral_relation={country=BBB relation=synthetic_unknown}",
            C(1),
            2,
            "BROKEN DIPLOMATIC RELATION EFFECT",
            None,
        ),
        (
            "fixture_bilateral_relation={country=ROOT country=BBB relation=guarantee}",
            C(1),
            2,
            "",
            Some("auxiliary reader replay is unavailable"),
        ),
        (
            "fixture_bilateral_relation={country=BBB relation=boost_party_popularity}",
            C(1),
            2,
            "",
            Some("null ideology-group identifier"),
        ),
        (
            "fixture_war_objective={target=BBB type=synthetic}",
            C(1),
            2,
            "",
            None,
        ),
        (
            "fixture_civil_conflict={ideology=synthetic size=0.5}",
            C(1),
            2,
            "",
            Some("effect key names"),
        ),
        (
            "fixture_purchase_order={seller=AAA buyer=BBB civilian_factories=2}",
            C(1),
            2,
            "Purchase \u{12}AAA Alpha/\u{12}BBB Beta/2/\n",
            None,
        ),
        (
            "fixture_empty_scalar=synthetic_empty fixture_guarantee_country=BBB",
            C(1),
            2,
            "Guarantee Beta\n",
            None,
        ),
        (
            "fixture_end_exile_status=BBB fixture_guarantee_country=CCC",
            C(1),
            2,
            "return Beta\nGuarantee Gamma\n",
            None,
        ),
        (
            "fixture_bilateral_relation={country=BBB relation=naval_blockade}",
            C(1),
            2,
            "Enact Blockade/beta\n",
            None,
        ),
        (
            "fixture_bilateral_relation={country=BBB relation=air_base_access}",
            C(1),
            2,
            "Enact Air/beta\n",
            None,
        ),
        (
            "fixture_bilateral_relation={country=BBB relation=offer_air_base_access}",
            C(1),
            2,
            "Enact Air/beta\n",
            None,
        ),
        (
            "fixture_bilateral_relation={country=BBB relation=guarantee}",
            C(1),
            2,
            "Enact Guarantee/beta\n",
            None,
        ),
        (
            "fixture_bilateral_relation={country=BBB relation=military_access}",
            C(1),
            2,
            "Enact Access/beta\n",
            None,
        ),
        (
            "fixture_bilateral_relation={country=BBB relation=offer_military_access}",
            C(1),
            2,
            "Enact Access/beta\n",
            None,
        ),
        (
            "fixture_bilateral_relation={country=BBB relation=improve_relation}",
            C(1),
            2,
            "Enact Improve/beta\n",
            None,
        ),
        (
            "fixture_bilateral_relation={country=BBB relation=non_aggression_pact}",
            C(1),
            2,
            "Enact Pact/beta\n",
            None,
        ),
        (
            "fixture_bilateral_relation={country=BBB relation=embargo}",
            C(1),
            2,
            "Enact Embargo/beta\n",
            None,
        ),
        (
            "fixture_bilateral_relation={country=BBB relation=market_access_rights}",
            C(1),
            2,
            "Enact Market/beta\n",
            None,
        ),
        (
            "fixture_bilateral_relation={country=BBB relation=war_relation}",
            C(1),
            2,
            "Enact Atwar/beta\n",
            None,
        ),
        (
            "fixture_bilateral_relation={country=BBB relation=send_attache}",
            C(1),
            2,
            "Enact Attache/beta\n",
            None,
        ),
        (
            "fixture_bilateral_relation={country=BBB relation=docking_rights}",
            C(1),
            2,
            "Enact Docking/beta\n",
            None,
        ),
        (
            "fixture_bilateral_relation={country=BBB relation=offer_docking_rights}",
            C(1),
            2,
            "Enact Docking/beta\n",
            None,
        ),
        (
            "fixture_bilateral_relation={country=BBB relation=licence_production_access}",
            C(1),
            2,
            "Enact Licence/beta\n",
            None,
        ),
        (
            "fixture_bilateral_relation={country=BBB relation=puppet}",
            C(1),
            2,
            "Enact /beta\n",
            None,
        ),
        (
            "fixture_bilateral_relation={country=BBB relation=lend_lease}",
            C(1),
            2,
            "Enact /beta\n",
            None,
        ),
        (
            "fixture_bilateral_relation={country=BBB relation=equipment_purchase_contract_relation}",
            C(1),
            2,
            "Enact /beta\n",
            None,
        ),
        (
            "fixture_alliance_goal=SYN_GOAL",
            C(1),
            2,
            "",
            Some("faction goal content"),
        ),
        ("fixture_alliance_goal=SYN_GOAL", C(3), 2, "", None),
        (
            "fixture_end_alliance_goal=SYN_GOAL",
            C(1),
            2,
            "",
            Some("faction goal content"),
        ),
        ("fixture_end_alliance_goal=SYN_GOAL", C(3), 2, "", None),
        (
            "fixture_alliance_manifest=SYN_GOAL",
            C(1),
            2,
            "",
            Some("faction goal content"),
        ),
        ("fixture_alliance_manifest=SYN_GOAL", C(3), 2, "", None),
        (
            "fixture_alliance_template=SYN_TEMPLATE",
            C(1),
            2,
            "",
            Some("faction template content"),
        ),
        (
            "fixture_alliance_template=SYN_TEMPLATE",
            C(3),
            2,
            "",
            Some("faction template content"),
        ),
        (
            "fixture_minimum_member_upgrade={upgrade=SYN_UPGRADE}",
            C(1),
            2,
            "",
            Some("faction upgrade content"),
        ),
        (
            "fixture_minimum_member_upgrade={upgrade=SYN_UPGRADE}",
            C(3),
            2,
            "",
            None,
        ),
        (
            "fixture_alliance_rule=SYN_RULE",
            C(1),
            2,
            "",
            Some("faction rule content"),
        ),
        ("fixture_alliance_rule=SYN_RULE", C(3), 2, "", None),
        (
            "fixture_influence_ratio=0.1",
            C(1),
            2,
            "Influence Alpha/Union/10%",
            None,
        ),
        (
            "fixture_influence_ratio=0.1",
            C(3),
            2,
            "Outside Gamma/10%",
            None,
        ),
        (
            "fixture_influence_score=1",
            C(1),
            2,
            "Influence Alpha/Union/1.00",
            None,
        ),
        (
            "fixture_influence_score=1",
            C(3),
            2,
            "Outside Gamma/1.00",
            None,
        ),
    ] {
        *fixture.frame.tables[TableId::SimulationFeatures.index()] =
            TableView::Complete(TableRows::SimulationFeatures(vec![SimulationFeatureRow {
                dlc_mask: mask,
                ..Default::default()
            }]));
        let mut scopes = EventScopes::new(vec![EventScope::of(object)]);
        let out = fixture.render(
            body,
            &mut scopes,
            Content {
                autonomy_states: Some(&autonomy_states),
                ..Content::default()
            },
        );
        assert_eq!(out.text, expected, "{body}/{object:?}: {:?}", out.refusals);
        match refusal {
            None => assert!(
                out.refusals.is_empty(),
                "{body}/{object:?}: {:?}",
                out.refusals,
            ),
            Some(reason) => {
                assert_eq!(
                    out.refusals.values().sum::<usize>(),
                    1,
                    "{body}: {:?}",
                    out.refusals,
                );
                assert!(
                    out.refusals.keys().next().unwrap().contains(reason),
                    "{body}: {:?}",
                    out.refusals,
                );
            }
        }
    }
    autonomy_states
        .states
        .push(content_reader_politics::autonomy::AutonomyState {
            id: "autonomy_free".into(),
            min_freedom_level: None,
        });
    for (body, expected) in [
        (
            "fixture_subject_level={target=BBB autonomy_state=autonomy_free}",
            "Set Alpha/Beta/Defined free\n",
        ),
        (
            "fixture_subject_release={target=BBB autonomy_state=autonomy_free}",
            "Autonomy Beta/Defined free\n",
        ),
    ] {
        let mut scopes = EventScopes::new(vec![EventScope::of(C(1))]);
        let out = fixture.render(
            body,
            &mut scopes,
            Content {
                autonomy_states: Some(&autonomy_states),
                ..Content::default()
            },
        );
        assert_eq!(out.text, expected, "{body}: {:?}", out.refusals);
        assert!(out.refusals.is_empty(), "{body}: {:?}", out.refusals);
    }
    for (body, refused) in [
        ("fixture_subject_release={target=BBB}", false),
        (
            "fixture_subject_level={target=event_target:absent autonomy_state=synthetic}",
            false,
        ),
        (
            "fixture_subject_release={target=BBB autonomy_state=synthetic}",
            true,
        ),
        (
            "fixture_subject_level={target=BBB autonomy_state=autonomy_free}",
            true,
        ),
    ] {
        let mut scopes = EventScopes::new(vec![EventScope::of(C(1))]);
        let out = fixture.render(body, &mut scopes, Content::default());
        assert!(out.text.is_empty(), "{body}: {:?}", out.refusals);
        assert_eq!(out.refusals.values().sum::<usize>(), usize::from(refused));
        assert!(
            out.refusals
                .keys()
                .all(|why| why.contains("autonomy states content unavailable"))
        );
    }
    for (body, expected) in [
        (
            "fixture_join_war={enemy=ROOT.FROM targeted_alliance=FROM.ROOT}",
            "War Alpha/Beta\n",
        ),
        (
            "fixture_join_war={enemy=ROOT.ROOT targeted_alliance=BBB}",
            "War Beta/Beta\n",
        ),
    ] {
        let mut graph = EventScopes::new(vec![
            EventScope {
                root: Link::Scope(1),
                from: Link::Scope(3),
                ..EventScope::of(C(1))
            },
            EventScope {
                root: Link::Scope(2),
                from: Link::Scope(3),
                ..EventScope::of(C(2))
            },
            EventScope::of(EventObject::Character(11)),
            EventScope {
                root: Link::Scope(1),
                ..EventScope::of(S(2))
            },
        ]);
        let out = fixture.render(body, &mut graph, Content::default());
        assert_eq!(out.text, expected, "{body}: {:?}", out.refusals);
        assert!(out.refusals.is_empty(), "{body}: {:?}", out.refusals);
    }
    for (template_token, dictionary_gap, expected, refusal) in [
        (None, true, "Spy \n", None),
        (
            Some(77),
            false,
            "",
            Some("token 77 has no lexical tokens spelling"),
        ),
        (
            Some(77),
            true,
            "",
            Some("lexical tokens is not recorded completely"),
        ),
    ] {
        if let TableView::Complete(TableRows::Factions(rows)) =
            &mut *fixture.frame.tables[TableId::Factions.index()]
        {
            rows[0].name.clear();
            rows[0].template_token = template_token;
        }
        *fixture.frame.tables[TableId::LexicalTokens.index()] = if dictionary_gap {
            TableView::Unavailable {
                reason: "synthetic missing template dictionary".into(),
            }
        } else {
            TableView::Complete(TableRows::LexicalTokens(vec![]))
        };
        let mut graph = EventScopes::new(vec![EventScope::of(C(1))]);
        let out = fixture.render("fixture_intelligence_lead=yes", &mut graph, Content::default());
        assert_eq!(out.text, expected, "{:?}", out.refusals);
        assert_eq!(
            out.refusals.values().sum::<usize>(),
            usize::from(refusal.is_some())
        );
        if let Some(reason) = refusal {
            assert!(
                out.refusals.keys().any(|why| why.contains(reason)),
                "{:?}",
                out.refusals
            );
        }
    }
}

#[test]
fn diplomacy_ordered_subjects() {
    game::enter();
    let mut fixture = Fixture::new();
    let countries = synthetic_countries(24);
    let mut localisation = "\u{feff}l_english:\n EFFECT_ADD_TO_FACTION:0 \"Join $COUNTRY$\"\n EFFECT_GIVE_GUARANTEE:0 \"Guarantee $COUNTRY$\"\n"
        .to_owned();
    for country in &countries {
        let tag = std::str::from_utf8(&country.tag).unwrap();
        localisation.push_str(&format!(" {tag}:0 \"{tag}\"\n"));
    }
    let (catalogue, _folder) = crate::fixtures::folder_catalogue(&[
        (
            "common/defines/test.lua",
            "NDefines={NGame={MAX_SCRIPTED_LOC_RECURSION=30}}",
        ),
        ("localisation/test_l_english.yml", &localisation),
    ]);
    fixture.catalogue = catalogue;
    fixture
        .catalogue
        .scripted_content()
        .unwrap()
        .set_synthetic_environment(Default::default());
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
    let target = String::from_utf8(countries[1].tag.clone()).unwrap();
    let last = String::from_utf8(countries.last().unwrap().tag.clone()).unwrap();
    let subjects: Vec<_> = countries.iter().skip(2).map(|row| row.id as i32).collect();
    fixture.set_countries(countries);
    let TableView::Complete(TableRows::CountryAppearance(rows)) =
        &mut *fixture.frame.tables[TableId::CountryAppearance.index()]
    else {
        unreachable!()
    };
    rows[1].subjects = subjects.into();
    let mut graph = EventScopes::new(vec![EventScope::of(C(1))]);
    let out = fixture.render(
        &format!("fixture_invite_country={target} fixture_guarantee_country={last}"),
        &mut graph,
        Content::default(),
    );
    assert!(out.refusals.is_empty(), "{:?}", out.refusals);
    assert_eq!(out.text.lines().count(), 24);
    assert!(out.text.starts_with(&format!("Join {target}\n")));
    assert!(
        out.text
            .ends_with(&format!("Join {last}\nGuarantee {last}\n"))
    );
}

#[test]
fn create_wargoal_hover_reads_complete_unused_goal_membership() {
    game::enter();
    let mut fixture = Fixture::new();
    let (catalogue, _folder) = crate::fixtures::folder_catalogue(&[
        (
            "common/defines/test.lua",
            "NDefines={NGame={MAX_SCRIPTED_LOC_RECURSION=30}}",
        ),
        (
            "localisation/test_l_english.yml",
            "\u{feff}l_english:\n AAA:0 \"Alpha\"\n BBB:0 \"Beta\"\n SynGoal:0 \"Custom goal\"\n EFFECT_CREATE_WARGOAL:0 \"Goal $COUNTRY$/$WARGOAL$/$TARGET$\"\n",
        ),
    ]);
    fixture.catalogue = catalogue;
    fixture
        .catalogue
        .scripted_content()
        .unwrap()
        .set_synthetic_environment(Default::default());
    let names = content_reader_script::key_names::KeyNames::default();
    let database =
        content_reader_wargoals::wargoals::parse(b"wargoal_types={SynGoal={}}", &names).unwrap();
    let content = Content {
        key_names: Some(&names),
        wargoals: Some(&database),
        ..Default::default()
    };
    *fixture.frame.tables[TableId::CountryWargoals.index()] =
        TableView::Complete(TableRows::CountryWargoals(vec![]));
    *fixture.frame.tables[TableId::WargoalTargets.index()] =
        TableView::Complete(TableRows::WargoalTargets(vec![]));
    let render = |fixture: &Fixture| {
        fixture.render_native(
            "fixture_war_objective={target=BBB type=SynGoal}",
            &mut EventScopes::new(vec![EventScope::of(C(1))]),
            content,
        )
    };
    let shown = render(&fixture);
    assert!(shown.refusals.is_empty(), "{:?}", shown.refusals);
    assert!(shown.text.contains("Alpha"));
    assert!(shown.text.contains("Custom goal"));
    let handle = (4u64 << 32) | 123;
    *fixture.frame.tables[TableId::CountryWargoals.index()] =
        TableView::Complete(TableRows::CountryWargoals(vec![CountryWargoalRow {
            country: 100,
            handle,
            actor: 100,
            recipient: 200,
            goal_type: 42,
            name_token: 42,
            ..Default::default()
        }]));
    *fixture.frame.tables[TableId::WargoalTargets.index()] =
        TableView::Complete(TableRows::WargoalTargets(vec![WargoalTargetRow {
            country: 100,
            kind: 3,
            value: handle,
            ..Default::default()
        }]));
    *fixture.frame.tables[TableId::LexicalTokens.index()] =
        TableView::Complete(TableRows::LexicalTokens(vec![LexicalTokenRow {
            token: 42,
            literal: b"SynGoal".to_vec(),
        }]));
    for table in [TableId::CountryWargoals, TableId::WargoalTargets] {
        assert!(
            crate::triggers::RECORDED_TABLES.contains(&table),
            "effect-description consumers must request {table:?}"
        );
    }
    let suppressed = render(&fixture);
    assert_eq!(suppressed.text, "");
    assert!(suppressed.refusals.is_empty());
    *fixture.frame.tables[TableId::WargoalTargets.index()] =
        TableView::Complete(TableRows::WargoalTargets(vec![]));
    let consumed = render(&fixture);
    assert!(consumed.refusals.is_empty(), "{:?}", consumed.refusals);
    assert!(consumed.text.contains("Custom goal"));
    let duplicate =
        content_reader_wargoals::wargoals::parse(b"wargoal_types={SynGoal={} SynGoal={}}", &names)
            .unwrap();
    let duplicate_content = Content {
        wargoals: Some(&duplicate),
        ..content
    };
    for (unused, refused) in [(false, false), (true, true)] {
        *fixture.frame.tables[TableId::WargoalTargets.index()] =
            TableView::Complete(TableRows::WargoalTargets(if unused {
                vec![WargoalTargetRow {
                    country: 100,
                    kind: 3,
                    value: handle,
                    ..Default::default()
                }]
            } else {
                vec![]
            }));
        let out = fixture.render_native(
            "fixture_war_objective={target=BBB type=SynGoal}",
            &mut EventScopes::new(vec![EventScope::of(C(1))]),
            duplicate_content,
        );
        assert_eq!(
            out.refusals.values().sum::<usize>(),
            usize::from(refused),
            "{:?}",
            out.refusals
        );
        assert_eq!(out.text.is_empty(), refused);
    }
    *fixture.frame.tables[TableId::States.index()] = TableView::Complete(TableRows::States(vec![
        StateRow {
            id: 1,
            ..Default::default()
        },
        StateRow {
            id: 2,
            ..Default::default()
        },
    ]));
    *fixture.frame.tables[TableId::WargoalTargets.index()] =
        TableView::Complete(TableRows::WargoalTargets(vec![
            WargoalTargetRow {
                country: 100,
                kind: 3,
                value: handle,
                ..Default::default()
            },
            WargoalTargetRow {
                country: 100,
                kind: 0,
                value: 1,
                ..Default::default()
            },
        ]));
    let partial = fixture.render_native(
        "fixture_war_objective={target=BBB type=SynGoal generator={1 2}}",
        &mut EventScopes::new(vec![EventScope::of(C(1))]),
        duplicate_content,
    );
    assert!(partial.refusals.is_empty(), "{:?}", partial.refusals);
    assert!(partial.text.contains("Custom goal"));
    let covered = fixture.render_native(
        "fixture_war_objective={target=BBB type=SynGoal generator={1}}",
        &mut EventScopes::new(vec![EventScope::of(C(1))]),
        content,
    );
    assert!(covered.refusals.is_empty(), "{:?}", covered.refusals);
    assert!(covered.text.is_empty());
}
