use crate::test_fixtures::{CAMPAIGN, ownership_frame, records};
use replay::{
    container::Replay,
    frame::{DecodedFrame, TableView},
    rows::{TableId, TableRows},
};
use std::sync::atomic::AtomicBool;

#[test]
fn artwork_problems_reach_the_reply_once_and_an_unidentified_holder_is_unknown() {
    use super::{IdeaCharacter, IdeaSpan, ScannedIdeas, reply};
    use crate::products::{CountryParams, politics_content::PoliticsContent};
    let (mut catalogue, _folder) = crate::catalogue::tests::folder_catalogue(&[
        (
            "common/defines/mod.lua",
            "NDefines = { NCharacter = { OFFICER_CORP_ADVISOR_ENTRIES_IN_MENU = { \"mod_officers\" } } }",
        ),
        (
            "common/ideas/mod.txt",
            "ideas = { mod_group = { mod_idea = { } } }",
        ),
        (
            "interface/mod.gfx",
            "spriteTypes = { spriteType = { name = GFX_idea_mod_idea noOfFrames = -2 } spriteType = { name = GFX_unused noOfFrames = -3 } }",
        ),
    ]);
    catalogue
        .warnings
        .push("unused catalogue sprite refusals: GFX_unused".into());
    let content = PoliticsContent::load(std::sync::Arc::new(catalogue)).unwrap();
    let replay =
        Replay::from_records(CAMPAIGN.into(), &[records(1, vec![idea_frame(100, &[])])]).unwrap();
    let params = CountryParams {
        branch_id: "1".into(),
        tag: "GER".into(),
    };
    let mut scanned = ScannedIdeas {
        spans: vec![IdeaSpan {
            token: "mod_idea".into(),
            first_hour: 100,
            last_hour: 100,
            slot: None,
            character: None,
        }],
        ..Default::default()
    };
    let shown = reply(
        &content,
        &replay,
        &params,
        &scanned,
        &AtomicBool::new(false),
    )
    .unwrap();
    assert_eq!(shown.artwork[0].icon, None);
    let problems: Vec<_> = shown
        .warnings
        .iter()
        .filter(|warning| warning.contains("GFX_idea_mod_idea"))
        .collect();
    assert_eq!(problems.len(), 1, "{:?}", shown.warnings);
    assert!(problems[0].contains("mod.gfx"));
    assert!(shown.warnings.iter().all(
        |warning| !warning.contains("GFX_unused") && !warning.contains("ideas have no artwork")
    ));
    scanned.spans[0].character = Some(IdeaCharacter {
        id: 7,
        token: String::new(),
        name: String::new(),
    });
    let shown = reply(
        &content,
        &replay,
        &params,
        &scanned,
        &AtomicBool::new(false),
    )
    .unwrap();
    assert_eq!(shown.artwork[0].icon, None);
    assert!(
        shown
            .warnings
            .iter()
            .any(|warning| warning.contains("artwork is unknown")
                && warning.contains("no recorded script token"))
    );
    assert!(
        shown
            .warnings
            .iter()
            .all(|warning| !warning.contains("no artwork in the recorded content"))
    );
}

/// GER (link 9001) holding the given ideas. `GER_schacht_idea` is an appointed
/// advisor: like the game, it is in the appointed-advisor list, not the active ideas.
fn idea_frame(hour: i32, ideas: &[&str]) -> DecodedFrame {
    let mut frame = ownership_frame(1, hour, 44);
    let advisor = |country, ordinal, character, idea: &str| replay::rows::HiredAdvisorRow {
        country,
        ordinal,
        character,
        idea: idea.as_bytes().to_vec(),
        slot: b"political_advisor".to_vec(),
    };
    let mut hired = vec![advisor(44, 0, 9, "FRA_advisor")];
    let mut rows: Vec<_> = ideas
        .iter()
        .filter(|token| {
            let appointed = **token == "GER_schacht_idea";
            if appointed {
                hired.push(advisor(9001, 0, 7, token));
            }
            !appointed
        })
        .enumerate()
        .map(|(ordinal, token)| replay::rows::ActiveIdeaRow {
            country: 9001,
            ordinal: ordinal as u32,
            token: token.as_bytes().to_vec(),
        })
        // Another country's idea is never GER's.
        .chain([replay::rows::ActiveIdeaRow {
            country: 44,
            ordinal: 0,
            token: b"FRA_spirit".to_vec(),
        }])
        .collect();
    rows.sort_by_key(|row| (row.country, row.ordinal));
    hired.sort_by_key(|row| (row.country, row.ordinal));
    *frame.tables[TableId::ActiveIdeas.index()] = TableView::Complete(TableRows::ActiveIdeas(rows));
    *frame.tables[TableId::HiredAdvisors.index()] =
        TableView::Complete(TableRows::HiredAdvisors(hired));
    // As recorded: the saved name in `token`, the script token as a lexer id.
    *frame.tables[TableId::Characters.index()] =
        TableView::Complete(TableRows::Characters(vec![replay::rows::CharacterRow {
            id: 7,
            country: 9001,
            name_token_id: 42,
            token: b"Hjalmar Schacht".to_vec(),
            ..Default::default()
        }]));
    *frame.tables[TableId::LexicalTokens.index()] =
        TableView::Complete(TableRows::LexicalTokens(vec![
            replay::rows::LexicalTokenRow {
                token: 42,
                literal: b"GER_schacht".to_vec(),
            },
        ]));
    frame
}

#[test]
fn idea_history_spans_break_at_gaps_and_advisors_keep_their_recorded_role() {
    use crate::products::idea_history::{IdeaCharacter, IdeaSpan, scan};
    let span = |token: &str, first_hour, last_hour| IdeaSpan {
        token: token.into(),
        first_hour,
        last_hour,
        slot: None,
        character: None,
    };
    // The recorded role goes with the advisor idea's span.
    let advisor = |first_hour, last_hour| IdeaSpan {
        slot: Some("political_advisor".into()),
        character: Some(IdeaCharacter {
            id: 7,
            token: "GER_schacht".into(),
            name: "Hjalmar Schacht".into(),
        }),
        ..span("GER_schacht_idea", first_hour, last_hour)
    };
    let frames = vec![
        idea_frame(100, &["civilian_economy", "GER_spirit"]),
        idea_frame(101, &["civilian_economy", "GER_spirit", "GER_schacht_idea"]),
        idea_frame(
            102,
            &[
                "partial_economic_mobilisation",
                "GER_spirit",
                "GER_schacht_idea",
            ],
        ),
        // Hour 103 is missing: every span ends at 102.
        idea_frame(104, &["partial_economic_mobilisation"]),
    ];
    let replay = Replay::from_records(CAMPAIGN.into(), &[records(1, frames)]).unwrap();
    let scanned = scan(&replay, 1, "GER", &AtomicBool::new(false)).unwrap();
    assert_eq!(
        scanned.spans,
        [
            span("GER_spirit", 100, 102),
            span("civilian_economy", 100, 101),
            advisor(101, 102),
            span("partial_economic_mobilisation", 102, 102),
            span("partial_economic_mobilisation", 104, 104),
        ]
    );
    assert_eq!(
        scanned.unobserved,
        [crate::products::country_groups::HourSpan {
            first_hour: 103,
            last_hour: 103
        }]
    );
    // The identity starts at its first hour and again after the missing hour.
    assert_eq!(scanned.identity_starts, [100, 104]);
    assert!(
        scan(&replay, 1, "FRA", &AtomicBool::new(false))
            .unwrap()
            .spans
            .iter()
            .all(|span| span.token == "FRA_spirit" || span.token == "FRA_advisor")
    );
}

#[test]
fn advisor_traits_select_the_first_native_name_match() {
    use crate::products::idea_history::{Joined, prepare_info};
    let (catalogue, _folder) = crate::catalogue::tests::folder_catalogue(&[
        (
            "common/country_leader/a.txt",
            "leader_traits = { Brilliant = { army_morale_factor = 0.1 } brilliant = { army_morale_factor = 0.2 } \
             1trait = { army_morale_factor = 0.3 } 2trait = { army_morale_factor = 0.4 } }",
        ),
        (
            "common/country_leader/b.txt",
            "leader_traits = { BRILLIANT = { army_morale_factor = 0.5 } lost = { odd = { } } }",
        ),
        (
            "common/country_leader/c.txt",
            "leader_traits = { Later = { army_morale_factor = 0.6 } }",
        ),
        (
            "localisation/traits_l_english.yml",
            "\u{feff}l_english:\n Brilliant:0 \"First trait\"\n bRiLLiant:0 \"Reference spelling\"\n",
        ),
    ]);
    let (traits, _) = installed_content::leader_traits::read(&catalogue.content.layers).unwrap();
    let characters = installed_content::characters::parse(
        b"characters = { holder = { advisor = { slot = political_advisor idea_token = advisor traits = { bRiLLiant 1trait 2trait Later Missing } } } }",
        &installed_content::key_names::KeyNames::default(),
    )
    .unwrap();
    let ideas = installed_content::ideas::IdeaContent::default();
    let localise = |key: &str, view| catalogue.localised(key, view);
    let joined = Joined {
        ideas: &ideas,
        characters: &characters,
        traits: &traits,
        localise: &localise,
    };
    let (advisor, findings) = prepare_info(&joined, "advisor", None, Err("no holder"), false);
    // 1412894E0 -> 140712D20 -> 140710200 keeps the first name match.
    // The following unread suffix cannot change that match, but can hide
    // a match before Later or define Missing.
    let first = &advisor.traits[0];
    assert!(first.known);
    assert_eq!(first.unknown, None);
    assert_eq!(first.lines[0].fixed5, "10000");
    assert_eq!(first.name.as_deref(), Some("First trait"));
    // Names rejected by the lexer are still distinct hashed-name lookups.
    assert_eq!(advisor.traits[1].lines[0].fixed5, "30000");
    assert_eq!(advisor.traits[2].lines[0].fixed5, "40000");
    for unknown in &advisor.traits[3..] {
        assert!(!unknown.known);
        assert_eq!(unknown.name, None);
        assert!(unknown.unknown.as_deref().unwrap().contains("b.txt"));
        assert!(findings.iter().any(|finding| {
            finding.kind == crate::content_check::Kind::Refused
                && finding.subject == format!("idea advisor trait {}", unknown.token)
                && Some(finding.message.as_str()) == unknown.unknown.as_deref()
        }));
    }
}

#[test]
fn ideas_join_content_for_category_slot_and_traits() {
    use crate::{
        content_check::Kind,
        products::idea_history::{Joined, categories, prepare_info},
    };
    let names = installed_content::key_names::KeyNames::default();
    let tags = installed_content::ideas::parse_tags(
        b"idea_categories = { government = { slot = economy character_slot = political_advisor character_slot = political_advisor }           country = { type = national_spirit } military_staff = { character_slot = theorist } }",
        &names,
    );
    // An officer-corps group's entries are character advisor templates
    let file = installed_content::ideas::parse_ideas(
        b"ideas = { economy = { law = yes civilian_economy = { modifier = { consumer_goods_factor = 0.35 } } }           country = { GER_spirit = { modifier = { STABILITY_FACTOR = 0.1 hidden_modifier = { Stability_Factor = 0.2 } } } }           theorist = { GER_theorist = { traits = { military_theorist } modifier = { army_org = 1 } } } }",
        &["theorist".to_owned()],
        &names,
    );
    let content = installed_content::ideas::IdeaContent::from_parsed(tags, file.ideas);
    // The advisor's seat and traits come from its character's advisor role.
    let mut characters = installed_content::characters::parse(
        b"characters = { GER_schacht = { advisor = { slot = political_advisor idea_token = GER_schacht_idea traits = { captain_of_industry } } }           GER_split = { instance = { allowed = { has_dlc = \"A\" } advisor = { slot = political_advisor idea_token = GER_split_idea traits = { a } } }             instance = { allowed = { NOT = { has_dlc = \"A\" } } advisor = { slot = political_advisor idea_token = GER_split_idea traits = { b } } } } }",
        &names,
    )
    .unwrap();
    characters.merge(file.advisors);
    let traits = installed_content::leader_traits::LeaderTraits::default();
    let localise =
        |key: &str, _: crate::catalogue::View| (key == "economy").then(|| "Economy".to_owned());
    let joined = Joined {
        ideas: &content,
        characters: &characters,
        traits: &traits,
        localise: &localise,
    };
    const NO_HOLDER: Result<&installed_content::characters::BuiltTemplate, &str> = Err("no holder");
    let (law, findings) = prepare_info(&joined, "civilian_economy", None, NO_HOLDER, false);
    assert!(findings.is_empty(), "{findings:?}");
    assert_eq!(
        (law.category.as_deref(), law.slot.as_deref(), law.law),
        (Some("government"), Some("economy"), true)
    );
    assert_eq!(law.effects[0].fixed5, "35000");
    let (spirit, findings) = prepare_info(&joined, "GER_spirit", None, NO_HOLDER, false);
    assert!(findings.is_empty(), "{findings:?}");
    assert_eq!(
        (spirit.category.as_deref(), spirit.slot),
        (Some("country"), None)
    );
    // One line per modifier, by the identity its folded key names
    // (1424A7A90), the last value kept and hidden by `hidden_modifier`.
    assert_eq!(
        spirit.effects,
        [crate::products::idea_history::EffectLine {
            key: "stability_factor".into(),
            fixed5: "20000".into(),
            hidden: true,
        }]
    );
    let (advisor, findings) = prepare_info(&joined, "GER_schacht_idea", None, NO_HOLDER, false);
    assert_eq!(findings.len(), 1);
    assert_eq!(findings[0].kind, Kind::Fallback);
    assert_eq!(
        findings[0].subject,
        "idea GER_schacht_idea trait captain_of_industry"
    );
    assert_eq!(
        (advisor.category.as_deref(), advisor.slot.as_deref()),
        (Some("government"), Some("political_advisor"))
    );
    assert_eq!(advisor.traits[0].token, "captain_of_industry");
    assert_eq!(content.idea("GER_theorist"), Ok(None));
    let (theorist, findings) = prepare_info(&joined, "GER_theorist", None, NO_HOLDER, false);
    assert_eq!(findings.len(), 1);
    assert_eq!(findings[0].kind, Kind::Fallback);
    assert_eq!(
        findings[0].subject,
        "idea GER_theorist trait military_theorist"
    );
    assert_eq!(
        (theorist.category.as_deref(), theorist.slot.as_deref()),
        (Some("military_staff"), Some("theorist"))
    );
    // The officer template keeps its role's `modifier` (1413CA970).
    assert!(theorist.effects_known);
    assert_eq!(
        (
            theorist.effects[0].key.as_str(),
            theorist.effects[0].fixed5.as_str()
        ),
        ("army_org", "100000")
    );
    assert_eq!(theorist.traits[0].token, "military_theorist");
    // Content has no such trait: its effects are unknown, not none.
    assert!(!advisor.traits[0].known);
    // Instances that disagree without a holder's template: the reason says
    // so rather than "not in the content files".
    let (split, findings) = prepare_info(
        &joined,
        "GER_split_idea",
        Some("political_advisor"),
        NO_HOLDER,
        false,
    );
    assert_eq!(findings.len(), 1);
    assert_eq!(findings[0].kind, Kind::Unsupported);
    assert_eq!(findings[0].subject, "idea GER_split_idea");
    assert_eq!(Some(findings[0].message.as_str()), split.unknown.as_deref());
    assert!(!split.effects_known);
    assert!(
        split
            .unknown
            .as_deref()
            .is_some_and(|reason| reason.starts_with(
                "advisor GER_split_idea: its character's alternatives define it differently"
            ) && reason.ends_with("no holder")),
        "{:?}",
        split.unknown
    );
    let (unknown, findings) = prepare_info(&joined, "MOD_mystery", None, NO_HOLDER, false);
    assert_eq!(findings.len(), 1);
    assert_eq!(findings[0].kind, Kind::Refused);
    assert_eq!(findings[0].subject, "idea MOD_mystery");
    assert_eq!((unknown.category, unknown.slot), (None, None));
    // An advisor content cannot resolve is placed by the seat the recording gave it.
    let (recorded, findings) = prepare_info(
        &joined,
        "MOD_mystery",
        Some("political_advisor"),
        NO_HOLDER,
        false,
    );
    assert!(findings.is_empty(), "{findings:?}");
    assert_eq!(
        (recorded.category.as_deref(), recorded.slot.as_deref()),
        (Some("government"), Some("political_advisor"))
    );
    let listed = categories(&joined);
    let government = &listed[0];
    assert_eq!(
        government
            .slots
            .iter()
            .map(|slot| (slot.token.as_str(), slot.character, slot.seats))
            .collect::<Vec<_>>(),
        [("economy", false, 1), ("political_advisor", true, 2)]
    );
    assert_eq!(government.slots[0].name.as_deref(), Some("Economy"));
    assert_eq!(government.slots[0].laws, ["civilian_economy"]);
    assert!(government.slots[1].laws.is_empty());
}

#[test]
fn idea_history_marks_gaps_in_order_and_to_the_branch_end() {
    use crate::products::{country_groups::HourSpan, idea_history::scan};
    let mut incomplete = idea_frame(103, &["GER_spirit"]);
    *incomplete.tables[TableId::ActiveIdeas.index()] = TableView::Unavailable {
        reason: "fixture".into(),
    };
    // Without the appointed advisors an hour is not observed either.
    let mut no_advisors = idea_frame(104, &["GER_spirit"]);
    *no_advisors.tables[TableId::HiredAdvisors.index()] = TableView::Unavailable {
        reason: "fixture".into(),
    };
    // Nor when the country exists but its identity cannot be read.
    let mut unmapped = idea_frame(105, &["GER_spirit"]);
    *unmapped.tables[TableId::CountryLinks.index()] =
        TableView::Complete(TableRows::CountryLinks(vec![]));
    let frames = vec![
        idea_frame(100, &["GER_spirit"]),
        // Hours 101 and 102 are missing, 103's ideas and 104's advisors are incomplete.
        incomplete,
        no_advisors,
        unmapped,
        idea_frame(106, &["GER_spirit"]),
    ];
    let mut replay = Replay::from_records(CAMPAIGN.into(), &[records(1, frames)]).unwrap();
    // The branch ends in a recorded gap: those hours are unobserved, not a removal.
    replay.gaps.push(replay::branches::Gap {
        branch: 1,
        first_hour: 107,
        last_hour: 109,
        reason: "fixture".into(),
    });
    let scanned = scan(&replay, 1, "GER", &AtomicBool::new(false)).unwrap();
    let span = |first_hour, last_hour| HourSpan {
        first_hour,
        last_hour,
    };
    assert_eq!(scanned.unobserved, [span(101, 105), span(107, 109)]);
    assert_eq!(scanned.identity_starts, [100, 106]);
}
