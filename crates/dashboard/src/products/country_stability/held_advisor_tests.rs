use crate::{
    catalogue::View,
    identity::built_template,
    products::{
        country_stability::{BreakdownLine, RecordedCharacter, line_group},
        idea_history::{Joined, prepare_info},
        politics_content::PoliticsContent,
    },
};
use replay::rows::{CharacterRow, LexicalTokenRow};
use std::sync::Arc;

/// An advisor role the character's instances disagree on is the role
/// of the template its recorded holder was built from: the Characters row's
/// file, line and tag token, read through the Politics content loader.
/// Both keys and dynamic tag tokens use folded lookup.
#[test]
fn a_disagreeing_advisor_is_its_holders_template() {
    engine::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
    let (catalogue, _folder) = crate::catalogue::tests::folder_catalogue(&[
        (
            "common/defines/00_defines.lua",
            "NDefines = { NCharacter = { OFFICER_CORP_ADVISOR_ENTRIES_IN_MENU = { \"theorist\" } } }",
        ),
        (
            "common/idea_tags/00.txt",
            "idea_categories = { government = { character_slot = political_advisor } }",
        ),
        (
            "common/characters/mod.txt",
            "characters = {\n\
             \tMOD_split = {\n\
             \t\tInstance = {\n\
             \t\t\tALLOWED = { has_dlc = \"A\" }\n\
             \t\t\tadvisor = { slot = political_advisor idea_token = MOD_split_idea traits = { a_trait } }\n\
             \t\t}\n\
             \t\tinstance = {\n\
             \t\t\tallowed = { NOT = { has_dlc = \"A\" } }\n\
             \t\t\tADVISOR = { Slot = political_advisor Idea_Token = MOD_split_idea Traits = { b_trait } }\n\
             \t\t}\n\
             \t}\n\
             }\n",
        ),
    ]);
    let content = PoliticsContent::load(Arc::new(catalogue)).unwrap();
    let lexical = [LexicalTokenRow {
        token: 40,
        literal: b"mod_split".to_vec(),
    }];
    let row = |line| CharacterRow {
        id: 7,
        template_present: 1,
        template_tag: 40,
        template_file: b"common/characters/mod.txt".to_vec(),
        template_line: line,
        ..Default::default()
    };
    let localise = |key: &str, view: View| content.localised(key, view);
    let joined = Joined {
        ideas: &content.ideas,
        characters: &content.characters,
        traits: &content.traits,
        localise: &localise,
    };
    for (line, trait_token) in [(4, "a_trait"), (8, "b_trait")] {
        let built = built_template(&row(line), Some(&lexical)).unwrap();
        let (found, findings) = prepare_info(
            &joined,
            "MOD_split_idea",
            Some("political_advisor"),
            Ok(&built),
            false,
        );
        assert!(found.effects_known, "{:?}", found.unknown);
        assert_eq!(found.unknown, None);
        assert_eq!(found.traits[0].token, trait_token);
        assert!(findings.iter().any(|finding| {
            finding.kind == crate::content_check::Kind::Fallback
                && finding.subject == format!("idea MOD_split_idea trait {trait_token}")
        }));
    }
    // What the hour does not hold is refused by name.
    let unspelled = built_template(&row(8), Some(&[])).unwrap_err();
    assert_eq!(
        unspelled,
        "character 7's template tag token 40 has no LexicalTokens row"
    );
    let absent = built_template(
        &CharacterRow {
            template_present: 0,
            ..row(8)
        },
        Some(&lexical),
    )
    .unwrap_err();
    assert!(absent.contains("recorded template is null"), "{absent}");
    let (found, findings) = prepare_info(
        &joined,
        "MOD_split_idea",
        Some("political_advisor"),
        Err(&unspelled),
        false,
    );
    assert!(!found.effects_known);
    assert!(findings.iter().any(|finding| {
        finding.kind == crate::content_check::Kind::Unsupported
            && Some(finding.message.as_str()) == found.unknown.as_deref()
    }));
    assert!(
        found
            .unknown
            .as_deref()
            .is_some_and(|reason| reason.ends_with(&unspelled)),
        "{:?}",
        found.unknown
    );
    // The stability breakdown groups the line by the same role.
    let line = BreakdownLine {
        line: 0,
        kind: "advisor".into(),
        identity: "MOD_split_idea".into(),
        identity_id: 7,
        secondary_raw: String::new(),
        value_raw: String::new(),
        text: None,
        label: None,
        group: None,
        group_kind: None,
    };
    let holder = RecordedCharacter {
        script: Some("MOD_split"),
        saved: "",
        template: built_template(&row(8), Some(&lexical)),
    };
    assert_eq!(line_group(&line, &content, Some(&holder)), (None, None));
    assert_eq!(line_group(&line, &content, None), (None, None));
    assert!(
        content
            .catalogue
            .refusal_lines()
            .iter()
            .any(|line| line.contains("not recorded at its read")),
        "{:?}",
        content.catalogue.refusal_lines()
    );
}
