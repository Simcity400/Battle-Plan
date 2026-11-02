use crate::test_fixtures::{CAMPAIGN, ownership_frame, records};
use replay::container::Replay;
use replay_data::{rows::TableId, rows::TableRows};
use replay_frame::{frame::DecodedFrame, frame::TableView};
use std::sync::atomic::AtomicBool;

#[test]
fn artwork_problems_reach_the_reply_once_and_an_unidentified_holder_is_unknown() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
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
pub(super) fn idea_frame(hour: i32, ideas: &[&str]) -> DecodedFrame {
    let mut frame = ownership_frame(1, hour, 44);
    let advisor = |country, ordinal, character, idea: &str| replay_data::rows::HiredAdvisorRow {
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
        .map(|(ordinal, token)| replay_data::rows::ActiveIdeaRow {
            country: 9001,
            ordinal: ordinal as u32,
            token: token.as_bytes().to_vec(),
        })
        // Another country's idea is never GER's.
        .chain([replay_data::rows::ActiveIdeaRow {
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
    *frame.tables[TableId::Characters.index()] = TableView::Complete(TableRows::Characters(vec![
        replay_data::rows::CharacterRow {
            id: 7,
            country: 9001,
            name_token_id: 42,
            token: b"Hjalmar Schacht".to_vec(),
            ..Default::default()
        },
    ]));
    *frame.tables[TableId::LexicalTokens.index()] =
        TableView::Complete(TableRows::LexicalTokens(vec![
            replay_data::rows::LexicalTokenRow {
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
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
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
    let (traits, _) = installed_content_core::leader_traits::read(&catalogue.content.layers).unwrap();
    let characters = installed_content_core::characters::parse(
        b"characters = { holder = { advisor = { slot = political_advisor idea_token = advisor traits = { bRiLLiant 1trait 2trait Later Missing } } } }",
        &installed_content_core::key_names::KeyNames::default(),
    )
    .unwrap();
    let ideas = installed_content_core::ideas::IdeaContent::default();
    let localise = |key: &str, view| catalogue.localised(key, view);
    let joined = Joined {
        ideas: &ideas,
        characters: &characters,
        traits: &traits,
        localise: &localise,
    };
    let (advisor, findings) = prepare_info(&joined, "advisor", None, Err("no holder"), false);
    // The first name match wins. The following unread suffix cannot change
    // that match, but can hide a match before Later or define Missing.
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
            finding.kind == crate::findings::Kind::Refused
                && finding.subject == format!("idea advisor trait {}", unknown.token)
                && Some(finding.message.as_str()) == unknown.unknown.as_deref()
        }));
    }
}

#[test]
fn ideas_join_content_for_category_slot_and_traits() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
    use crate::{
        findings::Kind,
        products::idea_history::{Joined, categories, prepare_info},
    };
    let names = installed_content_core::key_names::KeyNames::default();
    let tags = installed_content_core::ideas::parse_tags(
        b"idea_categories = { government = { slot = economy character_slot = political_advisor character_slot = political_advisor }           country = { type = national_spirit } military_staff = { character_slot = theorist } }",
        &names,
    );
    // An officer-corps group's entries are character advisor templates,
    // not ideas.
    let file = installed_content_core::ideas::parse_ideas(
        b"ideas = { economy = { law = yes civilian_economy = { modifier = { consumer_goods_factor = 0.35 } } }           country = { GER_spirit = { modifier = { STABILITY_FACTOR = 0.1 hidden_modifier = { Stability_Factor = 0.2 } } } }           theorist = { GER_theorist = { traits = { military_theorist } modifier = { army_org = 1 } } } }",
        &["theorist".to_owned()],
        &names,
    );
    let content = installed_content_core::ideas::IdeaContent::from_parsed(tags, file.ideas);
    // The advisor's seat and traits come from its character's advisor role.
    let mut characters = installed_content_core::characters::parse(
        b"characters = { GER_schacht = { advisor = { slot = political_advisor idea_token = GER_schacht_idea traits = { captain_of_industry } } }           GER_split = { instance = { allowed = { has_dlc = \"A\" } advisor = { slot = political_advisor idea_token = GER_split_idea traits = { a } } }             instance = { allowed = { NOT = { has_dlc = \"A\" } } advisor = { slot = political_advisor idea_token = GER_split_idea traits = { b } } } } }",
        &names,
    )
    .unwrap();
    characters.merge(file.advisors);
    let traits = installed_content_core::leader_traits::LeaderTraits::default();
    let localise =
        |key: &str, _: crate::catalogue::View| (key == "economy").then(|| "Economy".to_owned());
    let joined = Joined {
        ideas: &content,
        characters: &characters,
        traits: &traits,
        localise: &localise,
    };
    const NO_HOLDER: Result<&installed_content_core::characters::BuiltTemplate, &str> = Err("no holder");
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
    // One line per modifier, by the identity its folded key names,
    // the last value kept and hidden by hidden_modifier.
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
    // The officer template keeps its role's modifier.
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
    replay.gaps.push(replay_frame::branches::Gap {
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

#[test]
fn idea_history_additions_ignore_initial_gaps_and_holder_renames() {
    let mut renamed = idea_frame(102, &["mod_law", "GER_schacht_idea"]);
    if let TableView::Complete(TableRows::Characters(rows)) =
        &mut *renamed.tables[TableId::Characters.index()]
    {
        rows[0].token = b"Renamed advisor".to_vec();
    }
    let mut replaced = idea_frame(103, &["mod_law", "GER_schacht_idea"]);
    if let TableView::Complete(TableRows::HiredAdvisors(rows)) =
        &mut *replaced.tables[TableId::HiredAdvisors.index()]
    {
        rows.iter_mut()
            .find(|row| row.country == 9001)
            .unwrap()
            .character = 77;
    }
    let replay = Replay::from_records(
        CAMPAIGN.into(),
        &[records(
            1,
            vec![
                idea_frame(100, &["old_mod_law"]),
                idea_frame(101, &["mod_law", "GER_schacht_idea"]),
                renamed,
                replaced,
                idea_frame(105, &["after_gap_law", "GER_schacht_idea"]),
            ],
        )],
    )
    .unwrap();
    let scanned = super::scan_all(&replay, 1, &AtomicBool::new(false)).unwrap();
    let country = scanned.country("GER");
    assert_eq!(
        super::additions(country)
            .map(|span| (
                span.token.as_str(),
                span.first_hour,
                span.character.as_ref().map(|holder| holder.id)
            ))
            .collect::<Vec<_>>(),
        [
            ("GER_schacht_idea", 101, Some(7)),
            ("mod_law", 101, None),
            ("GER_schacht_idea", 103, Some(77)),
        ]
    );
    assert_eq!(country.identity_starts, [100, 105]);
    assert_eq!(super::unavailable_additions(country), 2);
}

#[test]
fn idea_history_all_country_scan_names_each_span_start() {
    use replay_data::{
        rows::ActiveIdeaRow, rows::CountryAppearanceRow, rows::CountryLinkRow, rows::CountryRow,
    };
    let make = |hour, changed: bool| {
        let mut frame = idea_frame(hour, &[]);
        let countries: Vec<_> = (1..=4)
            .map(|id| CountryRow {
                id,
                canonical_slot: id,
                tag: format!("opaque-{id}").into_bytes(),
                ..Default::default()
            })
            .collect();
        let links = countries
            .iter()
            .map(|row| CountryLinkRow {
                id: row.id,
                canonical_slot: row.canonical_slot,
            })
            .collect();
        let ideas = countries
            .iter()
            .map(|row| ActiveIdeaRow {
                country: row.id,
                ordinal: 0,
                token: if changed {
                    b"mod_law_b".to_vec()
                } else {
                    b"mod_law_a".to_vec()
                },
            })
            .collect();
        let appearances = countries
            .iter()
            .map(|row| CountryAppearanceRow {
                country: row.id,
                cosmetic_tag: if changed {
                    b"mod_renamed".to_vec()
                } else {
                    b"mod_previous".to_vec()
                },
                ..Default::default()
            })
            .collect();
        *frame.tables[TableId::Countries.index()] =
            TableView::Complete(TableRows::Countries(countries));
        *frame.tables[TableId::CountryLinks.index()] =
            TableView::Complete(TableRows::CountryLinks(links));
        *frame.tables[TableId::ActiveIdeas.index()] =
            TableView::Complete(TableRows::ActiveIdeas(ideas));
        *frame.tables[TableId::HiredAdvisors.index()] =
            TableView::Complete(TableRows::HiredAdvisors(vec![]));
        *frame.tables[TableId::CountryAppearance.index()] =
            TableView::Complete(TableRows::CountryAppearance(appearances));
        frame
    };
    let replay = Replay::from_records(
        CAMPAIGN.into(),
        &[records(
            1,
            vec![make(100, false), make(101, true), make(103, false)],
        )],
    )
    .unwrap();
    let branch = super::scan_all(&replay, 1, &AtomicBool::new(false)).unwrap();
    assert_eq!(branch.countries.len(), 4);
    for country in branch.countries.values() {
        assert_eq!(
            super::additions(country)
                .map(|span| (span.token.as_str(), span.first_hour))
                .collect::<Vec<_>>(),
            [("mod_law_b", 101)]
        );
        assert_eq!(country.span_naming.len(), country.spans.len());
        assert_eq!(
            country
                .spans
                .iter()
                .zip(&country.span_naming)
                .map(|(span, naming)| (
                    span.first_hour,
                    naming.look.as_ref().unwrap().cosmetic_tag.as_str()
                ))
                .collect::<Vec<_>>(),
            [
                (100, "mod_previous"),
                (101, "mod_renamed"),
                (103, "mod_previous")
            ]
        );
        assert_eq!(country.identity_starts, [100, 103]);
        assert_eq!(country.identity_naming.len(), country.identity_starts.len());
        assert_eq!(super::unavailable_additions(country), 1);
    }
    let selected =
        super::scan_selected(&replay, 1, Some("opaque-3"), &AtomicBool::new(false)).unwrap();
    assert_eq!(selected.countries.len(), 1);
    let country = selected.country("opaque-3");
    assert_eq!(country.spans, branch.country("opaque-3").spans);
    assert_eq!(country.identity_naming.len(), 2);
    assert!(country.span_naming.is_empty());
}

#[test]
fn idea_history_tokenless_advisors_are_counted_and_named() {
    let make = |hour| {
        let mut frame = idea_frame(hour, &[]);
        *frame.tables[TableId::HiredAdvisors.index()] =
            TableView::Complete(TableRows::HiredAdvisors(vec![
                replay_data::rows::HiredAdvisorRow {
                    country: 9001,
                    ordinal: 0,
                    character: 42,
                    idea: vec![],
                    slot: b"mod_seat".to_vec(),
                },
            ]));
        frame
    };
    let replay =
        Replay::from_records(CAMPAIGN.into(), &[records(1, vec![make(100), make(101)])]).unwrap();
    let topic = super::scan(&replay, 1, "GER", &AtomicBool::new(false)).unwrap();
    assert_eq!(
        topic.warnings,
        ["Some appointed advisors have no recorded idea, so their seats show as empty."]
    );
    let branch = super::scan_all(&replay, 1, &AtomicBool::new(false)).unwrap();
    let scanned = branch.country("GER");
    assert!(scanned.warnings.is_empty());
    assert_eq!(
        branch.refusals,
        [(
            "Appointed advisors have no recorded idea token, so cannot be named.".into(),
            1,
        )]
    );
    assert!(scanned.spans.is_empty());
}

#[test]
fn idea_feed_counts_malformed_rows_once_and_keeps_valid_additions() {
    use replay_data::{
        rows::ActiveIdeaRow, rows::CountryLinkRow, rows::CountryRow, rows::HiredAdvisorRow,
    };
    let mut malformed = idea_frame(101, &["mod_known_law"]);
    if let TableView::Complete(TableRows::ActiveIdeas(rows)) =
        &mut *malformed.tables[TableId::ActiveIdeas.index()]
    {
        rows.extend([
            ActiveIdeaRow {
                country: 777,
                ordinal: 0,
                token: b"unresolved_idea".to_vec(),
            },
            ActiveIdeaRow {
                country: 778,
                ordinal: 0,
                token: b"orphan_idea".to_vec(),
            },
            ActiveIdeaRow {
                country: 9001,
                ordinal: 1,
                token: vec![0xff],
            },
        ]);
        rows.sort_by_key(|row| (row.country, row.ordinal));
    }
    if let TableView::Complete(TableRows::HiredAdvisors(rows)) =
        &mut *malformed.tables[TableId::HiredAdvisors.index()]
    {
        rows.extend([
            HiredAdvisorRow {
                country: 9004,
                ordinal: 0,
                character: 54,
                idea: b"unresolved_holder".to_vec(),
                slot: b"seat_c".to_vec(),
            },
            HiredAdvisorRow {
                country: 777,
                ordinal: 0,
                character: 55,
                idea: b"unresolved_advisor".to_vec(),
                slot: b"seat_a".to_vec(),
            },
            HiredAdvisorRow {
                country: 777,
                ordinal: 1,
                character: 56,
                idea: b"unresolved_advisor".to_vec(),
                slot: b"seat_b".to_vec(),
            },
            HiredAdvisorRow {
                country: 9001,
                ordinal: 0,
                character: 57,
                idea: vec![0xfe],
                slot: b"valid_seat".to_vec(),
            },
            HiredAdvisorRow {
                country: 9001,
                ordinal: 1,
                character: 58,
                idea: b"bad_seat_idea".to_vec(),
                slot: vec![0xff],
            },
        ]);
        rows.sort_by_key(|row| (row.country, row.ordinal));
    }
    if let TableView::Complete(TableRows::Countries(rows)) =
        &mut *malformed.tables[TableId::Countries.index()]
    {
        rows.extend([9002, 9003].map(|id| CountryRow {
            id,
            canonical_slot: id,
            tag: vec![0xff],
            ..Default::default()
        }));
        rows.push(CountryRow {
            id: 9004,
            canonical_slot: 44,
            tag: b"mod_unresolved".to_vec(),
            ..Default::default()
        });
        rows.sort_by_key(|row| row.id);
    }
    if let TableView::Complete(TableRows::CountryLinks(rows)) =
        &mut *malformed.tables[TableId::CountryLinks.index()]
    {
        rows.extend([9002, 9003].map(|id| CountryLinkRow {
            id,
            canonical_slot: id,
        }));
        rows.extend([
            CountryLinkRow {
                id: 778,
                canonical_slot: 7777,
            },
            CountryLinkRow {
                id: 9004,
                canonical_slot: 55,
            },
        ]);
        rows.sort_by_key(|row| row.id);
    }
    let mut unchanged = malformed.clone();
    unchanged.hour = 102;
    let mut unrelated = malformed.clone();
    unrelated.hour = 103;
    *unrelated.tables[TableId::Characters.index()] =
        TableView::Complete(TableRows::Characters(vec![]));
    let mut renamed = unrelated.clone();
    renamed.hour = 104;
    if let TableView::Complete(TableRows::Countries(rows)) =
        &mut *renamed.tables[TableId::Countries.index()]
    {
        rows.iter_mut().find(|row| row.id == 9001).unwrap().tag = b"RENAMED".to_vec();
    }
    let mut renamed_unrelated = renamed.clone();
    renamed_unrelated.hour = 105;
    *renamed_unrelated.tables[TableId::LexicalTokens.index()] =
        TableView::Complete(TableRows::LexicalTokens(vec![]));
    let replay = Replay::from_records(
        CAMPAIGN.into(),
        &[records(
            1,
            vec![
                idea_frame(100, &[]),
                malformed,
                unchanged,
                unrelated,
                renamed,
                renamed_unrelated,
            ],
        )],
    )
    .unwrap();
    let feed = super::scan_all(&replay, 1, &AtomicBool::new(false)).unwrap();
    assert_eq!(feed.refusals, [
        ("ActiveIdeas owner has no resolved Countries holder; holding cannot be attributed.".into(), 1),
        ("ActiveIdeas owner is unresolved through CountryLinks; holding cannot be attributed.".into(), 1),
        ("Countries tag is not UTF-8; idea holder cannot be identified.".into(), 2),
        ("Held idea token is not UTF-8; holding cannot be identified.".into(), 2),
        ("HiredAdvisors owner has no resolved Countries holder; appointment cannot be attributed.".into(), 1),
        ("HiredAdvisors owner is unresolved through CountryLinks; appointment cannot be attributed.".into(), 2),
        ("HiredAdvisors seat is not UTF-8; appointment cannot be identified.".into(), 1),
    ]);
    let topic = super::scan(&replay, 1, "GER", &AtomicBool::new(false)).unwrap();
    for reason in [
        "Held idea token is not UTF-8; holding cannot be identified. (count: 2)",
        "HiredAdvisors seat is not UTF-8; appointment cannot be identified. (count: 1)",
    ] {
        assert!(
            topic.warnings.iter().any(|warning| warning == reason),
            "{:?}",
            topic.warnings
        );
    }
    let known = feed.country("GER");
    assert_eq!(
        super::additions(known)
            .map(|span| (&*span.token, span.first_hour, span.last_hour))
            .collect::<Vec<_>>(),
        [("mod_known_law", 101, 103)]
    );
    assert!(super::additions(feed.country("RENAMED")).next().is_none());
    assert_eq!(
        feed.country("mod_unresolved").unobserved,
        [crate::products::country_groups::HourSpan {
            first_hour: 101,
            last_hour: 105
        }]
    );
    assert_eq!(topic.spans, known.spans);
}

#[test]
fn idea_history_feed_preserves_each_unchanged_unresolved_hour_before_additions() {
    let mut unresolved = idea_frame(101, &["mod_old_law"]);
    if let TableView::Complete(TableRows::CountryLinks(rows)) =
        &mut *unresolved.tables[TableId::CountryLinks.index()]
    {
        rows.retain(|row| row.id != 9001);
    }
    let mut unchanged = unresolved.clone();
    unchanged.hour = 102;
    let replay = Replay::from_records(
        CAMPAIGN.into(),
        &[records(
            1,
            vec![
                idea_frame(100, &["mod_old_law"]),
                unresolved,
                unchanged,
                idea_frame(103, &["mod_new_law"]),
            ],
        )],
    )
    .unwrap();
    let feed = super::scan_all(&replay, 1, &AtomicBool::new(false)).unwrap();
    let country = feed.country("GER");
    assert_eq!(
        country.unobserved,
        [crate::products::country_groups::HourSpan {
            first_hour: 101,
            last_hour: 102
        }]
    );
    assert_eq!(super::additions(country).count(), 0);
    assert_eq!(super::unavailable_additions(country), 1);
}

#[test]
fn idea_history_effect_keys_include_shared_effect_and_text_sources_once() {
    let content = installed_content_core::ideas::IdeaContent::default();
    let characters = installed_content_core::characters::Characters::default();
    let traits = installed_content_core::leader_traits::LeaderTraits::default();
    let joined = super::Joined {
        ideas: &content,
        characters: &characters,
        traits: &traits,
        localise: &|_, _| None,
    };
    let (mut idea, _) = super::prepare_info(&joined, "mod_idea", None, Err("no holder"), false);
    let line = |key: &str| super::EffectLine {
        key: key.into(),
        fixed5: "1".into(),
        hidden: false,
    };
    let group = |target: &str| super::EffectGroup {
        target: Some(target.into()),
        instant: false,
        lines: vec![line("shared")],
        tooltips: vec!["target_tooltip".into()],
    };
    idea.effects = vec![line("shared")];
    idea.targeted = vec![group("mod_tag")];
    idea.equipment = vec![group("mod_equipment")];
    idea.research = vec![line("mod_research_category")];
    idea.tooltips = vec!["idea_tooltip".into()];
    idea.traits = vec![super::TraitEffects {
        token: "mod_trait".into(),
        known: true,
        unknown: None,
        name: None,
        lines: vec![line("trait_modifier")],
        targeted: vec![],
        equipment: vec![group("mod_trait_equipment")],
        tooltips: vec!["trait_tooltip".into()],
    }];
    let (modifiers, texts) = super::effect_keys([&idea, &idea]);
    assert_eq!(
        modifiers.iter().map(String::as_str).collect::<Vec<_>>(),
        ["shared", "trait_modifier"]
    );
    assert_eq!(
        texts.iter().map(String::as_str).collect::<Vec<_>>(),
        [
            "NO",
            "YES",
            "idea_tooltip",
            "mod_equipment",
            "mod_research_category_research",
            "mod_trait_equipment",
            "target_tooltip",
            "trait_tooltip"
        ]
    );
}

#[test]
fn idea_history_additions_after_incomplete_read_are_counted_not_invented() {
    let mut incomplete = idea_frame(101, &["old_mod_law"]);
    *incomplete.tables[TableId::ActiveIdeas.index()] = TableView::Unavailable {
        reason: "synthetic incomplete idea read".into(),
    };
    let replay = Replay::from_records(
        CAMPAIGN.into(),
        &[records(
            1,
            vec![
                idea_frame(100, &["old_mod_law"]),
                incomplete,
                idea_frame(102, &["new_mod_law", "GER_schacht_idea"]),
            ],
        )],
    )
    .unwrap();
    let branch = super::scan_all(&replay, 1, &AtomicBool::new(false)).unwrap();
    let country = branch.country("GER");
    assert_eq!(super::additions(country).count(), 0);
    assert_eq!(super::unavailable_additions(country), 2);
    assert_eq!(country.gap_starts, [102]);
}

#[test]
fn idea_history_feed_names_additions_only_at_their_span_start() {
    let make = |hour, ideas: &[&str], cosmetic: &[u8]| {
        let mut frame = idea_frame(hour, ideas);
        *frame.tables[TableId::CountryAppearance.index()] =
            TableView::Complete(TableRows::CountryAppearance(vec![
                replay_data::rows::CountryAppearanceRow {
                    country: 9001,
                    cosmetic_tag: cosmetic.to_vec(),
                    ..Default::default()
                },
            ]));
        *frame.tables[TableId::WarRelations.index()] = TableView::Unavailable {
            reason: format!("unneeded naming input changed at hour {hour}"),
        };
        frame
    };
    let replay = Replay::from_records(
        CAMPAIGN.into(),
        &[records(
            1,
            vec![
                make(100, &[], b"mod_initial"),
                make(101, &["mod_law_a"], b"mod_before"),
                make(102, &["mod_law_a"], b"mod_renamed"),
                make(103, &["mod_law_a", "mod_law_b"], b"mod_renamed"),
                make(104, &["mod_law_a", "mod_law_b"], &[0xff]),
            ],
        )],
    )
    .unwrap();
    let branch = super::scan_all(&replay, 1, &AtomicBool::new(false)).unwrap();
    let country = branch.country("GER");
    assert_eq!(country.spans.len(), 2);
    assert_eq!(country.span_naming.len(), 2);
    assert_eq!(
        super::named_additions(country)
            .map(|(span, naming)| (
                span.token.as_str(),
                span.first_hour,
                span.last_hour,
                naming.unwrap().look.as_ref().unwrap().cosmetic_tag.as_str(),
            ))
            .collect::<Vec<_>>(),
        [
            ("mod_law_a", 101, 104, "mod_before"),
            ("mod_law_b", 103, 104, "mod_renamed"),
        ]
    );
    let topic = super::scan(&replay, 1, "GER", &AtomicBool::new(false)).unwrap();
    assert_eq!(topic.spans, country.spans);
    assert!(topic.span_naming.is_empty());
    assert_eq!(topic.identity_naming.len(), 1);
    assert_eq!(
        topic.identity_naming[0].look.as_ref().unwrap().cosmetic_tag,
        "mod_initial"
    );
}

#[test]
fn indexed_idea_naming_preserves_independent_positive_empty_original_and_refusal() {
    use super::NamingCountry;
    use replay_data::{rows::Recorded, rows::tables};
    let mut frame = crate::scope::tests::positive_frame();
    for original in [Recorded::Value(Some(Vec::new())), Recorded::Value(None)] {
        let TableView::Complete(TableRows::CountryAppearance(rows)) =
            &mut *frame.tables[TableId::CountryAppearance.index()]
        else {
            panic!("appearance");
        };
        rows[0].original_tag_reference = Recorded::Value(Some(300));
        rows[0].original_tag = original;
        // Static generator gating cannot witness this independently recorded getter.
        rows[0].name_tag.clear();
        rows[0].cosmetic_tag = b"MOD_cosmetic".to_vec();
        let country = &frame.complete::<tables::Countries>().unwrap()[0];
        let appearance = &frame.complete::<tables::CountryAppearance>().unwrap()[0];
        let naming = NamingCountry::read_with_appearance(&frame, country, Some(appearance));
        assert_eq!(naming, NamingCountry::read(&frame, country));
        let look = naming.look.unwrap();
        assert_eq!(look.cosmetic_tag, "MOD_cosmetic");
        if matches!(appearance.original_tag, Recorded::Value(Some(_))) {
            assert_eq!(look.original_or_own_tag, Ok(String::new()));
        } else {
            assert!(
                look.original_or_own_tag
                    .unwrap_err()
                    .contains("capture refused")
            );
        }
    }
}

#[test]
fn prepared_ideas_keep_the_topic_scans_selected_table_refusals() {
    use replay_data::{
        rows::ActiveIdeaRow, rows::CountryAppearanceRow, rows::CountryLinkRow, rows::CountryRow,
        rows::ReservedCountryNamingRow,
    };
    let mut frame = idea_frame(100, &[]);
    if let TableView::Complete(TableRows::Countries(rows)) =
        &mut *frame.tables[TableId::Countries.index()]
    {
        rows.push(CountryRow {
            id: 700,
            canonical_slot: 0,
            tag: b"ZERO".to_vec(),
            ..Default::default()
        });
        rows.sort_by_key(|row| row.id);
    }
    if let TableView::Complete(TableRows::CountryLinks(rows)) =
        &mut *frame.tables[TableId::CountryLinks.index()]
    {
        rows.push(CountryLinkRow {
            id: 700,
            canonical_slot: 0,
        });
        rows.sort_by_key(|row| row.id);
    }
    if let TableView::Complete(TableRows::ActiveIdeas(rows)) =
        &mut *frame.tables[TableId::ActiveIdeas.index()]
    {
        rows.push(ActiveIdeaRow {
            country: 700,
            ordinal: 0,
            token: b"mod_idea".to_vec(),
        });
        rows.sort_by_key(|row| (row.country, row.ordinal));
    }
    *frame.tables[TableId::CountryAppearance.index()] =
        TableView::Complete(TableRows::CountryAppearance(vec![CountryAppearanceRow {
            country: 700,
            ..Default::default()
        }]));
    *frame.tables[TableId::ReservedCountryNaming.index()] =
        TableView::Complete(TableRows::ReservedCountryNaming(vec![
            ReservedCountryNamingRow {
                tag: b"ZERO".to_vec(),
                ..Default::default()
            },
        ]));
    let replay = Replay::from_records(CAMPAIGN.into(), &[records(1, vec![frame.clone()])]).unwrap();
    let mut observer = super::Scan::new(&replay, 1);
    observer.observe(&frame).unwrap();
    let old = super::scan(&replay, 1, "ZERO", &AtomicBool::new(false)).unwrap();
    let prepared = observer.country(&replay, 1, "ZERO").unwrap();
    assert_eq!(
        serde_json::to_value(&prepared).unwrap(),
        serde_json::to_value(old).unwrap()
    );
    assert!(
        prepared.identity_naming[0]
            .look
            .as_ref()
            .unwrap()
            .original_or_own_tag
            .as_ref()
            .unwrap_err()
            .contains("Table not selected for reconstruction")
    );
}

#[test]
fn unchanged_missing_character_template_dates_match_the_hourly_oracle() {
    use super::*;
    let mut frame = idea_frame(100, &["GER_schacht_idea"]);
    *frame.tables[TableId::Characters.index()] = TableView::Complete(TableRows::Characters(vec![]));
    let mut frames = Vec::new();
    let mut state = Scan::default();
    for hour in 100..=120 {
        frame.hour = hour;
        state.observe(&frame).unwrap();
        frames.push(frame.clone());
    }
    let replay = Replay::from_records(CAMPAIGN.into(), &[records(1, frames)]).unwrap();
    crate::prepared::observer_tests::assert_reply_parity(
        &state.country(&replay, 1, "GER").unwrap(),
        &scan(&replay, 1, "GER", &AtomicBool::new(false)).unwrap(),
    )
    .unwrap();
    assert!(
        state.states["GER"].result.templates[&7]
            .as_ref()
            .unwrap_err()
            .contains("hour 120")
    );
}

fn check_idea_country_delta_work(count: u32) {
    use super::IdeaInputs;
    use crate::prepared::frame_changes::FrameChangeTracker;
    use replay_data::{format::FormatVersion, rows::CountryLinkRow, rows::CountryRow};
    use replay_frame::{frame::FrameDecoder, frame::FrameEncoder};
    use std::sync::Arc;
    let format = FormatVersion::CURRENT;
    let mut frame = crate::test_fixtures::source(1, 100, None);
    *frame.tables[TableId::Countries.index()] = TableView::Complete(TableRows::Countries(
        (1..=count)
            .map(|id| CountryRow {
                id,
                canonical_slot: id,
                tag: format!("realm {id}").into_bytes(),
                ..Default::default()
            })
            .collect(),
    ));
    *frame.tables[TableId::CountryLinks.index()] = TableView::Complete(TableRows::CountryLinks(
        (1..=count)
            .map(|id| CountryLinkRow {
                id,
                canonical_slot: id,
            })
            .collect(),
    ));
    *frame.tables[TableId::ActiveIdeas.index()] =
        TableView::Complete(TableRows::ActiveIdeas(Vec::new()));
    *frame.tables[TableId::HiredAdvisors.index()] =
        TableView::Complete(TableRows::HiredAdvisors(Vec::new()));
    let mut states = [IdeaInputs::default(), IdeaInputs::default()];
    let mut trackers = [FrameChangeTracker::new(), FrameChangeTracker::new()];
    let mut encoder = FrameEncoder::new();
    let mut decoder = FrameDecoder::new();
    for hour in 100..=106 {
        frame.hour = hour;
        if let TableView::Complete(TableRows::Countries(rows)) =
            &mut *frame.tables[TableId::Countries.index()]
        {
            rows[0].political_power_raw = Some(i64::from(hour));
            if hour == 102 {
                rows[0].tag = b"changed alias".to_vec();
            }
            if hour == 105 {
                rows.pop();
            }
        }
        let wire = Arc::new(
            encoder
                .encode(frame.clone(), hour == 100 || hour == 104)
                .unwrap(),
        );
        let decoded = decoder.advance(&wire).unwrap();
        let changes = trackers[0].observe_updates(decoded, Arc::clone(&wire), format);
        let whole = trackers[1].observe(decoded, format);
        let before = states[0].keys.as_ref().map_or(0, |keys| keys.row_visits());
        let dirty = states[0].refresh(decoded, Some(&changes), &mut Default::default());
        let reference = states[1].refresh(decoded, Some(&whole), &mut Default::default());
        assert_eq!(dirty, reference, "dirty holders at {hour}");
        assert_eq!(states[0].holders, states[1].holders);
        // A kept reduced view can be reported rebuilt while its key allocation holds.
        let visits = states[0].keys.as_ref().unwrap().row_visits();
        states[0].refresh(decoded, Some(&changes), &mut Default::default());
        assert_eq!(states[0].keys.as_ref().unwrap().row_visits(), visits);
        if hour == 101 {
            let visits = states[0].keys.as_ref().unwrap().row_visits() - before;
            assert_eq!(
                visits, 1,
                "only the changed country's field mask is compared"
            );
        }
    }
}

#[test]
fn idea_country_balance_deltas_visit_only_changed_keys() {
    check_idea_country_delta_work(64);
}
