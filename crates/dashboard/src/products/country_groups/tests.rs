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
fn country_groups_leave_out_unknown_countries_and_localise_faction_keys() {
    use crate::products::country_groups::{faction_name, scan_branch};
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
    assert_eq!(faction_name("", None), Ok(None));
    assert_eq!(faction_name("My Pact", None), Ok(Some("My Pact".into())));
    let localisation = crate::catalogue::tests::loaded(concat!(
        "l_english:\n",
        " AXIS_NAME:0 \"Achse\"\n",
        " SCOPED_NAME:0 \"[ROOT.GetAdjective] Pact\"\n",
        " PRICE_NAME:0 \"Pakt $$ ]\"\n",
    ));
    assert_eq!(
        faction_name("AXIS_NAME", Some(&localisation)),
        Ok(Some("Achse".into()))
    );
    // Faction names reach their text boxes plain (140D77360): brackets are
    // glyphs.
    assert_eq!(
        faction_name("SCOPED_NAME", Some(&localisation)),
        Ok(Some("[ROOT.GetAdjective] Pact".into()))
    );
    // The game's localiser: `$$` is one `$` and a lone `]` a glyph.
    assert_eq!(
        faction_name("PRICE_NAME", Some(&localisation)),
        Ok(Some("Pakt $ ]".into()))
    );
    // A typed name is copied as its bytes.
    assert_eq!(
        faction_name("Pacte d'Acier", Some(&localisation)),
        Ok(Some("Pacte d'Acier".into()))
    );
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
