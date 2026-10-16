use super::*;
use dashboard_base::test_fixtures::{CAMPAIGN, records, source};
use replay::{
    frame::{DecodedFrame, TableView},
    rows::TableRows,
};

pub(super) fn base() -> i32 {
    let replay::cadence::Cadence::Daily(slot) =
        replay::cadence::cadence(TableId::OperativeMemberships)
    else {
        panic!("daily membership");
    };
    24_000 + i32::try_from(slot).unwrap()
}

pub(super) fn row(id: u64) -> OperativeMembershipRow {
    OperativeMembershipRow {
        id,
        roster_country: 17,
        owner_country: 17,
        assignment: 3,
        name: b"Literal operative".to_vec().into(),
        skill: Some(2),
        experience_raw: 123_456,
        captured_hour: engine::cgame_date::NULL_RAW_HOUR,
        ..Default::default()
    }
}

pub(super) fn frame(hour: i32, read: bool, rows: Vec<OperativeMembershipRow>) -> DecodedFrame {
    let mut frame = source(1, base() + hour, Some(1));
    *frame.tables[TableId::Countries.index()] = TableView::Complete(TableRows::Countries(vec![
        CountryRow {
            id: 7,
            canonical_slot: 1,
            tag: b"all!countries".to_vec(),
            ..Default::default()
        },
        CountryRow {
            id: 9,
            canonical_slot: 2,
            tag: b"target old".to_vec(),
            ..Default::default()
        },
    ]));
    *frame.tables[TableId::CountryLinks.index()] =
        TableView::Complete(TableRows::CountryLinks(vec![
            CountryLinkRow {
                id: 7,
                canonical_slot: 1,
            },
            CountryLinkRow {
                id: 9,
                canonical_slot: 2,
            },
            CountryLinkRow {
                id: 17,
                canonical_slot: 1,
            },
        ]));
    let mut rows = rows;
    rows.sort_by_key(|row| (row.id, row.roster_country, row.roster_kind));
    *frame.tables[TableId::OperativeMemberships.index()] =
        TableView::Complete(TableRows::OperativeMemberships(rows));
    if read {
        frame.metrics.table_ns[TableId::OperativeMemberships.index()] = 1;
    }
    *frame.tables[TableId::SimulationFeatures.index()] =
        TableView::Complete(TableRows::SimulationFeatures(vec![
            replay::rows::SimulationFeatureRow {
                dlc_mask: 1 << 5,
                ..Default::default()
            },
        ]));
    frame
}

fn history(frames: Vec<DecodedFrame>) -> OperativeRosterReply {
    let replay = Replay::from_records(CAMPAIGN.into(), &[records(1, frames)]).unwrap();
    scan(
        &replay,
        1,
        &["all!countries".into()],
        None,
        &AtomicBool::new(false),
        None,
        None,
    )
    .unwrap()
}

fn content_history(
    frames: Vec<DecodedFrame>,
    content: &super::super::operative_content::OperativeContent,
) -> OperativeRosterReply {
    let replay = Replay::from_records(CAMPAIGN.into(), &[records(1, frames)]).unwrap();
    scan(
        &replay,
        1,
        &["all!countries".into()],
        Some(content),
        &AtomicBool::new(false),
        None,
        None,
    )
    .unwrap()
}

#[test]
fn operatives_roster_manual_picks_follow_all_holders_and_keep_alias_gaps() {
    let frames = [
        (0, ["PRE", "OTHER", "C"]),
        (1, ["X", "OTHER", "C"]),
        (2, ["Y", "X", "C"]),
        (4, ["Y", "X", "C"]),
        (5, ["Z", "X", "Y"]),
    ]
    .into_iter()
    .map(|(offset, tags)| {
        let rows = [7, 9, 11]
            .into_iter()
            .enumerate()
            .map(|(index, country)| OperativeMembershipRow {
                owner_country: country,
                roster_country: country,
                ..row(index as u64 + 1)
            })
            .collect();
        let mut frame = frame(offset, true, rows);
        *frame.tables[TableId::Countries.index()] = TableView::Complete(TableRows::Countries(
            [7, 9, 11]
                .into_iter()
                .zip(tags)
                .enumerate()
                .map(|(index, (id, tag))| CountryRow {
                    id,
                    canonical_slot: index as u32 + 1,
                    tag: tag.as_bytes().to_vec(),
                    ..Default::default()
                })
                .collect(),
        ));
        *frame.tables[TableId::CountryLinks.index()] =
            TableView::Complete(TableRows::CountryLinks(
                [7, 9, 11]
                    .into_iter()
                    .enumerate()
                    .map(|(index, id)| CountryLinkRow {
                        id,
                        canonical_slot: index as u32 + 1,
                    })
                    .collect(),
            ));
        frame
    })
    .collect();
    let cancelled = AtomicBool::new(false);
    let source = Source::from_replay(
        dashboard_base::source::SourceKey {
            source_id: "holders".into(),
            revision: "1".into(),
        },
        "synthetic.battleplan".into(),
        std::sync::Arc::new(Replay::from_records(CAMPAIGN.into(), &[records(1, frames)]).unwrap()),
        &cancelled,
    )
    .unwrap();
    let reply = OperativesRoster::load(
        &source,
        &OperativeRosterParams {
            branch_id: "1".into(),
            tags: vec!["X".into()],
        },
        &LoadContext { content: None },
        &cancelled,
    )
    .unwrap();
    assert_eq!(
        reply
            .countries
            .iter()
            .map(|country| country.tag.as_str())
            .collect::<Vec<_>>(),
        ["OTHER", "PRE", "X", "Y", "Z"]
    );
    let y = reply
        .countries
        .iter()
        .find(|country| country.tag == "Y")
        .unwrap();
    assert_eq!(
        y.observed
            .iter()
            .map(|read| (read.first_hour - base(), read.last_hour - base()))
            .collect::<Vec<_>>(),
        [(2, 2), (4, 4)]
    );
    assert!(
        reply
            .countries
            .iter()
            .all(|country| country.operatives.iter().all(|entry| entry.id != "3"))
    );
    assert_eq!(
        reply
            .countries
            .iter()
            .find(|country| country.tag == "PRE")
            .unwrap()
            .operatives[0]
            .id,
        "1"
    );
}

#[test]
fn operatives_roster_off_read_aliases_preserve_census_dates_but_not_replacements_or_gaps() {
    let frames = [
        (0, 7, ["X", "OTHER"]),
        (1, 7, ["Y", "X"]),
        (2, 7, ["X", "OTHER"]),
        (3, 10, ["X", "OTHER"]),
        (5, 10, ["Y", "OTHER"]),
        (6, 10, ["Y", "X"]),
    ]
    .into_iter()
    .map(|(offset, first_id, tags)| {
        let membership_id = if offset == 6 { 3 } else { 1 };
        let membership_country = if offset == 6 { first_id } else { 7 };
        let mut frame = frame(
            offset,
            offset == 0 || offset == 6,
            vec![
                OperativeMembershipRow {
                    roster_country: membership_country,
                    owner_country: membership_country,
                    ..row(membership_id)
                },
                OperativeMembershipRow {
                    roster_country: 9,
                    owner_country: 9,
                    ..row(2)
                },
            ],
        );
        let mut countries: Vec<_> = [first_id, 9]
            .into_iter()
            .zip(tags)
            .enumerate()
            .map(|(index, (id, tag))| CountryRow {
                id,
                canonical_slot: index as u32 + 1,
                tag: tag.as_bytes().to_vec(),
                ..Default::default()
            })
            .collect();
        countries.sort_by_key(|country| country.id);
        let links = countries
            .iter()
            .map(|country| CountryLinkRow {
                id: country.id,
                canonical_slot: country.canonical_slot,
            })
            .collect();
        *frame.tables[TableId::Countries.index()] =
            TableView::Complete(TableRows::Countries(countries));
        *frame.tables[TableId::CountryLinks.index()] =
            TableView::Complete(TableRows::CountryLinks(links));
        // A timed slot peer must not turn the alias handoff into a new census.
        frame.metrics.table_ns[TableId::WingCumulative.index()] = 1;
        frame
    })
    .collect();
    let cancelled = AtomicBool::new(false);
    let source = Source::from_replay(
        dashboard_base::source::SourceKey {
            source_id: "off-read-holders".into(),
            revision: "1".into(),
        },
        "synthetic.battleplan".into(),
        std::sync::Arc::new(Replay::from_records(CAMPAIGN.into(), &[records(1, frames)]).unwrap()),
        &cancelled,
    )
    .unwrap();
    let reply = OperativesRoster::load(
        &source,
        &OperativeRosterParams {
            branch_id: "1".into(),
            tags: vec!["X".into()],
        },
        &LoadContext { content: None },
        &cancelled,
    )
    .unwrap();
    let x = reply
        .countries
        .iter()
        .find(|country| country.tag == "X")
        .unwrap();
    assert_eq!(
        x.identities
            .iter()
            .map(|identity| (
                identity.first_hour - base(),
                identity.last_hour - base(),
                identity.slot,
                identity.country_id,
            ))
            .collect::<Vec<_>>(),
        [
            (0, 0, 1, 7),
            (1, 1, 2, 9),
            (2, 2, 1, 7),
            (3, 3, 1, 10),
            (6, 6, 2, 9)
        ]
    );
    assert_eq!(
        x.observed
            .iter()
            .map(|observation| (
                observation.first_hour - base(),
                observation.last_hour - base(),
                observation.read_hour - base(),
            ))
            .collect::<Vec<_>>(),
        [(0, 2, 0), (6, 6, 6)]
    );
    assert_eq!(
        x.operatives
            .iter()
            .map(|entry| (
                entry.id.as_str(),
                entry.joined_hour - base(),
                entry.runs[0].first_hour - base(),
                entry.runs.last().unwrap().last_hour - base(),
            ))
            .collect::<Vec<_>>(),
        [
            ("1", 0, 0, 0),
            ("2", 0, 1, 1),
            ("1", 0, 2, 2),
            ("2", 6, 6, 6)
        ]
    );
    assert_eq!(
        x.operatives
            .iter()
            .map(|entry| &entry.key)
            .collect::<BTreeSet<_>>()
            .len(),
        4
    );
    let y = reply
        .countries
        .iter()
        .find(|country| country.tag == "Y")
        .unwrap();
    assert_eq!(
        y.observed
            .iter()
            .map(|observation| (
                observation.first_hour - base(),
                observation.last_hour - base(),
                observation.read_hour - base(),
            ))
            .collect::<Vec<_>>(),
        [(1, 1, 0), (6, 6, 6)]
    );
    assert_eq!(
        y.operatives
            .iter()
            .map(|entry| (entry.id.as_str(), entry.joined_hour - base()))
            .collect::<Vec<_>>(),
        [("1", 0), ("3", 6)]
    );
    assert_eq!(
        y.operatives[0].runs[0].value.owner_tag.as_deref(),
        Some("Y")
    );
    assert!(reply.countries.iter().all(|country| {
        country
            .operatives
            .iter()
            .all(|entry| entry.departed_hour.is_none() && entry.departed_reason.is_none())
    }));
    assert!(reply.countries.iter().all(|country| {
        country.observed.iter().all(|observation| {
            observation.last_hour < base() + 5 || observation.first_hour == base() + 6
        })
    }));
}

#[test]
fn operatives_roster_off_read_alias_handoff_preserves_known_empty_and_refuses_ambiguity() {
    for ambiguous in [false, true] {
        let frames = (0..=3)
            .map(|offset| {
                let mut frame = frame(offset, offset == 0 || offset == 3, Vec::new());
                if let TableView::Complete(TableRows::Countries(countries)) =
                    &mut *frame.tables[TableId::Countries.index()]
                {
                    countries[0].tag = if offset == 0 {
                        b"X".to_vec()
                    } else {
                        b"Y".to_vec()
                    };
                    if ambiguous && offset == 1 {
                        countries[1].tag = b"Y".to_vec();
                    }
                }
                frame
            })
            .collect();
        let replay = Replay::from_records(CAMPAIGN.into(), &[records(1, frames)]).unwrap();
        let reply = scan(
            &replay,
            1,
            &["X".into(), "Y".into()],
            None,
            &AtomicBool::new(false),
            None,
            None,
        )
        .unwrap();
        assert!(
            reply
                .countries
                .iter()
                .all(|country| country.operatives.is_empty())
        );
        let expected = if ambiguous {
            vec![(3, 3, 3)]
        } else {
            vec![(1, 2, 0), (3, 3, 3)]
        };
        assert_eq!(
            reply.countries[1]
                .observed
                .iter()
                .map(|observation| (
                    observation.first_hour - base(),
                    observation.last_hour - base(),
                    observation.read_hour - base(),
                ))
                .collect::<Vec<_>>(),
            expected
        );
    }
}

#[test]
fn operatives_roster_names_and_every_mission_use_recorded_content() {
    // Mission localisation keys resolve through the recorded content.
    let keys = [
        "NO_MISSION",
        "BUILD_INTEL_NETWORK",
        "QUIET_INTEL_NETWORK",
        "COUNTER_INTELLIGENCE",
        "ROOT_OUT_RESISTANCE",
        "BOOST_IDEOLOGY",
        "CONTROL_TRADE",
        "DIPLOMATIC_PRESSURE",
        "PROPAGANDA",
    ];
    let mut localisation = "\u{feff}l_english:\n MOD_OPERATIVE:0 \"Authored operative\"\n LEADER_SKILL_WITH_PROGRESS:0 \"Level $SKILL|0$, $PROGRESS|%1$\"\n".to_owned();
    for (variant, key) in keys.iter().enumerate() {
        localisation.push_str(&format!(
            " OPERATIVE_MISSION_{key}_TITLE:0 \"Authored mission {variant}\"\n"
        ));
    }
    let (content, _folder) = super::super::operative_content::test_content(&[
        ("localisation/english/fixture_l_english.yml", &localisation),
        (
            "common/unit_leader/fixture.txt",
            "leader_skills = { 2 = { type = operative cost = 80 } }",
        ),
    ]);
    let rows = (0..=8)
        .map(|variant| OperativeMembershipRow {
            id: u64::from(variant) + 1,
            // The collector represents native discriminator zero as None.
            action_variant: (variant != 0).then_some(variant),
            skill: Some(1),
            experience_raw: 4_000_000,
            name: match variant {
                0 => b"Literal operative".to_vec(),
                1 => b"MOD_OPERATIVE".to_vec(),
                2 => Vec::new(),
                _ => format!("Literal {variant}").into_bytes(),
            }
            .into(),
            ..row(1)
        })
        .collect();
    let reply = content_history(vec![frame(0, true, rows)], &content);
    assert_eq!(reply.countries[0].operatives.len(), 9);
    for (variant, entry) in reply.countries[0].operatives.iter().enumerate() {
        let value = &entry.runs[0].value;
        assert_eq!(
            value.assignment_name,
            Some(format!("Authored mission {variant}"))
        );
        assert_eq!(value.experience_text.as_deref(), Some("Level 1, 50.0%"));
        assert_eq!(value.experience_ratio, Some(0.5));
        assert_eq!(reply.countries[0].observed[0].read_hour, base());
    }
    let names: Vec<_> = reply.countries[0]
        .operatives
        .iter()
        .take(3)
        .map(|entry| entry.runs[0].value.name.as_deref())
        .collect();
    assert_eq!(
        names,
        [Some("Literal operative"), Some("Authored operative"), None]
    );
}

#[test]
fn operatives_roster_capture_date_is_independent_of_unrecorded_reserved_capturer() {
    let reply = history(vec![frame(
        0,
        true,
        vec![OperativeMembershipRow {
            assignment: 0,
            capturer_country: 0,
            captured_hour: base() - 5,
            ..row(1)
        }],
    )]);
    let value = &reply.countries[0].operatives[0].runs[0].value;
    assert_eq!(value.captured, Some(true));
    assert_eq!(value.capturer_tag, None);
    assert_eq!(value.captured_hour, Some(base() - 5));
}

#[test]
fn operatives_roster_capturer_high_bit_alias_is_resolved_as_recorded() {
    let mut frame = frame(
        0,
        true,
        vec![OperativeMembershipRow {
            assignment: 0,
            capturer_country: 0x8000_0011,
            captured_hour: base() - 1,
            ..row(1)
        }],
    );
    if let TableView::Complete(TableRows::CountryLinks(links)) =
        &mut *frame.tables[TableId::CountryLinks.index()]
    {
        links.push(CountryLinkRow {
            id: 0x8000_0011,
            canonical_slot: 2,
        });
    }
    let reply = history(vec![frame]);
    assert_eq!(
        reply.countries[0].operatives[0].runs[0]
            .value
            .capturer_tag
            .as_deref(),
        Some("target old")
    );
}

#[test]
fn operatives_roster_empty_native_c_string_needs_no_content_to_be_unnamed() {
    let reply = history(vec![frame(
        0,
        true,
        vec![OperativeMembershipRow {
            name: b"\0ignored".to_vec().into(),
            ..row(1)
        }],
    )]);
    let value = &reply.countries[0].operatives[0].runs[0].value;
    assert!(value.unnamed);
    assert_eq!(value.name.as_deref(), None);
    assert_eq!(value.name_missing, None);
}

#[test]
fn operatives_roster_alias_duplicates_include_terminal_and_conflicting_memberships() {
    for assignment in [3, 2, 5] {
        let first = OperativeMembershipRow {
            assignment,
            ..row(1)
        };
        let alias = OperativeMembershipRow {
            owner_country: 7,
            roster_country: 7,
            ..first.clone()
        };
        let reply = history(vec![frame(0, true, vec![first, alias])]);
        assert_eq!(reply.countries[0].operatives.len(), 1);
        assert!(!reply.countries[0].observed.is_empty());
    }
    let reply = history(vec![frame(
        0,
        true,
        vec![
            row(1),
            OperativeMembershipRow {
                owner_country: 7,
                roster_country: 7,
                assignment: 5,
                ..row(1)
            },
        ],
    )]);
    assert!(reply.countries[0].observed.is_empty());
    assert!(reply.countries[0].operatives.is_empty());
    assert!(
        reply
            .warnings
            .iter()
            .any(|warning| warning.contains("Conflicting operative memberships"))
    );
}

#[test]
fn operatives_roster_ambiguous_character_links_never_choose_a_portrait() {
    use replay::rows::CharacterOperativeRow;
    let mut frame = frame(0, true, vec![row(1)]);
    *frame.tables[TableId::CharacterOperatives.index()] =
        TableView::Complete(TableRows::CharacterOperatives(vec![
            CharacterOperativeRow {
                character: 8,
                reference: 1,
                resolved: true,
                category: 3,
                ..Default::default()
            },
            CharacterOperativeRow {
                character: 9,
                reference: 1,
                resolved: true,
                category: 3,
                ..Default::default()
            },
        ]));
    let reply = history(vec![frame]);
    assert!(!reply.countries[0].observed.is_empty());
    assert!(
        reply.countries[0].operatives[0].runs[0]
            .value
            .portrait
            .is_none()
    );
    assert!(
        reply
            .warnings
            .iter()
            .any(|warning| warning.contains("conflicting character links")
                && warning.contains("operative 1 portrait"))
    );
}

#[test]
fn operatives_roster_operation_metadata_has_its_own_read_and_gap_recovery() {
    use replay::rows::{LexicalTokenRow, OperationRow};
    let (content, _folder) = super::super::operative_content::test_content(&[(
        "common/operations/fixture.txt",
        "operation_fixture = { name = Authored_operation }",
    )]);
    let frames = (0..=4)
        .map(|hour| {
            let mut frame = frame(
                hour,
                hour == 0,
                vec![OperativeMembershipRow {
                    assignment: 4,
                    operation: Some(71).into(),
                    ..row(1)
                }],
            );
            *frame.tables[TableId::Operations.index()] = if hour == 2 {
                TableView::Unavailable {
                    reason: "fixture operation gap".into(),
                }
            } else {
                TableView::Complete(TableRows::Operations(vec![OperationRow {
                    id: 71,
                    definition_token: 18,
                    ..Default::default()
                }]))
            };
            *frame.tables[TableId::LexicalTokens.index()] =
                TableView::Complete(TableRows::LexicalTokens(vec![LexicalTokenRow {
                    token: 18,
                    literal: b"operation_fixture".to_vec(),
                }]));
            if hour == 1 || hour == 4 {
                frame.metrics.table_ns[TableId::Operations.index()] = 1;
            }
            frame
        })
        .collect();
    let reply = content_history(frames, &content);
    let runs = &reply.countries[0].operatives[0].runs;
    assert_eq!(
        runs.iter()
            .map(|run| (run.first_hour - base(), run.last_hour - base()))
            .collect::<Vec<_>>(),
        [(0, 0), (1, 1), (2, 3), (4, 4)]
    );
    assert_eq!(runs[0].value.assignment_name, None);
    assert_eq!(
        runs[1].value.assignment_name.as_deref(),
        Some("Authored_operation")
    );
    assert_eq!(reply.operations[0].read_hour, base() + 1);
    assert_eq!(runs[2].value.assignment_name, None);
    assert_eq!(reply.operations[1].read_hour, base() + 4);
    assert!(
        reply.countries[0]
            .observed
            .iter()
            .all(|read| read.read_hour == base())
    );
}

#[test]
fn operatives_roster_captured_status_projects_current_country_name_with_read_provenance() {
    use replay::rows::{CountryAppearanceRow, PoliticsRow};
    let (content, _folder) = super::super::operative_content::test_content(&[(
        "localisation/english/fixture_l_english.yml",
        "\u{feff}l_english:\n OPERATIVE_CAPTURED_BY:0 \"Captured by $CAPTURER$\"\n CAPTOR_OLD:0 \"Ordinary old name\"\n CAPTOR_NEW:0 \"Ordinary new name\"\n CAPTOR_OLD_DEF:0 \"old capturer\"\n CAPTOR_NEW_DEF:0 \"renamed capturer\"\n",
    )]);
    let frames = (0..=1)
        .map(|hour| {
            let mut frame = frame(
                hour,
                hour == 0,
                vec![OperativeMembershipRow {
                    assignment: 0,
                    capturer_country: 9,
                    captured_hour: base() - 2,
                    ..row(1)
                }],
            );
            if let TableView::Complete(TableRows::Countries(rows)) =
                &mut *frame.tables[TableId::Countries.index()]
            {
                rows[1].tag = if hour == 0 {
                    b"CAPTOR_OLD".to_vec()
                } else {
                    b"CAPTOR_NEW".to_vec()
                };
            }
            *frame.tables[TableId::CountryAppearance.index()] =
                TableView::Complete(TableRows::CountryAppearance(vec![
                    CountryAppearanceRow {
                        country: 7,
                        ..Default::default()
                    },
                    CountryAppearanceRow {
                        country: 9,
                        ..Default::default()
                    },
                ]));
            *frame.tables[TableId::Politics.index()] =
                TableView::Complete(TableRows::Politics(vec![
                    PoliticsRow {
                        country: 7,
                        ..Default::default()
                    },
                    PoliticsRow {
                        country: 9,
                        ..Default::default()
                    },
                ]));
            *frame.tables[TableId::PoliticalParties.index()] =
                TableView::Complete(TableRows::PoliticalParties(vec![]));
            *frame.tables[TableId::Autonomy.index()] =
                TableView::Complete(TableRows::Autonomy(vec![]));
            frame
        })
        .collect();
    let reply = content_history(frames, &content);
    let runs = &reply.countries[0].operatives[0].runs;
    assert_eq!(
        runs[0].value.assignment_name.as_deref(),
        Some("Captured by Old capturer")
    );
    assert_eq!(
        runs[1].value.assignment_name.as_deref(),
        Some("Captured by Renamed capturer")
    );
    assert_eq!(runs[1].value.capturer_tag.as_deref(), Some("CAPTOR_NEW"));
    assert!(
        runs.iter()
            .all(|run| run.value.captured_hour == Some(base() - 2))
    );
}

#[test]
fn operatives_roster_all_roster_kinds_and_assignments_are_not_player_gated() {
    let mut rows: Vec<_> = (0..=5)
        .map(|assignment| OperativeMembershipRow {
            id: u64::from(assignment) + 1,
            assignment,
            ..row(1)
        })
        .collect();
    rows.extend((1..=3).map(|roster_kind| OperativeMembershipRow {
        id: 100 + u64::from(roster_kind),
        roster_kind,
        ..row(1)
    }));
    let reply = history(vec![frame(0, true, rows)]);
    let country = &reply.countries[0];
    assert_eq!(country.operatives.len(), 6);
    assert_eq!(country.observed.len(), 1);
    assert!(
        country
            .operatives
            .iter()
            .all(|entry| entry.id.parse::<u64>().unwrap() < 100)
    );
    let killed = country
        .operatives
        .iter()
        .find(|entry| entry.id == "6")
        .unwrap();
    assert_eq!(killed.departed_hour, Some(base()));
    assert_eq!(killed.departed_reason.as_deref(), Some("Killed"));
    let captured = country
        .operatives
        .iter()
        .find(|entry| entry.id == "1")
        .unwrap();
    assert_eq!(captured.runs[0].value.captured, Some(true));
    assert!(captured.runs[0].value.capturer_tag.is_none());
    assert!(captured.runs[0].value.captured_hour.is_none());
}

#[test]
fn operatives_roster_capture_has_native_capturer_date_and_missing_skill_is_not_zero() {
    let captured = OperativeMembershipRow {
        assignment: 0,
        capturer_country: 9,
        captured_hour: base() - 3,
        skill: None,
        ..row(1)
    };
    let reply = history(vec![frame(0, true, vec![captured])]);
    let value = &reply.countries[0].operatives[0].runs[0].value;
    assert_eq!(value.capturer_tag.as_deref(), Some("target old"));
    assert_eq!(value.captured_hour, Some(base() - 3));
    assert_eq!(value.skill, None);
    assert_eq!(value.experience_raw, "123456");
}

#[test]
fn operatives_roster_departures_are_by_confirming_read_and_not_guessed_retirements() {
    let frames = (0..=6)
        .map(|hour| {
            frame(
                hour,
                hour % 3 == 0,
                if hour < 6 { vec![row(1)] } else { vec![] },
            )
        })
        .collect();
    let reply = history(frames);
    let entry = &reply.countries[0].operatives[0];
    assert_eq!(entry.joined_hour, base());
    assert_eq!(entry.departed_hour, Some(base() + 6));
    assert_eq!(
        entry.departed_reason.as_deref(),
        Some("Left (reason not recorded)")
    );
    assert_eq!(entry.runs.last().unwrap().last_hour, base() + 5);
    assert_eq!(reply.countries[0].observed[1].read_hour, base() + 3);
    assert_eq!(entry.runs.len(), 1);
    assert_eq!(
        reply.countries[0].observed.last().unwrap().first_hour,
        base() + 6
    );
}

#[test]
fn operatives_roster_prior_capture_does_not_establish_a_later_departure_cause() {
    let frames = (0..=3)
        .map(|hour| {
            frame(
                hour,
                hour == 0 || hour == 3,
                if hour < 3 {
                    vec![OperativeMembershipRow {
                        assignment: 0,
                        captured_hour: base() - 2,
                        ..row(1)
                    }]
                } else {
                    vec![]
                },
            )
        })
        .collect();
    let reply = history(frames);
    let entry = &reply.countries[0].operatives[0];
    assert_eq!(entry.departed_hour, Some(base() + 3));
    assert_eq!(
        entry.departed_reason.as_deref(),
        Some("Left (reason not recorded)")
    );
    assert_eq!(
        entry.runs.last().unwrap().value.captured_hour,
        Some(base() - 2)
    );
}

#[test]
fn operatives_roster_gaps_and_incomplete_tables_never_resume_from_carried_rows() {
    for unavailable in [false, true] {
        let frames = (0..=6)
            .filter(|hour| unavailable || *hour != 2)
            .map(|hour| {
                let mut frame = frame(hour, hour == 0 || hour == 5, vec![row(1)]);
                if unavailable && hour == 2 {
                    *frame.tables[TableId::OperativeMemberships.index()] = TableView::Unavailable {
                        reason: "fixture gap".into(),
                    };
                }
                frame
            })
            .collect();
        let reply = history(frames);
        let entry = &reply.countries[0].operatives[0];
        assert_eq!(
            entry
                .runs
                .iter()
                .map(|run| (run.first_hour - base(), run.last_hour - base()))
                .collect::<Vec<_>>(),
            [(0, 1), (5, 6)]
        );
        assert!(entry.departed_hour.is_none());
        assert_eq!(reply.countries[0].operatives.len(), 1);
    }
}

#[test]
fn operatives_roster_cadence_and_peer_timings_never_date_held_memberships() {
    let mut held = frame(0, false, vec![row(1)]);
    held.metrics.table_ns[TableId::WingCumulative.index()] = 1;
    let reply = history(vec![held]);
    assert!(reply.countries[0].observed.is_empty());
    assert!(reply.countries[0].operatives.is_empty());
    let mut next = frame(1, false, vec![row(1)]);
    next.metrics.table_ns[TableId::WingCumulative.index()] = 1;
    let reply = history(vec![frame(0, true, vec![row(1)]), next]);
    let country = &reply.countries[0];
    assert_eq!(country.observed.len(), 1);
    assert_eq!(country.observed[0].read_hour, base());
    assert_eq!(country.operatives[0].runs[0].last_hour, base() + 1);
    assert_eq!(country.observed[0].read_hour, base());
}

#[test]
fn operatives_roster_targets_follow_the_hour_but_reused_links_do_not_reassign_reads() {
    let frames = (0..=3)
        .map(|hour| {
            let mut frame = frame(
                hour,
                hour == 0,
                vec![OperativeMembershipRow {
                    target_country: Some(9).into(),
                    action_variant: Some(1),
                    ..row(1)
                }],
            );
            if let TableView::Complete(TableRows::Countries(countries)) =
                &mut *frame.tables[TableId::Countries.index()]
            {
                if hour >= 1 {
                    countries[1].tag = b"renamed target".to_vec();
                }
                if hour >= 2 {
                    countries[1].id = 10;
                }
            }
            if hour >= 2 {
                *frame.tables[TableId::CountryLinks.index()] =
                    TableView::Complete(TableRows::CountryLinks(vec![
                        CountryLinkRow {
                            id: 7,
                            canonical_slot: 1,
                        },
                        CountryLinkRow {
                            id: 9,
                            canonical_slot: 2,
                        },
                        CountryLinkRow {
                            id: 10,
                            canonical_slot: 2,
                        },
                        CountryLinkRow {
                            id: 17,
                            canonical_slot: 1,
                        },
                    ]));
            }
            frame
        })
        .collect();
    let reply = history(frames);
    let runs = &reply.countries[0].operatives[0].runs;
    assert_eq!(runs[0].value.target_tag.as_deref(), Some("target old"));
    assert_eq!(runs[1].value.target_tag.as_deref(), Some("renamed target"));
    assert_eq!(runs[2].value.target_tag, None);
    assert!(
        reply.countries[0]
            .observed
            .iter()
            .all(|read| read.read_hour == base())
    );
}

#[test]
fn operatives_roster_unknown_forms_are_named_counted_and_never_an_empty_roster() {
    let reply = history(vec![frame(
        0,
        true,
        vec![OperativeMembershipRow {
            roster_kind: 9,
            ..row(1)
        }],
    )]);
    assert!(reply.countries[0].observed.is_empty());
    assert!(
        reply
            .warnings
            .iter()
            .any(|warning| warning.contains("1 operative fields")
                && warning.contains("roster 9")
                && warning.contains("operative 1"))
    );
}

#[test]
fn operatives_roster_unresolved_roster_aliases_are_not_complete_empty_and_terminal_reads_do_not_repeat()
 {
    let reply = history(vec![frame(
        0,
        true,
        vec![OperativeMembershipRow {
            roster_country: 123_456,
            ..row(1)
        }],
    )]);
    assert!(reply.countries[0].observed.is_empty());
    assert!(
        reply
            .warnings
            .iter()
            .any(|warning| warning.contains("unresolved roster country")
                && warning.contains("123456"))
    );
    for assignment in [2, 5] {
        let reply = history(
            (0..=3)
                .map(|hour| {
                    frame(
                        hour,
                        true,
                        vec![OperativeMembershipRow {
                            assignment,
                            ..row(1)
                        }],
                    )
                })
                .collect(),
        );
        assert_eq!(reply.countries[0].operatives.len(), 1);
        assert_eq!(reply.countries[0].operatives[0].departed_hour, Some(base()));
        assert_eq!(reply.countries[0].operatives[0].runs.len(), 1);
    }
}

#[test]
fn operatives_roster_five_thousand_countries_have_sparse_complete_histories_and_size_accounting() {
    let count = 5000;
    let mut frame = frame(0, true, Vec::new());
    let countries: Vec<_> = (1..=count)
        .map(|id| CountryRow {
            id,
            canonical_slot: id,
            tag: format!("country {id}").into_bytes(),
            ..Default::default()
        })
        .collect();
    let links = (1..=count)
        .map(|id| CountryLinkRow {
            id,
            canonical_slot: id,
        })
        .collect();
    let rows = (1..=count)
        .map(|id| OperativeMembershipRow {
            id: u64::from(id),
            owner_country: id,
            roster_country: id,
            name: format!("Literal {id}").into_bytes().into(),
            assignment: 3,
            ..Default::default()
        })
        .collect();
    *frame.tables[TableId::Countries.index()] =
        TableView::Complete(TableRows::Countries(countries));
    *frame.tables[TableId::CountryLinks.index()] =
        TableView::Complete(TableRows::CountryLinks(links));
    *frame.tables[TableId::OperativeMemberships.index()] =
        TableView::Complete(TableRows::OperativeMemberships(rows));
    let replay = Replay::from_records(CAMPAIGN.into(), &[records(1, vec![frame])]).unwrap();
    // Exercise the builder for every country; ordinary UI requests select a
    // comparison set, without restricting which country can be selected.
    let tags = (1..=count)
        .map(|id| format!("country {id}"))
        .collect::<Vec<_>>();
    let cancelled = AtomicBool::new(false);
    let histories = dashboard_base::products::country_history::scan_histories(
        &replay,
        1,
        false,
        &[],
        &cancelled,
    )
    .unwrap();
    let identities =
        dashboard_base::products::country_history::selected_identities(&histories, &tags);
    let reply = scan(&replay, 1, &tags, None, &cancelled, None, Some(&identities)).unwrap();
    assert_eq!(reply.countries.len(), 5000);
    assert!(
        reply
            .countries
            .iter()
            .all(|country| country.operatives.len() == 1 && country.observed.len() == 1)
    );
    assert!(json_len(&reply, REPLY_BUDGET).unwrap() < REPLY_BUDGET);
    let mut budget = Budget(REPLY_BUDGET - 2);
    assert!(budget.add(&"oversize roster").is_err());
}

#[test]
#[cfg_attr(
    not(feature = "slow-tests"),
    ignore = "slow: runs in the full suite (--features slow-tests)"
)]
fn operatives_roster_cross_block_content_artwork_is_interned_and_equal_daily_reads_merge() {
    engine::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
    use replay::rows::{
        CharacterOperativeRow, CharacterPortrait, CharacterPortraitRow, LexicalTokenRow,
        OperationRow, PORTRAIT_LARGE,
    };
    // Synthetic content reaches both native portrait and operation artwork;
    // the second layer changes the authored title without touching installed content.
    let (_, base_folder) = dashboard_text::catalogue::tests::folder_catalogue(&[
        ("gfx/interface/present.txt", ""),
        (
            "interface/fixture.gfx",
            "spriteTypes = { spriteType = { name = GFX_synthetic_art texturefile = gfx/interface/synthetic.tga } }",
        ),
        (
            "common/defines/fixture.lua",
            "NDefines = { NCharacter = { OFFICER_CORP_ADVISOR_ENTRIES_IN_MENU = { \"fixture\" } } }",
        ),
        (
            "common/operations/fixture.txt",
            "synthetic_operation = { name = SYNTHETIC_TITLE map_icon = GFX_synthetic_art }",
        ),
        (
            "common/unit_leader/fixture.txt",
            "leader_skills = { 2 = { type = operative cost = 80 } }",
        ),
    ]);
    let (_, mod_folder) = dashboard_text::catalogue::tests::folder_catalogue(&[(
        "localisation/english/fixture_l_english.yml",
        "\u{feff}l_english:\n SYNTHETIC_TITLE:0 \"Layered authored operation\"\n SYNTHETIC_NAME:0 \"Layered operative\"\n LEADER_SKILL_WITH_PROGRESS:0 \"Level $SKILL|0$, $PROGRESS|%1$\"\n",
    )]);
    let mut image = vec![0; 18];
    image[2] = 2;
    image[12] = 96;
    image[14] = 96;
    image[16] = 32;
    image[17] = 0x28;
    let mut random = 17u32;
    for _ in 0..96 * 96 {
        random ^= random << 13;
        random ^= random >> 17;
        random ^= random << 5;
        image.extend_from_slice(&[random as u8, (random >> 8) as u8, (random >> 16) as u8, 255]);
    }
    std::fs::write(base_folder.0.join("gfx/interface/synthetic.tga"), image).unwrap();
    let layers = installed_content::ResolvedLayers::from_folders(&[
        (&base_folder.0, &[][..]),
        (&mod_folder.0, &[][..]),
    ])
    .unwrap();
    let content = super::super::operative_content::test_content_from_catalogue(
        std::sync::Arc::new(dashboard_text::catalogue::tests::layers_catalogue(layers)),
    );
    let rows: Vec<_> = (1..=64)
        .map(|id| OperativeMembershipRow {
            id,
            assignment: 4,
            operation: Some(71).into(),
            name: b"SYNTHETIC_NAME".to_vec().into(),
            skill: Some(1),
            ..row(id)
        })
        .collect();
    let mut template = frame(0, true, rows);
    *template.tables[TableId::CharacterOperatives.index()] =
        TableView::Complete(TableRows::CharacterOperatives(
            (1..=64)
                .map(|id| CharacterOperativeRow {
                    character: id as u32,
                    reference: id,
                    resolved: true,
                    category: 3,
                    ..Default::default()
                })
                .collect(),
        ));
    *template.tables[TableId::CharacterPortraits.index()] =
        TableView::Complete(TableRows::CharacterPortraits(
            (1..=64)
                .map(|character| CharacterPortraitRow {
                    character,
                    portraits: vec![CharacterPortrait {
                        category: 4,
                        size: PORTRAIT_LARGE,
                        kind: 1,
                        is_set: 1,
                        identity: b"gfx/interface/synthetic.tga".to_vec(),
                    }],
                })
                .collect(),
        ));
    *template.tables[TableId::Operations.index()] =
        TableView::Complete(TableRows::Operations(vec![OperationRow {
            id: 71,
            definition_token: 18,
            ..Default::default()
        }]));
    *template.tables[TableId::LexicalTokens.index()] =
        TableView::Complete(TableRows::LexicalTokens(vec![LexicalTokenRow {
            token: 18,
            literal: b"synthetic_operation".to_vec(),
        }]));
    // Cross the next encoded keyframe/block while retaining daily held reads.
    let days = i32::try_from(replay::KEYFRAME_INTERVAL / 24 + 1).unwrap();
    // Stream the frames to the real encoder, retaining shared table storage.
    let frames = (0..days * 24)
        .map(|hour| {
            let mut frame = template.clone();
            frame.hour = base() + hour;
            frame.metrics.table_ns[TableId::OperativeMemberships.index()] =
                u64::from(hour % 24 == 0);
            frame.metrics.table_ns[TableId::Operations.index()] = u64::from(hour % 24 == 0);
            frame
        })
        .collect();
    let reply = content_history(frames, &content);
    let country = &reply.countries[0];
    assert_eq!(country.observed.len(), days as usize);
    assert_eq!(reply.operations.len(), days as usize);
    assert_eq!(country.operatives.len(), 64);
    assert!(country.operatives.iter().all(|entry| entry.runs.len() == 1 && entry.runs[0].last_hour == base() + days * 24 - 1));
    assert!(
        country
            .operatives
            .iter()
            .all(
                |entry| entry.runs[0].value.name.as_deref() == Some("Layered operative")
                    && entry.runs[0].value.assignment_name.as_deref()
                        == Some("Layered authored operation")
            )
    );
    assert!(!reply.artwork.is_empty());
    assert!(reply.artwork.iter().any(|image| image.len() > 20_000));
    assert!(
        country
            .operatives
            .iter()
            .all(|entry| entry.runs[0].value.portrait.is_some()
                && entry.runs[0].value.assignment_icon.is_some())
    );
    assert!(reply.artwork.len() <= 2);
    assert!(reply.reasons.len() < 4);
    let bytes =
        json_len(&reply, REPLY_BUDGET).expect("cross-block reply below complete-response budget");
    assert!(bytes < 1 << 20, "compressed reply has {bytes} bytes");
}

#[test]
fn operatives_roster_absent_agency_is_reported_and_stops_holds() {
    let mut absent = frame(1, false, vec![row(1)]);
    *absent.tables[TableId::SimulationFeatures.index()] =
        TableView::Complete(TableRows::SimulationFeatures(vec![
            replay::rows::SimulationFeatureRow::default(),
        ]));
    *absent.tables[TableId::OperativeMemberships.index()] = TableView::Unavailable {
        reason: "feature absent".into(),
    };
    let reply = history(vec![
        frame(0, true, vec![row(1)]),
        absent,
        frame(2, false, vec![row(1)]),
    ]);
    assert_eq!(reply.countries[0].observed[0].last_hour, base());
    assert!(
        reply
            .warnings
            .iter()
            .any(|warning| warning.contains("agency feature is absent"))
    );
    assert_eq!(reply.agency_absent[0].first_hour, base() + 1);
    assert_eq!(reply.agency_absent[0].last_hour, base() + 1);
}

#[test]
fn operatives_roster_captured_native_day_status_uses_current_hour_and_preserves_read_date() {
    use replay::rows::{CountryAppearanceRow, PoliticsRow};
    let (content, _folder) = super::super::operative_content::test_content(&[
        (
            "common/defines/zz_capture.lua",
            "NDefines.NOperatives = { OPERATIVE_CAPTURE_DURATION_IN_DAYS = 17 }",
        ),
        (
            "localisation/english/fixture_l_english.yml",
            "\u{feff}l_english:\n OPERATIVE_CAPTURED_BY:0 \"Held by $CAPTURER$\"\n DURATION_DAYS:0 \"$NUM|.0$ remaining days\"\n",
        ),
    ]);
    let captured = engine::cdate::HOUR_EPOCH + 23;
    let frames = (0..=2)
        .map(|hour| {
            let mut frame = frame(
                hour,
                hour == 0,
                vec![OperativeMembershipRow {
                    assignment: 0,
                    capturer_country: 9,
                    captured_hour: captured,
                    ..row(1)
                }],
            );
            frame.hour = captured + hour;
            *frame.tables[TableId::CountryAppearance.index()] =
                TableView::Complete(TableRows::CountryAppearance(
                    [7, 9]
                        .map(|country| CountryAppearanceRow {
                            country,
                            ..Default::default()
                        })
                        .to_vec(),
                ));
            *frame.tables[TableId::Politics.index()] = TableView::Complete(TableRows::Politics(
                [7, 9]
                    .map(|country| PoliticsRow {
                        country,
                        ..Default::default()
                    })
                    .to_vec(),
            ));
            *frame.tables[TableId::PoliticalParties.index()] =
                TableView::Complete(TableRows::PoliticalParties(vec![]));
            *frame.tables[TableId::Autonomy.index()] =
                TableView::Complete(TableRows::Autonomy(vec![]));
            frame
        })
        .collect();
    let reply = content_history(frames, &content);
    let country = &reply.countries[0];
    let runs = &country.operatives[0].runs;
    assert_eq!(country.observed[0].read_hour, captured);
    assert_eq!(runs.len(), 2);
    assert_eq!(
        runs[0].value.assignment_duration.as_deref(),
        Some("17 remaining days")
    );
    assert_eq!(runs[0].value.assignment_progress, Some(0.0));
    assert_eq!(
        runs[1].value.assignment_duration.as_deref(),
        Some("16 remaining days")
    );
    assert_eq!(
        runs[1].value.assignment_progress,
        Some(f64::from(1.0f32 / 17.0))
    );
    assert_eq!(runs[1].last_hour, captured + 2);
    assert!(
        runs.iter()
            .all(|run| run.value.captured_hour == Some(captured))
    );
}

#[test]
fn operatives_roster_unknown_mission_variant_is_refused_even_without_content() {
    let reply = history(vec![frame(
        0,
        true,
        vec![OperativeMembershipRow {
            action_variant: Some(91),
            ..row(1)
        }],
    )]);
    let value = &reply.countries[0].operatives[0].runs[0].value;
    assert!(
        reply.reasons[value.assignment_missing.unwrap() as usize].contains("mission variant 91")
    );
    assert!(
        reply.warnings.iter().any(
            |warning| warning.contains("mission variant 91") && warning.contains("operative 1")
        )
    );
}

#[test]
fn operatives_roster_unresolved_other_country_does_not_hide_a_valid_roster() {
    let mut frame = frame(0, true, vec![row(1)]);
    if let TableView::Complete(TableRows::Countries(countries)) =
        &mut *frame.tables[TableId::Countries.index()]
    {
        countries.push(CountryRow {
            id: 11,
            canonical_slot: 4,
            tag: b"unresolved other country".to_vec(),
            ..Default::default()
        });
    }
    let reply = history(vec![frame]);
    assert_eq!(reply.countries[0].operatives.len(), 1);
    assert_eq!(reply.countries[0].observed.len(), 1);
    assert!(
        reply
            .warnings
            .iter()
            .any(|warning| warning.contains("country identity is unresolved")
                && warning.contains("country 11"))
    );
}

#[test]
fn operatives_roster_held_cooldown_advances_without_a_membership_read() {
    let (content, _folder) = super::super::operative_content::test_content(&[(
        "localisation/english/fixture_l_english.yml",
        "\u{feff}l_english:\n OPERATIVE_FORCED_INTO_HIDING:0 \"Authored hiding\"\n DURATION_DAYS:0 \"$NUM|.0$ authored days\"\n",
    )]);
    let operative = OperativeMembershipRow {
        assignment: 1,
        cooldown_kind: Some(3).into(),
        cooldown_start_hour: Some(base()).into(),
        cooldown_end_hour: Some(base() + 50).into(),
        ..row(1)
    };
    let reply = content_history(
        (0..=27)
            .map(|hour| frame(hour, hour == 0, vec![operative.clone()]))
            .collect(),
        &content,
    );
    assert!(
        reply.countries[0]
            .observed
            .iter()
            .all(|span| span.read_hour == base())
    );
    let runs = &reply.countries[0].operatives[0].runs;
    assert_eq!(
        runs[0].value.assignment_name.as_deref(),
        Some("Authored hiding")
    );
    assert_eq!(
        runs[0].value.assignment_duration.as_deref(),
        Some("3 authored days")
    );
    assert_eq!(
        runs.last().unwrap().value.assignment_duration.as_deref(),
        Some("1 authored days")
    );
    assert_eq!(runs.last().unwrap().value.assignment_progress, Some(0.5));
}

#[test]
fn operative_portrait_join_requires_full_reference_and_native_category() {
    let reference = (1u64 << 32) | 17;
    let rows = [
        CharacterOperativeRow {
            character: 8,
            reference,
            resolved: true,
            category: 2,
            ..Default::default()
        },
        CharacterOperativeRow {
            character: 9,
            reference: (2u64 << 32) | 17,
            resolved: true,
            category: 3,
            ..Default::default()
        },
        CharacterOperativeRow {
            character: 10,
            reference: (3u64 << 32) | 17,
            resolved: false,
            ..Default::default()
        },
    ];
    let links = character_links(&rows, &mut BoundedRefusals::new(REPLY_BUDGET));
    assert!(!links.contains_key(&reference));
    assert_eq!(links.get(&((2u64 << 32) | 17)), Some(&Some(9)));
    assert!(!links.contains_key(&((3u64 << 32) | 17)));
}

#[test]
fn captured_roster_resolves_five_thousand_distinct_captors() {
    use replay::rows::{CountryAppearanceRow, PoliticsRow};
    let count = 5000;
    let mut localisation = String::from(
        "\u{feff}l_english:\n OPERATIVE_CAPTURED_BY:0 \"Captured by $CAPTURER$\"\n DURATION_DAYS:0 \"Remaining span: $NUM|.0$ days\"\n",
    );
    for id in 1..=count {
        localisation.push_str(&format!(" country_{id}_DEF:0 \"Captor {id}\"\n"));
    }
    let (content, _folder) = super::super::operative_content::test_content(&[
        ("localisation/english/fixture_l_english.yml", &localisation),
        (
            "common/defines/zz_capture.lua",
            "NDefines.NOperatives = { OPERATIVE_CAPTURE_DURATION_IN_DAYS = 7 }",
        ),
    ]);
    let mut frame = frame(0, true, Vec::new());
    let countries: Vec<_> = (1..=count)
        .map(|id| CountryRow {
            id,
            canonical_slot: id,
            tag: format!("country_{id}").into_bytes(),
            ..Default::default()
        })
        .collect();
    let links = (1..=count)
        .map(|id| CountryLinkRow {
            id,
            canonical_slot: id,
        })
        .collect();
    let appearance = (1..=count)
        .map(|country| CountryAppearanceRow {
            country,
            ..Default::default()
        })
        .collect();
    let politics = (1..=count)
        .map(|country| PoliticsRow {
            country,
            ..Default::default()
        })
        .collect();
    let operatives = (1..=count)
        .map(|id| OperativeMembershipRow {
            id: u64::from(id),
            owner_country: id,
            roster_country: id,
            capturer_country: id % count + 1,
            captured_hour: base() - 24,
            assignment: 0,
            name: format!("Literal {id}").into_bytes().into(),
            ..Default::default()
        })
        .collect();
    *frame.tables[TableId::Countries.index()] =
        TableView::Complete(TableRows::Countries(countries));
    *frame.tables[TableId::CountryLinks.index()] =
        TableView::Complete(TableRows::CountryLinks(links));
    *frame.tables[TableId::CountryAppearance.index()] =
        TableView::Complete(TableRows::CountryAppearance(appearance));
    *frame.tables[TableId::Politics.index()] = TableView::Complete(TableRows::Politics(politics));
    *frame.tables[TableId::PoliticalParties.index()] =
        TableView::Complete(TableRows::PoliticalParties(vec![]));
    *frame.tables[TableId::Autonomy.index()] = TableView::Complete(TableRows::Autonomy(vec![]));
    *frame.tables[TableId::OperativeMemberships.index()] =
        TableView::Complete(TableRows::OperativeMemberships(operatives));
    let replay = Replay::from_records(CAMPAIGN.into(), &[records(1, vec![frame])]).unwrap();
    let tags: Vec<_> = (1..=count).map(|id| format!("country_{id}")).collect();
    let reply = scan(
        &replay,
        1,
        &tags,
        Some(&content),
        &AtomicBool::new(false),
        None,
        None,
    )
    .unwrap();
    assert_eq!(reply.countries.len(), 5000);
    for (index, country) in reply.countries.iter().enumerate() {
        let captor = (index as u32 + 1) % count + 1;
        let value = &country.operatives[0].runs[0].value;
        assert_eq!(
            value.capturer_tag.as_deref(),
            Some(format!("country_{captor}").as_str())
        );
        assert_eq!(
            value.assignment_name,
            Some(format!("Captured by Captor {captor}"))
        );
        assert_eq!(country.observed[0].read_hour, base());
    }
}

#[test]
fn captured_status_names_reserved_zero_without_inventing_a_country_or_flag() {
    use replay::rows::ReservedCountryNamingRow;
    let (content, _folder) = super::super::operative_content::test_content(&[
        (
            "localisation/english/fixture_l_english.yml",
            "\u{feff}l_english:\n ZERO_DEF:0 \"the reserved capturer\"\n OPERATIVE_CAPTURED_BY:0 \"Captured by $CAPTURER$\"\n DURATION_DAYS:0 \"Remaining span: $NUM|.0$ days\"\n",
        ),
        (
            "common/defines/zz_capture.lua",
            "NDefines.NOperatives = { OPERATIVE_CAPTURE_DURATION_IN_DAYS = 7 }",
        ),
    ]);
    for capturer_country in [0, 0x8000_0011] {
        let frames = (0..3)
            .map(|hour| {
                let mut frame = frame(
                    hour,
                    hour == 0,
                    vec![OperativeMembershipRow {
                        assignment: 0,
                        capturer_country,
                        captured_hour: base() - 5,
                        ..row(1)
                    }],
                );
                if capturer_country != 0
                    && let TableView::Complete(TableRows::CountryLinks(links)) =
                        &mut *frame.tables[TableId::CountryLinks.index()]
                {
                    links.push(CountryLinkRow {
                        id: capturer_country,
                        canonical_slot: 0,
                    });
                }
                if hour < 2 {
                    *frame.tables[TableId::ReservedCountryNaming.index()] =
                        TableView::Complete(TableRows::ReservedCountryNaming(vec![
                            ReservedCountryNamingRow {
                                tag: b"ZERO".to_vec(),
                                built: if hour == 0 { 0 } else { 2 },
                                ..Default::default()
                            },
                        ]));
                }
                frame
            })
            .collect();
        let reply = content_history(frames, &content);
        let runs = &reply.countries[0].operatives[0].runs;
        assert_eq!(
            runs[0].value.assignment_name.as_deref(),
            Some("Captured by ")
        );
        assert_eq!(
            runs[1].value.assignment_name.as_deref(),
            Some("Captured by The reserved capturer")
        );
        assert!(runs[2].value.assignment_name.is_none());
        assert!(runs[2].value.assignment_missing.is_some());
        assert!(runs.iter().all(|run| run.value.capturer_tag.is_none()));
        assert!(
            runs.iter()
                .all(|run| run.value.captured_hour == Some(base() - 5))
        );
        assert_eq!(reply.countries[0].observed[0].read_hour, base());
    }
}
