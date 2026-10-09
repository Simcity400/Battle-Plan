use super::*;
use crate::{
    SourceKey,
    test_fixtures::{CAMPAIGN, records, scale_frame, source},
};
use replay::{
    container::Replay,
    frame::{DecodedFrame, TableView},
    rows::*,
};

fn frame(hour: i32, count: u32) -> DecodedFrame {
    let mut frame = scale_frame(1, hour, count);
    // Raw zero is the separate reserved country; real synthetic actors use positive slots.
    if let TableView::Complete(TableRows::Countries(rows)) =
        &mut *frame.tables[TableId::Countries.index()]
    {
        for row in rows {
            row.canonical_slot += 1;
        }
    }
    if let TableView::Complete(TableRows::CountryLinks(rows)) =
        &mut *frame.tables[TableId::CountryLinks.index()]
    {
        for row in rows {
            row.canonical_slot += 1;
        }
    }
    for rows in [
        TableRows::HumanControllers(Vec::new()),
        TableRows::Factions(Vec::new()),
        TableRows::FactionMembers(Vec::new()),
        TableRows::WarRelations(Vec::new()),
        TableRows::Capitulation(Vec::new()),
        TableRows::RelationTuples(Vec::new()),
        TableRows::Subjects(Vec::new()),
        TableRows::Autonomy(Vec::new()),
        TableRows::PoliticalParties(Vec::new()),
        TableRows::Politics(Vec::new()),
        TableRows::ActiveIdeas(Vec::new()),
        TableRows::HiredAdvisors(Vec::new()),
        TableRows::CompletedFocuses(Vec::new()),
        TableRows::Projects(Vec::new()),
        TableRows::ProjectHistory(Vec::new()),
        TableRows::ResearchTechnologies(Vec::new()),
        TableRows::TechnologyDefinitions(Vec::new()),
        TableRows::SliceReads(Vec::new()),
        TableRows::LexicalTokens(Vec::new()),
    ] {
        let id = rows.table();
        *frame.tables[id.index()] = TableView::Complete(rows);
    }
    *frame.tables[TableId::CountryCounterInputs.index()] =
        TableView::Complete(TableRows::CountryCounterInputs(
            (0..count)
                .map(|index| CountryCounterInputRow {
                    country: 100_000 + index,
                    owned_state_count: 1,
                    ..Default::default()
                })
                .collect(),
        ));
    frame.metrics.table_ns[TableId::CountryCounterInputs.index()] = 1;
    *frame.tables[TableId::Focus.index()] = TableView::Complete(TableRows::Focus(
        (0..count)
            .map(|index| FocusRow {
                country: 100_000 + index,
                ..Default::default()
            })
            .collect(),
    ));
    *frame.tables[TableId::Nukes.index()] = TableView::Complete(TableRows::Nukes(
        (0..count)
            .map(|index| NukeRow {
                country: 100_000 + index,
                ..Default::default()
            })
            .collect(),
    ));
    frame
}

fn opened(frames: Vec<DecodedFrame>) -> Source {
    let replay = Replay::from_records(CAMPAIGN.into(), &[records(1, frames)]).unwrap();
    Source::from_replay(
        SourceKey {
            source_id: "feed-test".into(),
            revision: "one".into(),
        },
        "synthetic.battleplan".into(),
        Arc::new(replay),
        &AtomicBool::new(false),
    )
    .unwrap()
}

fn load(source: &Source, first: Option<i32>, last: Option<i32>) -> EventFeedReply {
    EventFeed::load(
        source,
        &EventFeedParams {
            branch_id: "1".into(),
            first_hour: first,
            last_hour: last,
        },
        &LoadContext { content: None },
        &AtomicBool::new(false),
    )
    .unwrap()
}

fn researched(sample: &mut DecodedFrame, level: i32, fresh: bool) {
    *sample.tables[TableId::ResearchTechnologies.index()] =
        TableView::Complete(TableRows::ResearchTechnologies(vec![
            ResearchTechnologyRow {
                country: 100_000,
                technology_handle: 5,
                level,
                researched: u8::from(level > 0),
                ..Default::default()
            },
        ]));
    *sample.tables[TableId::TechnologyDefinitions.index()] =
        TableView::Complete(TableRows::TechnologyDefinitions(vec![
            TechnologyDefinitionRow {
                technology_handle: 5,
                id: b"mod_tech".to_vec(),
                ..Default::default()
            },
        ]));
    *sample.tables[TableId::SliceReads.index()] =
        TableView::Complete(TableRows::SliceReads(vec![SliceReadRow {
            table: TableId::ResearchTechnologies.index() as u32,
            owner: 100_000,
            sample_hour: if fresh { sample.hour } else { 100 },
        }]));
}

fn faction_name_replies(
    literals: [&str; 2],
) -> (
    country_groups::CountryGroupReply,
    EventFeedReply,
    EventFeedReply,
) {
    let (_, folder) = crate::catalogue::tests::folder_catalogue(&[
        (
            "common/factions/templates/mod.txt",
            "mod_first={name=MOD_FIRST_NAME} mod_second={name=MOD_SECOND_NAME}",
        ),
        (
            "localisation/mod_l_english.yml",
            concat!(
                "\u{feff}l_english:\n",
                " MOD_FIRST_NAME:0 \"First mod pact\"\n",
                " MOD_SECOND_NAME:0 \"Second mod pact\"\n",
                " Stored:0 \"Never translate a literal\"\n",
            ),
        ),
    ]);
    let mut frames: Vec<_> = (100..=103).map(|hour| frame(hour, 1)).collect();
    for (index, literal) in literals.iter().enumerate() {
        let sample = &mut frames[index + 1];
        let token = 17 + index as u32;
        *sample.tables[TableId::Factions.index()] =
            TableView::Complete(TableRows::Factions(vec![FactionRow {
                id: 5,
                name: literal.as_bytes().to_vec(),
                template_token: Some(token),
                ..Default::default()
            }]));
        *sample.tables[TableId::FactionMembers.index()] =
            TableView::Complete(TableRows::FactionMembers(vec![FactionMemberRow {
                faction: 5,
                country: 100_000,
                ordinal: 0,
            }]));
        *sample.tables[TableId::LexicalTokens.index()] =
            TableView::Complete(TableRows::LexicalTokens(vec![LexicalTokenRow {
                token,
                literal: [b"mod_first".as_slice(), b"mod_second".as_slice()][index].to_vec(),
            }]));
    }
    let (source, context) = crate::test_fixtures::source_with_content(frames, &folder.0);
    let groups = country_groups::CountryGroups::load(
        &source,
        &country_groups::CountryGroupParams {
            branch_id: "1".into(),
        },
        &context,
        &AtomicBool::new(false),
    )
    .unwrap();
    let params = EventFeedParams {
        branch_id: "1".into(),
        first_hour: Some(100),
        last_hour: Some(103),
    };
    let feed = EventFeed::load(&source, &params, &context, &AtomicBool::new(false)).unwrap();
    let manifest = EventFeed::load(
        &source,
        &EventFeedParams {
            first_hour: None,
            last_hour: None,
            ..params
        },
        &context,
        &AtomicBool::new(false),
    )
    .unwrap();
    (groups, feed, manifest)
}

fn assert_faction_names_match_topic(literals: [&str; 2], expected: [&str; 2]) {
    let (groups, feed, manifest) = faction_name_replies(literals);
    assert_eq!(feed.rows.len(), 2);
    for (kind, event_hour, name_hour, expected) in [
        (EventFeedKind::FactionJoined, 101, 101, expected[0]),
        (EventFeedKind::FactionLeft, 103, 102, expected[1]),
    ] {
        let row = feed.rows.iter().find(|row| row.kind == kind).unwrap();
        let name = &feed.items[row.item.unwrap() as usize].name;
        let topic = groups.factions[0]
            .names
            .iter()
            .find(|run| run.first_hour <= name_hour && name_hour <= run.last_hour)
            .unwrap();
        assert_eq!(row.name_hour, name_hour);
        assert_eq!(row.hour, event_hour);
        assert_eq!(name.as_deref(), Some(expected));
        assert_eq!(name, &topic.name);
    }
    assert!(
        !manifest
            .unavailable
            .iter()
            .any(|entry| entry.kind == "factionName")
    );
}

#[test]
fn event_feed_faction_template_names_match_topic_at_join_and_dissolution() {
    assert_faction_names_match_topic(["", ""], ["First mod pact", "Second mod pact"]);
}

#[test]
fn event_feed_faction_stored_literals_match_topic_without_key_translation() {
    assert_faction_names_match_topic(
        ["Stored", "Changed stored name"],
        ["Stored", "Changed stored name"],
    );
}

#[test]
fn event_feed_faction_name_refusal_is_counted_once_for_join_and_leave() {
    let mut invalid = frame(101, 1);
    *invalid.tables[TableId::Factions.index()] =
        TableView::Complete(TableRows::Factions(vec![FactionRow {
            id: 5,
            name: vec![0xff],
            ..Default::default()
        }]));
    *invalid.tables[TableId::FactionMembers.index()] =
        TableView::Complete(TableRows::FactionMembers(vec![FactionMemberRow {
            faction: 5,
            country: 100_000,
            ordinal: 0,
        }]));
    let source = opened(vec![frame(100, 1), invalid, frame(102, 1)]);
    let manifest = load(&source, None, None);
    let missing: Vec<_> = manifest
        .unavailable
        .iter()
        .filter(|entry| entry.kind == "factionName")
        .collect();
    assert_eq!(missing.len(), 1);
    assert_eq!(missing[0].count, Some(1));
    let topic = country_groups::CountryGroups::load(
        &source,
        &country_groups::CountryGroupParams {
            branch_id: "1".into(),
        },
        &LoadContext { content: None },
        &AtomicBool::new(false),
    )
    .unwrap();
    assert!(topic.warnings.contains(&missing[0].reason));
    assert!(
        missing[0]
            .reason
            .contains("faction name (140D77360) is not UTF-8")
    );
    let rows = load(&source, Some(100), Some(102));
    assert_eq!(rows.rows.len(), 2);
    assert!(rows.rows.iter().all(|row| {
        let item = &rows.items[row.item.unwrap() as usize];
        item.name.is_none() && item.token.is_none()
    }));
}

#[test]
fn event_feed_mod_focus_and_law_keep_content_artwork_without_technology_events() {
    let (_, folder) = crate::catalogue::tests::folder_catalogue(&[
        (
            "common/defines/mod.lua",
            "NDefines={NCharacter={OFFICER_CORP_ADVISOR_ENTRIES_IN_MENU={}}, NFocus={FOCUS_POINT_DAYS=5}}",
        ),
        (
            "common/idea_tags/mod.txt",
            "idea_categories={mod_government={slot=mod_laws}}",
        ),
        (
            "common/ideas/mod.txt",
            "ideas={mod_laws={law=yes mod_law={picture=mod_law modifier={stability_factor=0.1}}}}",
        ),
        (
            "common/national_focus/mod.txt",
            "focus_tree={id=mod_tree focus={id=mod_focus icon=GFX_mod_focus cost=1}}",
        ),
        (
            "common/technologies/mod.txt",
            "technologies={mod_tech={research_cost=1}}",
        ),
        (
            "common/terrain/mod.txt",
            "categories={mod_ground={movement_cost=1}}",
        ),
        (
            "interface/mod.gfx",
            "spriteTypes={spriteType={name=GFX_mod_focus texturefile=\"gfx/mod.png\"} spriteType={name=GFX_idea_mod_law texturefile=\"gfx/mod.png\"} spriteType={name=GFX_mod_tech_medium texturefile=\"gfx/mod.png\"}}",
        ),
        (
            "localisation/english/mod_l_english.yml",
            "\u{feff}l_english:\n mod_focus:0 \"Mod focus name\"\n mod_law:0 \"Mod law name\"\n mod_tech:0 \"Mod technology name\"\n",
        ),
    ]);
    let image = folder.0.join("gfx/mod.png");
    std::fs::create_dir_all(image.parent().unwrap()).unwrap();
    let mut png = png::Encoder::new(std::fs::File::create(image).unwrap(), 1, 1);
    png.set_color(png::ColorType::Rgba);
    png.set_depth(png::BitDepth::Eight);
    png.write_header()
        .unwrap()
        .write_image_data(&[18, 42, 79, 255])
        .unwrap();
    let mut first = frame(100, 1);
    researched(&mut first, 0, true);
    let mut changed = frame(101, 1);
    researched(&mut changed, 1, true);
    *changed.tables[TableId::Focus.index()] =
        TableView::Complete(TableRows::Focus(vec![FocusRow {
            country: 100_000,
            completed_count: 1,
            ..Default::default()
        }]));
    *changed.tables[TableId::CompletedFocuses.index()] =
        TableView::Complete(TableRows::CompletedFocuses(vec![CompletedFocusRow {
            country: 100_000,
            ordinal: 0,
            token: b"mod_focus".to_vec(),
        }]));
    *changed.tables[TableId::ActiveIdeas.index()] =
        TableView::Complete(TableRows::ActiveIdeas(vec![ActiveIdeaRow {
            country: 100_000,
            ordinal: 0,
            token: b"mod_law".to_vec(),
        }]));
    for sample in [&mut first, &mut changed] {
        *sample.tables[TableId::CountryAppearance.index()] =
            TableView::Complete(TableRows::CountryAppearance(vec![CountryAppearanceRow {
                country: 100_000,
                original_tag_reference: Recorded::Value(Some(0)),
                ..Default::default()
            }]));
    }
    let (source, context) =
        crate::test_fixtures::source_with_content(vec![first, changed], &folder.0);
    let politics = politics_content::politics_content(&source, context.content().unwrap()).unwrap();
    assert!(politics.ideas.idea("mod_law").unwrap().unwrap().law);
    let reply = EventFeed::load(
        &source,
        &EventFeedParams {
            branch_id: "1".into(),
            first_hour: Some(100),
            last_hour: Some(101),
        },
        &context,
        &AtomicBool::new(false),
    )
    .unwrap();
    assert_eq!(reply.rows.len(), 2);
    assert!(
        reply
            .items
            .iter()
            .all(|item| item.token.as_deref() != Some("mod_tech"))
    );
    for (kind, name) in [
        (EventFeedKind::FocusCompleted, "Mod focus name"),
        (EventFeedKind::LawChanged, "Mod law name"),
    ] {
        let row = reply
            .rows
            .iter()
            .find(|row| row.kind == kind)
            .unwrap_or_else(|| {
                panic!(
                    "missing {kind:?}: rows={:?}, warnings={:?}, unavailable={:?}",
                    reply.rows, reply.warnings, reply.unavailable
                )
            });
        let item = &reply.items[row.item.unwrap() as usize];
        assert_eq!(item.name.as_deref(), Some(name), "{:?}", reply.warnings);
        assert!(
            item.icon
                .as_deref()
                .is_some_and(|icon| icon.starts_with("data:image/png;base64,")),
            "{kind:?}: {:?}",
            reply.warnings
        );
        assert_eq!((row.hour, row.read_hour, row.exact), (101, 101, false));
    }
    assert!(
        reply
            .items
            .iter()
            .find_map(|item| item.idea.as_ref())
            .unwrap()
            .law
    );
}

#[test]
fn event_feed_content_load_failure_retains_the_named_parser_reason() {
    let (_catalogue, folder) = crate::catalogue::tests::folder_catalogue(&[(
        "common/defines/mod.lua",
        "NDefines={NCharacter={OFFICER_CORP_ADVISOR_ENTRIES_IN_MENU={}} NFocus={}}",
    )]);
    let (source, context) =
        crate::test_fixtures::source_with_content(vec![frame(100, 1), frame(101, 1)], &folder.0);
    let reply = EventFeed::load(
        &source,
        &EventFeedParams {
            branch_id: "1".into(),
            first_hour: None,
            last_hour: None,
        },
        &context,
        &AtomicBool::new(false),
    )
    .unwrap();
    let refusal = reply
        .unavailable
        .iter()
        .find(|item| item.kind == "politicsContent")
        .unwrap();
    assert!(
        refusal.reason.contains("common/defines/mod.lua"),
        "{refusal:?}"
    );
    assert_eq!(refusal.count, Some(1));
}

#[test]
fn event_feed_ruling_advisor_and_project_share_topic_derivations_and_missing_law_is_counted() {
    let mut frames: Vec<_> = (100..=102).map(|hour| frame(hour, 1)).collect();
    for sample in &mut frames {
        let hour = sample.hour;
        *sample.tables[TableId::PoliticalParties.index()] =
            TableView::Complete(TableRows::PoliticalParties(vec![PoliticalPartyRow {
                country: 100_000,
                id: 1,
                is_ruling: 1,
                tag: 100_000,
                token: if hour == 100 {
                    b"old".to_vec()
                } else {
                    b"new".to_vec()
                },
                ..Default::default()
            }]));
        *sample.tables[TableId::Projects.index()] =
            TableView::Complete(TableRows::Projects(vec![ProjectRow {
                owner: 100_000,
                local_id: 0,
                definition_id: 8,
                started: 1,
                state: replay::rows::ProjectState {
                    current_offset: if hour == 102 { 0xe8 } else { 0x18 },
                    ..Default::default()
                },
                ..Default::default()
            }]));
        sample.metrics.table_ns[TableId::ProjectPrograms.index()] = 1;
        if hour >= 101 {
            *sample.tables[TableId::HiredAdvisors.index()] =
                TableView::Complete(TableRows::HiredAdvisors(vec![HiredAdvisorRow {
                    country: 100_000,
                    ordinal: 0,
                    character: 9,
                    idea: b"mod_advisor".to_vec(),
                    slot: b"mod_seat".to_vec(),
                }]));
            *sample.tables[TableId::ActiveIdeas.index()] =
                TableView::Complete(TableRows::ActiveIdeas(vec![ActiveIdeaRow {
                    country: 100_000,
                    ordinal: 0,
                    token: b"unknown_mod_law".to_vec(),
                }]));
        }
        if hour == 102 {
            *sample.tables[TableId::ProjectHistory.index()] =
                TableView::Complete(TableRows::ProjectHistory(vec![ProjectHistoryRow {
                    owner: 100_000,
                    project: 0,
                    prototype_reward_handle: 0x4c17,
                    option_handle: 0x4c17,
                    date: 101,
                    ..Default::default()
                }]));
        }
    }
    let source = opened(frames);
    let reply = load(&source, Some(100), Some(102));
    for (kind, hour, read, exact) in [
        (EventFeedKind::RulingPartyChanged, 101, 101, false),
        (EventFeedKind::AdvisorHired, 101, 101, false),
        (EventFeedKind::ProjectFinished, 101, 102, true),
    ] {
        let row = reply.rows.iter().find(|row| row.kind == kind).unwrap();
        assert_eq!((row.hour, row.read_hour, row.exact), (hour, read, exact));
    }
    assert!(
        load(&source, None, None)
            .unavailable
            .iter()
            .any(|entry| entry.kind == "lawChanged"
                && entry.reason.contains("unknown_mod_law")
                && entry.count == Some(1))
    );
    let project = reply
        .rows
        .iter()
        .find(|row| row.kind == EventFeedKind::ProjectFinished)
        .unwrap();
    assert_eq!(
        reply.items[project.item.unwrap() as usize].description,
        None
    );
}

#[test]
fn event_feed_excludes_recorded_technology_completions_but_keeps_other_milestones() {
    let mut frames: Vec<_> = (100..=102).map(|hour| frame(hour, 1)).collect();
    for sample in &mut frames {
        let hour = sample.hour;
        researched(sample, i32::from(hour >= 101), true);
        sample.metrics.table_ns[TableId::Nukes.index()] = 1;
        if let TableView::Complete(TableRows::Nukes(rows)) =
            &mut *sample.tables[TableId::Nukes.index()]
        {
            rows[0].amount_raw = if hour == 102 { 1_000_000_000 } else { 0 };
        }
        if hour >= 101 {
            if let TableView::Complete(TableRows::Focus(rows)) =
                &mut *sample.tables[TableId::Focus.index()]
            {
                rows[0].completed_count = 1;
            }
            *sample.tables[TableId::CompletedFocuses.index()] =
                TableView::Complete(TableRows::CompletedFocuses(vec![CompletedFocusRow {
                    country: 100_000,
                    ordinal: 0,
                    token: b"mod_focus".to_vec(),
                }]));
        }
        *sample.tables[TableId::Projects.index()] =
            TableView::Complete(TableRows::Projects(vec![ProjectRow {
                owner: 100_000,
                local_id: 0,
                definition_id: 8,
                started: 1,
                state: replay::rows::ProjectState {
                    current_offset: if hour == 102 { 0xe8 } else { 0x18 },
                    ..Default::default()
                },
                ..Default::default()
            }]));
        sample.metrics.table_ns[TableId::ProjectPrograms.index()] = 1;
        if hour == 102 {
            *sample.tables[TableId::ProjectHistory.index()] =
                TableView::Complete(TableRows::ProjectHistory(vec![ProjectHistoryRow {
                    owner: 100_000,
                    project: 0,
                    prototype_reward_handle: 0x4c17,
                    option_handle: 0x4c17,
                    date: 101,
                    ..Default::default()
                }]));
        }
    }
    let source = opened(frames);
    let manifest = load(&source, None, None);
    assert_eq!(
        manifest
            .windows
            .iter()
            .map(|window| window.count)
            .sum::<u32>(),
        3
    );
    assert!(
        manifest
            .unavailable
            .iter()
            .all(|entry| !entry.kind.starts_with("technology"))
    );
    let cached = source.state::<Cache>();
    let feed = Arc::clone(&cached.lock().unwrap().branches[0].1.feed);
    let reply = load(&source, Some(100), Some(102));
    assert_eq!(
        reply
            .rows
            .iter()
            .map(|row| (row.kind, row.hour, row.read_hour, row.exact))
            .collect::<Vec<_>>(),
        [
            (EventFeedKind::FocusCompleted, 101, 101, false),
            (EventFeedKind::ProjectFinished, 101, 102, true),
            (EventFeedKind::NukesIncreased, 102, 102, false),
        ]
    );
    assert!(
        reply
            .items
            .iter()
            .all(|item| item.token.as_deref() != Some("mod_tech"))
    );
    assert!(Arc::ptr_eq(
        &feed,
        &cached.lock().unwrap().branches[0].1.feed
    ));
}

#[test]
fn event_feed_missing_capture_is_explicit_and_window_validation_is_bounded() {
    let source = opened(vec![source(1, 100, Some(1))]);
    let index = load(&source, None, None);
    assert!(index.rows.is_empty());
    for kind in [
        "warStarted",
        "capitulated",
        "ceasedToExist",
        "factionJoined",
        "guaranteeGranted",
        "becameSubject",
        "rulingPartyChanged",
        "lawChanged",
        "focusCompleted",
        "projectFinished",
        "nukesIncreased",
        "nuclearDrops",
    ] {
        assert!(
            index.unavailable.iter().any(|entry| entry.kind == kind),
            "{kind}"
        );
    }
    assert!(
        EventFeed::validate(&EventFeedParams {
            branch_id: "1".into(),
            first_hour: Some(101),
            last_hour: None,
        })
        .is_err()
    );
    assert!(
        EventFeed::load(
            &source,
            &EventFeedParams {
                branch_id: "1".into(),
                first_hour: Some(99),
                last_hour: Some(100),
            },
            &LoadContext { content: None },
            &AtomicBool::new(false)
        )
        .unwrap_err()
        .to_string()
        .contains("outside")
    );
}

#[test]
fn event_feed_exact_dates_keep_discovery_hour_and_sampled_changes_are_upper_bounds() {
    let mut frames: Vec<_> = (100..=103).map(|hour| frame(hour, 3)).collect();
    for sample in &mut frames {
        let hour = sample.hour;
        if (101..103).contains(&hour) {
            *sample.tables[TableId::WarRelations.index()] =
                TableView::Complete(TableRows::WarRelations(vec![WarRelationRow {
                    first_country: 100_000,
                    second_country: 100_001,
                    start_hour: 101,
                    first_was_instigator: 1,
                    ..Default::default()
                }]));
        }
        if hour >= 102 {
            *sample.tables[TableId::Capitulation.index()] =
                TableView::Complete(TableRows::Capitulation(vec![CapitulationRow {
                    country: 100_001,
                    capitulated: 1,
                    capitulated_hour: 101,
                    ..Default::default()
                }]));
            *sample.tables[TableId::Subjects.index()] =
                TableView::Complete(TableRows::Subjects(vec![SubjectRow {
                    overlord: 100_000,
                    subject: 100_001,
                    start_hour: 101,
                    ..Default::default()
                }]));
            *sample.tables[TableId::RelationTuples.index()] =
                TableView::Complete(TableRows::RelationTuples(vec![RelationTupleRow {
                    owner_country: 100_000,
                    target_country: 100_001,
                    type_id: engine::cguarantee_relation::TYPE,
                    first_country: 100_000,
                    second_country: 100_001,
                    start_hour: 101,
                    ..Default::default()
                }]));
            *sample.tables[TableId::SliceReads.index()] =
                TableView::Complete(TableRows::SliceReads(vec![SliceReadRow {
                    table: TableId::RelationTuples.index() as u32,
                    owner: 100_000,
                    sample_hour: hour,
                }]));
        }
        if hour == 101 || hour == 102 {
            *sample.tables[TableId::Factions.index()] =
                TableView::Complete(TableRows::Factions(vec![FactionRow {
                    id: 5,
                    name: b"Recorded pact".to_vec(),
                    ..Default::default()
                }]));
            *sample.tables[TableId::FactionMembers.index()] =
                TableView::Complete(TableRows::FactionMembers(vec![FactionMemberRow {
                    country: 100_000,
                    faction: 5,
                    ordinal: 0,
                }]));
        }
    }
    let source = opened(frames);
    let reply = load(&source, Some(100), Some(103));
    for (kind, hour, read, exact) in [
        (EventFeedKind::WarStarted, 101, 101, true),
        (EventFeedKind::Capitulated, 101, 102, true),
        (EventFeedKind::BecameSubject, 101, 102, true),
        (EventFeedKind::GuaranteeGranted, 101, 102, true),
        (EventFeedKind::FactionJoined, 101, 101, false),
        (EventFeedKind::FactionLeft, 103, 103, false),
        (EventFeedKind::Peace, 103, 103, false),
    ] {
        let event = reply.rows.iter().find(|row| row.kind == kind).unwrap();
        assert_eq!(
            (event.hour, event.read_hour, event.name_hour, event.exact),
            (
                hour,
                read,
                if kind == EventFeedKind::FactionLeft {
                    hour - 1
                } else {
                    hour
                },
                exact
            )
        );
    }
    let index = load(&source, None, None);
    assert_eq!(
        index
            .windows
            .iter()
            .map(|window| window.count as usize)
            .sum::<usize>(),
        reply.rows.len()
    );
    assert!(reply.windows.is_empty());
    assert_eq!(source.state::<Cache>().lock().unwrap().branches.len(), 1);
}

#[test]
fn event_feed_renames_do_not_disappear_and_missing_hours_do_not_invent_changes() {
    let mut renamed = frame(101, 2);
    if let TableView::Complete(TableRows::Countries(rows)) =
        &mut *renamed.tables[TableId::Countries.index()]
    {
        rows[0].tag = b"new opaque tag".to_vec();
    }
    let mut disappeared = renamed.clone();
    disappeared.hour = 102;
    if let TableView::Complete(TableRows::CountryCounterInputs(rows)) =
        &mut *disappeared.tables[TableId::CountryCounterInputs.index()]
    {
        rows[0].owned_state_count = 0;
    }
    let source = opened(vec![frame(100, 2), renamed, disappeared]);
    let reply = load(&source, Some(100), Some(102));
    let events: Vec<_> = reply
        .rows
        .iter()
        .filter(|row| row.kind == EventFeedKind::CeasedToExist)
        .collect();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].tags, ["new opaque tag"]);
    assert_eq!(
        (events[0].hour, events[0].name_hour, events[0].exact),
        (102, 101, false)
    );
    let mut later = frame(103, 1);
    *later.tables[TableId::Factions.index()] =
        TableView::Complete(TableRows::Factions(vec![FactionRow {
            id: 5,
            name: b"Changed after gap".to_vec(),
            ..Default::default()
        }]));
    *later.tables[TableId::FactionMembers.index()] =
        TableView::Complete(TableRows::FactionMembers(vec![FactionMemberRow {
            country: 100_000,
            faction: 5,
            ordinal: 0,
        }]));
    let gap = opened(vec![frame(100, 2), later]);
    assert!(load(&gap, Some(100), Some(103)).rows.is_empty());
}

#[test]
fn event_feed_subject_rename_and_link_alias_do_not_start_another_relation() {
    let first = frame(100, 2);
    let mut relation = frame(101, 2);
    *relation.tables[TableId::Subjects.index()] =
        TableView::Complete(TableRows::Subjects(vec![SubjectRow {
            overlord: 100_000,
            subject: 100_001,
            start_hour: 101,
            ..Default::default()
        }]));
    let mut renamed = relation.clone();
    renamed.hour = 102;
    if let TableView::Complete(TableRows::Countries(rows)) =
        &mut *renamed.tables[TableId::Countries.index()]
    {
        rows[1].tag = b"renamed subject".to_vec();
    }
    let mut alias = renamed.clone();
    alias.hour = 103;
    if let TableView::Complete(TableRows::CountryLinks(rows)) =
        &mut *alias.tables[TableId::CountryLinks.index()]
    {
        rows.push(CountryLinkRow {
            id: 200_001,
            canonical_slot: 2,
        });
    }
    *alias.tables[TableId::Subjects.index()] =
        TableView::Complete(TableRows::Subjects(vec![SubjectRow {
            overlord: 100_000,
            subject: 200_001,
            start_hour: 101,
            ..Default::default()
        }]));
    let source = opened(vec![first, relation, renamed, alias]);
    let reply = load(&source, Some(100), Some(103));
    assert_eq!(
        reply
            .rows
            .iter()
            .filter(|row| row.kind == EventFeedKind::BecameSubject)
            .count(),
        1
    );

    // An already-running relation on the initial snapshot remains historical
    // after its country's tag changes; a new country under that tag is another identity.
    let mut initial = frame(100, 2);
    *initial.tables[TableId::Subjects.index()] =
        TableView::Complete(TableRows::Subjects(vec![SubjectRow {
            overlord: 100_000,
            subject: 100_001,
            start_hour: 90,
            ..Default::default()
        }]));
    let mut renamed = initial.clone();
    renamed.hour = 101;
    if let TableView::Complete(TableRows::Countries(rows)) =
        &mut *renamed.tables[TableId::Countries.index()]
    {
        rows[1].tag = b"renamed subject".to_vec();
    }
    let mut reused = renamed.clone();
    reused.hour = 102;
    if let TableView::Complete(TableRows::Countries(rows)) =
        &mut *reused.tables[TableId::Countries.index()]
    {
        rows[1].id = 300_001;
    }
    if let TableView::Complete(TableRows::CountryLinks(rows)) =
        &mut *reused.tables[TableId::CountryLinks.index()]
    {
        rows.push(CountryLinkRow {
            id: 300_001,
            canonical_slot: 1,
        });
    }
    let source = opened(vec![initial, renamed, reused]);
    assert!(
        load(&source, Some(100), Some(102))
            .rows
            .iter()
            .all(|row| row.kind != EventFeedKind::BecameSubject)
    );
}

#[test]
fn event_feed_nuclear_increase_uses_positive_displayed_stockpile_without_drop_inference() {
    let make = |hour, amount| {
        let mut sample = frame(hour, 1);
        sample.metrics.table_ns[TableId::Nukes.index()] = 1;
        *sample.tables[TableId::Nukes.index()] = TableView::Complete(TableRows::Nukes(vec![
            NukeRow {
                country: 100_000,
                nuke_type: 0,
                amount_raw: amount,
                ..Default::default()
            },
            NukeRow {
                country: 100_000,
                nuke_type: 1,
                amount_raw: 500_000_000,
                ..Default::default()
            },
        ]));
        sample
    };
    let source = opened(vec![make(100, 1_000_000_000), make(101, 2_500_000_000)]);
    let stockpile = country_nukes::scanned_branch(&source, 1, &AtomicBool::new(false)).unwrap();
    let runs = stockpile.series.values().next().unwrap();
    assert_eq!((runs[0].total, runs[1].total), (Some(15), Some(30)));
    let reply = load(&source, Some(100), Some(101));
    let events: Vec<_> = reply
        .rows
        .iter()
        .filter(|row| row.kind == EventFeedKind::NukesIncreased)
        .collect();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].amount, Some(15));
    assert_eq!(
        (events[0].hour, events[0].read_hour, events[0].exact),
        (101, 101, false)
    );
    assert!(
        load(&source, None, None)
            .unavailable
            .iter()
            .any(|missing| missing.kind == "nuclearDrops")
    );
}

#[test]
fn event_feed_five_thousand_countries_are_all_included_without_controller_status() {
    let first = frame(100, 5000);
    let mut last = frame(101, 5000);
    *last.tables[TableId::Nukes.index()] = TableView::Complete(TableRows::Nukes(
        (0..5000)
            .map(|index| NukeRow {
                country: 100_000 + index,
                amount_raw: 100_000_000,
                ..Default::default()
            })
            .collect(),
    ));
    let source = opened(vec![first, last]);
    let reply = load(&source, Some(100), Some(101));
    assert_eq!(reply.rows.len(), 5000);
    assert!(
        reply
            .rows
            .iter()
            .all(|row| row.kind == EventFeedKind::NukesIncreased
                && row.amount == Some(1)
                && !row.exact)
    );
    assert_eq!(
        reply
            .rows
            .iter()
            .map(|row| &row.tags[0])
            .collect::<BTreeSet<_>>()
            .len(),
        5000
    );
    assert!(crate::transport::json_len(&reply, MAX_PRODUCT_REPLY_BYTES).is_some());
}

#[test]
fn event_feed_windows_keep_all_rows_under_small_budget_and_refuse_oversized_hour() {
    let mut builder = Builder::new(0, 399);
    let item = builder
        .item(plain_item(
            Some("mod_art".into()),
            Some("Mod artwork".into()),
            Some("x".repeat(64)),
            None,
        ))
        .unwrap();
    for hour in 0..400 {
        builder.row(
            EventFeedKind::FocusCompleted,
            RowDate::by(hour),
            vec!["opaque".into()],
            Some(item),
            None,
        );
    }
    builder.feed.windows = windows(&builder.feed, 4096).unwrap();
    assert!(builder.feed.windows.len() > 1);
    let mut count = 0;
    for window in &builder.feed.windows {
        let page = reply(
            &builder.feed,
            "1",
            window.first_hour,
            window.last_hour,
            true,
        );
        assert!(crate::transport::json_len(&page, 4096).is_some());
        assert_eq!(page.items.len(), 1);
        assert!(page.rows.iter().all(|row| row.item == Some(0)));
        count += page.rows.len();
    }
    assert_eq!(count, 400);
    assert!(!window_fits(&builder.feed, "1", 0, 399, 4096));
    let mut one = Builder::new(100, 100);
    let item = one
        .item(plain_item(
            Some("large_mod_icon".into()),
            None,
            Some("x".repeat(8192)),
            None,
        ))
        .unwrap();
    one.row(
        EventFeedKind::FocusCompleted,
        RowDate::by(100),
        vec!["opaque".into()],
        Some(item),
        None,
    );
    assert!(
        windows(&one.feed, 4096)
            .unwrap_err()
            .to_string()
            .contains("large_mod_icon")
    );
}

#[test]
fn event_feed_cache_eviction_uses_last_access_not_branch_order() {
    let mut cache = Cache::default();
    let seed = CachedBranch {
        catalogue: None,
        feed: Arc::new(Builder::new(0, 1).finish().unwrap()),
    };
    for branch in 1..=4 {
        cache.put(branch, seed.clone());
    }
    cache.get(1, &None).unwrap();
    cache.put(5, seed);
    assert_eq!(
        cache.branches.iter().map(|(id, _)| *id).collect::<Vec<_>>(),
        [3, 4, 1, 5]
    );
}

#[test]
fn event_feed_standing_refusals_are_bounded_counted_and_manifest_only() {
    let mut builder = Builder::new(100, 101);
    for hour in 0..10_000 {
        builder.sampled_unavailable(
            "gap",
            "Gaps",
            format!("A pair was unresolved at {hour}: {}", "x".repeat(1000)),
            1,
        );
    }
    builder.feed.warnings.push("Standing content notice".into());
    builder.row(
        EventFeedKind::Peace,
        RowDate::by(101),
        vec!["opaque".into()],
        None,
        None,
    );
    let feed = builder.finish().unwrap();
    assert_eq!(feed.unavailable.len(), 1);
    assert_eq!(feed.unavailable[0].count, Some(10_000));
    assert!(feed.unavailable[0].reason.len() < 1024);
    let manifest = reply(&feed, "1", 100, 101, false);
    assert_eq!(manifest.unavailable.len(), 1);
    let page = reply(&feed, "1", 100, 101, true);
    assert!(page.unavailable.is_empty());
    assert!(page.warnings.is_empty());
    assert!(window_fits(&feed, "1", 100, 101, 1024));
}

#[test]
fn event_feed_item_dedup_uses_content_equality_without_serializing_artwork() {
    let mut builder = Builder::new(0, 0);
    let item = plain_item(
        Some("opaque".into()),
        Some("Recorded name".into()),
        Some("first artwork".into()),
        None,
    );
    assert_eq!(builder.item(item.clone()).unwrap(), 0);
    assert_eq!(builder.item(item.clone()).unwrap(), 0);
    let mut changed = item;
    changed.icon = Some("changed artwork".into());
    assert_eq!(builder.item(changed).unwrap(), 1);
    assert_eq!(builder.feed.items.len(), 2);
}

#[test]
fn event_feed_fixed_reasons_keep_separate_counts_and_standing_notices() {
    let mut builder = Builder::new(100, 100);
    builder.unavailable(
        "focusCompleted",
        "Focus completions",
        "Required tables incomplete",
        None,
    );
    builder.unavailable(
        "focusCompleted",
        "Focus completions",
        "Required tables incomplete",
        Some(2),
    );
    builder.unavailable(
        "focusCompleted",
        "Focus completions",
        "Definition token missing",
        Some(3),
    );
    builder.unavailable(
        "focusCompleted",
        "Focus completions",
        "Definition token missing",
        Some(4),
    );
    for reason in [
        "Content form A",
        "Content form B",
        "Content form C",
        "Content form D",
    ] {
        builder.unavailable(
            "contentPresentation",
            "Content presentation",
            reason,
            Some(1),
        );
    }
    let reasons = &builder.feed.unavailable;
    assert_eq!(reasons.len(), 7);
    assert!(
        reasons
            .iter()
            .any(|entry| entry.reason == "Required tables incomplete" && entry.count.is_none())
    );
    assert!(
        reasons
            .iter()
            .any(|entry| entry.reason == "Required tables incomplete" && entry.count == Some(2))
    );
    assert!(
        reasons
            .iter()
            .any(|entry| entry.reason == "Definition token missing" && entry.count == Some(7))
    );
    assert_eq!(
        reasons
            .iter()
            .filter(|entry| entry.kind == "contentPresentation")
            .map(|entry| entry.count.unwrap())
            .sum::<u32>(),
        4
    );
}
