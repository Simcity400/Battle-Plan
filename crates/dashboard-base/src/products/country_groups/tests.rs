use crate::{
    products::Product,
    test_fixtures::{CAMPAIGN, continuation_records, ownership_frame, records},
};
use replay::container::Replay;
use replay_data::{rows::TableId, rows::TableRows};
use replay_frame::{frame::DecodedFrame, frame::TableView};
use std::{
    path::PathBuf,
    sync::{Arc, atomic::AtomicBool},
};

fn group_source(replay: Replay) -> crate::Source {
    crate::Source::from_replay(
        crate::SourceKey {
            source_id: "groups-test".into(),
            revision: "1".into(),
        },
        PathBuf::from("synthetic.battleplan"),
        Arc::new(replay),
        &AtomicBool::new(false),
    )
    .unwrap()
}

#[test]
fn country_groups_keep_every_event_and_run_then_refuse_oversized_replies() {
    use super::{CountryGroupParams, CountryGroups, scan_branch};
    use crate::{
        products::{LoadContext, Product},
        test_fixtures::scale_frame,
    };
    for entries in [100_001, 500_001] {
        let countries = 1_100;
        let mut frame = scale_frame(1, 100, countries);
        let wars = (0..countries)
            .flat_map(|first| {
                (first + 1..countries).map(move |second| replay_data::rows::WarRelationRow {
                    first_country: 100_000 + first,
                    second_country: 100_000 + second,
                    start_hour: 100,
                    ..Default::default()
                })
            })
            .take(entries)
            .collect();
        *frame.tables[TableId::WarRelations.index()] =
            TableView::Complete(TableRows::WarRelations(wars));
        let source = group_source(
            Replay::from_records(CAMPAIGN.into(), &[records(1, vec![frame])]).unwrap(),
        );
        let cancelled = AtomicBool::new(false);
        let scan = Arc::new(scan_branch(source.replay_ref(), 1, &cancelled).unwrap());
        assert_eq!(scan.wars.len(), entries);
        assert_eq!(scan.war_starts.len(), entries);
        assert!(
            !scan
                .warnings
                .iter()
                .any(|warning| warning.contains("exceed"))
        );
        source
            .append_state::<super::GroupCache>()
            .lock()
            .unwrap()
            .entries
            .push((
                1,
                Arc::new(super::GroupBranch {
                    gate: std::sync::Mutex::new(()),
                    scan: std::sync::Mutex::new(Some(scan)),
                }),
            ));
        let reply = CountryGroups::load(
            &source,
            &CountryGroupParams {
                branch_id: "1".into(),
            },
            &LoadContext { content: None },
            &cancelled,
        );
        if entries == 100_001 {
            let reply = reply.unwrap();
            assert_eq!(reply.wars.len(), entries);
            assert_eq!(reply.war_starts.len(), entries);
            assert_eq!(reply.country_identities.len(), countries as usize);
        } else {
            assert_eq!(
                reply.unwrap_err().to_string(),
                "The complete country group reply is too large; nothing was omitted"
            );
        }
    }
}

#[test]
fn country_groups_native_identity_metadata_alone_can_refuse_the_reply() {
    use super::{CountryGroupParams, CountryGroups, GroupBranch, GroupCache, RunHistory};
    use crate::products::{LoadContext, MAX_PRODUCT_REPLY_BYTES, Product};

    let replay = Replay::from_records(
        CAMPAIGN.into(),
        &[records(1, vec![group_frame(100, &[], &[], &[])])],
    )
    .unwrap();
    let cancelled = AtomicBool::new(false);
    let mut scan = super::scan_branch(&replay, 1, &cancelled).unwrap();
    scan.identities.insert(
        "x".repeat(MAX_PRODUCT_REPLY_BYTES),
        RunHistory::from([super::CountryIdentityRun {
            first_hour: 100,
            last_hour: 100,
            slot: 7,
            country_id: 9001,
        }]),
    );
    let source = group_source(replay);
    source
        .append_state::<GroupCache>()
        .lock()
        .unwrap()
        .entries
        .push((
            1,
            Arc::new(GroupBranch {
                gate: std::sync::Mutex::new(()),
                scan: std::sync::Mutex::new(Some(Arc::new(scan))),
            }),
        ));
    assert_eq!(
        CountryGroups::load(
            &source,
            &CountryGroupParams {
                branch_id: "1".into(),
            },
            &LoadContext { content: None },
            &cancelled,
        )
        .unwrap_err()
        .to_string(),
        "The complete country group reply is too large; nothing was omitted"
    );
}

#[test]
fn country_groups_rescan_evicted_branches_without_losing_reply_data() {
    use super::{CACHED_BRANCHES, CountryGroupParams, CountryGroups, GroupCache};
    use crate::products::{LoadContext, Product};
    let branches = (1..=CACHED_BRANCHES as u64 + 1)
        .map(|branch| {
            let mut frame = group_frame(100, &[9001], &[(5, "Pact", &[9001, 44])], &[(44, 9001)]);
            frame.branch = branch;
            records(branch, vec![frame])
        })
        .collect::<Vec<_>>();
    let source = group_source(Replay::from_records(CAMPAIGN.into(), &branches).unwrap());
    let load = |branch: u64| {
        CountryGroups::load(
            &source,
            &CountryGroupParams {
                branch_id: branch.to_string(),
            },
            &LoadContext { content: None },
            &AtomicBool::new(false),
        )
        .unwrap()
    };
    let first = serde_json::to_value(load(1)).unwrap();
    let cache = source.append_state::<GroupCache>();
    let original = cache.lock().unwrap().entries[0]
        .1
        .scan
        .lock()
        .unwrap()
        .clone()
        .unwrap();
    for branch in 2..=CACHED_BRANCHES as u64 + 1 {
        load(branch);
    }
    {
        let cached = cache.lock().unwrap();
        assert_eq!(cached.entries.len(), CACHED_BRANCHES);
        assert!(cached.entries.iter().all(|(branch, _)| *branch != 1));
    }
    assert_eq!(serde_json::to_value(load(1)).unwrap(), first);
    let cached = cache.lock().unwrap();
    let rescanned = cached
        .entries
        .iter()
        .find(|(branch, _)| *branch == 1)
        .unwrap()
        .1
        .scan
        .lock()
        .unwrap()
        .clone()
        .unwrap();
    assert!(!Arc::ptr_eq(&original, &rescanned));
}

#[test]
fn country_groups_append_shares_closed_run_chunks_and_preserves_prior_reads() {
    use super::{scan_branch, scan_from};
    use std::sync::Arc;
    let cancelled = AtomicBool::new(false);
    let first = Replay::from_records(
        CAMPAIGN.into(),
        &[records(1, vec![group_frame(100, &[9001], &[], &[])])],
    )
    .unwrap();
    let second = first
        .append(&[continuation_records(
            &first,
            1,
            vec![group_frame(101, &[44], &[], &[])],
        )])
        .unwrap();
    let prior = scan_branch(&second, 1, &cancelled).unwrap();
    let next = second
        .append(&[continuation_records(
            &second,
            1,
            vec![group_frame(102, &[44], &[], &[])],
        )])
        .unwrap();
    let extended = scan_from(&next, 1, &cancelled, Some(&prior)).unwrap();
    assert!(Arc::ptr_eq(
        &prior.humans.segments[&0],
        &extended.humans.segments[&0]
    ));
    assert_eq!(prior.humans()[1].last_hour, 101);
    assert_eq!(extended.humans()[1].last_hour, 102);
    assert_eq!(
        extended.humans(),
        scan_branch(&next, 1, &cancelled).unwrap().humans()
    );
}

#[test]
fn country_groups_faction_name_runs_append_without_changing_prior_names() {
    use super::{scan_branch, scan_from};
    let cancelled = AtomicBool::new(false);
    let first = Replay::from_records(
        CAMPAIGN.into(),
        &[records(
            1,
            vec![group_frame(100, &[], &[(7, "First", &[9001])], &[])],
        )],
    )
    .unwrap();
    let prior = scan_branch(&first, 1, &cancelled).unwrap();
    let next = first
        .append(&[continuation_records(
            &first,
            1,
            vec![group_frame(101, &[], &[(7, "Renamed", &[9001])], &[])],
        )])
        .unwrap();
    let extended = scan_from(&next, 1, &cancelled, Some(&prior)).unwrap();
    assert_eq!(prior.faction_name_at(7, 100).as_deref(), Some("First"));
    assert_eq!(extended.faction_name_at(7, 100).as_deref(), Some("First"));
    assert_eq!(extended.faction_name_at(7, 101).as_deref(), Some("Renamed"));
}

/// Countries GER (link 9001), FRA (44), CTL (55) and MOD (66), with the given
/// human countries, factions (id, name, members) and wars at this hour.
fn group_frame(
    hour: i32,
    humans: &[u32],
    factions: &[(u64, &str, &[u32])],
    wars: &[(u32, u32)],
) -> DecodedFrame {
    let mut frame = ownership_frame(1, hour, 44);
    let mut humans: Vec<_> = humans
        .iter()
        .map(|&country| replay_data::rows::HumanControllerRow { country, count: 1 })
        .collect();
    humans.sort_by_key(|row| row.country);
    *frame.tables[TableId::HumanControllers.index()] =
        TableView::Complete(TableRows::HumanControllers(humans));
    *frame.tables[TableId::Factions.index()] = TableView::Complete(TableRows::Factions(
        factions
            .iter()
            .map(|&(id, name, _)| replay_data::rows::FactionRow {
                id,
                name: name.as_bytes().to_vec(),
                ..Default::default()
            })
            .collect(),
    ));
    let mut members: Vec<_> = factions
        .iter()
        .flat_map(|&(faction, _, members)| {
            members.iter().enumerate().map(move |(ordinal, &country)| {
                replay_data::rows::FactionMemberRow {
                    country,
                    faction,
                    ordinal: ordinal as u32,
                }
            })
        })
        .collect();
    members.sort_by_key(|member| member.country);
    *frame.tables[TableId::FactionMembers.index()] =
        TableView::Complete(TableRows::FactionMembers(members));
    let mut relations: Vec<_> = wars
        .iter()
        .map(
            |&(first_country, second_country)| replay_data::rows::WarRelationRow {
                first_country,
                second_country,
                ..Default::default()
            },
        )
        .collect();
    relations.sort_by_key(|row| (row.first_country, row.second_country));
    *frame.tables[TableId::WarRelations.index()] =
        TableView::Complete(TableRows::WarRelations(relations));
    // As recorded: the capitulation table is present, here with no one capitulated.
    *frame.tables[TableId::Capitulation.index()] =
        TableView::Complete(TableRows::Capitulation(Vec::new()));
    frame
}

#[test]
fn country_groups_follow_joins_leaves_new_factions_and_break_at_gaps() {
    use crate::products::country_groups::{
        HourSpan, MembershipRun, UnobservedHours, WarPairRun, scan_branch,
    };
    let run = |tag: &str, first_hour: i32, last_hour: i32| MembershipRun {
        tag: tag.into(),
        first_hour,
        last_hour,
    };
    let frames = vec![
        group_frame(100, &[9001], &[(5, "AXIS_NAME", &[9001])], &[]),
        group_frame(
            101,
            &[9001, 44],
            &[(5, "AXIS_NAME", &[9001, 66])],
            &[(44, 9001)],
        ),
        group_frame(
            102,
            &[9001, 44],
            &[(5, "AXIS_NAME", &[9001, 66]), (6, "Custom Pact", &[44])],
            // The same war listed from both sides in one hour is still one run.
            &[(44, 9001), (9001, 44)],
        ),
        // Hour 103 is missing: every run ends at 102.
        group_frame(
            104,
            &[9001],
            &[(6, "Custom Pact", &[44, 55])],
            &[(44, 9001)],
        ),
    ];
    let replay = Replay::from_records(CAMPAIGN.into(), &[records(1, frames)]).unwrap();
    let scan = scan_branch(&replay, 1, &AtomicBool::new(false)).unwrap();
    assert_eq!(
        scan.humans(),
        [
            run("GER", 100, 102),
            run("FRA", 101, 102),
            run("GER", 104, 104)
        ]
    );
    let (name, first, last, members) = scan.faction(5).unwrap();
    assert_eq!((name, first, last), ("AXIS_NAME", 100, 102));
    assert_eq!(members, [run("GER", 100, 102), run("MOD", 101, 102)]);
    let (name, first, last, members) = scan.faction(6).unwrap();
    assert_eq!((name, first, last), ("Custom Pact", 102, 104));
    assert_eq!(
        members,
        [
            run("FRA", 102, 102),
            run("FRA", 104, 104),
            run("CTL", 104, 104)
        ]
    );
    let pair = |first_hour, last_hour| WarPairRun {
        first: "FRA".into(),
        second: "GER".into(),
        first_hour,
        last_hour,
    };
    assert_eq!(scan.wars(), [pair(101, 102), pair(104, 104)]);
    // The missing hour is listed, so its breaks are not read as joins or leaves.
    let missing = vec![HourSpan {
        first_hour: 103,
        last_hour: 103,
    }];
    assert_eq!(
        scan.unobserved(),
        &UnobservedHours {
            humans: missing.clone(),
            factions: missing.clone(),
            wars: missing
        }
    );
    assert!(scan.warnings().is_empty(), "{:?}", scan.warnings());
}

#[test]
fn country_groups_drop_ended_wars_and_keep_each_history_s_gaps_its_own() {
    use crate::products::country_groups::{HourSpan, MembershipRun, WarPairRun, scan_branch};
    let mut ended = group_frame(101, &[9001, 44], &[], &[(44, 9001)]);
    if let TableView::Complete(TableRows::WarRelations(rows)) =
        &mut *ended.tables[TableId::WarRelations.index()]
    {
        // Recorded as ending at hour 101: no longer at war from that hour.
        rows[0].start_hour = 50;
        rows[0].end_hour = 101;
    }
    *ended.tables[TableId::Factions.index()] = TableView::Unavailable {
        reason: "fixture".into(),
    };
    let frames = vec![
        group_frame(100, &[9001], &[], &[(44, 9001)]),
        ended,
        group_frame(102, &[9001, 44], &[], &[]),
    ];
    let mut replay = Replay::from_records(CAMPAIGN.into(), &[records(1, frames)]).unwrap();
    // The branch ends in a recorded gap: those hours are unobserved, not leaves.
    replay.gaps.push(replay_frame::branches::Gap {
        branch: 1,
        first_hour: 103,
        last_hour: 105,
        reason: "fixture".into(),
    });
    let scan = scan_branch(&replay, 1, &AtomicBool::new(false)).unwrap();
    assert_eq!(
        scan.wars(),
        [WarPairRun {
            first: "FRA".into(),
            second: "GER".into(),
            first_hour: 100,
            last_hour: 100
        }]
    );
    // FRA's human join at 101 is not hidden by the factions table missing that hour.
    assert_eq!(
        scan.humans(),
        [
            MembershipRun {
                tag: "GER".into(),
                first_hour: 100,
                last_hour: 102
            },
            MembershipRun {
                tag: "FRA".into(),
                first_hour: 101,
                last_hour: 102
            }
        ]
    );
    let span = |first_hour, last_hour| HourSpan {
        first_hour,
        last_hour,
    };
    assert_eq!(scan.unobserved().humans, [span(103, 105)]);
    assert_eq!(scan.unobserved().wars, [span(103, 105)]);
    assert_eq!(scan.unobserved().factions, [span(101, 101), span(103, 105)]);
}

#[test]
fn country_groups_leave_out_unknown_countries() {
    use crate::products::country_groups::scan_branch;
    let replay = Replay::from_records(
        CAMPAIGN.into(),
        &[records(
            1,
            vec![group_frame(100, &[777], &[(5, "", &[9001, 888])], &[])],
        )],
    )
    .unwrap();
    let scan = scan_branch(&replay, 1, &AtomicBool::new(false)).unwrap();
    assert!(scan.humans().is_empty());
    assert_eq!(scan.faction(5).unwrap().3.len(), 1);
    // The lists that named an unknown country are incomplete that hour: the humans
    // list, and that faction's own roster (not every faction's); the wars list was complete.
    let hour = [crate::products::country_groups::HourSpan {
        first_hour: 100,
        last_hour: 100,
    }];
    assert_eq!(scan.unobserved().humans, hour);
    assert!(scan.unobserved().factions.is_empty());
    assert_eq!(scan.faction_incomplete(5), Some(&hour[..]));
    assert!(scan.unobserved().wars.is_empty());
    assert!(
        scan.warnings()
            .iter()
            .any(|w| w.starts_with("2 recorded country references"))
    );
}

fn template_frame(hour: i32, literal: &str, template: Option<(u32, &str)>) -> DecodedFrame {
    let mut frame = group_frame(hour, &[], &[(5, literal, &[9001])], &[]);
    if let TableView::Complete(TableRows::Factions(rows)) =
        &mut *frame.tables[TableId::Factions.index()]
    {
        rows[0].template_token = template.map(|(token, _)| token);
    }
    *frame.tables[TableId::LexicalTokens.index()] = TableView::Complete(TableRows::LexicalTokens(
        template
            .map(|(token, literal)| replay_data::rows::LexicalTokenRow {
                token,
                literal: literal.as_bytes().to_vec(),
            })
            .into_iter()
            .collect(),
    ));
    frame
}

fn named_factions(
    frames: Vec<DecodedFrame>,
    templates: &installed_content_core::faction_templates::Catalog,
    localisation: &installed_content_localisation::localisation::Localisation,
) -> Vec<super::FactionGroup> {
    let replay = Replay::from_records(CAMPAIGN.into(), &[records(1, frames)]).unwrap();
    let scan = super::scan_branch(&replay, 1, &AtomicBool::new(false)).unwrap();
    scan.faction_groups(
        Ok(templates),
        localisation,
        "english",
        &mut crate::catalogue::Refusals::default(),
    )
    .into_iter()
    .map(super::FactionGroupView::into_owned)
    .collect()
}

#[test]
fn country_groups_faction_names_resolve_templates_and_literal_wins() {
    let templates = crate::catalogue::tests::loaded_templates("demo = { name = DEMO_NAME }");
    let localisation = crate::catalogue::tests::loaded(
        "l_english:\n DEMO_NAME:0 \"Demo Pact\"\n Literal:0 \"Do not translate\"\n",
    );
    let factions = named_factions(
        vec![
            template_frame(100, "", Some((17, "demo"))),
            template_frame(101, "Literal", None),
        ],
        &templates,
        &localisation,
    );
    assert_eq!(factions[0].names[0].name.as_deref(), Some("Demo Pact"));
    assert_eq!(factions[0].names[1].name.as_deref(), Some("Literal"));
    assert_eq!(factions[0].name.as_deref(), Some("Literal"));
    assert_eq!(factions[0].name_reason, None);
    assert!(factions[0].names.iter().all(|run| run.reason.is_none()));
}

#[test]
fn country_groups_faction_names_keep_plain_localisation_controls() {
    let templates = crate::catalogue::tests::loaded_templates(
        "scoped = { name = SCOPED_NAME } price = { name = PRICE_NAME }",
    );
    let localisation = crate::catalogue::tests::loaded(concat!(
        "l_english:\n",
        " SCOPED_NAME:0 \"[ROOT.GetAdjective] Pact\"\n",
        " PRICE_NAME:0 \"Pakt $$ ]\"\n",
    ));
    let factions = named_factions(
        vec![
            template_frame(100, "", Some((17, "scoped"))),
            template_frame(101, "", Some((18, "price"))),
            template_frame(102, "Pacte d'Acier", None),
        ],
        &templates,
        &localisation,
    );
    let names = &factions[0].names;
    assert_eq!(names[0].name.as_deref(), Some("[ROOT.GetAdjective] Pact"));
    assert_eq!(names[1].name.as_deref(), Some("Pakt $ ]"));
    assert_eq!(names[2].name.as_deref(), Some("Pacte d'Acier"));
    assert!(names.iter().all(|run| run.reason.is_none()));
}

#[test]
fn country_groups_faction_names_display_undefined_localisation_keys_as_their_text() {
    let templates = crate::catalogue::tests::loaded_templates("demo = { name = UNDEFINED_NAME }");
    let localisation = crate::catalogue::tests::loaded("l_english:\n OTHER:0 \"Other\"\n");
    let factions = named_factions(
        vec![template_frame(100, "", Some((17, "demo")))],
        &templates,
        &localisation,
    );
    assert_eq!(factions[0].name.as_deref(), Some("UNDEFINED_NAME"));
    assert_eq!(factions[0].name_reason, None);
    assert_eq!(factions[0].names[0].reason, None);
}

#[test]
fn country_groups_faction_names_distinguish_null_template_from_unread_content_and_dictionary() {
    let templates = crate::catalogue::tests::loaded_templates("demo = { name = DEMO_NAME }");
    let localisation = crate::catalogue::tests::loaded("l_english:\n DEMO_NAME:0 \"Demo Pact\"\n");
    let mut missing_dictionary = template_frame(101, "", Some((17, "demo")));
    *missing_dictionary.tables[TableId::LexicalTokens.index()] = TableView::Unavailable {
        reason: "not captured".into(),
    };
    let factions = named_factions(
        vec![
            template_frame(100, "", Some((18, "missing"))),
            missing_dictionary,
            template_frame(102, "", None),
        ],
        &templates,
        &localisation,
    );
    let names = &factions[0].names;
    assert_eq!(names.len(), 3);
    assert!(
        names[0]
            .reason
            .as_ref()
            .unwrap()
            .contains("not in the recorded content")
    );
    assert!(names[1].reason.as_ref().unwrap().contains("LexicalTokens"));
    assert_eq!(names[2].name.as_deref(), Some(""));
    assert_eq!(names[2].reason, None);
    assert!(names[..2].iter().all(|run| run.name.is_none()));
}

#[test]
fn country_groups_faction_names_follow_hour_operands_and_break_at_gaps() {
    let templates = crate::catalogue::tests::loaded_templates(
        "first = { name = FIRST_NAME } second = { name = SECOND_NAME }",
    );
    let localisation = crate::catalogue::tests::loaded(
        "l_english:\n FIRST_NAME:0 \"First Pact\"\n SECOND_NAME:0 \"Second Pact\"\n",
    );
    let mut missing_members = template_frame(101, "", Some((22, "second")));
    *missing_members.tables[TableId::FactionMembers.index()] = TableView::Unavailable {
        reason: "missing roster".into(),
    };
    let mut missing_links = template_frame(104, "", Some((22, "second")));
    *missing_links.tables[TableId::CountryLinks.index()] = TableView::Unavailable {
        reason: "missing links".into(),
    };
    let factions = named_factions(
        vec![
            template_frame(100, "", Some((21, "first"))),
            missing_members,
            template_frame(102, "", Some((22, "second"))),
            missing_links,
        ],
        &templates,
        &localisation,
    );
    assert_eq!((factions[0].first_hour, factions[0].last_hour), (100, 102));
    let names = &factions[0].names;
    assert_eq!(names.len(), 3);
    assert_eq!((names[0].first_hour, names[0].last_hour), (100, 100));
    assert_eq!((names[1].first_hour, names[1].last_hour), (101, 102));
    assert_eq!((names[2].first_hour, names[2].last_hour), (104, 104));
    assert_eq!(names[0].name.as_deref(), Some("First Pact"));
    assert_eq!(names[1].name.as_deref(), Some("Second Pact"));
    assert_eq!(names[2].name.as_deref(), Some("Second Pact"));
}

#[test]
fn country_groups_faction_names_refuse_invalid_literal_bytes() {
    let templates = crate::catalogue::tests::loaded_templates("demo = { name = DEMO_NAME }");
    let localisation = crate::catalogue::tests::loaded("l_english:\n DEMO_NAME:0 \"Demo Pact\"\n");
    let mut frame = template_frame(100, "", Some((17, "demo")));
    if let TableView::Complete(TableRows::Factions(rows)) =
        &mut *frame.tables[TableId::Factions.index()]
    {
        rows[0].name = vec![0xff];
    }
    let factions = named_factions(vec![frame], &templates, &localisation);
    assert_eq!(factions[0].name, None);
    assert_eq!(
        factions[0].name_reason.as_deref(),
        Some("faction name is not UTF-8")
    );
}

#[test]
fn country_groups_faction_names_use_mod_localisation_override() {
    let (_, vanilla) = crate::catalogue::tests::folder_catalogue(&[
        (
            "common/factions/templates/00_demo.txt",
            "demo = { name = DEMO_NAME }",
        ),
        (
            "localisation/demo_l_english.yml",
            "\u{feff}l_english:\n DEMO_NAME:0 \"Original Pact\"\n",
        ),
    ]);
    let (_, modded) = crate::catalogue::tests::folder_catalogue(&[(
        "localisation/demo_l_english.yml",
        "\u{feff}l_english:\n DEMO_NAME:0 \"Modded Pact\"\n",
    )]);
    let layers = installed_content_core::ResolvedLayers::from_folders(&[
        (&vanilla.0, &[][..]),
        (&modded.0, &[][..]),
    ])
    .unwrap();
    let catalogue = crate::catalogue::tests::layers_catalogue(layers);
    let factions = named_factions(
        vec![template_frame(100, "", Some((17, "demo")))],
        &catalogue.content.layers.faction_templates().unwrap(),
        &catalogue.localisation,
    );
    assert_eq!(factions[0].name.as_deref(), Some("Modded Pact"));
    assert_eq!(factions[0].name_reason, None);
}

#[test]
fn country_groups_faction_names_keep_known_empty_names_and_plain_text_refusals() {
    let templates = crate::catalogue::tests::loaded_templates(
        "unbound={} localised_empty={name=EMPTY_NAME} icon={name=ICON_NAME}",
    );
    let localisation = crate::catalogue::tests::loaded(
        "l_english:\n EMPTY_NAME:0 \"\"\n ICON_NAME:0 \"\u{a3}missing_icon\"\n",
    );
    let factions = named_factions(
        vec![
            template_frame(100, "", Some((17, "unbound"))),
            template_frame(101, "", Some((18, "localised_empty"))),
            template_frame(102, "", Some((19, "icon"))),
        ],
        &templates,
        &localisation,
    );
    assert_eq!(factions[0].names[0].name.as_deref(), Some(""));
    assert_eq!(factions[0].names[0].reason, None);
    assert_eq!(factions[0].names[0].first_hour, 100);
    assert_eq!(factions[0].names[0].last_hour, 101);
    assert_eq!(
        factions[0].names[1].reason.as_deref(),
        Some("Faction name has no plain display text")
    );
    assert_eq!(factions[0].names[1].name, None);
}

#[test]
fn country_groups_faction_names_keep_content_unavailable_reason_without_hiding_literals() {
    let replay = Replay::from_records(
        CAMPAIGN.into(),
        &[records(
            1,
            vec![
                template_frame(100, "", Some((17, "demo"))),
                template_frame(101, "My Pact", None),
            ],
        )],
    )
    .unwrap();
    let scan = super::scan_branch(&replay, 1, &AtomicBool::new(false)).unwrap();
    let mut localisation = installed_content_localisation::localisation::Localisation::default();
    localisation.language = "english".into();
    let factions = scan.faction_groups(
        Err("Faction naming content is unavailable".into()),
        &localisation,
        "english",
        &mut crate::catalogue::Refusals::default(),
    );
    assert_eq!(factions[0].names[0].name, None);
    assert_eq!(
        factions[0].names[0].reason.as_deref(),
        Some("Faction naming content is unavailable")
    );
    assert_eq!(factions[0].names[1].name.as_deref(), Some("My Pact"));
    assert_eq!(factions[0].names[1].reason, None);
}

#[test]
fn country_groups_date_war_starts_and_capitulations_as_the_engine_does() {
    use crate::products::country_groups::{Capitulation, WarStart, scan_branch};
    let frame = |hour, capitulated: bool| {
        let mut frame = group_frame(hour, &[], &[], &[]);
        // GER's side (9001) started a war with FRA (44) at hour 100; GER is listed second.
        *frame.tables[TableId::WarRelations.index()] =
            TableView::Complete(TableRows::WarRelations(vec![
                replay_data::rows::WarRelationRow {
                    first_country: 44,
                    second_country: 9001,
                    start_hour: 100,
                    first_was_instigator: 0,
                    ..Default::default()
                },
            ]));
        *frame.tables[TableId::Capitulation.index()] =
            TableView::Complete(TableRows::Capitulation(vec![
                replay_data::rows::CapitulationRow {
                    country: 44,
                    capitulated: u8::from(capitulated),
                    capitulated_hour: 101,
                    ..Default::default()
                },
            ]));
        frame
    };
    // At hour 102 GER's link id names another identity: the war and the
    // capitulation keep the countries they were first seen with.
    let mut renamed = frame(102, true);
    if let TableView::Complete(TableRows::Countries(rows)) =
        &mut *renamed.tables[TableId::Countries.index()]
    {
        for row in rows.iter_mut().filter(|row| row.tag == b"GER") {
            row.tag = b"BBB".to_vec();
        }
    }
    let replay = Replay::from_records(
        CAMPAIGN.into(),
        &[records(
            1,
            vec![frame(100, false), frame(101, true), renamed],
        )],
    )
    .unwrap();
    let scan = scan_branch(&replay, 1, &AtomicBool::new(false)).unwrap();
    assert_eq!(
        scan.war_starts(),
        [WarStart {
            hour: 100,
            attacking: "GER".into(),
            defending: "FRA".into()
        }]
    );
    assert_eq!(
        scan.capitulations(),
        [Capitulation {
            hour: 101,
            tag: "FRA".into()
        }]
    );
    assert_eq!(
        scan.identity_changes(),
        [crate::products::country_groups::IdentityChange {
            hour: 102,
            from: "GER".into(),
            to: "BBB".into()
        }]
    );
    assert!(scan.warnings().is_empty(), "{:?}", scan.warnings());
    // A war dated before it was first seen, after an unrecorded hour or once its
    // id names another country, cannot be named at its own hour: it is left out.
    let without_war = |hour| {
        let mut frame = frame(hour, false);
        *frame.tables[TableId::WarRelations.index()] =
            TableView::Complete(TableRows::WarRelations(vec![]));
        frame
    };
    let late = |hour, start_hour| {
        let mut frame = frame(hour, false);
        if let TableView::Complete(TableRows::WarRelations(rows)) =
            &mut *frame.tables[TableId::WarRelations.index()]
        {
            rows[0].start_hour = start_hour;
        }
        frame
    };
    let mut renamed_late = late(101, 100);
    if let TableView::Complete(TableRows::Countries(rows)) =
        &mut *renamed_late.tables[TableId::Countries.index()]
    {
        for row in rows.iter_mut().filter(|row| row.tag == b"GER") {
            row.tag = b"BBB".to_vec();
        }
    }
    for frames in [
        vec![without_war(100), late(102, 101)],
        vec![without_war(100), renamed_late],
    ] {
        let replay = Replay::from_records(CAMPAIGN.into(), &[records(1, frames)]).unwrap();
        let scan = scan_branch(&replay, 1, &AtomicBool::new(false)).unwrap();
        assert!(scan.war_starts().is_empty(), "{:?}", scan.war_starts());
        assert!(
            scan.warnings()
                .iter()
                .any(|warning| warning.contains("which countries they were is not known")),
            "{:?}",
            scan.warnings()
        );
    }
    // An hour whose capitulations cannot be read says so.
    let mut unread = frame(103, true);
    *unread.tables[TableId::Capitulation.index()] = TableView::Unavailable {
        reason: "fixture".into(),
    };
    let replay = Replay::from_records(
        CAMPAIGN.into(),
        &[records(1, vec![frame(100, false), unread])],
    )
    .unwrap();
    let scan = scan_branch(&replay, 1, &AtomicBool::new(false)).unwrap();
    assert!(
        scan.warnings()
            .iter()
            .any(|warning| warning.contains("Capitulations could not be read"))
    );
}

#[test]
fn country_groups_follow_each_faction_s_leader_at_order_zero() {
    use crate::products::country_groups::{MembershipRun, scan_branch};
    let run = |tag: &str, first_hour: i32, last_hour: i32| MembershipRun {
        tag: tag.into(),
        first_hour,
        last_hour,
    };
    let frames = vec![
        group_frame(100, &[], &[(7, "Pact", &[9001, 44])], &[]),
        group_frame(101, &[], &[(7, "Pact", &[9001, 44, 55])], &[]),
        // Leadership passes to FRA: the member order changes, the roster does not.
        group_frame(102, &[], &[(7, "Pact", &[44, 9001, 55])], &[]),
        // An order-zero member the links cannot name: no leader is known this hour.
        group_frame(103, &[], &[(7, "Pact", &[888, 44])], &[]),
        group_frame(104, &[], &[(7, "Pact", &[44])], &[]),
        // Hour 105 is missing: the leader's run breaks there.
        group_frame(106, &[], &[(7, "Pact", &[44])], &[]),
    ];
    let replay = Replay::from_records(CAMPAIGN.into(), &[records(1, frames)]).unwrap();
    let scan = scan_branch(&replay, 1, &AtomicBool::new(false)).unwrap();
    assert_eq!(
        scan.faction_leaders(7).unwrap(),
        [
            run("GER", 100, 101),
            run("FRA", 102, 102),
            run("FRA", 104, 104),
            run("FRA", 106, 106)
        ]
    );
    // Membership itself is unchanged by the change of leader.
    assert_eq!(
        scan.faction(7).unwrap().3,
        [
            run("GER", 100, 102),
            run("FRA", 100, 104),
            run("CTL", 101, 102),
            run("FRA", 106, 106)
        ]
    );
    let hour = crate::products::country_groups::HourSpan {
        first_hour: 103,
        last_hour: 103,
    };
    assert_eq!(scan.faction_incomplete(7), Some(&[hour][..]));
}

#[test]
fn appended_group_scan_matches_a_full_scan_without_changing_the_prior_revision() {
    use super::{cached_scan, scan_branch};
    use std::sync::Arc;
    let cancelled = AtomicBool::new(false);
    let first = Replay::from_records(
        CAMPAIGN.into(),
        &[records(
            1,
            vec![
                group_frame(100, &[9001], &[(7, "First Pact", &[9001, 44])], &[(44, 66)]),
                group_frame(101, &[9001], &[(7, "First Pact", &[9001, 44])], &[(44, 66)]),
            ],
        )],
    )
    .unwrap();
    let added = continuation_records(
        &first,
        1,
        vec![
            group_frame(
                102,
                &[9001, 44],
                &[(7, "Renamed Pact", &[44, 9001, 55])],
                &[(44, 66)],
            ),
            // Equal membership across the gap must still make separate runs.
            group_frame(
                104,
                &[9001, 44],
                &[(7, "Renamed Pact", &[44, 9001, 55])],
                &[(44, 66)],
            ),
        ],
    );
    let next = first.append(&[added]).unwrap();
    assert_eq!(&next.index[..first.index.len()], &first.index);
    let make = |revision: &str, replay| {
        crate::Source::from_replay(
            crate::SourceKey {
                source_id: "groups".into(),
                revision: revision.into(),
            },
            "memory.battleplan".into(),
            Arc::new(replay),
            &cancelled,
        )
        .unwrap()
    };
    let first_source = make("1", first);
    let first_scan = cached_scan(&first_source, 1, &cancelled).unwrap();
    let first_history = first_scan.faction_history(&cancelled).unwrap();
    let next_source = make("2", next);
    next_source.inherit_append_states(&first_source);
    let extended = cached_scan(&next_source, 1, &cancelled).unwrap();
    let full = scan_branch(next_source.replay_ref(), 1, &cancelled).unwrap();
    assert_eq!(extended.humans(), full.humans());
    assert_eq!(extended.faction(7), full.faction(7));
    assert_eq!(extended.faction_leaders(7), full.faction_leaders(7));
    assert_eq!(extended.wars(), full.wars());
    assert_eq!(extended.war_starts(), full.war_starts());
    assert_eq!(extended.war_start_reads, full.war_start_reads);
    assert_eq!(extended.capitulation_reads, full.capitulation_reads);
    assert_eq!(extended.capitulations_observed, full.capitulations_observed);
    assert_eq!(
        extended.unattributed_war_starts,
        full.unattributed_war_starts
    );
    assert_eq!(
        extended.unattributed_capitulations,
        full.unattributed_capitulations
    );
    assert_eq!(
        extended.capitulation_unread_hours,
        full.capitulation_unread_hours
    );
    assert!(first_scan.capitulations_observed);
    assert!(first_scan.capitulation_reads.is_empty());
    assert_eq!(extended.identity_changes(), full.identity_changes());
    assert_eq!(extended.unobserved(), full.unobserved());
    assert_eq!(extended.warnings(), full.warnings());
    let history = extended.faction_history(&cancelled).unwrap();
    assert!(!Arc::ptr_eq(&first_history, &history));
    assert!(Arc::ptr_eq(
        &first_history,
        &first_scan.faction_history(&cancelled).unwrap()
    ));
    let full_history = full.faction_history(&cancelled).unwrap();
    assert_eq!(history.factions, full_history.factions);
    assert_eq!(history.country_reads, full_history.country_reads);
    assert_eq!(history.country_presence, full_history.country_presence);
    assert_eq!(history.faction_reads, full_history.faction_reads);
    assert_eq!(history.unobserved, full_history.unobserved);
    assert_eq!(history.incomplete, full_history.incomplete);
    assert_eq!(history.end_recorded, full_history.end_recorded);
    assert_eq!(
        first_scan
            .faction_history(&cancelled)
            .unwrap()
            .faction_reads[0]
            .last_hour,
        101
    );
    assert!(Arc::ptr_eq(
        &first_scan.continuation.rosters.factions["7"].segments[&0],
        &extended.continuation.rosters.factions["7"].segments[&0]
    ));
    assert_eq!(
        extended.faction_name_at(7, 100).as_deref(),
        Some("First Pact")
    );
    assert_eq!(
        extended.faction_name_at(7, 102).as_deref(),
        Some("Renamed Pact")
    );
    assert_eq!(extended.faction_name_at(7, 103), None);
    assert_eq!(first_scan.faction(7).unwrap().2, 101);
    assert_eq!(first_scan.faction_name_at(7, 102), None);
    assert!(Arc::ptr_eq(
        &first_scan,
        &cached_scan(&first_source, 1, &cancelled).unwrap()
    ));
    assert!(Arc::ptr_eq(
        &extended,
        &cached_scan(&next_source, 1, &cancelled).unwrap()
    ));
}

#[test]
fn inherited_group_cache_recomputes_trailing_coverage_when_only_bounds_change() {
    use super::{HourSpan, cached_scan};
    use std::sync::Arc;
    let cancelled = AtomicBool::new(false);
    let first = Replay::from_records(
        CAMPAIGN.into(),
        &[records(1, vec![group_frame(100, &[], &[], &[])])],
    )
    .unwrap();
    let mut next = first.append(&[]).unwrap();
    next.gaps.push(replay_frame::branches::Gap {
        branch: 1,
        first_hour: 101,
        last_hour: 105,
        reason: "fixture".into(),
    });
    let make = |revision: &str, replay| {
        crate::Source::from_replay(
            crate::SourceKey {
                source_id: "groups".into(),
                revision: revision.into(),
            },
            "memory.battleplan".into(),
            Arc::new(replay),
            &cancelled,
        )
        .unwrap()
    };
    let first_source = make("1", first);
    let first_scan = cached_scan(&first_source, 1, &cancelled).unwrap();
    let first_history = first_scan.faction_history(&cancelled).unwrap();
    let next_source = make("2", next);
    next_source.inherit_append_states(&first_source);
    let extended = cached_scan(&next_source, 1, &cancelled).unwrap();
    assert_eq!(
        extended.unobserved().factions,
        [HourSpan {
            first_hour: 101,
            last_hour: 105
        }]
    );
    assert!(first_scan.unobserved().factions.is_empty());
    let history = extended.faction_history(&cancelled).unwrap();
    assert!(!Arc::ptr_eq(&first_history, &history));
    assert!(Arc::ptr_eq(
        &first_history,
        &first_scan.faction_history(&cancelled).unwrap()
    ));
    assert_eq!(history.unobserved, extended.unobserved().factions);
    assert!(!history.end_recorded);
    assert!(first_scan.faction_history(&cancelled).unwrap().end_recorded);
}

#[test]
fn country_groups_identity_bounds_split_replaced_tags_renames_and_gaps() {
    let holder = |hour, id, slot, tag: &[u8]| {
        let mut frame = group_frame(hour, &[], &[], &[]);
        if let TableView::Complete(TableRows::Countries(rows)) =
            &mut *frame.tables[TableId::Countries.index()]
        {
            let row = rows.iter_mut().find(|row| row.id == 9001).unwrap();
            row.id = id;
            row.canonical_slot = slot;
            row.tag = tag.to_vec();
            rows.sort_by_key(|row| row.id);
        }
        if let TableView::Complete(TableRows::CountryLinks(rows)) =
            &mut *frame.tables[TableId::CountryLinks.index()]
        {
            let row = rows.iter_mut().find(|row| row.id == 9001).unwrap();
            row.id = id;
            row.canonical_slot = slot;
            rows.sort_by_key(|row| row.id);
        }
        frame
    };
    let first = Replay::from_records(
        CAMPAIGN.into(),
        &[records(
            1,
            vec![
                holder(100, 9001, 7, b"GER"),
                holder(101, 9001, 7, b"GER"),
                holder(102, 9002, 11, b"GER"),
                holder(103, 9002, 11, b"Renamed"),
            ],
        )],
    )
    .unwrap();
    let prior = super::scan_branch(&first, 1, &AtomicBool::new(false)).unwrap();
    let next = first
        .append(&[continuation_records(
            &first,
            1,
            vec![
                holder(104, 9002, 11, b"Renamed"),
                holder(106, 9002, 11, b"Renamed"),
            ],
        )])
        .unwrap();
    let extended = super::scan_from(&next, 1, &AtomicBool::new(false), Some(&prior)).unwrap();
    let full = super::scan_branch(&next, 1, &AtomicBool::new(false)).unwrap();
    let runs = |scan: &super::Scan, tag: &str| {
        scan.identities[tag]
            .iter()
            .map(|run| (run.first_hour, run.last_hour, run.slot, run.country_id))
            .collect::<Vec<_>>()
    };
    assert_eq!(
        runs(&extended, "GER"),
        [(100, 101, 7, 9001), (102, 102, 11, 9002)]
    );
    assert_eq!(
        runs(&extended, "Renamed"),
        [(103, 104, 11, 9002), (106, 106, 11, 9002)]
    );
    assert_eq!(runs(&prior, "Renamed"), [(103, 103, 11, 9002)]);
    for tag in extended.identities.keys() {
        assert_eq!(runs(&extended, tag), runs(&full, tag));
    }
    let source = group_source(next);
    let cached = super::cached_scan(&source, 1, &AtomicBool::new(false)).unwrap();
    let reply = super::CountryGroups::load(
        &source,
        &super::CountryGroupParams {
            branch_id: "1".into(),
        },
        &crate::products::LoadContext { content: None },
        &AtomicBool::new(false),
    )
    .unwrap();
    let catalogue = serde_json::to_value(&reply.country_identities).unwrap();
    assert_eq!(
        catalogue,
        serde_json::to_value(super::IdentityCatalogue(&cached.identities)).unwrap()
    );
    assert_eq!(
        reply
            .country_identities
            .iter()
            .find(|history| history.tag == "GER")
            .unwrap()
            .identities
            .iter()
            .map(|run| (run.first_hour, run.last_hour, run.slot, run.country_id))
            .collect::<Vec<_>>(),
        [(100, 101, 7, 9001), (102, 102, 11, 9002)]
    );
    assert!(std::sync::Arc::ptr_eq(
        &cached,
        &super::cached_scan(&source, 1, &AtomicBool::new(false)).unwrap()
    ));
}

#[test]
fn cached_group_scan_honours_cancellation_and_stops_members_of_unlisted_factions() {
    use super::{cached_scan, scan_branch};
    use std::sync::Arc;
    let mut dissolved = group_frame(101, &[], &[], &[]);
    // Daily member rows can remain held after their faction disappears.
    *dissolved.tables[TableId::FactionMembers.index()] =
        TableView::Complete(TableRows::FactionMembers(vec![
            replay_data::rows::FactionMemberRow {
                country: 9001,
                faction: 7,
                ordinal: 0,
            },
        ]));
    let replay = Replay::from_records(
        CAMPAIGN.into(),
        &[records(
            1,
            vec![
                group_frame(100, &[], &[(7, "Pact", &[9001])], &[]),
                dissolved,
            ],
        )],
    )
    .unwrap();
    let scan = scan_branch(&replay, 1, &AtomicBool::new(false)).unwrap();
    assert_eq!(scan.faction(7).unwrap().3[0].last_hour, 100);
    assert_eq!(scan.faction_leaders(7).unwrap()[0].last_hour, 100);
    assert!(scan.faction_history(&AtomicBool::new(true)).is_err());
    assert!(scan.history.lock().unwrap().is_none());
    let history = scan.faction_history(&AtomicBool::new(false)).unwrap();
    assert_eq!(history.factions["7"][0].last_hour, 100);
    assert!(scan.faction_history(&AtomicBool::new(true)).is_err());
    let source = crate::Source::from_replay(
        crate::SourceKey {
            source_id: "groups".into(),
            revision: "1".into(),
        },
        "memory.battleplan".into(),
        Arc::new(replay),
        &AtomicBool::new(false),
    )
    .unwrap();
    cached_scan(&source, 1, &AtomicBool::new(false)).unwrap();
    assert_eq!(
        cached_scan(&source, 1, &AtomicBool::new(true))
            .err()
            .unwrap()
            .kind(),
        std::io::ErrorKind::Interrupted
    );
}

#[test]
fn country_groups_non_utf8_name_operands_preserve_bytes_and_refuse_display() {
    use super::scan_branch;
    let mut frame = group_frame(100, &[], &[(7, "Pact", &[9001])], &[]);
    if let TableView::Complete(TableRows::Factions(rows)) =
        &mut *frame.tables[TableId::Factions.index()]
    {
        rows[0].name = vec![0xff];
    }
    let replay = Replay::from_records(CAMPAIGN.into(), &[records(1, vec![frame])]).unwrap();
    let scan = scan_branch(&replay, 1, &AtomicBool::new(false)).unwrap();
    assert_eq!(scan.faction_name_operands(7, 100).unwrap().literal, [0xff]);
    let mut localisation = installed_content_localisation::localisation::Localisation::default();
    localisation.language = "english".into();
    assert!(
        scan.resolved_faction_name_at(
            7,
            100,
            Err("no template content".into()),
            &localisation,
            "english"
        )
        .is_err()
    );
    assert_eq!(scan.faction(7).unwrap().3[0].tag, "GER");
    assert_eq!(
        scan.faction_history(&AtomicBool::new(false))
            .unwrap()
            .factions["7"][0]
            .name,
        None
    );
    assert!(
        scan.warnings()
            .iter()
            .any(|warning| warning.starts_with("1 faction names were not valid text"))
    );
}

#[test]
fn country_groups_faction_names_keep_all_changes_or_refuse_the_complete_reply() {
    use super::{
        CountryGroupParams, CountryGroups, GroupBranch, GroupCache, NameOperands, RunHistory,
    };
    use crate::products::{LoadContext, MAX_PRODUCT_REPLY_BYTES, Product};
    use std::sync::Arc;

    let tail = "x".repeat(1024);
    for count in [5_000, MAX_PRODUCT_REPLY_BYTES / tail.len() + 1] {
        let replay = Replay::from_records(
            CAMPAIGN.into(),
            &[records(
                1,
                vec![group_frame(100, &[], &[(1, "Literal", &[])], &[])],
            )],
        )
        .unwrap();
        let cancelled = AtomicBool::new(false);
        let mut scan = super::scan_branch(&replay, 1, &cancelled).unwrap();
        let faction = scan.factions.get_mut(&1).unwrap();
        // Exercise the complete name history without changing membership histories.
        faction.names = RunHistory::default();
        for hour in 0..count {
            faction.names.push(NameOperands {
                dictionary: None,
                first_hour: hour as i32,
                last_hour: hour as i32,
                literal: format!("{hour}:{tail}").into_bytes(),
                template_token: None,
                lexical_read: 0,
                tokens: None,
                template: Err("Faction template not recorded".into()),
            });
        }
        let source = crate::Source::from_replay(
            crate::SourceKey {
                source_id: "faction-name-budget".into(),
                revision: "1".into(),
            },
            Default::default(),
            Arc::new(replay),
            &cancelled,
        )
        .unwrap();
        source
            .append_state::<GroupCache>()
            .lock()
            .unwrap()
            .entries
            .push((
                1,
                Arc::new(GroupBranch {
                    gate: std::sync::Mutex::new(()),
                    scan: std::sync::Mutex::new(Some(Arc::new(scan))),
                }),
            ));
        let result = CountryGroups::load(
            &source,
            &CountryGroupParams {
                branch_id: "1".into(),
            },
            &LoadContext { content: None },
            &cancelled,
        );
        if count == 5_000 {
            let reply = result.unwrap();
            let names = &reply.factions[0].names;
            assert_eq!(names.len(), count);
            let last = format!("{}:{tail}", count - 1);
            assert_eq!(names.last().unwrap().name.as_deref(), Some(last.as_str()));
        } else {
            let error = result.unwrap_err();
            assert_eq!(
                error.to_string(),
                "The complete country group reply is too large; nothing was omitted"
            );
        }
    }
}

#[test]
fn country_groups_faction_literals_refuse_a_different_viewer_language() {
    let templates = installed_content_core::faction_templates::Catalog::default();
    let mut localisation = installed_content_localisation::localisation::Localisation::default();
    localisation.language = "french".into();
    let factions = named_factions(
        vec![template_frame(100, "My Pact", None)],
        &templates,
        &localisation,
    );
    assert_eq!(factions[0].name, None);
    assert_eq!(
        factions[0].name_reason.as_deref(),
        Some(
            "faction name is text the game stored in english without its key, and the viewer's language is french"
        )
    );
}

#[test]
fn country_groups_faction_names_keep_renames_and_dictionary_gaps_separate_from_membership() {
    let templates = crate::catalogue::tests::loaded_templates("demo={name=DEMO_NAME} empty={}");
    let localisation = crate::catalogue::tests::loaded("l_english:\n DEMO_NAME:0 \"Demo Pact\"\n");
    let mut missing = template_frame(103, "", Some((17, "demo")));
    *missing.tables[TableId::LexicalTokens.index()] = TableView::Unavailable {
        reason: "missing template dictionary".into(),
    };
    let replay = Replay::from_records(
        CAMPAIGN.into(),
        &[records(
            1,
            vec![
                template_frame(100, "First Pact", None),
                template_frame(101, "Renamed Pact", None),
                template_frame(102, "", Some((17, "demo"))),
                missing,
                template_frame(104, "", Some((17, "demo"))),
                template_frame(105, "", Some((18, "empty"))),
            ],
        )],
    )
    .unwrap();
    let scan = super::scan_branch(&replay, 1, &AtomicBool::new(false)).unwrap();
    let mut refused = crate::catalogue::Refusals::default();
    let factions = scan.faction_groups(Ok(&templates), &localisation, "english", &mut refused);
    let faction = &factions[0];
    let names = &faction.names;
    assert_eq!(
        names
            .iter()
            .map(|run| (run.first_hour, run.last_hour, run.name.as_deref()))
            .collect::<Vec<_>>(),
        [
            (100, 100, Some("First Pact")),
            (101, 101, Some("Renamed Pact")),
            (102, 102, Some("Demo Pact")),
            (103, 103, None),
            (104, 104, Some("Demo Pact")),
            (105, 105, Some("")),
        ],
    );
    let reason = names[3].reason.as_ref().unwrap();
    assert!(reason.contains("LexicalTokens"));
    assert!(reason.contains("missing template dictionary"));
    assert!(
        names
            .iter()
            .enumerate()
            .all(|(index, run)| index == 3 || run.reason.is_none())
    );
    assert_eq!(refused.lines("faction names are").len(), 1);
    assert_eq!(faction.name.as_deref(), Some(""));
    assert_eq!(faction.name_reason, None);
    assert_eq!(
        faction.members.iter().collect::<Vec<_>>(),
        [&super::MembershipRun {
            tag: "GER".into(),
            first_hour: 100,
            last_hour: 105,
        }],
    );
    assert_eq!(
        faction.leaders.iter().collect::<Vec<_>>(),
        faction.members.iter().collect::<Vec<_>>(),
    );
    assert!(faction.incomplete.is_empty());
    assert!(scan.unobserved().factions.is_empty());
}

#[test]
fn country_groups_faction_names_do_not_start_lifetime_on_an_unread_roster() {
    let mut first = template_frame(100, "First name", None);
    *first.tables[TableId::FactionMembers.index()] = TableView::Unavailable {
        reason: "missing".into(),
    };
    let templates = installed_content_core::faction_templates::Catalog::default();
    let mut localisation = installed_content_localisation::localisation::Localisation::default();
    localisation.language = "english".into();
    let factions = named_factions(
        vec![first, template_frame(101, "Later name", None)],
        &templates,
        &localisation,
    );
    assert_eq!((factions[0].first_hour, factions[0].last_hour), (101, 101));
    assert_eq!(factions[0].names[0].first_hour, 100);
    assert_eq!(factions[0].names[0].name.as_deref(), Some("First name"));
}

fn faction_source(frame: DecodedFrame) -> crate::Source {
    let replay = Replay::from_records(CAMPAIGN.into(), &[records(1, vec![frame])]).unwrap();
    crate::Source::from_replay(
        crate::SourceKey {
            source_id: "faction-content".into(),
            revision: "1".into(),
        },
        Default::default(),
        std::sync::Arc::new(replay),
        &AtomicBool::new(false),
    )
    .unwrap()
}

#[test]
fn country_groups_literal_names_do_not_resolve_content_but_check_viewer_language() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
    use crate::products::{ContentOptions, LoadContext, Product};
    let (_, folder) = crate::catalogue::tests::folder_catalogue(&[]);
    std::fs::write(folder.0.join("settings.txt"), "language=\"l_french\"").unwrap();
    let context = LoadContext {
        content: Some(ContentOptions {
            options: installed_content_core::ResolveOptions {
                game_dir: folder.0.join("unavailable-game"),
                user_dir: folder.0.clone(),
                cache_dir: folder.0.join("cache"),
            },
            allow_unverified: false,
            presentation: Default::default(),
        }),
    };
    let source = faction_source(template_frame(100, "My Pact", None));
    let reply = super::CountryGroups::load(
        &source,
        &super::CountryGroupParams {
            branch_id: "1".into(),
        },
        &context,
        &AtomicBool::new(false),
    )
    .unwrap();
    assert_eq!(reply.factions[0].name, None);
    assert_eq!(
        reply.factions[0].name_reason.as_deref(),
        Some(
            "faction name is text the game stored in english without its key, and the viewer's language is french"
        )
    );
    std::fs::write(folder.0.join("settings.txt"), "language=\"l_english\"").unwrap();
    let reply = super::CountryGroups::load(
        &source,
        &super::CountryGroupParams {
            branch_id: "1".into(),
        },
        &context,
        &AtomicBool::new(false),
    )
    .unwrap();
    assert_eq!(reply.factions[0].name.as_deref(), Some("My Pact"));
    assert!(reply.warnings.is_empty(), "{:?}", reply.warnings);
}

#[test]
fn country_groups_template_names_ignore_gfx_failures_and_unrelated_warnings() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
    use crate::products::{ContentOptions, LoadContext, Product};
    use std::sync::Arc;
    let (_, folder) = crate::catalogue::tests::folder_catalogue(&[
        (
            "common/factions/templates/test.txt",
            "demo={name=DEMO_NAME} bad={visible=yes}",
        ),
        (
            "localisation/test_l_english.yml",
            "\u{feff}l_english:\n DEMO_NAME:0 \"Demo Pact\"\n",
        ),
        (
            "localisation/missing_bom_l_english.yml",
            "l_english:\n WRONG:0 \"Wrong\"\n",
        ),
        ("interface/test.gfx", "spriteTypes={}"),
        ("settings.txt", "language=\"l_english\""),
    ]);
    let layers = installed_content_core::ResolvedLayers::from_folders(&[(&folder.0, &[][..])]).unwrap();
    let content = Arc::new(installed_content_core::content::ResolvedContent {
        layers,
        stats: Default::default(),
        changed: Vec::new(),
    });
    let template_warnings = content.layers.faction_templates().unwrap().warnings.clone();
    assert!(!template_warnings.is_empty());
    assert!(
        !installed_content_localisation::localisation::read(&content.layers, "english")
            .unwrap()
            .logged
            .is_empty()
    );
    // A verified interface file changed: the full catalogue must fail to read it.
    std::fs::write(folder.0.join("interface/test.gfx"), "changed").unwrap();
    assert!(installed_content_gfx::gfx::read(&content.layers).is_err());
    let options = ContentOptions {
        options: installed_content_core::ResolveOptions {
            game_dir: folder.0.clone(),
            user_dir: folder.0.clone(),
            cache_dir: folder.0.join("cache"),
        },
        allow_unverified: true,
        presentation: Default::default(),
    };
    let source = faction_source(template_frame(100, "", Some((17, "demo"))));
    source.set_test_content(options.options.clone(), content);
    let context = LoadContext {
        content: Some(options),
    };
    let persisted = super::scan_branch(source.replay_ref(), 1, &AtomicBool::new(false))
        .unwrap()
        .saved_bytes()
        .unwrap();
    let restored = super::Scan::read(persisted.as_slice()).unwrap();
    let reply = super::CountryGroups::load(
        &source,
        &super::CountryGroupParams {
            branch_id: "1".into(),
        },
        &context,
        &AtomicBool::new(false),
    )
    .unwrap();
    assert_eq!(
        serde_json::to_value(restored.reply(&source, "1", &context).unwrap()).unwrap(),
        serde_json::to_value(&reply).unwrap()
    );
    assert_eq!(reply.factions[0].name.as_deref(), Some("Demo Pact"));
    let actual: Vec<_> = reply
        .warnings
        .iter()
        .filter(|line| !line.starts_with("UNVERIFIED CONTENT"))
        .cloned()
        .collect();
    assert_eq!(actual, template_warnings);
}

#[test]
fn country_groups_link_tags_keep_balances_and_follow_identity_changes() {
    use crate::products::country_groups::{
        HourSpan, IdentityChange, MembershipRun, WarPairRun, WarStart, scan_branch,
    };
    let run = |tag: &str, first_hour, last_hour| MembershipRun {
        tag: tag.into(),
        first_hour,
        last_hour,
    };
    let change = |hour, from: &str, to: &str| IdentityChange {
        hour,
        from: from.into(),
        to: to.into(),
    };
    let pair = |first: &str, second: &str, first_hour, last_hour| WarPairRun {
        first: first.into(),
        second: second.into(),
        first_hour,
        last_hour,
    };
    for unavailable in [TableId::Countries, TableId::CountryLinks] {
        let frames = (100..=109)
            .map(|hour| {
                // 11 is an alias of 9001. All histories must resolve the alias.
                let mut frame = group_frame(hour, &[11], &[(7, "Pact", &[11, 55])], &[(11, 55)]);
                if let TableView::Complete(TableRows::Countries(rows)) =
                    &mut *frame.tables[TableId::Countries.index()]
                {
                    let country = rows.iter_mut().find(|row| row.id == 9001).unwrap();
                    country.political_power_raw = Some(i64::from(hour));
                    if hour >= 102 {
                        country.tag = b"BBB".to_vec();
                    }
                    if hour == 109 {
                        rows.iter_mut().find(|row| row.id == 44).unwrap().tag = b"CCC".to_vec();
                    }
                }
                if hour >= 104
                    && let TableView::Complete(TableRows::CountryLinks(rows)) =
                        &mut *frame.tables[TableId::CountryLinks.index()]
                {
                    rows.iter_mut()
                        .find(|row| row.id == 11)
                        .unwrap()
                        .canonical_slot = 8;
                }
                if let TableView::Complete(TableRows::WarRelations(rows)) =
                    &mut *frame.tables[TableId::WarRelations.index()]
                {
                    rows[0].first_was_instigator = 1;
                    rows[0].start_hour = match hour {
                        100..=101 => 100,
                        102..=103 => 102,
                        104..=105 => 104,
                        // First seen after an unread hour: its participants are unknown.
                        106..=107 => 106,
                        _ => hour,
                    };
                }
                if hour == 106 {
                    *frame.tables[unavailable.index()] = TableView::Unavailable {
                        reason: "fixture identity read failed".into(),
                    };
                }
                frame
            })
            .collect();
        let replay = Replay::from_records(CAMPAIGN.into(), &[records(1, frames)]).unwrap();
        let scan = scan_branch(&replay, 1, &AtomicBool::new(false)).unwrap();
        let expected = [
            run("GER", 100, 101),
            run("BBB", 102, 103),
            run("FRA", 104, 105),
            run("FRA", 107, 108),
            run("CCC", 109, 109),
        ];
        assert_eq!(scan.humans(), expected, "{unavailable:?}");
        assert_eq!(scan.faction_leaders(7).unwrap(), expected);
        assert_eq!(
            scan.faction(7).unwrap().3,
            [
                run("GER", 100, 101),
                run("CTL", 100, 105),
                run("BBB", 102, 103),
                run("FRA", 104, 105),
                run("FRA", 107, 108),
                run("CTL", 107, 109),
                run("CCC", 109, 109),
            ]
        );
        assert_eq!(
            scan.wars(),
            [
                pair("CTL", "GER", 100, 101),
                pair("BBB", "CTL", 102, 103),
                pair("CTL", "FRA", 104, 105),
                pair("CTL", "FRA", 107, 108),
                pair("CCC", "CTL", 109, 109),
            ]
        );
        // Both canonical and alias links rename; each tag change is emitted once.
        assert_eq!(
            scan.identity_changes(),
            [
                change(102, "GER", "BBB"),
                change(104, "BBB", "FRA"),
                change(109, "FRA", "CCC"),
            ]
        );
        let starts: Vec<_> = [
            (100, "GER"),
            (102, "BBB"),
            (104, "FRA"),
            (108, "FRA"),
            (109, "CCC"),
        ]
        .into_iter()
        .map(|(hour, attacking)| WarStart {
            hour,
            attacking: attacking.into(),
            defending: "CTL".into(),
        })
        .collect();
        assert_eq!(scan.war_starts(), starts);
        let missing = [HourSpan {
            first_hour: 106,
            last_hour: 106,
        }];
        assert_eq!(scan.unobserved().humans, missing);
        assert_eq!(scan.unobserved().factions, missing);
        assert_eq!(scan.unobserved().wars, missing);
        assert!(
            scan.warnings()
                .iter()
                .any(|warning| warning.contains("which countries they were is not known"))
        );
    }
}

#[test]
fn country_groups_faction_names_bound_tokens_follow_each_dictionary_read() {
    let templates = crate::catalogue::tests::loaded_templates(
        "demo={name={localization_key=NAME ID=token:17}}",
    );
    let localisation = crate::catalogue::tests::loaded("l_english:\n NAME:0 \"Pact $ID$\"\n");
    let frames = [(100, Some(31)), (101, Some(47)), (102, None)]
        .into_iter()
        .map(|(hour, token)| {
            let mut frame = template_frame(hour, "", Some((17, "demo")));
            if let TableView::Complete(TableRows::LexicalTokens(rows)) =
                &mut *frame.tables[TableId::LexicalTokens.index()]
                && let Some(token) = token
            {
                rows.push(replay_data::rows::LexicalTokenRow {
                    token,
                    literal: b"17".to_vec(),
                });
            }
            frame
        })
        .collect();
    let factions = named_factions(frames, &templates, &localisation);
    let names = &factions[0].names;
    assert_eq!(names.len(), 3);
    assert_eq!(names[0].name.as_deref(), Some("Pact 31"));
    assert_eq!(names[1].name.as_deref(), Some("Pact 47"));
    assert_eq!(names[2].name, None);
    assert!(
        names[2]
            .reason
            .as_ref()
            .unwrap()
            .contains("token 17 is absent from LexicalTokens")
    );
}

#[test]
fn country_groups_equal_dictionary_reads_share_one_token_index() {
    let first = Replay::from_records(
        CAMPAIGN.into(),
        &[records(
            1,
            vec![template_frame(100, "", Some((17, "demo")))],
        )],
    )
    .unwrap();
    let next = first
        .append(&[continuation_records(
            &first,
            1,
            vec![template_frame(101, "", Some((17, "demo")))],
        )])
        .unwrap();
    let scan = super::scan_branch(&next, 1, &AtomicBool::new(false)).unwrap();
    // The second block reads the dictionary again, which still dates the operands.
    let names: Vec<_> = scan.factions[&5].names.iter().collect();
    assert_eq!(names.len(), 2);
    let version = |name: &super::NameOperands| name.tokens.as_ref().map(|tokens| tokens.version());
    assert_eq!(version(names[0]), Some(0));
    assert_eq!(version(names[1]), Some(0));
    assert_eq!(scan.continuation.dictionaries.count(), 1);
}

#[test]
fn the_cached_group_scan_supplies_dated_names_rosters_and_country_read_spans_together() {
    use super::{CountryGroupParams, CountryGroups, cached_scan};
    use crate::{
        Source, SourceKey,
        products::{LoadContext, Product},
    };
    let mut missing = group_frame(101, &[], &[(77, "New name", &[9001, 999999])], &[]);
    *missing.tables[TableId::Countries.index()] = TableView::Unavailable {
        reason: "missing countries".into(),
    };
    let replay = Arc::new(
        Replay::from_records(
            CAMPAIGN.into(),
            &[records(
                1,
                vec![
                    group_frame(100, &[], &[(77, "Original", &[9001])], &[]),
                    missing,
                    group_frame(102, &[], &[(77, "New name", &[9001])], &[]),
                ],
            )],
        )
        .unwrap(),
    );
    let cancelled = AtomicBool::new(false);
    let source = Source::from_replay(
        SourceKey {
            source_id: "cached-groups".into(),
            revision: "1".into(),
        },
        "memory.battleplan".into(),
        replay,
        &cancelled,
    )
    .unwrap();
    let cached = cached_scan(&source, 1, &cancelled).unwrap();
    CountryGroups::load(
        &source,
        &CountryGroupParams {
            branch_id: "1".into(),
        },
        &LoadContext { content: None },
        &cancelled,
    )
    .unwrap();
    assert!(Arc::ptr_eq(
        &cached,
        &cached_scan(&source, 1, &cancelled).unwrap()
    ));
    let history = cached.faction_history(&cancelled).unwrap();
    assert_eq!(
        history.factions["77"]
            .iter()
            .map(|run| (
                run.first_hour,
                run.last_hour,
                run.name.as_deref(),
                run.complete
            ))
            .collect::<Vec<_>>(),
        [
            (100, 100, Some("Original"), true),
            (101, 101, Some("New name"), false),
            (102, 102, Some("New name"), true)
        ]
    );
    assert_eq!(
        history
            .country_reads
            .iter()
            .map(|span| (span.first_hour, span.last_hour))
            .collect::<Vec<_>>(),
        [(100, 100), (102, 102)]
    );
    assert_eq!(cached.faction_leaders(77).unwrap().len(), 2);
}

fn many_country_frame() -> DecodedFrame {
    use replay_data::{rows::CountryLinkRow, rows::CountryRow};
    let ids: Vec<_> = (1..=24).collect();
    let mut frame = group_frame(100, &[], &[], &[]);
    *frame.tables[TableId::Countries.index()] = TableView::Complete(TableRows::Countries(
        ids.iter()
            .map(|&id| CountryRow {
                id,
                canonical_slot: id,
                tag: format!("Tag-{id}").into_bytes(),
                ..Default::default()
            })
            .collect(),
    ));
    *frame.tables[TableId::CountryLinks.index()] = TableView::Complete(TableRows::CountryLinks(
        ids.iter()
            .map(|&id| CountryLinkRow {
                id,
                canonical_slot: id,
            })
            .collect(),
    ));
    frame
}

#[test]
fn faction_history_keeps_all_country_read_evidence() {
    let template = many_country_frame();
    let frames = (0..101)
        .map(|index| {
            let mut frame = template.clone();
            frame.hour = 100 + index * 2;
            frame
        })
        .collect();
    let replay = Replay::from_records(CAMPAIGN.into(), &[records(1, frames)]).unwrap();
    let scan = super::scan_branch(&replay, 1, &AtomicBool::new(false)).unwrap();
    let history = scan.faction_history(&AtomicBool::new(false)).unwrap();
    assert_eq!(history.country_presence.len(), 24);
    assert_eq!(
        history
            .country_presence
            .values()
            .map(Vec::len)
            .sum::<usize>(),
        2_424
    );
    assert_eq!(history.country_reads.len(), 101);
    assert!(scan.humans().is_empty());
    assert!(scan.wars().is_empty());
}

#[test]
fn country_groups_unread_countries_do_not_attribute_events_changes_or_unmapped_ids() {
    let first = group_frame(100, &[9001], &[(77, "Original", &[9001])], &[]);
    let mut unread = group_frame(101, &[999999], &[(77, "Renamed", &[999999])], &[(44, 9001)]);
    *unread.tables[TableId::Countries.index()] = TableView::Unavailable {
        reason: "fixture".into(),
    };
    let mut next = group_frame(102, &[9001], &[(77, "Renamed", &[9001])], &[(44, 9001)]);
    if let TableView::Complete(TableRows::Countries(rows)) =
        &mut *next.tables[TableId::Countries.index()]
    {
        for row in rows.iter_mut().filter(|row| row.tag == b"GER") {
            row.tag = b"After gap".to_vec();
        }
    }
    for frame in [&mut unread, &mut next] {
        if let TableView::Complete(TableRows::WarRelations(rows)) =
            &mut *frame.tables[TableId::WarRelations.index()]
        {
            rows[0].start_hour = 101;
        }
        *frame.tables[TableId::Capitulation.index()] =
            TableView::Complete(TableRows::Capitulation(vec![
                replay_data::rows::CapitulationRow {
                    country: 9001,
                    capitulated: 1,
                    capitulated_hour: 101,
                    ..Default::default()
                },
            ]));
    }
    let replay =
        Replay::from_records(CAMPAIGN.into(), &[records(1, vec![first, unread, next])]).unwrap();
    let scan = super::scan_branch(&replay, 1, &AtomicBool::new(false)).unwrap();
    let history = scan.faction_history(&AtomicBool::new(false)).unwrap();
    assert!(scan.identity_changes().is_empty());
    assert!(scan.war_starts.is_empty());
    assert!(scan.capitulations.is_empty());
    assert!(
        !scan
            .warnings
            .iter()
            .any(|warning| warning.contains("recorded country references")),
        "{:?}",
        scan.warnings
    );
    let rosters = &history.factions["77"];
    assert_eq!(rosters[1].name.as_deref(), Some("Renamed"));
    assert!(!rosters[1].complete);
    assert!(rosters[1].members.is_empty());
    assert_eq!(rosters[2].members, ["After gap"]);
}

#[test]
fn faction_rosters_follow_a_changed_raw_country_id_with_unchanged_tags_and_links() {
    let first = group_frame(100, &[], &[(77, "Pact", &[44])], &[]);
    // FRA keeps its tag and slot, but its row id moves to 45: link 44 no longer names a
    // country, so the member read through link 44 is unmapped this hour.
    let mut moved = group_frame(101, &[], &[(77, "Pact", &[44])], &[]);
    if let TableView::Complete(TableRows::Countries(rows)) =
        &mut *moved.tables[TableId::Countries.index()]
    {
        rows.iter_mut().find(|row| row.id == 44).unwrap().id = 45;
        rows.sort_by_key(|row| row.id);
    }
    let replay = Replay::from_records(CAMPAIGN.into(), &[records(1, vec![first, moved])]).unwrap();
    let scan = super::scan_branch(&replay, 1, &AtomicBool::new(false)).unwrap();
    let history = scan.faction_history(&AtomicBool::new(false)).unwrap();
    let rosters = &history.factions["77"];
    assert_eq!(rosters[0].members, ["FRA"]);
    assert!(rosters[0].complete);
    assert!(
        !rosters[1].complete,
        "a stale id lookup would still map link 44 to FRA"
    );
    assert!(rosters[1].members.is_empty());
}

#[test]
fn faction_names_and_rosters_share_catalogue_reads_without_inventing_lifetimes() {
    use crate::products::country_groups::HourSpan;
    let mut unread = group_frame(
        100,
        &[],
        &[(77, "First name", &[44]), (88, "Empty pact", &[])],
        &[],
    );
    *unread.tables[TableId::Countries.index()] = TableView::Unavailable {
        reason: "country read failed".into(),
    };
    let mut last = group_frame(
        102,
        &[],
        &[(77, "Last name", &[44]), (88, "Empty pact", &[])],
        &[],
    );
    *last.tables[TableId::FactionMembers.index()] = TableView::Unavailable {
        reason: "member read failed".into(),
    };
    let replay = Replay::from_records(
        CAMPAIGN.into(),
        &[records(
            1,
            vec![
                unread,
                group_frame(
                    101,
                    &[],
                    &[(77, "Middle name", &[44]), (88, "Empty pact", &[])],
                    &[],
                ),
                last,
            ],
        )],
    )
    .unwrap();
    let scan = super::scan_branch(&replay, 1, &AtomicBool::new(false)).unwrap();
    let history = scan.faction_history(&AtomicBool::new(false)).unwrap();
    let mut localisation = installed_content_localisation::localisation::Localisation::default();
    localisation.language = "english".into();
    let factions = scan.faction_groups(
        Ok(&installed_content_core::faction_templates::Catalog::default()),
        &localisation,
        "english",
        &mut crate::catalogue::Refusals::default(),
    );
    let faction = factions.iter().find(|faction| faction.id == "77").unwrap();
    // The name read is independent of main's roster-known lifetime.
    assert_eq!((faction.first_hour, faction.last_hour), (101, 101));
    assert_eq!(faction.name.as_deref(), Some("Last name"));
    assert_eq!(
        faction
            .names
            .iter()
            .map(|run| (run.first_hour, run.name.as_deref()))
            .collect::<Vec<_>>(),
        [
            (100, Some("First name")),
            (101, Some("Middle name")),
            (102, Some("Last name"))
        ]
    );
    assert_eq!(
        history.faction_reads,
        [HourSpan {
            first_hour: 100,
            last_hour: 102
        }]
    );
    assert_eq!(
        history.country_reads,
        [HourSpan {
            first_hour: 101,
            last_hour: 102
        }]
    );
    let rosters = &history.factions["77"];
    assert!(!rosters[0].complete);
    assert!(rosters[1].complete);
    assert!(!rosters[2].complete);
    assert_eq!(rosters[1].members, ["FRA"]);
    assert!(
        history.factions["88"][0].complete,
        "an empty roster needs no country identity"
    );
    let history_names = super::name_runs(
        77,
        &history.names[&77],
        Ok(&installed_content_core::faction_templates::Catalog::default()),
        &localisation,
        "english",
        &mut crate::catalogue::Refusals::default(),
    );
    assert_eq!(
        history_names, faction.names,
        "totals and group labels use the same hourly evidence"
    );
}

#[test]
fn sampled_group_names_and_rosters_match_hourly_reads_through_rename_and_dissolution() {
    use super::{HourSpan, Scan};
    use crate::products::LoadContext;

    let blocks = [
        vec![group_frame(100, &[], &[(77, "First", &[9001])], &[])],
        vec![
            group_frame(101, &[], &[(77, "Renamed", &[9001]), (78, "", &[44])], &[]),
            group_frame(102, &[], &[(77, "Renamed", &[9001]), (78, "", &[44])], &[]),
        ],
        vec![group_frame(103, &[], &[], &[])],
    ];
    let mut replay =
        Replay::from_records(CAMPAIGN.into(), &[records(1, blocks[0].clone())]).unwrap();
    for block in &blocks[1..] {
        replay = replay
            .append(&[continuation_records(&replay, 1, block.clone())])
            .unwrap();
    }
    let source = group_source(replay);
    let flag = AtomicBool::new(false);
    let context = LoadContext { content: None };
    let mut sampled = Scan::new(source.replay_ref(), 1);
    assert_eq!(
        sampled.sample_tail(&source, 1, 99, &flag).unwrap(),
        [HourSpan {
            first_hour: 100,
            last_hour: 103
        }]
    );
    let reply = sampled.reply(&source, "1", &context).unwrap();
    let hourly = super::scan_branch(source.replay_ref(), 1, &flag).unwrap();
    assert_eq!(
        serde_json::to_value(&reply).unwrap(),
        serde_json::to_value(hourly.reply(&source, "1", &context).unwrap()).unwrap()
    );
    assert_eq!(reply.factions[0].names.len(), 2);
    assert_eq!(reply.factions[0].name.as_deref(), Some("Renamed"));
    assert_eq!(reply.factions[0].members[0].last_hour, 102);
    assert_eq!(reply.factions[1].name.as_deref(), Some(""));
}

#[test]
fn group_names_without_reads_stay_unknown_in_replies_snapshots_and_resumed_samples() {
    use super::{RunHistory, Scan};
    use crate::products::LoadContext;

    let initial = group_frame(
        100,
        &[],
        &[(77, "Unread", &[9001]), (78, "Unread", &[44])],
        &[],
    );
    let first =
        Replay::from_records(CAMPAIGN.into(), &[records(1, vec![initial.clone()])]).unwrap();
    let mut scan = Scan::new(&first, 1);
    scan.observe(&initial).unwrap();
    // A derived prefix may retain a roster without either a history entry or a read.
    scan.continuation.names.remove(&77);
    scan.continuation.names.insert(78, RunHistory::default());
    let check = |scan: &Scan, source: &crate::Source| {
        let reply = scan
            .reply(source, "1", &LoadContext { content: None })
            .unwrap();
        assert_eq!(reply.factions.len(), 2);
        for faction in &reply.factions {
            assert_eq!(faction.name, None);
            assert_eq!(
                faction.name_reason.as_deref(),
                Some("Faction has no recorded name read")
            );
            assert!(faction.names.is_empty(), "no invented supplying hour");
            assert_eq!(faction.recorded_name, "Unread");
            assert_eq!(faction.members.len(), 1);
        }
        assert!(reply.warnings.iter().any(|warning| warning ==
            "2 faction names are not shown: Faction has no recorded name read; faction 77, faction 78"));
    };
    let first_source = group_source(first);
    check(&scan, &first_source);
    scan.finish(first_source.replay_ref(), 1);
    let mut resumed = Scan::read(scan.saved_bytes().unwrap().as_slice()).unwrap();
    check(&resumed, &first_source);
    let mut unread = group_frame(101, &[], &[], &[]);
    *unread.tables[TableId::Factions.index()] = TableView::Unavailable {
        reason: "synthetic missing name table".into(),
    };
    let appended = first_source
        .replay_ref()
        .append(&[continuation_records(
            first_source.replay_ref(),
            1,
            vec![unread],
        )])
        .unwrap();
    let source = group_source(appended);
    resumed
        .sample_tail(&source, 1, 100, &AtomicBool::new(false))
        .unwrap();
    check(&resumed, &source);
}

#[test]
fn group_change_names_refuse_a_dissolved_faction_without_a_name_history() {
    use super::{Scan, changes::Prepared};
    use crate::prepared::frame_changes::FrameChangeTracker;

    let first = group_frame(100, &[], &[(77, "Unread", &[9001])], &[]);
    let last = group_frame(101, &[], &[], &[]);
    let source = group_source(
        Replay::from_records(
            CAMPAIGN.into(),
            &[records(1, vec![first.clone(), last.clone()])],
        )
        .unwrap(),
    );
    let mut scan = Scan::new(source.replay_ref(), 1);
    let mut changes = Prepared::default();
    let mut tracker = FrameChangeTracker::new();
    let context = crate::products::LoadContext { content: None };
    changes
        .observe_changed(
            &mut scan,
            &first,
            &tracker.observe(&first, source.replay_ref().format),
            &source,
            &context,
        )
        .unwrap();
    scan.continuation.names.remove(&77);
    changes
        .observe_changed(
            &mut scan,
            &last,
            &tracker.observe(&last, source.replay_ref().format),
            &source,
            &context,
        )
        .unwrap();
    let state = serde_json::to_value(changes).unwrap();
    assert_eq!(
        state["name_refusals"]["Faction 77 has no recorded name at 100"],
        serde_json::json!([77])
    );
}

#[test]
fn prepared_scan_roundtrip_reuses_reply_builder_names_dictionaries_and_append_state() {
    use super::{CountryGroupParams, CountryGroups, Scan};
    use crate::products::LoadContext;
    let flag = AtomicBool::new(false);
    let frames = vec![
        template_frame(100, "", Some((17, "demo"))),
        template_frame(101, "", Some((17, "demo"))),
        template_frame(104, "Renamed", None),
    ];
    let mut replay =
        Replay::from_records(CAMPAIGN.into(), &[records(1, frames[..1].to_vec())]).unwrap();
    let mut prepared = Scan::new(&replay, 1);
    for frame in frames {
        prepared.begin_block();
        prepared.observe(&frame).unwrap();
        prepared.finish(&replay, 1);
        prepared = Scan::read(prepared.saved_bytes().unwrap().as_slice()).unwrap();
        if frame.hour != 104 {
            let next_hour = if frame.hour == 100 { 101 } else { 104 };
            let next = if next_hour == 104 {
                template_frame(104, "Renamed", None)
            } else {
                template_frame(101, "", Some((17, "demo")))
            };
            replay = replay
                .append(&[continuation_records(&replay, 1, vec![next])])
                .unwrap();
        }
    }
    prepared.finish(&replay, 1);
    let source = group_source(replay);
    let context = LoadContext { content: None };
    assert_eq!(
        serde_json::to_value(prepared.reply(&source, "1", &context).unwrap()).unwrap(),
        serde_json::to_value(
            CountryGroups::load(
                &source,
                &CountryGroupParams {
                    branch_id: "1".into()
                },
                &context,
                &flag
            )
            .unwrap()
        )
        .unwrap()
    );
    let templates = crate::catalogue::tests::loaded_templates("demo={name=DEMO_NAME}");
    let localisation = crate::catalogue::tests::loaded("l_english:\n DEMO_NAME:0 \"Demo Pact\"\n");
    let scan = super::scan_branch(source.replay_ref(), 1, &flag).unwrap();
    let resolve = |scan: &Scan| {
        let mut refused = crate::catalogue::Refusals::default();
        serde_json::to_value(scan.faction_groups(
            Ok(&templates),
            &localisation,
            "english",
            &mut refused,
        ))
        .unwrap()
    };
    assert_eq!(resolve(&prepared), resolve(&scan));
}

#[test]
fn prepared_groups_resume_preserves_faction_evidence_and_query_reply() {
    use super::Scan;
    use crate::products::LoadContext;
    let flag = AtomicBool::new(false);
    let frames: Vec<_> = [100, 101, 102, 104, 105, 106]
        .into_iter()
        .map(|hour| {
            let members: &[u32] = if hour >= 105 { &[44] } else { &[44, 9001] };
            let mut frame = group_frame(
                hour,
                &[9001],
                &[(77, "Pact", members), (78, "Empty", &[])],
                &[],
            );
            if hour >= 104
                && let TableView::Complete(TableRows::Countries(rows)) =
                    &mut *frame.tables[TableId::Countries.index()]
            {
                rows.iter_mut().find(|row| row.id == 9001).unwrap().tag = b"renamed realm".to_vec();
            }
            let missing = match hour {
                102 => Some(TableId::CountryLinks),
                105 => Some(TableId::FactionMembers),
                _ => None,
            };
            if let Some(table) = missing {
                *frame.tables[table.index()] = TableView::Unavailable {
                    reason: "synthetic group proof gap".into(),
                };
            }
            frame
        })
        .collect();
    let replay = Replay::from_records(CAMPAIGN.into(), &[records(1, frames.clone())]).unwrap();
    let mut oracle = Scan::new(&replay, 1);
    let mut resumed = Scan::new(&replay, 1);
    oracle.begin_block();
    for frame in &frames {
        oracle.observe(frame).unwrap();
    }
    oracle.finish(&replay, 1);
    for batch in frames.chunks(2) {
        resumed.begin_block();
        for frame in batch {
            resumed.observe(frame).unwrap();
        }
        resumed.finish(&replay, 1);
        resumed = Scan::read(resumed.saved_bytes().unwrap().as_slice()).unwrap();
        let proof = resumed.faction_history(&flag).unwrap();
        assert_eq!(
            proof.faction_reads.last().unwrap().last_hour,
            batch.last().unwrap().hour
        );
        assert!(resumed.continuation.rosters.views.is_none());
        assert_eq!(resumed.continuation.rosters.current.len(), 2);
    }
    let expected = oracle.faction_history(&flag).unwrap();
    let actual = resumed.faction_history(&flag).unwrap();
    assert_eq!(actual.factions, expected.factions);
    assert_eq!(actual.country_reads, expected.country_reads);
    assert_eq!(actual.country_presence, expected.country_presence);
    assert_eq!(actual.faction_reads, expected.faction_reads);
    assert_eq!(actual.unobserved, expected.unobserved);
    assert_eq!(actual.incomplete, expected.incomplete);
    assert_eq!(actual.end_recorded, expected.end_recorded);
    assert_eq!(actual.last_hour, expected.last_hour);
    let source = group_source(replay);
    let context = LoadContext { content: None };
    assert_eq!(
        serde_json::to_value(resumed.reply(&source, "1", &context).unwrap()).unwrap(),
        serde_json::to_value(oracle.reply(&source, "1", &context).unwrap()).unwrap()
    );
}

#[test]
fn prepared_group_roster_continuation_grows_with_changes_not_ten_thousand_hours() {
    use super::Scan;
    let mut frame = group_frame(
        10_000,
        &[],
        &[(77, "Pact", &[44, 9001]), (78, "Empty", &[])],
        &[],
    );
    let replay = Replay::from_records(CAMPAIGN.into(), &[records(1, vec![frame.clone()])]).unwrap();
    let mut scan = Scan::new(&replay, 1);
    let evidence = |scan: &Scan| {
        let bytes = scan.saved_bytes().unwrap();
        let (state, _) = Scan::saved_parts(&bytes).unwrap();
        let roster = &state["continuation"]["rosters"];
        assert!(
            roster.is_object(),
            "faction evidence must survive serialization"
        );
        assert!(roster.get("views").is_none());
        assert!(roster.get("current").is_none());
        (
            roster.clone(),
            serde_json::to_vec(roster).unwrap().len(),
            bytes.len(),
        )
    };
    let mut first_size = None;
    for hour in 10_000..20_000 {
        frame.hour = hour;
        if matches!(hour, 10_000 | 15_000) {
            scan.begin_block();
        }
        scan.observe(&frame).unwrap();
        if hour == 10_000 {
            first_size = Some(evidence(&scan));
        } else if hour == 14_999 {
            scan = Scan::read(scan.saved_bytes().unwrap().as_slice()).unwrap();
            assert_eq!(scan.continuation.rosters.current.len(), 2);
        }
    }
    let (steady, steady_size, steady_compressed) = evidence(&scan);
    let (first, first_size, first_compressed) = first_size.unwrap();
    assert_eq!(
        steady_size, first_size,
        "only fixed-width endpoint dates changed"
    );
    assert_eq!(first["factions"]["77"].as_array().unwrap().len(), 1);
    assert_eq!(steady["factions"]["77"].as_array().unwrap().len(), 1);
    assert_eq!(steady["factions"]["78"].as_array().unwrap().len(), 1);
    assert_eq!(steady["country_reads"].as_array().unwrap().len(), 1);
    assert_eq!(steady["faction_reads"].as_array().unwrap().len(), 1);
    assert!(
        steady["country_presence"]
            .as_object()
            .unwrap()
            .values()
            .all(|spans| spans.as_array().unwrap().len() == 1)
    );
    frame = group_frame(20_000, &[], &[(77, "Pact", &[44]), (78, "Empty", &[])], &[]);
    scan.observe(&frame).unwrap();
    for (hour, missing) in [
        (20_001, Some(TableId::CountryLinks)),
        (20_002, None),
        (20_003, Some(TableId::Factions)),
        (20_004, None),
    ] {
        let mut read = frame.clone();
        read.hour = hour;
        if let Some(table) = missing {
            *read.tables[table.index()] = TableView::Unavailable {
                reason: "synthetic proof change".into(),
            };
        }
        scan.observe(&read).unwrap();
    }
    let (changed, changed_size, changed_compressed) = evidence(&scan);
    assert!(changed_size > steady_size);
    assert_eq!(changed["country_reads"].as_array().unwrap().len(), 2);
    assert_eq!(changed["faction_reads"].as_array().unwrap().len(), 2);
    assert!(changed["factions"]["77"].as_array().unwrap().len() > 1);
    println!(
        "roster evidence JSON: {first_size} -> {steady_size} unchanged -> {changed_size} changed; complete compressed scan: {first_compressed} -> {steady_compressed} -> {changed_compressed} bytes"
    );
}

#[test]
fn prepared_groups_snapshot_grows_with_dictionary_changes_not_repeated_reads() {
    use super::{CountryGroupParams, CountryGroups, Scan};
    use crate::products::LoadContext;
    let flag = AtomicBool::new(false);
    let templates = crate::catalogue::tests::loaded_templates(
        "demo={name={localization_key=NAME ID=token:17}}",
    );
    let localisation = crate::catalogue::tests::loaded("l_english:\n NAME:0 \"Pact $ID$\"\n");
    let frame = |hour, additions: &[(u32, &[u8])]| {
        let mut frame = template_frame(hour, "", Some((17, "demo")));
        if let TableView::Complete(TableRows::LexicalTokens(rows)) =
            &mut *frame.tables[TableId::LexicalTokens.index()]
        {
            rows.extend(
                (1_000..1_512).map(|token| replay_data::rows::LexicalTokenRow {
                    token,
                    literal: format!("synthetic_dictionary_entry_{token}").into_bytes(),
                }),
            );
            rows.extend(additions.iter().map(|(token, literal)| {
                replay_data::rows::LexicalTokenRow {
                    token: *token,
                    literal: literal.to_vec(),
                }
            }));
            rows.sort_by_key(|row| row.token);
        }
        frame
    };
    let mut frames: Vec<_> = (100..228).map(|hour| frame(hour, &[(31, b"17")])).collect();
    frames.push(frame(228, &[(31, b"other"), (47, b"17")]));
    frames.push(frame(229, &[(47, b"17"), (55, b"17")]));
    frames.push(frame(230, &[(31, b"17")]));
    let mut unread = frame(232, &[]);
    *unread.tables[TableId::LexicalTokens.index()] = TableView::Truncated {
        rows: TableRows::LexicalTokens(Vec::new()),
        missing: 3,
    };
    frames.push(unread);
    frames.push(frame(233, &[(31, b"17")]));
    frames.extend((234..362).map(|hour| {
        let mut frame = frame(hour, &[]);
        if let TableView::Complete(TableRows::Factions(rows)) =
            &mut *frame.tables[TableId::Factions.index()]
        {
            rows[0].name = b"Literal".to_vec();
            rows[0].template_token = (hour % 2 == 0).then_some(hour as u32);
        }
        *frame.tables[TableId::LexicalTokens.index()] = TableView::Unavailable {
            reason: "synthetic dictionary read failed".into(),
        };
        frame
    }));
    let replay = Replay::from_records(CAMPAIGN.into(), &[records(1, frames.clone())]).unwrap();
    let mut prepared = Scan::new(&replay, 1);
    for (index, frame) in frames.iter().enumerate() {
        prepared.begin_block();
        prepared.observe(frame).unwrap();
        if matches!(index, 63 | 196) {
            prepared.finish(&replay, 1);
            prepared = Scan::read(prepared.saved_bytes().unwrap().as_slice()).unwrap();
        }
    }
    prepared.finish(&replay, 1);
    let bytes = prepared.saved_bytes().unwrap();
    let (state, dictionaries) = Scan::saved_parts(&bytes).unwrap();
    assert!(state.get("rosters").is_none());
    assert!(state["continuation"]["lexical_dictionary"].is_null());
    assert!(state["factions"]["5"].get("names").is_none());
    assert!(
        state["continuation"]["names"]["5"]
            .as_array()
            .unwrap()
            .len()
            <= 7
    );
    let mut input = replay_data::codec::Decoder::new(&dictionaries);
    let saved = crate::faction_names::DictionaryHistory::decode(&mut input).unwrap();
    input.finish().unwrap();
    // Equal rows read again, across both resumes, keep their version.
    assert_eq!(saved.count(), 5);
    let base: usize = (1_000..1_512)
        .map(|token| format!("synthetic_dictionary_entry_{token}").len())
        .sum();
    assert!(
        dictionaries.len() < 2 * base,
        "only the base dictionary and row changes are saved"
    );
    let restored = Scan::read(bytes.as_slice()).unwrap();
    let source = group_source(replay);
    let oracle = CountryGroups::load(
        &source,
        &CountryGroupParams {
            branch_id: "1".into(),
        },
        &LoadContext { content: None },
        &flag,
    )
    .unwrap();
    assert_eq!(
        serde_json::to_value(
            restored
                .reply(&source, "1", &LoadContext { content: None })
                .unwrap()
        )
        .unwrap(),
        serde_json::to_value(oracle).unwrap()
    );
    let resolve = |scan: &Scan| {
        let mut refused = crate::catalogue::Refusals::default();
        let names = serde_json::to_value(scan.faction_groups(
            Ok(&templates),
            &localisation,
            "english",
            &mut refused,
        ))
        .unwrap();
        (names, refused.lines("faction names are"))
    };
    let oracle = super::scan_branch(source.replay_ref(), 1, &flag).unwrap();
    assert_eq!(oracle.continuation.names[&5].len(), 7);
    assert_eq!(resolve(&restored), resolve(&oracle));
    let names = resolve(&restored).0;
    assert_eq!(names[0]["names"][0]["name"], "Pact 31");
    assert_eq!(names[0]["names"][1]["name"], "Pact 47");
    assert!(
        names[0]["names"][2]["reason"]
            .as_str()
            .unwrap()
            .contains("several lexer tokens")
    );
    assert!(
        names[0]["names"][4]["reason"]
            .as_str()
            .unwrap()
            .contains("3 missing rows")
    );
    assert_eq!(names[0]["names"][6]["name"], "Literal");
    assert_eq!(names[0]["names"][6]["firstHour"], 234);
    assert_eq!(names[0]["names"][6]["lastHour"], 361);
}

#[test]
#[ignore = "reference recording measurement"]
fn reference_prepared_groups_state_size_and_reply_parity() {
    let path = PathBuf::from(
        std::env::var("BATTLE_PLAN_GROUPS_RECORDING").or_else(|_| std::env::var("HOI4_GROUPS_RECORDING"))
            .expect("BATTLE_PLAN_GROUPS_RECORDING must name the reference recording"),
    );
    let flag = AtomicBool::new(false);
    let source = crate::Source::open(
        crate::SourceKey {
            source_id: "groups-reference".into(),
            revision: crate::protocol::replay_revision(&path).unwrap(),
        },
        path,
        &flag,
    )
    .unwrap();
    for branch in &source.replay_ref().branches {
        let scan = super::scan_branch(source.replay_ref(), branch.id, &flag).unwrap();
        let bytes = scan.saved_bytes().unwrap();
        println!(
            "country-groups branch {}: {} compressed state bytes",
            branch.id,
            bytes.len()
        );
        let (state, dictionaries) = super::Scan::saved_parts(&bytes).unwrap();
        let json = serde_json::to_vec(&state).unwrap();
        fn profile(label: &str, value: &serde_json::Value) {
            let json = serde_json::to_vec(value).unwrap();
            let count = match value {
                serde_json::Value::Array(rows) => rows.len(),
                serde_json::Value::Object(rows) => rows.len(),
                _ => 1,
            };
            println!(
                "FIELD {label}: count={count} json={} zstd={}",
                json.len(),
                zstd::stream::encode_all(json.as_slice(), 3).unwrap().len()
            );
        }
        for (field, value) in state.as_object().unwrap() {
            profile(field, value);
            if matches!(field.as_str(), "continuation" | "rosters") {
                for (nested, value) in value.as_object().unwrap() {
                    profile(&format!("{field}.{nested}"), value);
                }
            }
        }
        for field in ["names", "members", "leaders", "incomplete"] {
            let rows: Vec<_> = state["factions"]
                .as_object()
                .unwrap()
                .values()
                .filter_map(|faction| faction[field].as_array())
                .flat_map(|rows| rows.iter().cloned())
                .collect();
            profile(
                &format!("factions.{field}"),
                &serde_json::Value::Array(rows),
            );
        }
        println!(
            "DICTIONARIES versions={} bytes={}",
            crate::faction_names::DictionaryHistory::decode(&mut replay_data::codec::Decoder::new(
                &dictionaries
            ))
            .unwrap()
            .count(),
            dictionaries.len()
        );
        let name_runs: usize = state["continuation"]["names"]
            .as_object()
            .unwrap()
            .values()
            .map(|rows| rows.as_array().unwrap().len())
            .sum();
        let identity_runs: usize = state["identities"]
            .as_object()
            .unwrap()
            .values()
            .map(|rows| rows.as_array().unwrap().len())
            .sum();
        println!(
            "COUNTS names={name_runs} identities={identity_runs} blocks={} json_total={}",
            scan.continuation.blocks,
            json.len()
        );
        let restored = super::Scan::read(bytes.as_slice()).unwrap();
        let context = crate::products::LoadContext { content: None };
        assert_eq!(
            serde_json::to_value(
                scan.reply(&source, &branch.id.to_string(), &context)
                    .unwrap()
            )
            .unwrap(),
            serde_json::to_value(
                restored
                    .reply(&source, &branch.id.to_string(), &context)
                    .unwrap()
            )
            .unwrap()
        );
    }
}

#[test]
fn group_changes_keep_quiet_hours_and_closed_history_flat_and_capitulations_skip_other_joins() {
    use super::{Scan, changes::Prepared};
    use crate::prepared::{
        frame_changes::FrameChangeTracker,
        observer_tests::{assert_flat, scaled_work, unchanged_frames},
    };
    let flag = AtomicBool::new(false);
    let context = crate::products::LoadContext { content: None };
    let initial = group_frame(100, &[9001], &[(77, "Pact", &[9001, 44])], &[(9001, 44)]);
    let source = group_source(
        Replay::from_records(CAMPAIGN.into(), &[records(1, vec![initial.clone()])]).unwrap(),
    );
    let format = source.replay_ref().format;
    assert_flat(
        scaled_work(10, |hours| {
            let mut scan = Scan::new(source.replay_ref(), 1);
            let mut events = Prepared::default();
            let mut tracker = FrameChangeTracker::new();
            events.observe_changed(
                &mut scan,
                &initial,
                &tracker.observe(&initial, format),
                &source,
                &context,
            )?;
            unchanged_frames(
                scan.continuation.work,
                &initial,
                hours as i32,
                format,
                |frame, changes| {
                    events.observe_changed(&mut scan, frame, changes, &source, &context)?;
                    Ok(scan.continuation.work)
                },
            )
        })
        .unwrap(),
    );
    assert_flat(
        scaled_work(10, |history| {
            let mut scan = Scan::new(source.replay_ref(), 1);
            let mut events = Prepared::default();
            let mut tracker = FrameChangeTracker::new();
            let mut current = initial.clone();
            for index in 0..history {
                current.hour = 100 + index as i32;
                if let TableView::Complete(TableRows::Factions(rows)) =
                    &mut *current.tables[TableId::Factions.index()]
                {
                    rows[0].name = format!("Pact {index}").into_bytes();
                }
                events.observe_changed(
                    &mut scan,
                    &current,
                    &tracker.observe(&current, format),
                    &source,
                    &context,
                )?;
            }
            unchanged_frames(
                scan.continuation.work,
                &current,
                10,
                format,
                |frame, changes| {
                    events.observe_changed(&mut scan, frame, changes, &source, &context)?;
                    Ok(scan.continuation.work)
                },
            )
        })
        .unwrap(),
    );
    let mut scan = Scan::new(source.replay_ref(), 1);
    let mut events = Prepared::default();
    let mut tracker = FrameChangeTracker::new();
    events
        .observe_changed(
            &mut scan,
            &initial,
            &tracker.observe(&initial, format),
            &source,
            &context,
        )
        .unwrap();
    let before = scan.continuation.work;
    let mut changed = initial.clone();
    changed.hour += 1;
    *changed.tables[TableId::Capitulation.index()] =
        TableView::Complete(TableRows::Capitulation(vec![
            replay_data::rows::CapitulationRow {
                country: 44,
                capitulated: 1,
                capitulated_hour: 101,
                ..Default::default()
            },
        ]));
    events
        .observe_changed(
            &mut scan,
            &changed,
            &tracker.observe(&changed, format),
            &source,
            &context,
        )
        .unwrap();
    assert_eq!(
        scan.continuation.work.row_visits - before.row_visits,
        1,
        "a capitulation changes no membership or naming join"
    );
    assert_eq!(
        scan.continuation.member_open.len(),
        2,
        "independent updates must not close members"
    );
    assert_eq!(
        scan.continuation.war_open.len(),
        1,
        "independent updates must not close wars"
    );
    scan.finish(source.replay_ref(), 1);
    assert_eq!(
        scan.faction_history(&flag).unwrap().factions["77"]
            .last()
            .unwrap()
            .last_hour,
        101
    );
}

#[test]
fn group_deferred_open_state_matches_oracle_after_quiet_commits_resume_append_missing_tables_and_expiry()
 {
    use super::{Scan, changes::Prepared};
    use crate::prepared::frame_changes::FrameChangeTracker;
    let flag = AtomicBool::new(false);
    let context = crate::products::LoadContext { content: None };
    for missing in [None, Some(TableId::FactionMembers)] {
        let mut initial = group_frame(100, &[9001], &[(77, "Pact", &[9001, 44])], &[(9001, 44)]);
        if let TableView::Complete(TableRows::WarRelations(rows)) =
            &mut *initial.tables[TableId::WarRelations.index()]
        {
            rows[0].end_hour = 109;
        }
        if let Some(table) = missing {
            *initial.tables[table.index()] = TableView::Unavailable {
                reason: "synthetic missing membership".into(),
            };
        }
        let mut frames: Vec<_> = (100..=110)
            .map(|hour| {
                let mut frame = initial.clone();
                frame.hour = hour;
                frame
            })
            .collect();
        let mut after = initial.clone();
        after.hour = 113;
        frames.push(after.clone());
        after.hour = 114;
        frames.push(after);
        let source = group_source(
            Replay::from_records(CAMPAIGN.into(), &[records(1, frames.clone())]).unwrap(),
        );
        let mut fixture = crate::prepared::series::tests::Fixture::new();
        let mut scan = Scan::new(source.replay_ref(), 1);
        let mut events = Prepared::default();
        for (index, batch) in [&frames[..6], &frames[6..11], &frames[11..]]
            .into_iter()
            .enumerate()
        {
            scan.begin_block();
            let mut tracker = FrameChangeTracker::new();
            for frame in batch {
                events
                    .observe_changed(
                        &mut scan,
                        frame,
                        &tracker.observe(frame, source.replay_ref().format),
                        &source,
                        &context,
                    )
                    .unwrap();
            }
            let mut writer = if index == 2 {
                fixture.append_writer()
            } else {
                fixture.writer()
            };
            events.flush(&mut writer, 1, &scan, &flag).unwrap();
            writer
                .write_at(
                    "country-groups-1",
                    0,
                    scan.saved_bytes().unwrap().as_slice(),
                )
                .unwrap();
            writer.commit().unwrap();
            scan = Scan::read(
                fixture
                    .store
                    .read(&fixture.key, "country-groups-1", 0)
                    .unwrap()
                    .unwrap(),
            )
            .unwrap();
            events = serde_json::from_slice(&serde_json::to_vec(&events).unwrap()).unwrap();
        }
        scan.finish(source.replay_ref(), 1);
        let expected = super::scan_branch(source.replay_ref(), 1, &flag).unwrap();
        crate::prepared::observer_tests::assert_reply_parity(
            &scan.reply(&source, "1", &context).unwrap(),
            &expected.reply(&source, "1", &context).unwrap(),
        )
        .unwrap();
        let actual = scan.faction_history(&flag).unwrap();
        let oracle = expected.faction_history(&flag).unwrap();
        assert_eq!(actual.factions, oracle.factions);
        assert_eq!(actual.country_reads, oracle.country_reads);
        assert_eq!(actual.country_presence, oracle.country_presence);
        assert_eq!(actual.faction_reads, oracle.faction_reads);
        assert_eq!(
            actual.country_reads,
            vec![
                super::HourSpan {
                    first_hour: 100,
                    last_hour: 110
                },
                super::HourSpan {
                    first_hour: 113,
                    last_hour: 114
                },
            ]
        );
        assert_eq!(scan.wars()[0].last_hour, 108);
    }
}

#[test]
fn group_membership_change_reads_deferred_unchanged_faction_name() {
    use super::{Scan, changes::Prepared};
    use crate::prepared::{
        frame_changes::FrameChangeTracker,
        series::{Reader, tests::Fixture},
    };
    let flag = AtomicBool::new(false);
    let context = crate::products::LoadContext { content: None };
    let initial = group_frame(100, &[], &[(77, "Pact", &[9001])], &[]);
    let mut frames: Vec<_> = (100..=105)
        .map(|hour| {
            let mut frame = initial.clone();
            frame.hour = hour;
            frame
        })
        .collect();
    let mut join = initial.clone();
    join.hour = 106;
    let changed = group_frame(106, &[], &[(77, "Pact", &[9001, 44])], &[]);
    join.tables[TableId::FactionMembers.index()] =
        changed.tables[TableId::FactionMembers.index()].clone();
    frames.push(join);
    let source =
        group_source(Replay::from_records(CAMPAIGN.into(), &[records(1, frames.clone())]).unwrap());
    let actual = Fixture::new();
    let expected = Fixture::new();
    let mut scan = Scan::new(source.replay_ref(), 1);
    let mut oracle = Scan::new(source.replay_ref(), 1);
    let mut events = Prepared::default();
    let mut oracle_events = Prepared::default();
    let mut tracker = FrameChangeTracker::new();
    for frame in &frames {
        events
            .observe_changed(
                &mut scan,
                frame,
                &tracker.observe(frame, source.replay_ref().format),
                &source,
                &context,
            )
            .unwrap();
        oracle_events
            .observe(&mut oracle, frame, &source, &context)
            .unwrap();
    }
    for (fixture, events, scan) in [
        (&actual, &mut events, &scan),
        (&expected, &mut oracle_events, &oracle),
    ] {
        let mut writer = fixture.writer();
        events.flush(&mut writer, 1, scan, &flag).unwrap();
        writer.commit().unwrap();
    }
    let mut actual_rows = Reader::new(
        &actual.store,
        &actual.key,
        "country-group-changes-1".into(),
        &flag,
    )
    .unwrap();
    let mut expected_rows = Reader::new(
        &expected.store,
        &expected.key,
        "country-group-changes-1".into(),
        &flag,
    )
    .unwrap();
    let actual = actual_rows.events(&[0], 100, 106, 0, 10).unwrap();
    let expected = expected_rows.events(&[0], 100, 106, 0, 10).unwrap();
    assert_eq!(
        actual
            .rows
            .iter()
            .map(|row| (row.key, row.hour, row.ordinal, &row.row))
            .collect::<Vec<_>>(),
        expected
            .rows
            .iter()
            .map(|row| (row.key, row.hour, row.ordinal, &row.row))
            .collect::<Vec<_>>()
    );
    assert_eq!(actual.rows.len(), 1);
    assert_eq!(actual.rows[0].row["name"]["Ok"], "Pact");
}

#[test]
fn group_projected_holder_change_reads_only_changed_country_operands() {
    use super::{Scan, changes::Prepared};
    let initial = group_frame(100, &[], &[(77, "Pact", &[9001, 44])], &[]);
    let mut renamed = initial.clone();
    renamed.hour = 101;
    if let TableView::Complete(TableRows::Countries(rows)) =
        &mut *renamed.tables[TableId::Countries.index()]
    {
        rows[0].tag = b"renamed-holder".to_vec();
    }
    let source = group_source(
        Replay::from_records(
            CAMPAIGN.into(),
            &[records(1, vec![initial.clone(), renamed.clone()])],
        )
        .unwrap(),
    );
    let context = crate::products::LoadContext { content: None };
    let mut scan = Scan::new(source.replay_ref(), 1);
    let mut events = Prepared::default();
    let mut tracker = crate::prepared::frame_changes::FrameChangeTracker::new();
    events
        .observe_changed(
            &mut scan,
            &initial,
            &tracker.observe(&initial, source.replay_ref().format),
            &source,
            &context,
        )
        .unwrap();
    let before = scan.continuation.work;
    events
        .observe_changed(
            &mut scan,
            &renamed,
            &tracker.observe(&renamed, source.replay_ref().format),
            &source,
            &context,
        )
        .unwrap();
    assert_eq!(
        scan.continuation.work.row_visits - before.row_visits,
        renamed
            .complete::<replay_data::rows::tables::Countries>()
            .unwrap()
            .len(),
        "unchanged native links and membership tables stay unread"
    );
    scan.finish(source.replay_ref(), 1);
    let expected = super::scan_branch(source.replay_ref(), 1, &AtomicBool::new(false)).unwrap();
    crate::prepared::observer_tests::assert_reply_parity(
        &scan.reply(&source, "1", &context).unwrap(),
        &expected.reply(&source, "1", &context).unwrap(),
    )
    .unwrap();
}

#[test]
fn group_dictionary_revisions_match_oracle_across_two_commits_resume_append_and_equal_reads() {
    use super::{Scan, changes::Prepared};
    use crate::prepared::frame_changes::FrameChangeTracker;
    let context = crate::products::LoadContext { content: None };
    let flag = AtomicBool::new(false);
    let initial = template_frame(100, "", Some((9, "mod_template")));
    let frames: Vec<_> = (100..=103)
        .map(|hour| {
            let mut frame = initial.clone();
            frame.hour = hour;
            if hour >= 102
                && let TableView::Complete(TableRows::LexicalTokens(rows)) =
                    &mut *frame.tables[TableId::LexicalTokens.index()]
            {
                rows[0].literal = b"later_template".to_vec();
            }
            frame
        })
        .collect();
    let original =
        group_source(Replay::from_records(CAMPAIGN.into(), &[records(1, frames.clone())]).unwrap());
    let mut scan = Scan::new(original.replay_ref(), 1);
    let mut events = Prepared::default();
    let fixture = crate::prepared::series::tests::Fixture::new();
    for batch in frames.chunks(2) {
        scan.begin_block();
        let mut tracker = FrameChangeTracker::new();
        for frame in batch {
            events
                .observe_changed(
                    &mut scan,
                    frame,
                    &tracker.observe(frame, original.replay_ref().format),
                    &original,
                    &context,
                )
                .unwrap();
        }
        let mut writer = fixture.writer();
        events.flush(&mut writer, 1, &scan, &flag).unwrap();
        writer
            .write_at(
                "country-groups-1",
                0,
                scan.saved_bytes().unwrap().as_slice(),
            )
            .unwrap();
        writer.commit().unwrap();
        scan = Scan::read(
            fixture
                .store
                .read(&fixture.key, "country-groups-1", 0)
                .unwrap()
                .unwrap(),
        )
        .unwrap();
        events = serde_json::from_slice(&serde_json::to_vec(&events).unwrap()).unwrap();
    }
    let mut appended = frames.last().unwrap().clone();
    appended.hour = 104;
    appended.metrics.table_ns[TableId::LexicalTokens.index()] = 1;
    let tail = continuation_records(original.replay_ref(), 1, vec![appended.clone()]);
    let source = group_source(original.replay_ref().append(&[tail]).unwrap());
    scan.begin_block();
    let mut tracker = FrameChangeTracker::new();
    events
        .observe_changed(
            &mut scan,
            &appended,
            &tracker.observe(&appended, source.replay_ref().format),
            &source,
            &context,
        )
        .unwrap();
    scan.finish(source.replay_ref(), 1);
    let oracle = super::scan_branch(source.replay_ref(), 1, &flag).unwrap();
    crate::prepared::observer_tests::assert_reply_parity(
        &scan.reply(&source, "1", &context).unwrap(),
        &oracle.reply(&source, "1", &context).unwrap(),
    )
    .unwrap();
    assert_eq!(scan.faction_name_at(5, 104), oracle.faction_name_at(5, 104));
}

#[test]
fn group_quiet_missing_coverage_holds_its_snapshot_until_publication() {
    use super::{Scan, changes::Prepared};
    let context = crate::products::LoadContext { content: None };
    let mut first = group_frame(100, &[], &[(77, "Pact", &[9001])], &[]);
    *first.tables[TableId::HumanControllers.index()] = TableView::Unavailable {
        reason: "missing humans".into(),
    };
    let frames: Vec<_> = (100..=110)
        .map(|hour| {
            let mut frame = first.clone();
            frame.hour = hour;
            frame
        })
        .collect();
    let source =
        group_source(Replay::from_records(CAMPAIGN.into(), &[records(1, frames.clone())]).unwrap());
    let mut scan = Scan::new(source.replay_ref(), 1);
    let mut events = Prepared::default();
    let mut tracker = crate::prepared::frame_changes::FrameChangeTracker::new();
    events
        .observe_changed(
            &mut scan,
            &first,
            &tracker.observe(&first, source.replay_ref().format),
            &source,
            &context,
        )
        .unwrap();
    let snapshot = scan.continuation.unobserved.clone();
    for frame in &frames[1..] {
        events
            .observe_changed(
                &mut scan,
                frame,
                &tracker.observe(frame, source.replay_ref().format),
                &source,
                &context,
            )
            .unwrap();
        assert!(
            Arc::ptr_eq(&snapshot, &scan.continuation.unobserved),
            "quiet hours must not copy accumulated missing coverage"
        );
    }
    assert_eq!(snapshot.humans[0].last_hour, 100);
    scan.finish(source.replay_ref(), 1);
    assert_eq!(scan.unobserved.humans[0].last_hour, 110);
    assert_eq!(snapshot.humans[0].last_hour, 100);
    let oracle = super::scan_branch(source.replay_ref(), 1, &AtomicBool::new(false)).unwrap();
    crate::prepared::observer_tests::assert_reply_parity(
        &scan.reply(&source, "1", &context).unwrap(),
        &oracle.reply(&source, "1", &context).unwrap(),
    )
    .unwrap();
}

#[test]
fn restored_group_scan_saves_what_continuous_observation_saves() {
    // A template-only faction name whose token the recorded LexicalTokens
    // does not spell: its template never resolves or reads the dictionary.
    let frames: Vec<_> = (100..108)
        .map(|hour| {
            let mut frame = template_frame(hour, "", Some((17, "demo")));
            *frame.tables[TableId::LexicalTokens.index()] =
                TableView::Complete(TableRows::LexicalTokens(vec![
                    replay_data::rows::LexicalTokenRow {
                        token: 3,
                        literal: b"other".to_vec(),
                    },
                ]));
            frame
        })
        .collect();
    let replay = Replay::from_records(CAMPAIGN.into(), &[records(1, frames.clone())]).unwrap();
    let blocks = [&frames[..4], &frames[4..]];
    crate::prepared::observer_tests::assert_restore_continues(
        replay.format,
        &blocks,
        || super::Scan::new(&replay, 1),
        |scan, frame, changes, _| {
            if blocks.iter().any(|block| block[0].hour == frame.hour) {
                scan.begin_block();
            }
            scan.advance_changed(frame, changes).unwrap();
        },
        |scan, writer, _| {
            scan.finish(&replay, 1);
            writer
                .write_at(
                    "country-groups-1",
                    0,
                    scan.saved_bytes().unwrap().as_slice(),
                )
                .unwrap();
        },
        |fixture| {
            super::Scan::read(
                fixture
                    .store
                    .read(&fixture.key, "country-groups-1", 0)
                    .unwrap()
                    .unwrap(),
            )
            .unwrap()
        },
    );
}

#[test]
fn dictionary_versions_read_each_hour_s_rows_and_resume_as_continuous_scans_for_1500_countries() {
    use super::{Scan, changes::Prepared};
    use crate::{
        faction_names::{LexicalNames, template_spelling},
        prepared::{
            frame_changes::FrameChangeTracker,
            series::tests::{Fixture, assert_payloads_equal},
        },
    };
    use replay_data::rows::{FactionMemberRow, FactionRow, LexicalTokenRow, tables};
    use std::collections::{BTreeMap, BTreeSet};
    let templates = crate::catalogue::tests::loaded_templates(
        "pact_a={name={localization_key=NAME ID=token:arg_a}} \
         pact_b={name={localization_key=NAME ID=token:arg_b}} plain={name=PLAIN}",
    );
    let localisation = crate::catalogue::tests::loaded(
        "l_english:\n NAME:0 \"Pact $ID$\"\n PLAIN:0 \"Plain Pact\"\n",
    );
    let dictionary = |hour: i32| {
        if hour % 17 == 16 {
            return TableView::Unavailable {
                reason: "synthetic dictionary gap".into(),
            };
        }
        let phase = hour as u32 / 6;
        let mut rows: BTreeMap<u32, Vec<u8>> = (10..2_010)
            .map(|token| (token, format!("entry_{token}").into_bytes()))
            .chain(
                [(1, "pact_a"), (2, "pact_b"), (3, "plain")]
                    .map(|(token, literal)| (token, literal.as_bytes().to_vec())),
            )
            .chain((3_000..3_000 + phase).map(|token| (token, format!("late_{token}").into_bytes())))
            .collect();
        rows.insert(100 + phase, b"arg_a".to_vec());
        rows.insert(500 + phase % 3, b"arg_b".to_vec());
        if phase % 4 == 3 {
            rows.insert(1_999, b"ARG_B".to_vec());
        }
        if phase % 5 == 4 {
            rows.remove(&(100 + phase));
        }
        let rows = TableRows::LexicalTokens(
            rows.into_iter()
                .map(|(token, literal)| LexicalTokenRow { token, literal })
                .collect(),
        );
        if hour % 23 == 22 {
            TableView::Truncated { rows, missing: 2 }
        } else {
            TableView::Complete(rows)
        }
    };
    let factions = |hour: i32| {
        let mut factions = Vec::new();
        let mut members = Vec::new();
        for id in 1..=12_u64 {
            // One founded late and one dissolved; one named literally over its template.
            if (id == 11 && hour < 20) || (id == 12 && hour >= 40) {
                continue;
            }
            let (name, template_token) = match id % 3 {
                0 => (format!("Literal {id}").into_bytes(), None),
                1 => (Vec::new(), Some(1)),
                _ if id == 8 && hour >= 30 => (b"Renamed eight".to_vec(), Some(2)),
                _ => (Vec::new(), Some(if id == 5 { 3 } else { 2 })),
            };
            factions.push(FactionRow {
                id,
                name,
                template_token,
                ..Default::default()
            });
            // A hundred members each; every ten hours one country moves on.
            let first = (id as u32 - 1) * 100 + hour as u32 / 10;
            members.extend(
                (first..first + 100)
                    .enumerate()
                    .map(|(ordinal, country)| FactionMemberRow {
                        country: 100_000 + country,
                        faction: id,
                        ordinal: ordinal as u32,
                    }),
            );
        }
        members.sort_by_key(|member| member.country);
        (
            TableView::Complete(TableRows::Factions(factions)),
            TableView::Complete(TableRows::FactionMembers(members)),
        )
    };
    let mut frames: Vec<DecodedFrame> = Vec::new();
    for hour in 0..96 {
        let mut frame = frames.last().cloned().unwrap_or_else(|| {
            let mut frame = crate::test_fixtures::scale_frame(1, 0, 1_500);
            *frame.tables[TableId::HumanControllers.index()] =
                TableView::Complete(TableRows::HumanControllers(vec![
                    replay_data::rows::HumanControllerRow {
                        country: 100_000,
                        count: 1,
                    },
                ]));
            *frame.tables[TableId::WarRelations.index()] =
                TableView::Complete(TableRows::WarRelations(Vec::new()));
            *frame.tables[TableId::Capitulation.index()] =
                TableView::Complete(TableRows::Capitulation(Vec::new()));
            frame
        });
        frame.hour = 1_000 + hour;
        // Unchanged tables share the hour before's allocation, as decoded.
        let (named, members) = factions(hour);
        for (table, view) in [
            (TableId::LexicalTokens, dictionary(hour)),
            (TableId::Factions, named),
            (TableId::FactionMembers, members),
        ] {
            if frames
                .last()
                .is_none_or(|last| *last.tables[table.index()] != view)
            {
                frame.tables[table.index()] = view.into();
            }
        }
        frames.push(frame);
    }
    let source =
        group_source(Replay::from_records(CAMPAIGN.into(), &[records(1, frames.clone())]).unwrap());
    let context = crate::products::LoadContext { content: None };
    let flag = AtomicBool::new(false);
    let fixtures = [Fixture::new(), Fixture::new()];
    let mut observers = [
        (Scan::new(source.replay_ref(), 1), Prepared::default()),
        (Scan::new(source.replay_ref(), 1), Prepared::default()),
    ];
    for (ordinal, block) in frames.chunks(8).enumerate() {
        for ((scan, events), fixture) in observers.iter_mut().zip(&fixtures) {
            scan.begin_block();
            let mut tracker = FrameChangeTracker::new();
            for frame in block {
                events
                    .observe_changed(
                        scan,
                        frame,
                        &tracker.observe(frame, source.replay_ref().format),
                        &source,
                        &context,
                    )
                    .unwrap();
            }
            scan.finish(source.replay_ref(), 1);
            let mut writer = fixture.writer();
            events.flush(&mut writer, 1, scan, &flag).unwrap();
            writer
                .write_at(
                    "country-groups-1",
                    0,
                    scan.saved_bytes().unwrap().as_slice(),
                )
                .unwrap();
            writer
                .write_at(
                    "country-group-state-1",
                    0,
                    serde_json::to_vec(&*events).unwrap().as_slice(),
                )
                .unwrap();
            writer.commit().unwrap();
        }
        assert_payloads_equal(
            &fixtures[1].payloads(),
            &fixtures[0].payloads(),
            &format!("resumed vs continuous after commit {ordinal}"),
        );
        let read = |section: &str| {
            fixtures[1]
                .store
                .read(&fixtures[1].key, section, 0)
                .unwrap()
                .unwrap()
        };
        observers[1] = (
            Scan::read(read("country-groups-1")).unwrap(),
            serde_json::from_reader(read("country-group-state-1")).unwrap(),
        );
    }
    let [(continuous, _), (resumed, _)] = observers;
    let mut read = 0;
    let mut names = BTreeSet::new();
    for frame in &frames {
        let expected = LexicalNames::new(frame.tables[TableId::LexicalTokens.index()].clone());
        for row in frame.complete::<tables::Factions>().unwrap() {
            let resolve = |scan: &Scan| {
                scan.resolved_faction_name_at(
                    row.id,
                    frame.hour,
                    Ok(&templates),
                    &localisation,
                    "english",
                )
            };
            let name = resolve(&continuous);
            assert_eq!(
                resolve(&resumed),
                name,
                "faction {} at {}",
                row.id,
                frame.hour
            );
            names.insert(name.unwrap_or_else(|reason| reason));
            let template = template_spelling(frame, row.template_token);
            for scan in [&continuous, &resumed] {
                let operands = scan.faction_name_operands(row.id, frame.hour).unwrap();
                assert_eq!(operands.literal, row.name);
                // A literal name keeps the template spelled where it began.
                if row.name.is_empty() {
                    assert_eq!(
                        operands.template, template,
                        "faction {} at {}",
                        row.id, frame.hour
                    );
                }
                // Only a resolved template holds its hour's dictionary.
                assert_eq!(
                    operands.tokens.is_some(),
                    row.name.is_empty() && matches!(template, Ok(Some(_)))
                );
                let Some(tokens) = &operands.tokens else {
                    continue;
                };
                read += 1;
                for name in [
                    &b"arg_a"[..],
                    b"ARG_B",
                    b"Arg_B",
                    b"pact_b",
                    b"entry_1500",
                    b"",
                ] {
                    assert_eq!(
                        tokens.token_of(name),
                        expected.token_of(name),
                        "faction {} at {} reads {}",
                        row.id,
                        frame.hour,
                        String::from_utf8_lossy(name)
                    );
                }
            }
        }
    }
    assert!(read > 1_000);
    for expected in [
        "Pact 100",
        "Pact 502",
        "Plain Pact",
        "Literal 3",
        "Renamed eight",
    ] {
        assert!(names.contains(expected), "{expected}: {names:?}");
    }
    for reason in [
        "ARG_B names several lexer tokens",
        "token arg_a is absent from LexicalTokens",
        "synthetic dictionary gap",
        "2 missing rows",
    ] {
        assert!(
            names.iter().any(|name| name.contains(reason)),
            "{reason}: {names:?}"
        );
    }
    let groups = |scan: &Scan| {
        serde_json::to_value(scan.faction_groups(
            Ok(&templates),
            &localisation,
            "english",
            &mut crate::catalogue::Refusals::default(),
        ))
        .unwrap()
    };
    assert_eq!(groups(&resumed), groups(&continuous));
    crate::prepared::observer_tests::assert_reply_parity(
        &resumed.reply(&source, "1", &context).unwrap(),
        &continuous.reply(&source, "1", &context).unwrap(),
    )
    .unwrap();
    let (_, dictionaries) = Scan::saved_parts(&continuous.saved_bytes().unwrap()).unwrap();
    let one_table: usize = (10..2_010_u32)
        .map(|token| format!("entry_{token}").len() + 4)
        .sum();
    assert!(continuous.continuation.dictionaries.count() > 16);
    assert!(dictionaries.len() < 2 * one_table);
}

#[test]
fn restored_groups_refuse_dictionary_versions_their_names_did_not_read() {
    use super::Scan;
    let frame = template_frame(100, "", Some((17, "demo")));
    let replay = Replay::from_records(CAMPAIGN.into(), &[records(1, vec![frame.clone()])]).unwrap();
    let mut scan = Scan::new(&replay, 1);
    scan.observe(&frame).unwrap();
    scan.finish(&replay, 1);
    let (state, dictionaries) = Scan::saved_parts(&scan.saved_bytes().unwrap()).unwrap();
    assert_eq!(dictionaries[0], 1, "one version, counted in one byte");
    let read = |dictionaries: &[u8]| {
        Scan::read(
            Scan::saved_from_parts(&state, dictionaries)
                .unwrap()
                .as_slice(),
        )
    };
    assert!(read(&dictionaries).is_ok());
    let mut unread = dictionaries.clone();
    unread[0] = 2;
    let mut full = Vec::new();
    replay_data::codec::put_varint(&mut full, u32::MAX.into());
    full.push(0);
    for tampered in [unread, full] {
        let error = read(&tampered).err().expect("an unread version is refused");
        assert_eq!(
            error.to_string(),
            "Country group dictionaries are not the ones their names read"
        );
    }
}
