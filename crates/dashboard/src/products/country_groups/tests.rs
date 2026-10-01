use crate::test_fixtures::{CAMPAIGN, ownership_frame, records};
use replay::{
    container::Replay,
    frame::{DecodedFrame, TableView},
    rows::{TableId, TableRows},
};
use std::sync::atomic::AtomicBool;

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
