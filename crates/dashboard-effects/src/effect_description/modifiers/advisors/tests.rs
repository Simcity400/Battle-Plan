use super::*;
use crate::fixtures::{Folder, folder_catalogue};
use crate::effect_description::tests::{Fixture, fresh};
use replay_data::rows::*;
use replay_frame::frame::TableView;
use crate::fixtures::game;

fn fixture() -> (Fixture, Folder) {
    let mut fixture = super::super::tests::fixture();
    let (mut catalogue, folder) = folder_catalogue(&[
        (
            "common/defines/test.lua",
            "NDefines={NGame={MAX_SCRIPTED_LOC_RECURSION=23},NCharacter={OFFICER_CORP_ADVISOR_ENTRIES_IN_MENU={\"synthetic_slot\"}}}",
        ),
        (
            "common/characters/test.txt",
            "characters={synthetic={name=synthetic_name advisor={slot=synthetic_slot idea_token=SYN_ADVISOR modifier={political_power_gain=999}}} synthetic_bonus={advisor={slot=synthetic_slot idea_token=SYN_BONUS_ADVISOR research_bonus={industry=0.1}}}}",
        ),
        (
            "common/country_leader/test.txt",
            "leader_traits={synthetic_trait={targeted_modifier={tag=BBB political_power_gain=0.5}}}",
        ),
    ]);
    catalogue.localisation = fixture.catalogue.localisation.clone();
    catalogue
        .scripted_content()
        .unwrap()
        .set_synthetic_environment(Default::default());
    fixture.catalogue = catalogue;
    *fixture.frame.tables[TableId::AdvisorTokens.index()] =
        TableView::Complete(TableRows::AdvisorTokens(vec![AdvisorTokenRow {
            token: b"SYN_ADVISOR".to_vec(),
            ..Default::default()
        }]));
    *fixture.frame.tables[TableId::Characters.index()] =
        TableView::Complete(TableRows::Characters(
            [(10, "First"), (20, "Second")]
                .into_iter()
                .map(|(id, name)| CharacterRow {
                    id,
                    country: 100,
                    name: name.as_bytes().to_vec(),
                    ..Default::default()
                })
                .collect(),
        ));
    *fixture.frame.tables[TableId::AdvisorRoles.index()] =
        TableView::Complete(TableRows::AdvisorRoles(
            [10, 20]
                .into_iter()
                .map(|character| AdvisorRoleRow {
                    character,
                    country: 100,
                    idea: b"SYN_ADVISOR".to_vec(),
                    tooltip_keys: Recorded::NotRecorded,
                    modifier_cache: Recorded::Value(Some(AdvisorModifierCache {
                        entries: vec![
                            CachedModifier {
                                definition: 1,
                                value: 200_000,
                            },
                            CachedModifier {
                                definition: 0,
                                value: 100_000,
                            },
                        ],
                    })),
                    ..Default::default()
                })
                .collect(),
        ));
    *fixture.frame.tables[TableId::AvailableAdvisors.index()] =
        TableView::Complete(TableRows::AvailableAdvisors(vec![AvailableAdvisorRow {
            country: 100,
            character: 10,
            idea: b"SYN_ADVISOR".to_vec(),
            ..Default::default()
        }]));
    (fixture, folder)
}

fn roles(fixture: &mut Fixture) -> &mut Vec<AdvisorRoleRow> {
    let TableView::Complete(TableRows::AdvisorRoles(rows)) =
        &mut *fixture.frame.tables[TableId::AdvisorRoles.index()]
    else {
        unreachable!()
    };
    rows
}

#[test]
fn available_order_precedes_retired_and_multi_names_do_not_need_the_cache() {
    game::enter();
    let (mut fixture, _folder) = fixture();
    *fixture.frame.tables[TableId::AvailableAdvisors.index()] =
        TableView::Complete(TableRows::AvailableAdvisors(vec![
            AvailableAdvisorRow {
                country: 100,
                ordinal: 2,
                character: 10,
                idea: b"SYN_ADVISOR".to_vec(),
            },
            AvailableAdvisorRow {
                country: 100,
                ordinal: 8,
                character: 20,
                idea: b"SYN_ADVISOR".to_vec(),
            },
            AvailableAdvisorRow {
                country: 200,
                ordinal: 0,
                character: 20,
                idea: b"SYN_ADVISOR".to_vec(),
            },
        ]));
    *fixture.frame.tables[TableId::CountryRetiredCharacters.index()] =
        TableView::Complete(TableRows::CountryRetiredCharacters(vec![
            CountryCharacterRow {
                country: 100,
                ordinal: 0,
                character: Some(20),
            },
        ]));
    *fixture.frame.tables[TableId::AdvisorRoles.index()] = TableView::Unavailable {
        reason: "synthetic unavailable".into(),
    };
    let ideas = content_reader_politics::ideas::IdeaContent::from_parsed(Vec::new(), Vec::new());
    for (body, expected) in [
        ("fixture_gain_ideas={SYN_ADVISOR SYN_ADVISOR}", "Add First, First"),
        (
            "fixture_lose_ideas={SYN_ADVISOR SYN_ADVISOR}",
            "Remove First, First",
        ),
        (
            "fixture_idea_details={SYN_ADVISOR SYN_ADVISOR}",
            "First, First",
        ),
    ] {
        let result = fixture.render(
            body,
            &mut fresh(),
            Content {
                ideas: Some(&ideas),
                ..Default::default()
            },
        );
        assert_eq!(result.text, expected);
        assert!(result.refusals.is_empty(), "{:?}", result.refusals);
    }
    let file = content_reader_politics::ideas::parse_ideas(
        b"ideas={country={SYN_ADVISOR={name=SYN_ADVISOR}}}",
        &[],
        &Default::default(),
    );
    let ideas = content_reader_politics::ideas::IdeaContent::from_parsed(Vec::new(), file.ideas);
    let names = content_reader_politics::names::NameLists::default();
    for (body, expected) in [
        ("fixture_gain_ideas={SYN_ADVISOR SYN_ADVISOR}", "Add First, First"),
        (
            "fixture_idea_details={SYN_ADVISOR SYN_ADVISOR}",
            "FirstSYN_ADVISOR, FirstSYN_ADVISOR",
        ),
    ] {
        let result = fixture.render(
            body,
            &mut fresh(),
            Content {
                ideas: Some(&ideas),
                name_lists: Some(&names),
                ..Default::default()
            },
        );
        assert!(result.refusals.is_empty(), "{:?}", result.refusals);
        assert_eq!(result.text, expected);
    }
}

#[test]
fn retired_fallback_folds_the_token_and_keeps_record_order() {
    game::enter();
    let (mut fixture, _folder) = fixture();
    let TableView::Complete(TableRows::AvailableAdvisors(rows)) =
        &mut *fixture.frame.tables[TableId::AvailableAdvisors.index()]
    else {
        unreachable!()
    };
    rows[0].idea = b"syn_advisor".to_vec();
    *fixture.frame.tables[TableId::CountryRetiredCharacters.index()] =
        TableView::Complete(TableRows::CountryRetiredCharacters(vec![
            CountryCharacterRow {
                country: 100,
                ordinal: 3,
                character: Some(20),
            },
            CountryCharacterRow {
                country: 100,
                ordinal: 9,
                character: Some(10),
            },
            CountryCharacterRow {
                country: 200,
                ordinal: 0,
                character: Some(10),
            },
        ]));
    roles(&mut fixture)[1].idea = b"syn_advisor".to_vec();
    let result = fixture.render(
        "fixture_gain_ideas={SYN_ADVISOR SYN_ADVISOR}",
        &mut fresh(),
        Content::default(),
    );
    assert_eq!(result.text, "Add Second, Second");
    assert!(result.refusals.is_empty());
    *fixture.frame.tables[TableId::CountryRetiredCharacters.index()] =
        TableView::Complete(TableRows::CountryRetiredCharacters(Vec::new()));
    let result = fixture.render("fixture_gain_ideas=SYN_ADVISOR", &mut fresh(), Content::default());
    assert!(result.text.is_empty());
    assert!(result.refusals.is_empty());
}

#[test]
fn cached_numbers_use_definition_ids_and_order_not_authored_modifiers() {
    game::enter();
    let (mut fixture, _folder) = fixture();
    let later = AdvisorRoleRow {
        ordinal: 4,
        modifier_cache: Recorded::Value(None),
        ..roles(&mut fixture)[0].clone()
    };
    roles(&mut fixture).insert(1, later);
    *fixture.frame.tables[TableId::LexicalTokens.index()] = TableView::Unavailable {
        reason: "synthetic unavailable".into(),
    };
    let TableView::Complete(TableRows::ModifierDefinitions(definitions)) =
        &mut *fixture.frame.tables[TableId::ModifierDefinitions.index()]
    else {
        unreachable!()
    };
    definitions[1].category_mask = 1 << 31;
    for (body, prefix) in [
        ("fixture_gain_ideas=SYN_ADVISOR", "Add "),
        ("fixture_lose_ideas=SYN_ADVISOR", "Remove "),
        ("fixture_idea_details=SYN_ADVISOR", ""),
    ] {
        let result = fixture.render(body, &mut fresh(), Content::default());
        assert_eq!(
            result.text,
            format!("{prefix}First:\nDefense: \u{11}G+2.00\u{11}!, Attack: \u{11}G+1.00\u{11}!\n")
        );
        assert_eq!(
            result.refusals.values().sum::<usize>(),
            1,
            "{:?}",
            result.refusals
        );
        assert!(
            result
                .refusals
                .keys()
                .next()
                .unwrap()
                .contains("tooltip keys of advisor roles not recorded")
        );
    }
}

#[test]
fn known_empty_numeric_cache_still_refuses_the_unrecorded_tooltip_keys() {
    game::enter();
    let (mut fixture, _folder) = fixture();
    roles(&mut fixture)[0].modifier_cache = Recorded::Value(Some(AdvisorModifierCache::default()));
    *fixture.frame.tables[TableId::ModifierDefinitions.index()] = TableView::Unavailable {
        reason: "unused with no numeric entries".into(),
    };
    let result = fixture.render(
        "fixture_idea_details=SYN_ADVISOR",
        &mut fresh(),
        Content::default(),
    );
    assert_eq!(result.text, "First:\n\n");
    assert_eq!(result.refusals.values().sum::<usize>(), 1);
    assert!(
        result
            .refusals
            .keys()
            .next()
            .unwrap()
            .contains("tooltip keys of advisor roles not recorded")
    );
}

#[test]
fn advisor_research_and_recorded_trait_targets_surround_the_stored_numbers() {
    game::enter();
    let (mut fixture, _folder) = fixture();
    roles(&mut fixture)[0].idea = b"SYN_BONUS_ADVISOR".to_vec();
    *fixture.frame.tables[TableId::AdvisorTokens.index()] =
        TableView::Complete(TableRows::AdvisorTokens(vec![AdvisorTokenRow {
            token: b"SYN_BONUS_ADVISOR".to_vec(),
            ..Default::default()
        }]));
    *fixture.frame.tables[TableId::AvailableAdvisors.index()] =
        TableView::Complete(TableRows::AvailableAdvisors(vec![AvailableAdvisorRow {
            country: 100,
            character: 10,
            idea: b"SYN_BONUS_ADVISOR".to_vec(),
            ..Default::default()
        }]));
    *fixture.frame.tables[TableId::CharacterRoleTraits.index()] =
        TableView::Complete(TableRows::CharacterRoleTraits(vec![
            CharacterRoleTraitRow {
                character: 10,
                kind: CharacterRoleTraitRow::ADVISOR,
                role: 0,
                tokens: vec![42],
                definitions: vec![1],
            },
        ]));
    let TableView::Complete(TableRows::LexicalTokens(tokens)) =
        &mut *fixture.frame.tables[TableId::LexicalTokens.index()]
    else {
        unreachable!()
    };
    tokens.push(LexicalTokenRow {
        token: 3,
        literal: b"political_power_gain".to_vec(),
    });
    let TableView::Complete(TableRows::ModifierDefinitions(definitions)) =
        &mut *fixture.frame.tables[TableId::ModifierDefinitions.index()]
    else {
        unreachable!()
    };
    definitions.push(ModifierDefinitionRow {
        index: 2,
        token: 3,
        ..definitions[0].clone()
    });
    let result = fixture.render(
        "fixture_idea_details=SYN_BONUS_ADVISOR",
        &mut fresh(),
        Content::default(),
    );
    assert_eq!(
        result.text,
        "First:\nIndustry: 10%, Defense: \u{11}G+2.00\u{11}!, Attack: \u{11}G+1.00\u{11}!, \u{12}BBBBeta Attack: \u{11}G+0.50\u{11}!\n"
    );
    assert_eq!(
        result.refusals.values().sum::<usize>(),
        1,
        "{:?}",
        result.refusals
    );
    assert!(
        result
            .refusals
            .keys()
            .next()
            .unwrap()
            .contains("tooltip keys of advisor roles not recorded")
    );
}

#[test]
fn unavailable_or_absent_cache_and_rosters_are_named_not_empty_blocks() {
    game::enter();
    let (mut fixture, _folder) = fixture();
    for (cache, reason) in [
        (
            Recorded::NotRecorded,
            "modifier cache of advisor roles not recorded",
        ),
        (
            Recorded::Value(None),
            "modifier cache of advisor roles unavailable",
        ),
    ] {
        roles(&mut fixture)[0].modifier_cache = cache;
        let result = fixture.render(
            "fixture_idea_details=SYN_ADVISOR",
            &mut fresh(),
            Content::default(),
        );
        assert!(result.text.is_empty());
        assert_eq!(result.refusals.values().sum::<usize>(), 1);
        assert!(result.refusals.keys().next().unwrap().contains(reason));
    }
    *fixture.frame.tables[TableId::AvailableAdvisors.index()] = TableView::Unavailable {
        reason: "synthetic unavailable".into(),
    };
    let result = fixture.render("fixture_gain_ideas=SYN_ADVISOR", &mut fresh(), Content::default());
    assert!(result.text.is_empty());
    assert_eq!(result.refusals.values().sum::<usize>(), 1);
    assert!(
        result
            .refusals
            .keys()
            .next()
            .unwrap()
            .contains("AvailableAdvisors")
    );
}

#[test]
fn format99_advisor_ideas_render_recorded_tooltips_in_slot_order() {
    game::enter();
    let (mut fixture, _folder) = fixture();
    fixture.catalogue.localisation = std::sync::Arc::new(
        fixture
            .catalogue
            .localisation
            .iter()
            .map(|(key, value)| (key.to_owned(), value.to_owned()))
            .chain([
                ("CACHE_FIRST".into(), "First tip".into()),
                ("CACHE_SECOND".into(), "Second tip".into()),
            ])
            .collect(),
    );
    std::sync::Arc::make_mut(&mut fixture.catalogue.localisation).language =
        fixture.catalogue.recorded_language.clone();
    for (keys, tips) in [
        (Some(Vec::new()), ""),
        (
            Some(vec![
                b"CACHE_SECOND".to_vec(),
                b"CACHE_FIRST\0ignored".to_vec(),
            ]),
            ", Second tip, First tip",
        ),
        (None, ""),
    ] {
        roles(&mut fixture)[0].tooltip_keys = Recorded::Value(keys);
        for (body, prefix) in [
            ("fixture_gain_ideas=SYN_ADVISOR", "Add "),
            ("fixture_lose_ideas=SYN_ADVISOR", "Remove "),
            ("fixture_idea_details=SYN_ADVISOR", ""),
        ] {
            let result = fixture.render(body, &mut fresh(), Content::default());
            assert_eq!(
                result.text,
                format!(
                    "{prefix}First:\nDefense: \u{11}G+2.00\u{11}!, Attack: \u{11}G+1.00\u{11}!{tips}\n"
                )
            );
            if matches!(roles(&mut fixture)[0].tooltip_keys, Recorded::Value(None)) {
                assert_eq!(result.refusals.values().sum::<usize>(), 1);
                assert!(
                    result
                        .refusals
                        .keys()
                        .next()
                        .unwrap()
                        .contains("tooltip keys of advisor roles capture unavailable")
                );
            } else {
                assert!(result.refusals.is_empty(), "{:?}", result.refusals);
            }
        }
    }
}

#[test]
fn a_pool_read_earlier_in_the_day_still_names_its_advisor() {
    game::enter();
    let (mut fixture, _folder) = fixture();
    let read_at = fixture.frame.hour - 23;
    *fixture.frame.tables[TableId::SliceReads.index()] =
        TableView::Complete(TableRows::SliceReads(vec![SliceReadRow {
            table: TableId::AvailableAdvisors.index() as u32,
            owner: 100,
            sample_hour: read_at,
        }]));
    *fixture.frame.tables[TableId::AdvisorRoles.index()] = TableView::Unavailable {
        reason: "synthetic unavailable".into(),
    };
    let ideas = content_reader_politics::ideas::IdeaContent::from_parsed(Vec::new(), Vec::new());
    let result = fixture.render(
        "fixture_gain_ideas={SYN_ADVISOR SYN_ADVISOR}",
        &mut fresh(),
        Content {
            ideas: Some(&ideas),
            ..Default::default()
        },
    );
    assert!(result.refusals.is_empty(), "{:?}", result.refusals);
    assert_eq!(result.text, "Add First, First");
}
