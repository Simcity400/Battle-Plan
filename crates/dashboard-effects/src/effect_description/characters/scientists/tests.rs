use super::*;
use crate::fixtures::{Folder, folder_catalogue};
use crate::effect_description::tests::Fixture;
use crate::scripted_text::Link;
use replay_data::rows::*;
use replay_frame::frame::TableView;
use crate::fixtures::game;

fn fixture() -> (Fixture, Folder) {
    let mut fixture = Fixture::new();
    let (catalogue, folder) = folder_catalogue(&[
        (
            "common/defines/test.lua",
            "NDefines={NGame={MAX_SCRIPTED_LOC_RECURSION=23}, NCharacter={OFFICER_CORP_ADVISOR_ENTRIES_IN_MENU={}}}",
        ),
        (
            "common/special_projects/specialization/test.txt",
            "specialization_synthetic_energy={} specialization_synthetic_lift={}",
        ),
        (
            "common/scientist_traits/test.txt",
            concat!(
                "syn_observant={name=BRIGHT_NAME} ",
                "syn_practiced={modifier={custom_modifier_tooltip=TRAIT_TOOLTIP}} ",
                "syn_scoped={modifier={research_speed_factor=0.25 custom_modifier_tooltip=TRAIT_TOOLTIP}}"
            ),
        ),
        (
            "localisation/test_l_english.yml",
            concat!(
                "\u{feff}l_english:\n AAA:0 \"Alpha\"\n BBB:0 \"Beta\"\n",
                " specialization_synthetic_energy:0 \"Physics\"\n AAA_specialization_synthetic_energy:0 \"Alpha physics\"\n specialization_synthetic_lift:0 \"Air\"\n",
                " SCIENTIST_ADD_XP_SPECIALIZATION_EFFECT:0 \"XP$XP|0$/$SPECIALIZATION$ [Root.GetTag]\"\n",
                " SCIENTIST_ADD_LEVEL_SPECIALIZATION_EFFECT:0 \"Level$LEVELS|0$/$SPECIALIZATION$ [Root.GetTag]\"\n",
                " SPECIAL_PROJECT_INJURE_SCIENTIST_FOR_DAYS:0 \"Injury$DAYS|0$/[Root.GetName]/[From.GetName]/[Prev.GetName]\"\n",
                " SCIENTIST_ADD_TRAIT_EFFECT:0 \"Trait$TRAIT_NAME$ [Root.GetTag]\"\n",
                " EFFECT_GENERATE_SCIENTIST_CHARACTER:0 \"Generate$TRAITS$/[Root.GetTag]\"\n",
                " EFFECT_GENERATE_SCIENTIST_CHARACTER_TRAIT:0 \"$TRAIT_NAME$\"\n",
                " EFFECT_REMOVE_SCIENTIST_ROLE:0 \"Remove $NAME$/[Root.GetTag]\"\n",
                " EFFECT_ADD_SCIENTIST_ROLE:0 \"Add $CHARACTER$/[Root.GetTag]\"\n",
                " BRIGHT_NAME:0 \"Bright\"\n syn_practiced:0 \"Quick\"\n TRAIT_TOOLTIP:0 \"Tooltip[Root.GetTag]\"\n",
                " syn_scoped:0 \"Scope\"\n SCIENTIST_SCOPE:0 \"Research [Root.GetTag]\\nFrom [From.GetTag]\"\n"
            ),
        ),
    ]);
    catalogue
        .scripted_content()
        .unwrap()
        .set_synthetic_environment(Default::default());
    fixture.catalogue = catalogue;
    *fixture.frame.tables[TableId::Characters.index()] =
        TableView::Complete(TableRows::Characters(vec![
            CharacterRow {
                id: 7,
                country: 100,
                name: b"Alice".to_vec(),
                name_token_id: 10,
                ..Default::default()
            },
            CharacterRow {
                id: 8,
                country: 100,
                name: b"Ben".to_vec(),
                name_token_id: 11,
                ..Default::default()
            },
        ]));
    *fixture.frame.tables[TableId::CountryCharacters.index()] =
        TableView::Complete(TableRows::CountryCharacters(vec![
            CountryCharacterRow {
                country: 100,
                ordinal: 0,
                character: Some(7),
            },
            CountryCharacterRow {
                country: 100,
                ordinal: 1,
                character: Some(8),
            },
        ]));
    *fixture.frame.tables[TableId::Scientists.index()] =
        TableView::Complete(TableRows::Scientists(vec![ScientistRow {
            country: 100,
            character: 7,
            ..Default::default()
        }]));
    *fixture.frame.tables[TableId::LexicalTokens.index()] =
        TableView::Complete(TableRows::LexicalTokens(vec![
            LexicalTokenRow {
                token: 10,
                literal: b"synthetic_alice".to_vec(),
            },
            LexicalTokenRow {
                token: 11,
                literal: b"synthetic_ben".to_vec(),
            },
        ]));
    crate::effect_description::characters::tests::empty_variable_stores(&mut fixture);
    (fixture, folder)
}

fn caller() -> EventScopes {
    let mut scientist = EventScope::of(EventObject::Character(7));
    scientist.root = Link::Scope(1);
    scientist.from = Link::Scope(2);
    scientist.prev = Link::Scope(2);
    EventScopes::new(vec![
        scientist,
        EventScope::of(EventObject::Country(1)),
        EventScope::of(EventObject::Country(2)),
    ])
}

#[test]
fn scientist_trait_uses_full_category_mask_and_caller_scope() {
    game::enter();
    let (mut fixture, _folder) = fixture();
    let TableView::Complete(TableRows::LexicalTokens(tokens)) =
        &mut *fixture.frame.tables[TableId::LexicalTokens.index()]
    else {
        unreachable!()
    };
    tokens.push(LexicalTokenRow {
        token: 31,
        literal: b"research_speed_factor".to_vec(),
    });
    *fixture.frame.tables[TableId::ModifierDefinitions.index()] =
        TableView::Complete(TableRows::ModifierDefinitions(vec![
            ModifierDefinitionRow {
                token: 31,
                category_mask: 1 << 31,
                flags: crate::modifier_text::POSITIVE_IS_GOOD,
                precision: 2,
                name_key: b"SCIENTIST_SCOPE".to_vec(),
                ..Default::default()
            },
        ]));
    let out = fixture.render(
        "fixture_researcher_trait=syn_scoped",
        &mut caller(),
        Content::default(),
    );
    assert_eq!(
        out.text,
        "TraitScope [Root.GetTag]\n  Research AAA\n  From BBB: \u{11}G+0.25\u{11}!\n  TooltipAAA"
    );
    assert!(out.refusals.is_empty(), "{:?}", out.refusals);
    fixture.with_renderer(&mut caller(), Content::default(), |renderer| {
        let scopes = renderer.evaluator().unwrap().event_scopes(0).unwrap();
        let traits = renderer
            .character_content()
            .unwrap()
            .scientist_traits
            .as_ref()
            .unwrap();
        let modifiers = &traits.get("syn_scoped").unwrap().unwrap().modifiers;
        assert_eq!(
            renderer
                .modifier_list_with(modifiers, "", scopes, ModifierListOptions::default())
                .unwrap(),
            "TooltipAAA"
        );
    });
}

#[test]
fn active_non_scientists_keep_empty_gates() {
    game::enter();
    let (fixture, _folder) = fixture();
    let mut scopes = caller();
    scopes.scopes[0].this = EventObject::Character(8);
    let out = fixture.render("fixture_researcher_experience={specialization=specialization_synthetic_energy experience=275} fixture_researcher_level={specialization=specialization_synthetic_lift level=4} fixture_researcher_recovery=13 fixture_researcher_trait=syn_practiced", &mut scopes, Content::default());
    assert_eq!(out.text, "");
    assert!(out.refusals.is_empty(), "{:?}", out.refusals);
}

#[test]
fn unrostered_scientist_presence_is_named_and_counted() {
    game::enter();
    let (mut fixture, _folder) = fixture();
    *fixture.frame.tables[TableId::Scientists.index()] =
        TableView::Complete(TableRows::Scientists(vec![]));
    *fixture.frame.tables[TableId::CountryCharacters.index()] =
        TableView::Complete(TableRows::CountryCharacters(vec![]));
    *fixture.frame.tables[TableId::CountryRetiredCharacters.index()] =
        TableView::Complete(TableRows::CountryRetiredCharacters(vec![
            CountryCharacterRow {
                country: 100,
                ordinal: 0,
                character: Some(7),
            },
        ]));
    let out = fixture.render(
        "fixture_researcher_recovery=13",
        &mut caller(),
        Content::default(),
    );
    assert_eq!(out.text, "");
    assert_eq!(out.refusals, BTreeMap::from([("synthetic effect fixture_researcher_recovery: key fixture_researcher_recovery: unrostered character 7 scientist holder is unrecorded".to_owned(), 1)]));
}

#[test]
fn scientist_role_targets_use_named_character_and_current_fallback() {
    game::enter();
    let (fixture, _folder) = fixture();
    let out = fixture.render("fixture_assign_researcher={character=synthetic_ben scientist={}} fixture_dismiss_researcher={character=synthetic_ben}", &mut caller(), Content::default());
    assert_eq!(out.text, "Add Ben/AAA\nRemove Ben/AAA");
    assert!(out.refusals.is_empty(), "{:?}", out.refusals);
}

#[test]
fn missing_and_reserved_trait_ids_keep_empty() {
    game::enter();
    let (fixture, _folder) = fixture();
    let out = fixture.render("fixture_researcher_trait=missing_trait fixture_researcher_trait=\"syn_observant\" fixture_researcher_trait=3 fixture_researcher_experience={specialization=missing_specialization experience=275}", &mut caller(), Content::default());
    assert_eq!(out.text, "");
    assert!(out.refusals.is_empty(), "{:?}", out.refusals);
}
