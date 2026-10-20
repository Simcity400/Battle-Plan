use super::*;
use crate::products::country_history::{CountryIdentityRun, RecordedCountry, RecordedHistories};
use crate::test_fixtures::{CAMPAIGN, ownership_frame, records};
use replay::container::Replay;
use replay_data::{rows::FactionMemberRow, rows::FactionRow, rows::TableId, rows::TableRows};
use replay_frame::{frame::DecodedFrame, frame::TableView};
use std::sync::atomic::Ordering;

pub(super) fn opened(
    replay: Arc<Replay>,
    source_id: &str,
    path: std::path::PathBuf,
    cancelled: &AtomicBool,
) -> Source {
    Source::from_replay(
        crate::SourceKey {
            source_id: source_id.into(),
            revision: "1".into(),
        },
        path,
        replay,
        cancelled,
    )
    .unwrap()
}

// Existing sum fixtures have one unchanged native holder per opaque tag. All
// cases still use the production native admission and indexed sum paths.
fn fixture_identities(history: &History, tracks: &[CountryTrack]) -> RecordedHistories {
    let tags: BTreeSet<_> = history
        .factions
        .values()
        .flatten()
        .flat_map(|run| run.members.iter())
        .chain(tracks.iter().map(|track| &track.tag))
        .collect();
    let first_hour = history
        .faction_reads
        .first()
        .map_or(0, |span| span.first_hour);
    let mut out = RecordedHistories::default();
    out.countries = tags
        .into_iter()
        .enumerate()
        .map(|(index, tag)| {
            let mut country = RecordedCountry::default();
            country.identities = Arc::new(vec![CountryIdentityRun {
                first_hour,
                last_hour: history.last_hour,
                slot: index as u32,
                country_id: index as u32,
            }]);
            (tag.clone(), country)
        })
        .collect();
    out
}

fn sum_tracks(
    history: &History,
    tracks: &[CountryTrack],
    membership: FactionMembership,
    cancelled: &AtomicBool,
) -> io::Result<Sums> {
    let members = NativeMembers::new(
        history,
        &fixture_identities(history, tracks),
        membership,
        cancelled,
    )?;
    sum_prepared(
        history,
        &members.measures(tracks, cancelled)?,
        &members.rosters,
        None,
        cancelled,
    )
}

fn counted_member_runs(
    history: &History,
    rosters: &[RosterRun],
    membership: FactionMembership,
) -> Vec<MembershipRun> {
    let history = History {
        factions: BTreeMap::from([("fixture".into(), rosters.to_vec())]),
        last_hour: history.last_hour,
        end_recorded: history.end_recorded,
        faction_reads: history.faction_reads.clone(),
        unobserved: history.unobserved.clone(),
        ..History::default()
    };
    NativeMembers::new(
        &history,
        &fixture_identities(&history, &[]),
        membership,
        &AtomicBool::new(false),
    )
    .unwrap()
    .counted
    .remove("fixture")
    .unwrap()
}

#[test]
fn native_end_and_ever_members_follow_earlier_aliases_without_admitting_reused_tags() {
    use replay_data::rows::CountryFactoriesRow;
    let frames = (100..=101)
        .map(|hour| {
            let mut frame = recorded(
                hour,
                &[(401, "Pact", if hour == 100 { &[44] } else { &[9001] })],
            );
            if hour == 101
                && let TableView::Complete(TableRows::Countries(rows)) =
                    &mut *frame.tables[TableId::Countries.index()]
            {
                for row in rows {
                    row.tag = if row.id == 9001 {
                        b"new B".to_vec()
                    } else {
                        b"GER".to_vec()
                    };
                }
            }
            *frame.tables[TableId::CountryFactories.index()] =
                TableView::Complete(TableRows::CountryFactories(vec![
                    CountryFactoriesRow {
                        country: 44,
                        military_total_raw: if hour == 100 { 500_000 } else { 7_700_000 },
                        ..Default::default()
                    },
                    CountryFactoriesRow {
                        country: 9001,
                        military_total_raw: if hour == 100 { 1_000_000 } else { 2_000_000 },
                        ..Default::default()
                    },
                ]));
            frame
        })
        .collect();
    let cancelled = AtomicBool::new(false);
    let replay = Arc::new(Replay::from_records(CAMPAIGN.into(), &[records(1, frames)]).unwrap());
    let source = opened(
        replay,
        "native-faction-aliases",
        Default::default(),
        &cancelled,
    );
    let mut params = FactionTotalsParams {
        branch_id: "1".into(),
        source: FactionMeasure::Factories {
            mode: FactoryCount::Built,
            groups: vec![FactoryGroupKey::Military],
        },
        membership: FactionMembership::End,
        top: 1,
        faction_id: Some("401".into()),
        parts: Some(true),
        inventory: None,
    };
    let end =
        FactionTotals::load(&source, &params, &LoadContext { content: None }, &cancelled).unwrap();
    assert_eq!(
        values(&end.series[0].runs),
        [(100, 100, 10), (101, 101, 20)]
    );
    assert!(
        !end.series[0].runs[1].starts_segment,
        "a proved rename retains native country continuity"
    );
    assert_eq!(
        end.series[0].counted_members,
        [
            MembershipRun {
                tag: "GER".into(),
                first_hour: 100,
                last_hour: 100
            },
            MembershipRun {
                tag: "new B".into(),
                first_hour: 101,
                last_hour: 101
            },
        ]
    );
    assert_eq!(
        end.member_tracks
            .as_ref()
            .unwrap()
            .iter()
            .filter(|track| !track.runs.is_empty())
            .map(|track| track.tag.as_str())
            .collect::<Vec<_>>(),
        ["GER", "new B"]
    );
    assert_eq!(
        end.member_tracks.as_ref().unwrap().len(),
        5,
        "unselected native holders reserve fixed-colour slots"
    );
    params.membership = FactionMembership::Ever;
    let ever =
        FactionTotals::load(&source, &params, &LoadContext { content: None }, &cancelled).unwrap();
    assert_eq!(
        values(&ever.series[0].runs),
        [(100, 100, 15), (101, 101, 97)]
    );
    assert_eq!(ever.member_tracks.as_ref().unwrap().len(), 5);
    params.membership = FactionMembership::Hour;
    let hourly =
        FactionTotals::load(&source, &params, &LoadContext { content: None }, &cancelled).unwrap();
    assert_eq!(
        values(&hourly.series[0].runs),
        [(100, 100, 5), (101, 101, 20)]
    );
}

#[test]
fn compressed_hour_rosters_use_the_current_holder_and_missing_native_identity_is_never_zero() {
    let history = history(vec![roster(0, 3, "Pact", &["A"])], 3);
    let mut country = RecordedCountry::default();
    country.identities = Arc::new(vec![
        CountryIdentityRun {
            first_hour: 0,
            last_hour: 1,
            slot: 7,
            country_id: 41,
        },
        CountryIdentityRun {
            first_hour: 3,
            last_hour: 3,
            slot: 8,
            country_id: 42,
        },
    ]);
    let mut identities = RecordedHistories::default();
    identities.countries.insert("A".into(), country);
    identities.identity_reads = vec![
        HourSpan {
            first_hour: 0,
            last_hour: 1,
        },
        HourSpan {
            first_hour: 3,
            last_hour: 3,
        },
    ];
    let tracks = [track("A", &[(0, 3, Some(10), true)])];
    let cancelled = AtomicBool::new(false);
    let hourly =
        NativeMembers::new(&history, &identities, FactionMembership::Hour, &cancelled).unwrap();
    let sums = sum_prepared(
        &history,
        &hourly.measures(&tracks, &cancelled).unwrap(),
        &hourly.rosters,
        None,
        &cancelled,
    )
    .unwrap();
    assert_eq!(values(&sums.series["401"]), [(0, 1, 10), (3, 3, 10)]);
    assert_eq!(hourly.unread_hours, 1);
    let end =
        NativeMembers::new(&history, &identities, FactionMembership::End, &cancelled).unwrap();
    let sums = sum_prepared(
        &history,
        &end.measures(&tracks, &cancelled).unwrap(),
        &end.rosters,
        None,
        &cancelled,
    )
    .unwrap();
    assert_eq!(values(&sums.series["401"]), [(0, 1, 0), (3, 3, 10)]);
    assert_eq!(
        end.counted["401"],
        [MembershipRun {
            tag: "A".into(),
            first_hour: 3,
            last_hour: 3
        }]
    );
    let ever =
        NativeMembers::new(&history, &identities, FactionMembership::Ever, &cancelled).unwrap();
    assert!(ever.reasons["401"].contains("native country identity"));
    assert!(ever.counted["401"].is_empty());
}

#[test]
fn changing_rosters_visit_only_their_intersecting_native_tag_epochs() {
    let history = history(
        (0..1000)
            .map(|hour| roster(hour, hour, &format!("Name {hour}"), &["reused A"]))
            .collect(),
        999,
    );
    let mut country = RecordedCountry::default();
    country.identities = Arc::new(
        (0..1000)
            .map(|hour| CountryIdentityRun {
                first_hour: hour,
                last_hour: hour,
                slot: hour as u32,
                country_id: hour as u32 + 9001,
            })
            .collect(),
    );
    let mut identities = RecordedHistories::default();
    identities.countries.insert("reused A".into(), country);
    for mode in [
        FactionMembership::Hour,
        FactionMembership::Ever,
        FactionMembership::End,
    ] {
        let members =
            NativeMembers::new(&history, &identities, mode, &AtomicBool::new(false)).unwrap();
        assert_eq!(
            members.identity_visits,
            if mode == FactionMembership::End {
                1
            } else {
                1000
            }
        );
        assert!(members.reasons.is_empty());
        assert!(NativeMembers::new(&history, &identities, mode, &AtomicBool::new(true)).is_err());
    }
}

fn scan_history(replay: &Replay, branch: u64, cancelled: &AtomicBool) -> io::Result<Arc<History>> {
    country_groups::scan_branch(replay, branch, cancelled)?.faction_history(cancelled)
}

fn roster(first_hour: i32, last_hour: i32, name: &str, members: &[&str]) -> RosterRun {
    RosterRun {
        first_hour,
        last_hour,
        name: Some(name.into()),
        members: members.iter().map(|tag| (*tag).into()).collect(),
        complete: true,
    }
}

fn track(tag: &str, values: &[(i32, i32, Option<i64>, bool)]) -> CountryTrack {
    CountryTrack {
        tag: tag.into(),
        runs: values
            .iter()
            .map(
                |&(first_hour, last_hour, value, starts_segment)| MeasureRun {
                    first_hour,
                    last_hour,
                    value,
                    starts_segment,
                },
            )
            .collect(),
    }
}

fn history(rosters: Vec<RosterRun>, last_hour: i32) -> History {
    History {
        factions: BTreeMap::from([("401".into(), rosters)]),
        last_hour,
        end_recorded: true,
        faction_reads: vec![HourSpan {
            first_hour: 0,
            last_hour,
        }],
        ..History::default()
    }
}

fn values(runs: &[FactionTotalRun]) -> Vec<(i32, i32, i64)> {
    runs.iter()
        .map(|run| (run.first_hour, run.last_hour, run.value))
        .collect()
}

pub(super) fn recorded(hour: i32, factions: &[(u64, &str, &[u32])]) -> DecodedFrame {
    let mut frame = ownership_frame(1, hour, 44);
    *frame.tables[TableId::Factions.index()] = TableView::Complete(TableRows::Factions(
        factions
            .iter()
            .map(|&(id, name, _)| FactionRow {
                id,
                name: name.as_bytes().into(),
                ..Default::default()
            })
            .collect(),
    ));
    let mut members: Vec<_> = factions
        .iter()
        .flat_map(|&(faction, _, members)| {
            members
                .iter()
                .enumerate()
                .map(move |(ordinal, &country)| FactionMemberRow {
                    country,
                    faction,
                    ordinal: ordinal as u32,
                })
        })
        .collect();
    members.sort_by_key(|member| member.country);
    *frame.tables[TableId::FactionMembers.index()] =
        TableView::Complete(TableRows::FactionMembers(members));
    frame
}

#[test]
fn recorded_names_members_and_presence_are_dated_by_their_own_hour() {
    let replay = Replay::from_records(
        CAMPAIGN.into(),
        &[records(
            1,
            vec![
                recorded(100, &[(401, "First recorded name", &[9001])]),
                recorded(101, &[(401, "Renamed by the player", &[9001, 44])]),
                recorded(102, &[(9223372036854775809, "A later faction", &[44])]),
            ],
        )],
    )
    .unwrap();
    let history = scan_history(&replay, 1, &AtomicBool::new(false)).unwrap();
    assert_eq!(
        history.factions["401"][0].name.as_deref(),
        Some("First recorded name")
    );
    assert_eq!(
        history.factions["401"][1].name.as_deref(),
        Some("Renamed by the player")
    );
    assert_eq!(history.factions["401"][1].last_hour, 101);
    assert_eq!(history.factions["9223372036854775809"][0].first_hour, 102);
    assert_eq!(
        member_runs(&history.factions["401"]),
        [
            MembershipRun {
                tag: "GER".into(),
                first_hour: 100,
                last_hour: 101
            },
            MembershipRun {
                tag: "FRA".into(),
                first_hour: 101,
                last_hour: 101
            },
        ]
    );
}

#[test]
fn incomplete_recorded_rosters_and_mismatched_country_links_cannot_supply_partial_sums() {
    let mut missing = recorded(101, &[(401, "Pact", &[9001, 44])]);
    *missing.tables[TableId::FactionMembers.index()] = TableView::Unavailable {
        reason: "unread roster".into(),
    };
    let mut bad_link = recorded(102, &[(401, "Pact", &[9001, 44])]);
    if let TableView::Complete(TableRows::Countries(countries)) =
        &mut *bad_link.tables[TableId::Countries.index()]
    {
        countries
            .iter_mut()
            .find(|country| country.id == 9001)
            .unwrap()
            .canonical_slot = 12345;
    }
    let replay = Replay::from_records(
        CAMPAIGN.into(),
        &[records(
            1,
            vec![
                recorded(100, &[(401, "Pact", &[9001, 44])]),
                missing,
                bad_link,
                recorded(103, &[(401, "Pact", &[9001, 44])]),
            ],
        )],
    )
    .unwrap();
    let history = scan_history(&replay, 1, &AtomicBool::new(false)).unwrap();
    assert_eq!(history.incomplete, 2);
    let tracks = [
        track("GER", &[(100, 103, Some(10), true)]),
        track("FRA", &[(100, 103, Some(5), true)]),
    ];
    for membership in [
        FactionMembership::Hour,
        FactionMembership::End,
        FactionMembership::Ever,
    ] {
        let totals = sum_tracks(&history, &tracks, membership, &AtomicBool::new(false))
            .unwrap()
            .series;
        if membership == FactionMembership::Ever {
            assert!(totals["401"].is_empty());
        } else if membership == FactionMembership::End {
            assert_eq!(values(&totals["401"]), [(100, 103, 15)]);
        } else {
            assert_eq!(values(&totals["401"]), [(100, 100, 15), (103, 103, 15)]);
            assert!(totals["401"][1].starts_segment);
        }
    }
}

#[test]
fn a_recorded_country_in_two_factions_leaves_both_rosters_incomplete() {
    let replay = Replay::from_records(
        CAMPAIGN.into(),
        &[records(
            1,
            vec![recorded(
                100,
                // These two raw links name the fixture's same canonical country.
                &[(401, "One pact", &[9001]), (909, "Other pact", &[11])],
            )],
        )],
    )
    .unwrap();
    let history = scan_history(&replay, 1, &AtomicBool::new(false)).unwrap();
    assert_eq!(history.incomplete, 2);
    let tracks = [track("GER", &[(100, 100, Some(10), true)])];
    assert!(
        sum_tracks(
            &history,
            &tracks,
            FactionMembership::Hour,
            &AtomicBool::new(false)
        )
        .unwrap()
        .series
        .values()
        .all(Vec::is_empty)
    );
}

#[test]
fn an_empty_recorded_faction_needs_no_country_links_but_an_unread_roster_is_a_gap() {
    let mut frames = vec![recorded(99, &[])];
    for hour in 100..=102 {
        let mut frame = recorded(
            hour,
            &[
                (401, "Recorded empty pact", &[]),
                (909, "Unresolved member", &[9001]),
            ],
        );
        for table in [TableId::Countries, TableId::CountryLinks] {
            *frame.tables[table.index()] = TableView::Unavailable {
                reason: "unread countries".into(),
            };
        }
        if hour == 101 {
            *frame.tables[TableId::FactionMembers.index()] = TableView::Unavailable {
                reason: "unread roster".into(),
            };
        }
        frames.push(frame);
    }
    let replay = Arc::new(Replay::from_records(CAMPAIGN.into(), &[records(1, frames)]).unwrap());
    let history = scan_history(&replay, 1, &AtomicBool::new(false)).unwrap();
    for membership in [
        FactionMembership::Hour,
        FactionMembership::End,
        FactionMembership::Ever,
    ] {
        let totals = sum_tracks(&history, &[], membership, &AtomicBool::new(false))
            .unwrap()
            .series;
        if membership == FactionMembership::Ever {
            assert!(totals["401"].is_empty());
        } else if membership == FactionMembership::End {
            assert_eq!(values(&totals["401"]), [(100, 102, 0)]);
        } else {
            assert_eq!(values(&totals["401"]), [(100, 100, 0), (102, 102, 0)]);
            assert!(totals["401"][1].starts_segment);
        }
        assert!(
            totals["909"].is_empty(),
            "actual members still require their country identity"
        );
    }
    let source = opened(
        replay,
        "empty-faction",
        "memory.battleplan".into(),
        &AtomicBool::new(false),
    );
    let mut params = FactionTotalsParams {
        branch_id: "1".into(),
        source: FactionMeasure::Factories {
            mode: FactoryCount::Built,
            groups: vec![],
        },
        membership: FactionMembership::Hour,
        top: 10,
        faction_id: None,
        parts: None,
        inventory: None,
    };
    let reply = FactionTotals::load(
        &source,
        &params,
        &LoadContext { content: None },
        &AtomicBool::new(false),
    )
    .unwrap();
    assert_eq!(reply.series.len(), 1);
    assert_eq!(reply.series[0].id, "401");
    assert_eq!(
        values(&reply.series[0].runs),
        [(100, 100, 0), (102, 102, 0)]
    );
    params.inventory = Some(true);
    let inventory = FactionTotals::load(
        &source,
        &params,
        &LoadContext { content: None },
        &AtomicBool::new(false),
    )
    .unwrap();
    assert_eq!(inventory.series.len(), 2);
    let unknown = inventory
        .series
        .iter()
        .find(|series| series.id == "909")
        .unwrap();
    assert!(unknown.runs.is_empty());
    assert!(
        !unknown.membership.is_empty(),
        "a native faction remains reachable without known geometry"
    );
}

#[test]
fn sums_require_every_counted_member_and_the_hours_recorded_roster() {
    let mut history = history(
        vec![roster(0, 3, "Recorded pact", &["opaque A", "opaque B"])],
        3,
    );
    let tracks = [
        track("opaque A", &[(0, 3, Some(10), true)]),
        track("opaque B", &[(0, 1, Some(5), true), (3, 3, Some(7), false)]),
    ];
    for rule in [
        FactionMembership::Hour,
        FactionMembership::End,
        FactionMembership::Ever,
    ] {
        let totals = sum_tracks(&history, &tracks, rule, &AtomicBool::new(false))
            .unwrap()
            .series;
        assert_eq!(values(&totals["401"]), [(0, 1, 15), (3, 3, 17)]);
        assert!(
            totals["401"][1].starts_segment,
            "a member gap breaks the faction line"
        );
    }
    history.factions.get_mut("401").unwrap()[0].complete = false;
    let complete_tracks = [
        track("opaque A", &[(0, 3, Some(10), true)]),
        track("opaque B", &[(0, 3, Some(5), true)]),
    ];
    for rule in [
        FactionMembership::Hour,
        FactionMembership::End,
        FactionMembership::Ever,
    ] {
        assert!(
            sum_tracks(&history, &complete_tracks, rule, &AtomicBool::new(false))
                .unwrap()
                .series["401"]
                .is_empty()
        );
    }
}

#[test]
fn frozen_members_count_annexed_and_not_yet_released_countries_as_proven_zero() {
    let annexed = history(
        vec![
            roster(0, 1, "Pact", &["A", "B"]),
            roster(2, 3, "Pact", &["A"]),
        ],
        3,
    );
    let tracks = [
        track("A", &[(0, 3, Some(10), true)]),
        track("B", &[(0, 1, Some(5), true), (2, 3, Some(0), true)]),
    ];
    for rule in [FactionMembership::Hour, FactionMembership::Ever] {
        assert_eq!(
            values(
                &sum_tracks(&annexed, &tracks, rule, &AtomicBool::new(false))
                    .unwrap()
                    .series["401"]
            ),
            [(0, 1, 15), (2, 3, 10)]
        );
    }
    assert_eq!(
        values(
            &sum_tracks(
                &annexed,
                &tracks,
                FactionMembership::End,
                &AtomicBool::new(false)
            )
            .unwrap()
            .series["401"]
        ),
        [(0, 3, 10)]
    );
    let released = history(
        vec![
            roster(0, 1, "Pact", &["A"]),
            roster(2, 3, "Pact", &["A", "B"]),
        ],
        3,
    );
    let tracks = [
        track("A", &[(0, 3, Some(10), true)]),
        track("B", &[(0, 1, Some(0), true), (2, 3, Some(5), true)]),
    ];
    for rule in [
        FactionMembership::Hour,
        FactionMembership::End,
        FactionMembership::Ever,
    ] {
        assert_eq!(
            values(
                &sum_tracks(&released, &tracks, rule, &AtomicBool::new(false))
                    .unwrap()
                    .series["401"]
            ),
            [(0, 1, 10), (2, 3, 15)]
        );
    }
}

#[test]
fn factions_keep_their_identity_and_leave_when_dissolved() {
    let mut history = history(
        vec![
            roster(1, 1, "First name", &["A"]),
            roster(2, 2, "Renamed", &["A", "B"]),
        ],
        5,
    );
    history.factions.insert(
        "9223372036854775809".into(),
        vec![roster(4, 5, "Later pact", &["B"])],
    );
    let tracks = [
        track("A", &[(0, 5, Some(10), true)]),
        track("B", &[(0, 5, Some(5), true)]),
    ];
    let totals = sum_tracks(
        &history,
        &tracks,
        FactionMembership::End,
        &AtomicBool::new(false),
    )
    .unwrap()
    .series;
    assert_eq!(values(&totals["401"]), [(1, 2, 15)]);
    assert_eq!(
        roster_reason(&history, &history.factions["401"], FactionMembership::End).as_deref(),
        None
    );
    assert_eq!(
        dissolution_hour(&history, &history.factions["401"], FactionMembership::End),
        Some(2)
    );
    assert_eq!(values(&totals["9223372036854775809"]), [(4, 5, 5)]);
    assert_eq!(
        member_runs(&history.factions["401"]),
        [
            MembershipRun {
                tag: "A".into(),
                first_hour: 1,
                last_hour: 2
            },
            MembershipRun {
                tag: "B".into(),
                first_hour: 2,
                last_hour: 2
            },
        ]
    );
    history.end_recorded = false;
    assert_eq!(
        dissolution_hour(&history, &history.factions["401"], FactionMembership::End),
        None,
        "an unread branch end cannot prove that the faction dissolved"
    );
    let totals = sum_tracks(
        &history,
        &tracks,
        FactionMembership::End,
        &AtomicBool::new(false),
    )
    .unwrap()
    .series;
    assert!(
        totals.values().all(Vec::is_empty),
        "an unread branch end is no end roster"
    );
}

#[test]
fn member_identity_changes_break_a_faction_even_when_the_sum_stays_equal() {
    let history = history(vec![roster(0, 3, "Pact", &["A"])], 3);
    let tracks = [track(
        "A",
        &[(0, 1, Some(10), true), (2, 3, Some(10), true)],
    )];
    let totals = sum_tracks(
        &history,
        &tracks,
        FactionMembership::Hour,
        &AtomicBool::new(false),
    )
    .unwrap()
    .series;
    assert_eq!(values(&totals["401"]), [(0, 1, 10), (2, 3, 10)]);
    assert!(totals["401"][1].starts_segment);
}

#[test]
fn absent_factory_evidence_is_dated_by_complete_country_reads() {
    let history = History {
        country_reads: vec![
            HourSpan {
                first_hour: 0,
                last_hour: 2,
            },
            HourSpan {
                first_hour: 4,
                last_hour: 6,
            },
        ],
        country_presence: HashMap::from([(
            "B".into(),
            vec![
                HourSpan {
                    first_hour: 1,
                    last_hour: 2,
                },
                HourSpan {
                    first_hour: 4,
                    last_hour: 4,
                },
            ],
        )]),
        ..History::default()
    };
    let absent = |tag: &str| {
        let mut runs = Vec::new();
        for_each_absent(
            &history.country_reads,
            history
                .country_presence
                .get(tag)
                .map(Vec::as_slice)
                .unwrap_or_default(),
            &AtomicBool::new(false),
            |first_hour, last_hour| {
                runs.push(MeasureRun {
                    first_hour,
                    last_hour,
                    value: Some(0),
                    starts_segment: true,
                });
                Ok(())
            },
        )
        .unwrap();
        runs
    };
    let runs = absent("B");
    assert_eq!(
        runs.iter()
            .map(|run| (run.first_hour, run.last_hour, run.value))
            .collect::<Vec<_>>(),
        [(0, 0, Some(0)), (5, 6, Some(0))]
    );
    let runs = absent("never-present");
    assert_eq!(
        runs.iter()
            .map(|run| (run.first_hour, run.last_hour))
            .collect::<Vec<_>>(),
        [(0, 2), (4, 6)]
    );
}

#[test]
fn absence_stream_keeps_presence_across_reads_and_checks_bounds_cancellation_and_callback_errors() {
    let cancelled = AtomicBool::new(false);
    let reads = [
        HourSpan {
            first_hour: 0,
            last_hour: 2,
        },
        HourSpan {
            first_hour: 4,
            last_hour: 6,
        },
    ];
    let present = [HourSpan {
        first_hour: 1,
        last_hour: 5,
    }];
    let mut spans = Vec::new();
    for_each_absent(&reads, &present, &cancelled, |first, last| {
        spans.push((first, last));
        Ok(())
    })
    .unwrap();
    assert_eq!(spans, [(0, 0), (6, 6)]);
    spans.clear();
    for_each_absent(
        &[HourSpan {
            first_hour: i32::MIN,
            last_hour: i32::MAX,
        }],
        &[
            HourSpan {
                first_hour: i32::MIN,
                last_hour: i32::MIN,
            },
            HourSpan {
                first_hour: i32::MAX,
                last_hour: i32::MAX,
            },
        ],
        &cancelled,
        |first, last| {
            spans.push((first, last));
            Ok(())
        },
    )
    .unwrap();
    assert_eq!(spans, [(i32::MIN + 1, i32::MAX - 1)]);
    let error = for_each_absent(&reads, &[], &cancelled, |_, _| {
        Err(io::Error::other("reply budget reached"))
    })
    .unwrap_err();
    assert_eq!(error.to_string(), "reply budget reached");
    let mut visits = 0usize;
    let error = for_each_absent(&reads, &[], &cancelled, |_, _| {
        visits += 1;
        cancelled.store(true, Ordering::Release);
        Ok(())
    })
    .unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::Interrupted);
    assert_eq!(visits, 1);
}

#[test]
fn country_measure_runs_continue_past_the_old_scan_ceiling_and_still_cancel() {
    let mut count = 1_000_000;
    let mut runs = Vec::new();
    let run = MeasureRun {
        first_hour: 0,
        last_hour: 0,
        value: Some(0),
        starts_segment: true,
    };
    push_measure(&mut runs, run.clone(), &mut count, &AtomicBool::new(false)).unwrap();
    assert_eq!(count, 1_000_001);
    assert_eq!(runs.len(), 1);
    count = 1_003_519;
    let error = push_measure(&mut runs, run, &mut count, &AtomicBool::new(true)).unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::Interrupted);
    assert_eq!(runs.len(), 1);
}

#[test]
fn the_shared_span_helper_keeps_duplicate_hours_and_widened_boundaries() {
    let mut spans = Vec::new();
    assert!(country_groups::record_hour(&mut spans, i32::MAX));
    assert!(!country_groups::record_hour(&mut spans, i32::MAX));
    assert_eq!(
        spans,
        [HourSpan {
            first_hour: i32::MAX,
            last_hour: i32::MAX
        }]
    );
}

#[test]
fn faction_totals_are_shared_and_top_membership_keeps_honest_gaps() {
    let mut history = history(vec![roster(0, 3, "First pact", &["A"])], 3);
    history
        .factions
        .insert("909".into(), vec![roster(0, 3, "Other pact", &["B"])]);
    let tracks = [
        track("A", &[(0, 1, Some(10), true), (3, 3, Some(30), true)]),
        track("B", &[(0, 3, Some(20), true)]),
    ];
    let totals = sum_tracks(
        &history,
        &tracks,
        FactionMembership::Hour,
        &AtomicBool::new(false),
    )
    .unwrap()
    .series;
    let top = crate::ranking::top_membership(
        &totals,
        1,
        |run| (run.first_hour, run.last_hour),
        |run| Some(run.value),
        &AtomicBool::new(false),
    )
    .unwrap();
    assert_eq!(
        top["909"],
        [HourSpan {
            first_hour: 0,
            last_hour: 2
        }]
    );
    assert_eq!(
        top["401"],
        [HourSpan {
            first_hour: 3,
            last_hour: 3
        }]
    );
    assert!(
        FactionTotals::validate(&FactionTotalsParams {
            branch_id: "1".into(),
            source: FactionMeasure::Factories {
                mode: FactoryCount::Built,
                groups: vec![FactoryGroupKey::Military]
            },
            membership: FactionMembership::Hour,
            top: 10,
            faction_id: None,
            parts: None,
            inventory: None,
        })
        .is_ok()
    );
}

#[test]
fn five_thousand_country_tracks_sum_into_fifty_factions_without_an_hourly_rescan() {
    let mut history = History {
        last_hour: 10_000,
        end_recorded: true,
        faction_reads: vec![HourSpan {
            first_hour: 0,
            last_hour: 10_000,
        }],
        ..History::default()
    };
    let mut tracks = Vec::with_capacity(5_000);
    for faction in 0..50 {
        let members: Vec<_> = (0..100)
            .map(|member| format!("mod-country-{}", faction * 100 + member))
            .collect();
        history.factions.insert(
            format!("{}", faction + 1000),
            vec![RosterRun {
                first_hour: 0,
                last_hour: 10_000,
                name: Some(format!("Mod pact {faction}")),
                members: members.clone(),
                complete: true,
            }],
        );
        for tag in members {
            tracks.push(track(
                &tag,
                &[(0, 4999, Some(1), true), (5000, 10_000, Some(2), false)],
            ));
        }
    }
    for membership in [
        FactionMembership::Hour,
        FactionMembership::End,
        FactionMembership::Ever,
    ] {
        let totals = sum_tracks(&history, &tracks, membership, &AtomicBool::new(false))
            .unwrap()
            .series;
        assert_eq!(totals.len(), 50);
        assert!(
            totals
                .values()
                .all(|runs| values(runs) == [(0, 4999, 100), (5000, 10_000, 200)])
        );
    }
}

#[test]
fn the_shared_sweep_honours_cancellation_and_refuses_overflow_as_a_gap() {
    let history = history(vec![roster(0, 0, "Pact", &["A", "B"])], 0);
    let tracks = [
        track("A", &[(0, 0, Some(i64::MAX), true)]),
        track("B", &[(0, 0, Some(1), true)]),
    ];
    assert!(
        sum_tracks(
            &history,
            &tracks,
            FactionMembership::Hour,
            &AtomicBool::new(false)
        )
        .unwrap()
        .series["401"]
            .is_empty()
    );
    assert_eq!(
        sum_tracks(
            &history,
            &tracks,
            FactionMembership::Hour,
            &AtomicBool::new(true)
        )
        .unwrap_err()
        .kind(),
        io::ErrorKind::Interrupted
    );
    let history = self::history(vec![roster(0, 2, "Pact", &["A", "B"])], 2);
    let tracks = [
        track(
            "A",
            &[
                (0, 0, Some(1), true),
                (1, 1, Some(MAX_EXACT_JS_INTEGER as i64), false),
                (2, 2, Some(1), false),
            ],
        ),
        track("B", &[(0, 2, Some(1), true)]),
    ];
    let totals = sum_tracks(
        &history,
        &tracks,
        FactionMembership::Hour,
        &AtomicBool::new(false),
    )
    .unwrap()
    .series;
    assert_eq!(values(&totals["401"]), [(0, 0, 2), (2, 2, 2)]);
    assert!(
        totals["401"][1].starts_segment,
        "an unsafe JavaScript integer is an honest gap"
    );
}

#[test]
fn reply_members_keep_every_recorded_roster_change_and_escape_exactly() {
    let rosters = [
        roster(0, 0, "Pact", &["opaque\\\"tag", "B"]),
        roster(1, 1, "Pact", &["opaque\\\"tag"]),
        roster(2, 2, "Pact", &["opaque\\\"tag", "B"]),
    ];
    let members = member_runs(&rosters);
    assert_eq!(members.len(), 3);
    assert_eq!(members[0].last_hour, 2);
    assert_eq!(members[1].last_hour, 0);
    assert_eq!(members[2].first_hour, 2);
    let json = serde_json::to_value(&members).unwrap();
    assert_eq!(json[0]["tag"], "opaque\\\"tag");
}

#[test]
fn factories_and_territory_wire_the_same_faction_product() {
    use replay_data::rows::CountryFactoriesRow;
    let mut frames = Vec::new();
    for hour in 100..=101 {
        let mut frame = recorded(
            hour,
            &[(
                401,
                if hour == 100 { "Old name" } else { "New name" },
                &[9001, 44],
            )],
        );
        frame.metrics.table_ns[TableId::States.index()] = 1;
        // The generic ownership fixture uses sparse slots; Territory needs the
        // native slot coverage supplied by a complete CountryLinks table.
        let mut slots = HashMap::new();
        if let TableView::Complete(TableRows::Countries(countries)) =
            &mut *frame.tables[TableId::Countries.index()]
        {
            for (slot, country) in countries.iter_mut().enumerate() {
                slots.insert(country.canonical_slot, slot as u32);
                country.canonical_slot = slot as u32;
            }
        }
        if let TableView::Complete(TableRows::CountryLinks(links)) =
            &mut *frame.tables[TableId::CountryLinks.index()]
        {
            for link in links {
                link.canonical_slot = slots[&link.canonical_slot];
            }
        }
        *frame.tables[TableId::CountryFactories.index()] =
            TableView::Complete(TableRows::CountryFactories(vec![
                CountryFactoriesRow {
                    country: 44,
                    military_total_raw: 500_000,
                    ..Default::default()
                },
                CountryFactoriesRow {
                    country: 9001,
                    military_total_raw: 1_000_000,
                    ..Default::default()
                },
            ]));
        frames.push(frame);
    }
    let replay = Arc::new(Replay::from_records(CAMPAIGN.into(), &[records(1, frames)]).unwrap());
    let source = opened(
        replay,
        "shared-factions",
        "memory.battleplan".into(),
        &AtomicBool::new(false),
    );
    let mut params = FactionTotalsParams {
        branch_id: "1".into(),
        source: FactionMeasure::Factories {
            mode: FactoryCount::Built,
            groups: vec![FactoryGroupKey::Military],
        },
        membership: FactionMembership::Hour,
        top: 10,
        faction_id: None,
        parts: None,
        inventory: None,
    };
    let factories = FactionTotals::load(
        &source,
        &params,
        &LoadContext { content: None },
        &AtomicBool::new(false),
    )
    .unwrap();
    assert_eq!(factories.series.len(), 1);
    assert_eq!(values(&factories.series[0].runs), [(100, 101, 15)]);
    assert_eq!(
        factories.series[0].names[0].name.as_deref(),
        Some("Old name")
    );
    assert_eq!(
        factories.series[0].names[1].name.as_deref(),
        Some("New name")
    );
    for membership in [
        FactionMembership::Hour,
        FactionMembership::End,
        FactionMembership::Ever,
    ] {
        params.membership = membership;
        let ranked = FactionTotals::load(
            &source,
            &params,
            &LoadContext { content: None },
            &AtomicBool::new(false),
        )
        .unwrap();
        params.faction_id = Some("401".into());
        params.parts = Some(true);
        let chosen = FactionTotals::load(
            &source,
            &params,
            &LoadContext { content: None },
            &AtomicBool::new(false),
        )
        .unwrap();
        assert_eq!(chosen.series[0].runs, ranked.series[0].runs);
        assert!(ranked.member_tracks.is_none());
        let members = chosen.member_tracks.as_ref().unwrap();
        assert_eq!(members.len(), 4);
        assert_eq!(
            members
                .iter()
                .filter(|member| !member.runs.is_empty())
                .count(),
            2
        );
        assert!(members.iter().all(|member| !member.identities.is_empty()));
        assert_eq!(
            members
                .iter()
                .filter(|member| !member.runs.is_empty())
                .map(|member| member.runs[0]
                    .value
                    .as_ref()
                    .unwrap()
                    .parse::<i64>()
                    .unwrap())
                .sum::<i64>(),
            15
        );
        assert!(
            members
                .iter()
                .filter(|member| !member.runs.is_empty())
                .flat_map(|member| &member.identities)
                .all(|identity| [44, 9001].contains(&identity.country_id))
        );
        params.parts = None;
        params.faction_id = None;
    }
    params.membership = FactionMembership::Hour;
    params.source = FactionMeasure::Territory {
        measure: TerritoryMeasure::States,
        rule: TerritoryRule::Owned,
    };
    let territory = FactionTotals::load(
        &source,
        &params,
        &LoadContext { content: None },
        &AtomicBool::new(false),
    )
    .unwrap();
    assert_eq!(territory.series.len(), 1);
    // The fixture owns one freshly read state across the two faction members.
    assert_eq!(territory.series[0].runs.first().unwrap().value, 1);
    assert_eq!(territory.series[0].names, factories.series[0].names);
    assert_eq!(territory.series[0].members, factories.series[0].members);
    assert_eq!(
        source
            .state::<country_factories::Scans>()
            .lock()
            .unwrap()
            .runs,
        1,
        "the adapter reuses the shared country scan"
    );
}

#[test]
fn anyone_ever_refuses_a_member_seen_only_in_an_incomplete_roster() {
    let mut incomplete = roster(1, 1, "Pact", &["A", "Only partly observed"]);
    incomplete.complete = false;
    let history = history(
        vec![
            roster(0, 0, "Pact", &["A"]),
            incomplete,
            roster(2, 2, "Pact", &["A"]),
        ],
        2,
    );
    let tracks = [
        track("A", &[(0, 2, Some(10), true)]),
        track("Only partly observed", &[(0, 2, Some(50), true)]),
    ];
    let totals = sum_tracks(
        &history,
        &tracks,
        FactionMembership::Ever,
        &AtomicBool::new(false),
    )
    .unwrap();
    assert!(totals.series["401"].is_empty());
    assert!(totals.recorded.is_empty());
    assert!(
        roster_reason(&history, &history.factions["401"], FactionMembership::Ever)
            .unwrap()
            .contains("not completely recorded")
    );
}

#[test]
fn known_overflow_is_a_counted_gap_and_recovers_at_the_exact_next_read() {
    let history = history(vec![roster(0, 3, "Pact", &["A", "B"])], 3);
    let tracks = [
        track(
            "A",
            &[
                (0, 1, Some(MAX_EXACT_JS_INTEGER as i64), true),
                (2, 3, Some(10), false),
            ],
        ),
        track("B", &[(0, 3, Some(1), true)]),
    ];
    let sums = sum_tracks(
        &history,
        &tracks,
        FactionMembership::Hour,
        &AtomicBool::new(false),
    )
    .unwrap();
    assert_eq!(sums.overflowed, 2);
    assert_eq!(values(&sums.series["401"]), [(2, 3, 11)]);
    assert!(sums.series["401"][0].starts_segment);
    assert_eq!(
        sums.recorded,
        [HourSpan {
            first_hour: 2,
            last_hour: 3
        }]
    );
}

#[test]
fn world_share_proofs_reject_unknown_and_overlapping_fixed_rosters() {
    let mut history = history(
        vec![roster(0, 0, "First", &["A"]), roster(1, 2, "First", &["B"])],
        2,
    );
    history
        .factions
        .insert("909".into(), vec![roster(0, 2, "Other", &["A"])]);
    let tracks = [
        track("A", &[(0, 2, Some(1), true)]),
        track("B", &[(0, 2, Some(1), true)]),
    ];
    let sums = sum_tracks(
        &history,
        &tracks,
        FactionMembership::Ever,
        &AtomicBool::new(false),
    )
    .unwrap();
    assert_eq!(
        sums.recorded,
        [HourSpan {
            first_hour: 0,
            last_hour: 2
        }]
    );
    assert!(
        sums.disjoint.is_empty(),
        "an ever member cannot take two world slices"
    );
    history.factions.get_mut("401").unwrap()[0].complete = false;
    let sums = sum_tracks(
        &history,
        &tracks,
        FactionMembership::Ever,
        &AtomicBool::new(false),
    )
    .unwrap();
    assert!(sums.recorded.is_empty());
    assert!(sums.disjoint.is_empty());
}

#[test]
fn chosen_faction_and_ranked_factions_use_identical_empty_totals_and_lifespans() {
    let cancelled = AtomicBool::new(false);
    let replay = Arc::new(
        Replay::from_records(
            CAMPAIGN.into(),
            &[records(
                1,
                vec![
                    recorded(100, &[(401, "Empty pact", &[])]),
                    recorded(101, &[(401, "Renamed empty pact", &[])]),
                    recorded(102, &[]),
                ],
            )],
        )
        .unwrap(),
    );
    let source = opened(
        replay,
        "same-faction-sum",
        "memory.battleplan".into(),
        &cancelled,
    );
    let context = LoadContext { content: None };
    let cached = country_groups::cached_scan(&source, 1, &cancelled).unwrap();
    let raw_history = super::history(&source, 1, &cancelled).unwrap();
    for membership in [
        FactionMembership::Hour,
        FactionMembership::End,
        FactionMembership::Ever,
    ] {
        let mut params = FactionTotalsParams {
            branch_id: "1".into(),
            source: FactionMeasure::Factories {
                mode: FactoryCount::Built,
                groups: vec![],
            },
            membership,
            top: 30,
            faction_id: None,
            parts: None,
            inventory: None,
        };
        let ranked = FactionTotals::load(&source, &params, &context, &cancelled).unwrap();
        params.faction_id = Some("401".into());
        let chosen = FactionTotals::load(&source, &params, &context, &cancelled).unwrap();
        assert_eq!(chosen.series.len(), 1);
        assert_eq!(chosen.series[0].runs, ranked.series[0].runs);
        assert_eq!(values(&chosen.series[0].runs), [(100, 101, 0)]);
        assert!(chosen.series[0].unavailable_reason.is_none());
        assert_eq!(
            chosen.series[0].dissolution_hour,
            (membership == FactionMembership::End).then_some(101)
        );
        assert_eq!(chosen.unit.as_deref(), Some("factories"));
    }
    assert!(
        Arc::ptr_eq(
            &raw_history,
            &super::history(&source, 1, &cancelled).unwrap()
        ),
        "ranked, chosen and membership requests reuse the same raw history projection"
    );
    assert!(Arc::ptr_eq(
        &raw_history,
        &cached.faction_history(&cancelled).unwrap()
    ));
}

#[test]
fn anyone_ever_refuses_unread_catalogues_before_or_after_the_visible_lifespan() {
    for unread_hour in [100, 102] {
        let mut frames: Vec<_> = (100..=102)
            .map(|hour| {
                recorded(
                    hour,
                    if hour == 101 {
                        &[(401, "Visible pact", &[9001])]
                    } else {
                        &[]
                    },
                )
            })
            .collect();
        *frames[(unread_hour - 100) as usize].tables[TableId::Factions.index()] =
            TableView::Unavailable {
                reason: "unread catalogue".into(),
            };
        let replay = Replay::from_records(CAMPAIGN.into(), &[records(1, frames)]).unwrap();
        let history = scan_history(&replay, 1, &AtomicBool::new(false)).unwrap();
        assert!(history.factions["401"].iter().all(|run| run.complete));
        let sums = sum_tracks(
            &history,
            &[track("GER", &[(100, 102, Some(10), true)])],
            FactionMembership::Ever,
            &AtomicBool::new(false),
        )
        .unwrap();
        assert!(
            sums.series["401"].is_empty(),
            "the unread edge could conceal another member"
        );
        assert!(
            roster_reason(&history, &history.factions["401"], FactionMembership::Ever).is_some()
        );
    }
}

#[test]
fn rotating_five_thousand_ever_members_share_one_effective_roster() {
    let rosters: Vec<_> = (0..5000)
        .map(|hour| {
            roster(
                hour,
                hour,
                &format!("Name {hour}"),
                &[&format!("opaque {hour}")],
            )
        })
        .collect();
    let history = history(rosters, 4999);
    let tracks: Vec<_> = (0..5000)
        .map(|index| track(&format!("opaque {index}"), &[(0, 4999, Some(1), true)]))
        .collect();
    let sums = sum_tracks(
        &history,
        &tracks,
        FactionMembership::Ever,
        &AtomicBool::new(false),
    )
    .unwrap();
    assert_eq!(values(&sums.series["401"]), [(0, 4999, 5000)]);
    assert_eq!(
        sums.roster_events, 1,
        "recorded names and current members do not copy the fixed ever set"
    );
    assert_eq!(
        sums.member_visits, 10000,
        "the fixed set is subscribed once and removed once"
    );
}

#[test]
fn global_proof_spans_use_only_the_exact_shared_reply_budget() {
    let mut reply = FactionTotalsReply {
        membership_mode: FactionMembership::Hour,
        branch_id: "1".into(),
        recorded: vec![
            HourSpan {
                first_hour: 0,
                last_hour: 0
            };
            400_000
        ],
        disjoint: vec![],
        series: vec![],
        unit: None,
        unavailable_reason: None,
        warnings: vec![],
        member_tracks: None,
    };
    // The former 64-byte estimate refused these smaller serialized spans.
    super::super::check_reply_budget(&reply).unwrap();
    reply
        .warnings
        .push("x".repeat(super::super::MAX_PRODUCT_REPLY_BYTES));
    let error = super::super::check_reply_budget(&reply).unwrap_err();
    assert!(error.to_string().contains("transport reply budget"));
}

fn empty_faction_reply(name: &str, count: u32) -> io::Result<FactionTotalsReply> {
    let factions: Vec<_> = (1..=count)
        .map(|id| (u64::from(id), name, &[] as &[u32]))
        .collect();
    let cancelled = AtomicBool::new(false);
    let replay = Arc::new(
        Replay::from_records(
            CAMPAIGN.into(),
            &[records(1, vec![recorded(100, &factions)])],
        )
        .unwrap(),
    );
    let source = opened(
        replay,
        "faction-reply-budget",
        "memory.battleplan".into(),
        &cancelled,
    );
    let params = FactionTotalsParams {
        branch_id: "1".into(),
        source: FactionMeasure::Factories {
            mode: FactoryCount::Built,
            groups: vec![],
        },
        membership: FactionMembership::Hour,
        top: u64::from(count),
        faction_id: None,
        parts: None,
        inventory: None,
    };
    FactionTotals::validate(&params).unwrap();
    FactionTotals::load(&source, &params, &LoadContext { content: None }, &cancelled)
}

#[test]
fn requests_above_thirty_factions_return_all_five_thousand() {
    let reply = empty_faction_reply("Recorded pact", 5000).unwrap();
    assert_eq!(reply.series.len(), 5000);
    assert!(
        reply
            .series
            .iter()
            .all(|series| values(&series.runs) == [(100, 100, 0)])
    );
}

#[test]
fn faction_reply_refuses_only_the_shared_serialized_transport_budget() {
    let name = "\"".repeat(5_000_000);
    let reply = empty_faction_reply(&name, 1).unwrap();
    assert_eq!(
        reply.series[0].names[0].name.as_deref(),
        Some(name.as_str())
    );
    assert!(crate::transport::json_len(&reply, super::super::MAX_PRODUCT_REPLY_BYTES).is_some());
    let too_large = "\"".repeat(super::super::MAX_PRODUCT_REPLY_BYTES / 2);
    let error = empty_faction_reply(&too_large, 1).unwrap_err();
    assert!(error.to_string().contains("transport reply budget"));
}

fn dissolved_end_reply(incomplete_last: bool, unread_end: bool) -> FactionTotalsReply {
    use replay_data::rows::CountryFactoriesRow;
    let mut frames = vec![
        recorded(100, &[(401, "First name", &[9001])]),
        recorded(101, &[(401, "Second name", &[9001, 44])]),
        recorded(102, &[(401, "Last name", &[44])]),
        recorded(103, &[]),
        recorded(104, &[]),
    ];
    for frame in &mut frames {
        *frame.tables[TableId::CountryFactories.index()] =
            TableView::Complete(TableRows::CountryFactories(vec![
                CountryFactoriesRow {
                    country: 44,
                    military_total_raw: 500_000,
                    ..Default::default()
                },
                CountryFactoriesRow {
                    country: 9001,
                    military_total_raw: 1_000_000,
                    ..Default::default()
                },
            ]));
    }
    if incomplete_last {
        *frames[2].tables[TableId::FactionMembers.index()] = TableView::Unavailable {
            reason: "last roster unread".into(),
        };
    }
    if unread_end {
        *frames[4].tables[TableId::Factions.index()] = TableView::Unavailable {
            reason: "branch end unread".into(),
        };
    }
    let cancelled = AtomicBool::new(false);
    let replay = Arc::new(Replay::from_records(CAMPAIGN.into(), &[records(1, frames)]).unwrap());
    let source = opened(
        replay,
        "dissolved-end-members",
        "memory.battleplan".into(),
        &cancelled,
    );
    FactionTotals::load(
        &source,
        &FactionTotalsParams {
            branch_id: "1".into(),
            source: FactionMeasure::Factories {
                mode: FactoryCount::Built,
                groups: vec![FactoryGroupKey::Military],
            },
            membership: FactionMembership::End,
            top: 1,
            faction_id: Some("401".into()),
            parts: None,
            inventory: None,
        },
        &LoadContext { content: None },
        &cancelled,
    )
    .unwrap()
}

#[test]
fn dissolved_end_members_use_the_changed_last_roster_over_the_recorded_lifetime() {
    let reply = dissolved_end_reply(false, false);
    let series = &reply.series[0];
    assert_eq!(values(&series.runs), [(100, 102, 5)]);
    assert_eq!(series.dissolution_hour, Some(102));
    assert!(series.unavailable_reason.is_none());
    assert_eq!(series.names[0].name.as_deref(), Some("First name"));
    assert_eq!(series.names[2].name.as_deref(), Some("Last name"));
    assert_eq!(
        serde_json::to_value(&reply).unwrap()["series"][0]["dissolutionHour"],
        102,
        "the shared UI contract carries the last recorded hour for its dated note"
    );
    assert!(
        !reply
            .warnings
            .iter()
            .any(|warning| warning.contains("member sets"))
    );
}

#[test]
fn dissolved_end_members_with_an_incomplete_last_roster_stay_gaps_with_a_warning() {
    let reply = dissolved_end_reply(true, false);
    let series = &reply.series[0];
    assert!(
        series.runs.is_empty(),
        "an earlier complete roster cannot replace the last one"
    );
    assert_eq!(series.dissolution_hour, Some(102));
    assert_eq!(
        series.unavailable_reason.as_deref(),
        Some(
            "This faction's last recorded member list is incomplete, so its total at the end is not known"
        )
    );
    assert!(
        reply
            .warnings
            .iter()
            .any(|warning| warning.starts_with("1 faction member sets"))
    );
    assert!(
        reply
            .warnings
            .iter()
            .any(|warning| warning.starts_with("1 faction roster reads"))
    );
}

#[test]
fn dissolved_end_members_still_refuse_an_unread_branch_end() {
    let reply = dissolved_end_reply(false, true);
    let series = &reply.series[0];
    assert!(series.runs.is_empty());
    assert_eq!(series.dissolution_hour, None);
    assert_eq!(
        series.unavailable_reason.as_deref(),
        Some("This faction's membership was not recorded at the end of this campaign branch")
    );
    assert!(
        reply
            .warnings
            .iter()
            .any(|warning| warning.starts_with("1 faction member sets"))
    );
}

#[test]
fn faction_sweeps_keep_effective_rosters_and_totals_past_the_former_run_ceiling() {
    let mut history = History {
        last_hour: 200,
        end_recorded: true,
        faction_reads: vec![HourSpan {
            first_hour: 0,
            last_hour: 200,
        }],
        ..History::default()
    };
    let mut tracks = Vec::new();
    for id in 0..5000 {
        let tag = format!("Opaque:{id}");
        let rosters = (0..=200)
            .map(|hour| {
                let members = if hour % 2 == 0 {
                    vec![tag.clone()]
                } else {
                    vec![]
                };
                RosterRun {
                    first_hour: hour,
                    last_hour: hour,
                    name: None,
                    members,
                    complete: true,
                }
            })
            .collect();
        history.factions.insert(id.to_string(), rosters);
        tracks.push(track(&tag, &[(0, 200, Some(1), true)]));
    }
    let sums = sum_tracks(
        &history,
        &tracks,
        FactionMembership::Hour,
        &AtomicBool::new(false),
    )
    .unwrap();
    assert_eq!(sums.roster_events, 1_005_000);
    assert_eq!(sums.series.values().map(Vec::len).sum::<usize>(), 1_005_000);
    assert!(
        sums.series
            .values()
            .all(|runs| runs.iter().enumerate().all(|(hour, run)| {
                run.first_hour == hour as i32
                    && run.last_hour == hour as i32
                    && run.value == if hour % 2 == 0 { 1 } else { 0 }
            }))
    );
    assert_eq!(
        sums.recorded,
        [HourSpan {
            first_hour: 0,
            last_hour: 200
        }]
    );
    assert_eq!(sums.disjoint, sums.recorded);
}

#[test]
fn end_members_require_a_recorded_catalogue_immediately_after_dissolution() {
    let mut history = history(vec![roster(0, 1, "Pact", &["A"])], 3);
    history.faction_reads = vec![
        HourSpan {
            first_hour: 0,
            last_hour: 1,
        },
        HourSpan {
            first_hour: 3,
            last_hour: 3,
        },
    ];
    let tracks = [track("A", &[(0, 3, Some(7), true)])];
    let totals = sum_tracks(
        &history,
        &tracks,
        FactionMembership::End,
        &AtomicBool::new(false),
    )
    .unwrap();
    assert!(totals.series["401"].is_empty());
    assert!(
        roster_reason(&history, &history.factions["401"], FactionMembership::End)
            .unwrap()
            .contains("dissolution was not recorded")
    );
    assert_eq!(
        dissolution_hour(&history, &history.factions["401"], FactionMembership::End),
        None
    );
    assert!(
        counted_member_runs(&history, &history.factions["401"], FactionMembership::End).is_empty()
    );
}

#[test]
fn end_members_ignore_incomplete_intermediate_rosters_and_report_the_counted_set() {
    let mut middle = roster(1, 1, "Pact", &["Partial"]);
    middle.complete = false;
    let history = history(
        vec![
            roster(0, 0, "Pact", &["A"]),
            middle,
            roster(2, 3, "Pact", &["B"]),
        ],
        3,
    );
    let tracks = [
        track("A", &[(0, 3, Some(11), true)]),
        track("B", &[(0, 3, Some(7), true)]),
    ];
    let end = sum_tracks(
        &history,
        &tracks,
        FactionMembership::End,
        &AtomicBool::new(false),
    )
    .unwrap();
    assert_eq!(values(&end.series["401"]), [(0, 3, 7)]);
    assert_eq!(
        counted_member_runs(&history, &history.factions["401"], FactionMembership::End),
        [MembershipRun {
            tag: "B".into(),
            first_hour: 0,
            last_hour: 3
        }]
    );
    let hourly = sum_tracks(
        &history,
        &tracks,
        FactionMembership::Hour,
        &AtomicBool::new(false),
    )
    .unwrap();
    assert_eq!(values(&hourly.series["401"]), [(0, 0, 11), (2, 3, 7)]);
}

#[test]
fn counted_members_match_hour_end_and_ever_sums() {
    let history = history(
        vec![roster(0, 1, "Pact", &["A"]), roster(2, 3, "Pact", &["B"])],
        3,
    );
    let rosters = &history.factions["401"];
    assert_eq!(
        counted_member_runs(&history, rosters, FactionMembership::Hour),
        member_runs(rosters)
    );
    assert_eq!(
        counted_member_runs(&history, rosters, FactionMembership::End),
        [MembershipRun {
            tag: "B".into(),
            first_hour: 0,
            last_hour: 3
        }]
    );
    assert_eq!(
        counted_member_runs(&history, rosters, FactionMembership::Ever),
        [
            MembershipRun {
                tag: "A".into(),
                first_hour: 0,
                last_hour: 3
            },
            MembershipRun {
                tag: "B".into(),
                first_hour: 0,
                last_hour: 3
            }
        ]
    );
}

#[test]
fn faction_top_accepts_the_full_ui_safe_integer_count() {
    let params: FactionTotalsParams = serde_json::from_value(serde_json::json!({
        "branchId": "1", "source": { "kind": "territory", "measure": "states", "rule": "controlled" },
        "membership": "hour", "top": MAX_EXACT_JS_INTEGER,
    })).unwrap();
    assert_eq!(params.top, MAX_EXACT_JS_INTEGER);
    FactionTotals::validate(&params).unwrap();
}

#[test]
fn faction_totals_use_main_template_names_and_dated_refusal_reasons() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
    use crate::products::ContentOptions;
    let (_, folder) = crate::catalogue::tests::folder_catalogue(&[
        (
            "common/factions/templates/test.txt",
            "demo={name=DEMO_NAME}",
        ),
        (
            "localisation/test_l_english.yml",
            "\u{feff}l_english:\n DEMO_NAME:0 \"Template Pact\"\n",
        ),
        ("settings.txt", "language=\"l_english\""),
    ]);
    let frames = [
        (100, "", "demo"),
        (101, "My literal", "demo"),
        (102, "", "missing_template"),
    ]
    .into_iter()
    .map(|(hour, literal, template)| {
        let mut frame = recorded(hour, &[(401, literal, &[])]);
        if let TableView::Complete(TableRows::Factions(rows)) =
            &mut *frame.tables[TableId::Factions.index()]
        {
            rows[0].template_token = Some(17);
        }
        *frame.tables[TableId::LexicalTokens.index()] =
            TableView::Complete(TableRows::LexicalTokens(vec![
                replay_data::rows::LexicalTokenRow {
                    token: 17,
                    literal: template.as_bytes().to_vec(),
                },
            ]));
        if hour == 101 {
            *frame.tables[TableId::FactionMembers.index()] = TableView::Unavailable {
                reason: "roster unread".into(),
            };
        }
        frame
    })
    .collect();
    let cancelled = AtomicBool::new(false);
    let replay = Arc::new(Replay::from_records(CAMPAIGN.into(), &[records(1, frames)]).unwrap());
    let source = opened(
        replay,
        "dated-faction-total-names",
        Default::default(),
        &cancelled,
    );
    let options = ContentOptions {
        options: installed_content_core::ResolveOptions {
            game_dir: folder.0.clone(),
            user_dir: folder.0.clone(),
            cache_dir: folder.0.join("cache"),
        },
        allow_unverified: true,
    };
    let layers = installed_content_core::ResolvedLayers::from_folders(&[(&folder.0, &[][..])]).unwrap();
    source.set_test_content(
        options.options.clone(),
        Arc::new(installed_content_core::content::ResolvedContent {
            layers,
            stats: Default::default(),
        }),
    );
    let context = LoadContext {
        content: Some(options),
    };
    let reply = FactionTotals::load(
        &source,
        &FactionTotalsParams {
            branch_id: "1".into(),
            source: FactionMeasure::Factories {
                mode: FactoryCount::Built,
                groups: vec![],
            },
            membership: FactionMembership::End,
            top: 1,
            faction_id: Some("401".into()),
            parts: None,
            inventory: None,
        },
        &context,
        &cancelled,
    )
    .unwrap();
    let groups = country_groups::CountryGroups::load(
        &source,
        &country_groups::CountryGroupParams {
            branch_id: "1".into(),
        },
        &context,
        &cancelled,
    )
    .unwrap();
    let series = &reply.series[0];
    assert_eq!(series.names, groups.factions[0].names);
    assert_eq!(series.names[0].name.as_deref(), Some("Template Pact"));
    assert_eq!(series.names[1].name.as_deref(), Some("My literal"));
    assert_eq!(
        series.names[1].first_hour, 101,
        "unread membership cannot hide the literal name read"
    );
    assert_eq!(series.names[2].name, None);
    assert!(
        series.names[2]
            .reason
            .as_ref()
            .unwrap()
            .contains("missing_template")
    );
    assert_eq!(
        values(&series.runs),
        [(100, 102, 0)],
        "only the complete final roster counts in end mode"
    );
}

#[test]
fn occupied_faction_sums_include_foreign_control_inside_the_same_faction_and_refuse_missing_reads()
{
    use replay_data::rows::{ProvinceRow, StateRow};
    let reads = (100..=102)
        .map(|hour| {
            let mut read = recorded(hour, &[(401, "Synthetic pact", &[44, 9001])]);
            *read.tables[TableId::States.index()] =
                TableView::Complete(TableRows::States(vec![StateRow {
                    id: 1,
                    owner: 44,
                    controller: 9001,
                    ..Default::default()
                }]));
            *read.tables[TableId::Provinces.index()] =
                TableView::Complete(TableRows::Provinces(vec![ProvinceRow {
                    id: 1,
                    owner: 9001,
                    controller: 44,
                    victory_points: -7,
                    ..Default::default()
                }]));
            if hour == 101 {
                *read.tables[TableId::States.index()] = TableView::Unavailable {
                    reason: "missing state member amount".into(),
                };
            }
            read
        })
        .collect();
    let replay = Arc::new(Replay::from_records(CAMPAIGN.into(), &[records(1, reads)]).unwrap());
    let flag = AtomicBool::new(false);
    let source = opened(
        replay,
        "occupied-faction",
        "memory.battleplan".into(),
        &flag,
    );
    for membership in [
        FactionMembership::Hour,
        FactionMembership::End,
        FactionMembership::Ever,
    ] {
        for (measure, amount) in [
            (TerritoryMeasure::States, 1),
            (TerritoryMeasure::Provinces, 1),
            (TerritoryMeasure::VictoryPoints, -7),
        ] {
            let reply = FactionTotals::load(
                &source,
                &FactionTotalsParams {
                    branch_id: "1".into(),
                    source: FactionMeasure::Territory {
                        measure,
                        rule: TerritoryRule::Occupied,
                    },
                    membership,
                    top: 10,
                    faction_id: Some("401".into()),
                    parts: Some(true),
                    inventory: None,
                },
                &LoadContext { content: None },
                &flag,
            )
            .unwrap();
            let expected = if matches!(measure, TerritoryMeasure::States) {
                vec![(100, 100, amount), (102, 102, amount)]
            } else {
                vec![(100, 102, amount)]
            };
            assert_eq!(
                values(&reply.series[0].runs),
                expected,
                "{membership:?}, {measure:?}"
            );
            assert!(reply.member_tracks.as_ref().unwrap().iter().any(|member| {
                member
                    .runs
                    .iter()
                    .any(|run| run.value.as_deref() == Some(&amount.to_string()))
            }));
        }
    }
}
