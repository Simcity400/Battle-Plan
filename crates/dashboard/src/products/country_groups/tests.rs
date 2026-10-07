use crate::test_fixtures::{CAMPAIGN, ownership_frame, records};
use replay::{
    container::Replay,
    frame::{DecodedFrame, TableView},
    rows::{TableId, TableRows},
};
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
        let countries = 5_000;
        let mut frame = scale_frame(1, 100, countries);
        let wars = (0..countries)
            .flat_map(|first| {
                (first + 1..countries).map(move |second| replay::rows::WarRelationRow {
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
            .state::<Vec<(u64, Arc<super::Scan>)>>()
            .lock()
            .unwrap()
            .push((1, scan));
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
        } else {
            assert_eq!(
                reply.unwrap_err().to_string(),
                "The complete country group reply is too large; nothing was omitted"
            );
        }
    }
}

#[test]
fn country_groups_rescan_evicted_branches_without_losing_reply_data() {
    use super::{CACHED_BRANCHES, CountryGroupParams, CountryGroups, Scan};
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
    let cache = source.state::<Vec<(u64, Arc<Scan>)>>();
    let original = Arc::clone(&cache.lock().unwrap()[0].1);
    for branch in 2..=CACHED_BRANCHES as u64 + 1 {
        load(branch);
    }
    {
        let cached = cache.lock().unwrap();
        assert_eq!(cached.len(), CACHED_BRANCHES);
        assert!(cached.iter().all(|(branch, _)| *branch != 1));
    }
    assert_eq!(serde_json::to_value(load(1)).unwrap(), first);
    let cached = cache.lock().unwrap();
    let rescanned = &cached.iter().find(|(branch, _)| *branch == 1).unwrap().1;
    assert!(!Arc::ptr_eq(&original, rescanned));
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
        .map(|&country| replay::rows::HumanControllerRow { country, count: 1 })
        .collect();
    humans.sort_by_key(|row| row.country);
    *frame.tables[TableId::HumanControllers.index()] =
        TableView::Complete(TableRows::HumanControllers(humans));
    *frame.tables[TableId::Factions.index()] = TableView::Complete(TableRows::Factions(
        factions
            .iter()
            .map(|&(id, name, _)| replay::rows::FactionRow {
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
                replay::rows::FactionMemberRow {
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
            |&(first_country, second_country)| replay::rows::WarRelationRow {
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
    replay.gaps.push(replay::branches::Gap {
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
            .map(|(token, literal)| replay::rows::LexicalTokenRow {
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
    templates: &installed_content::faction_templates::Catalog,
    localisation: &installed_content::localisation::Localisation,
) -> Vec<super::FactionGroup> {
    let replay = Replay::from_records(CAMPAIGN.into(), &[records(1, frames)]).unwrap();
    let scan = super::scan_branch(&replay, 1, &AtomicBool::new(false)).unwrap();
    scan.faction_groups(
        Ok(templates),
        localisation,
        "english",
        &mut crate::catalogue::Refusals::default(),
    )
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
    assert_eq!(
        names[2].reason.as_deref(),
        Some("Faction name has no plain display text"),
    );
    assert!(names.iter().all(|run| run.name.is_none()));
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
        Some("faction name (140D77360) is not UTF-8")
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
    let layers =
        installed_content::ResolvedLayers::from_folders(&[(&vanilla.0, &[][..]), (&modded.0, &[][..])])
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
fn country_groups_faction_names_keep_empty_bound_and_plain_text_reasons() {
    let templates =
        crate::catalogue::tests::loaded_templates("unbound = { } icon = { name = ICON_NAME }");
    let localisation =
        crate::catalogue::tests::loaded("l_english:\n ICON_NAME:0 \"\u{a3}missing_icon\"\n");
    let factions = named_factions(
        vec![
            template_frame(100, "", Some((17, "unbound"))),
            template_frame(101, "", Some((18, "icon"))),
        ],
        &templates,
        &localisation,
    );
    assert!(
        factions[0].names[0]
            .reason
            .as_ref()
            .unwrap()
            .contains("has no bound name")
    );
    assert_eq!(
        factions[0].names[1].reason.as_deref(),
        Some("Faction name has no plain display text")
    );
    assert!(factions[0].names.iter().all(|run| run.name.is_none()));
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
    let mut localisation = installed_content::localisation::Localisation::default();
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
                replay::rows::WarRelationRow {
                    first_country: 44,
                    second_country: 9001,
                    start_hour: 100,
                    first_was_instigator: 0,
                    ..Default::default()
                },
            ]));
        *frame.tables[TableId::Capitulation.index()] =
            TableView::Complete(TableRows::Capitulation(vec![
                replay::rows::CapitulationRow {
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
fn country_groups_faction_names_keep_all_changes_or_refuse_the_complete_reply() {
    use super::{CountryGroupParams, CountryGroups, NameOperands, Scan};
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
        // Exercise the reply's new name history without growing capped membership histories.
        let tokens = Arc::clone(&faction.names[0].tokens);
        faction.names = (0..count)
            .map(|hour| NameOperands {
                first_hour: hour as i32,
                last_hour: hour as i32,
                literal: format!("{hour}:{tail}").into_bytes(),
                template_token: None,
                lexical_read: 0,
                tokens: Arc::clone(&tokens),
                template: Err("Faction template not recorded".into()),
            })
            .collect();
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
            .state::<Vec<(u64, Arc<Scan>)>>()
            .lock()
            .unwrap()
            .push((1, Arc::new(scan)));
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
    let templates = installed_content::faction_templates::Catalog::default();
    let mut localisation = installed_content::localisation::Localisation::default();
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
            "faction name (140D77360) is text the game stored in english without its key, and the viewer's language is french"
        )
    );
}

#[test]
fn country_groups_faction_names_restart_when_the_same_tokens_dictionary_recovers() {
    let templates = crate::catalogue::tests::loaded_templates("demo = { name = DEMO_NAME }");
    let localisation = crate::catalogue::tests::loaded("l_english:\n DEMO_NAME:0 \"Demo Pact\"\n");
    let mut missing = template_frame(101, "", Some((17, "demo")));
    *missing.tables[TableId::LexicalTokens.index()] = TableView::Unavailable {
        reason: "missing".into(),
    };
    let factions = named_factions(
        vec![
            template_frame(100, "", Some((17, "demo"))),
            missing,
            template_frame(102, "", Some((17, "demo"))),
        ],
        &templates,
        &localisation,
    );
    let names = &factions[0].names;
    assert_eq!(names.len(), 3);
    assert_eq!(names[0].name.as_deref(), Some("Demo Pact"));
    assert!(names[1].reason.as_ref().unwrap().contains("LexicalTokens"));
    assert_eq!(names[2].name.as_deref(), Some("Demo Pact"));
}

#[test]
fn country_groups_faction_names_do_not_start_lifetime_on_an_unread_roster() {
    let mut first = template_frame(100, "First name", None);
    *first.tables[TableId::FactionMembers.index()] = TableView::Unavailable {
        reason: "missing".into(),
    };
    let templates = installed_content::faction_templates::Catalog::default();
    let mut localisation = installed_content::localisation::Localisation::default();
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
    use crate::products::{ContentOptions, LoadContext, Product};
    let (_, folder) = crate::catalogue::tests::folder_catalogue(&[]);
    std::fs::write(folder.0.join("settings.txt"), "language=\"l_french\"").unwrap();
    let context = LoadContext {
        content: Some(ContentOptions {
            options: installed_content::ResolveOptions {
                game_dir: folder.0.join("unavailable-game"),
                user_dir: folder.0.clone(),
                cache_dir: folder.0.join("cache"),
            },
            allow_unverified: false,
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
            "faction name (140D77360) is text the game stored in english without its key, and the viewer's language is french"
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
    let layers = installed_content::ResolvedLayers::from_folders(&[(&folder.0, &[][..])]).unwrap();
    let content = Arc::new(installed_content::content::ResolvedContent {
        layers,
        stats: Default::default(),
    });
    let template_warnings = content.layers.faction_templates().unwrap().warnings.clone();
    assert!(!template_warnings.is_empty());
    assert!(
        !installed_content::localisation::read(&content.layers, "english")
            .unwrap()
            .logged
            .is_empty()
    );
    // A verified interface file changed: the full catalogue must fail to read it.
    std::fs::write(folder.0.join("interface/test.gfx"), "changed").unwrap();
    assert!(installed_content::gfx::read(&content.layers).is_err());
    let options = ContentOptions {
        options: installed_content::ResolveOptions {
            game_dir: folder.0.clone(),
            user_dir: folder.0.clone(),
            cache_dir: folder.0.join("cache"),
        },
        allow_unverified: true,
    };
    let source = faction_source(template_frame(100, "", Some((17, "demo"))));
    source.set_test_content(options.options.clone(), content);
    let reply = super::CountryGroups::load(
        &source,
        &super::CountryGroupParams {
            branch_id: "1".into(),
        },
        &LoadContext {
            content: Some(options),
        },
        &AtomicBool::new(false),
    )
    .unwrap();
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
                rows.push(replay::rows::LexicalTokenRow {
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
