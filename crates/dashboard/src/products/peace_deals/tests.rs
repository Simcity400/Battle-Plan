use super::*;
use replay::{
    branches::{BranchReason, BranchStart, Parentage},
    frame::{FrameEncoder, Metrics, TableView},
    identity::{Identity, Roster, SampleBoundary, SessionStart, Verification},
    rows::*,
    spool::Record,
};

const CAMPAIGN: &[u8] = b"derived-peace-fixture";

fn frame(hour: i32, count: u32, settled: bool, conferences: &[u64]) -> DecodedFrame {
    let mut tables = std::array::from_fn(|_| TableView::Unavailable {
        reason: "outside peace fixture".into(),
    });
    tables[TableId::Campaign.index()] =
        TableView::Complete(TableRows::Campaign(vec![CampaignRow {
            id: CAMPAIGN.into(),
        }]));
    tables[TableId::Countries.index()] = TableView::Complete(TableRows::Countries(
        (1..=count)
            .map(|id| CountryRow {
                id: 10_000 + id,
                canonical_slot: id,
                tag: format!("country/{id}").into_bytes(),
                ..Default::default()
            })
            .collect(),
    ));
    tables[TableId::CountryLinks.index()] = TableView::Complete(TableRows::CountryLinks(
        [1_000, 10_000]
            .into_iter()
            .flat_map(|base| {
                (1..=count).map(move |id| CountryLinkRow {
                    id: base + id,
                    canonical_slot: id,
                })
            })
            .collect(),
    ));
    tables[TableId::LexicalTokens.index()] =
        TableView::Complete(TableRows::LexicalTokens(vec![LexicalTokenRow {
            token: 77,
            literal: b"take_states".to_vec(),
        }]));
    tables[TableId::States.index()] = TableView::Complete(TableRows::States(
        (1..=count)
            .map(|id| StateRow {
                id,
                owner: 1_000
                    + if settled && id % 2 == 1 && id < count {
                        id + 1
                    } else {
                        id
                    },
                ..Default::default()
            })
            .collect(),
    ));
    tables[TableId::Provinces.index()] = TableView::Complete(TableRows::Provinces(
        (1..=count)
            .flat_map(|id| {
                [
                    ProvinceRow {
                        id: id * 2,
                        state: id,
                        victory_points: 7,
                        ..Default::default()
                    },
                    ProvinceRow {
                        id: id * 2 + 1,
                        state: id,
                        victory_points: -2,
                        ..Default::default()
                    },
                ]
            })
            .collect(),
    ));
    tables[TableId::WarRelations.index()] = TableView::Complete(TableRows::WarRelations(
        (1..count)
            .step_by(2)
            .map(|id| WarRelationRow {
                first_country: 1_000 + id,
                second_country: 1_001 + id,
                start_hour: 80,
                end_hour: if settled { 101 } else { 0 },
                ..Default::default()
            })
            .collect(),
    ));
    tables[TableId::PeaceConferences.index()] = TableView::Complete(TableRows::PeaceConferences(
        conferences
            .iter()
            .enumerate()
            .map(|(ordinal, &id)| PeaceConferenceRow {
                id,
                ordinal: ordinal as u32,
                current_turn: hour - 100,
                finalized: 0,
            })
            .collect(),
    ));
    tables[TableId::Subjects.index()] = TableView::Complete(TableRows::Subjects(vec![]));
    tables[TableId::PeaceSettlements.index()] =
        TableView::Complete(TableRows::PeaceSettlements(vec![]));
    tables[TableId::CountryCounterInputs.index()] =
        TableView::Complete(TableRows::CountryCounterInputs(
            (1..=count)
                .map(|id| CountryCounterInputRow {
                    country: id + 1_000,
                    owned_state_count: if settled && id % 2 == 1 && id < count {
                        0
                    } else {
                        1
                    },
                    ..Default::default()
                })
                .collect(),
        ));
    tables[TableId::Politics.index()] = TableView::Complete(TableRows::Politics(
        (1..=count)
            .map(|id| PoliticsRow {
                country: id + 1_000,
                ruling_party: Some(3),
                ..Default::default()
            })
            .collect(),
    ));
    tables[TableId::PoliticalParties.index()] = TableView::Complete(TableRows::PoliticalParties(
        (1..=count)
            .map(|id| PoliticalPartyRow {
                country: id + 1_000,
                id: 3,
                token: b"authored_group".to_vec(),
                ..Default::default()
            })
            .collect(),
    ));
    tables[TableId::CountryAppearance.index()] = TableView::Complete(TableRows::CountryAppearance(
        (1..=count)
            .map(|id| CountryAppearanceRow {
                country: id + 1_000,
                leader_ideology: b"authored_ideology".to_vec(),
                ..Default::default()
            })
            .collect(),
    ));
    let mut metrics = Metrics::default();
    metrics.table_ns[TableId::PeaceConferences.index()] = 1;
    DecodedFrame {
        branch: 1,
        hour,
        tables: tables.map(Into::into),
        metrics,
    }
}

fn replay(frames: Vec<DecodedFrame>) -> Replay {
    let first = frames[0].hour;
    let identity = Identity {
        game_version: "synthetic".into(),
        exe_hash: [1; 32],
        exe_size: 1,
        vanilla_hash: [2; 32],
        dlc: vec![],
        mods: vec![],
        coverage_policy: 1,
        language: "l_english".into(),
        recorder_version: "test".into(),
        collector_hash: [3; 32],
        verification: Verification::Verified {
            launcher: Roster::default(),
            memory: Roster::default(),
        },
    };
    let mut records = vec![
        Record::SessionStart(SessionStart {
            id: 1,
            process_id: 1,
            process_start_time: 1,
            qpc_frequency: 1,
            sample_boundary: SampleBoundary::EndOfHour,
            identity,
        }),
        Record::BranchStart(BranchStart {
            id: 1,
            session: 1,
            first_hour: first,
            loaded_hour: None,
            parent: Parentage::Unknown,
            reason: BranchReason::SessionStart,
        }),
    ];
    let mut encoder = FrameEncoder::new();
    records.extend(
        frames
            .into_iter()
            .map(|frame| Record::Frame(encoder.encode(frame, false).unwrap())),
    );
    Replay::from_records(CAMPAIGN.into(), &[records]).unwrap()
}

fn scan(frames: Vec<DecodedFrame>) -> PeaceDealsReply {
    scan_branch(&replay(frames), 1, &AtomicBool::new(false)).unwrap()
}

fn settled_row() -> PeaceSettlementRow {
    PeaceSettlementRow {
        sequence: 8,
        conference: Some(9),
        hour: Some(101),
        winners: vec![1002],
        losers: vec![1001],
        actions: vec![SettledPeaceAction {
            token: 991,
            country: 1002,
            turn: 1,
            ordinal: 2,
            taker: 1002,
            giver: 1001,
            negotiator: 1003,
            state: Some(1),
            flags: Some([0; 4]),
            ..Default::default()
        }],
        transfers: vec![SettledPeaceTransfer {
            state: 1,
            taker: 1002,
            automatic: false,
        }],
        subjects: vec![SettledPeaceSubject {
            overlord: 1002,
            subject: 1003,
        }],
        ..Default::default()
    }
}
fn with_settlement(mut frame: DecodedFrame, row: PeaceSettlementRow) -> DecodedFrame {
    frame.tables[TableId::PeaceSettlements.index()] =
        TableView::Complete(TableRows::PeaceSettlements(vec![row])).into();
    frame.tables[TableId::LexicalTokens.index()] =
        TableView::Complete(TableRows::LexicalTokens(vec![LexicalTokenRow {
            token: 991,
            literal: b"custom_frontier_demand".to_vec(),
        }]))
        .into();
    frame
}

#[test]
fn exact_actions_replace_derivation_and_capture_conferences_missed_between_hours() {
    for roster in [vec![9], vec![]] {
        let result = scan(vec![
            frame(100, 3, false, &roster),
            with_settlement(frame(101, 3, true, &[]), settled_row()),
        ]);
        let deal = result
            .deals
            .iter()
            .find(|d| d.conference_id.as_deref() == Some("9"))
            .unwrap();
        assert!(!deal.derived && !deal.participants_derived);
        let captured = deal.captured.as_ref().unwrap();
        assert_eq!(captured.actions[0].ordinal, 2);
        assert_eq!(captured.actions[0].negotiator, 1003);
        assert_eq!(
            captured.actions[0].token_name.as_deref(),
            Some("custom_frontier_demand")
        );
        assert_eq!(
            captured.subjects[0].subject_tag.as_deref(),
            Some("country/3")
        );
        assert_eq!(deal.items.len(), 1);
        assert!(!deal.items[0].derived);
        assert_eq!(
            result
                .transfers
                .links
                .iter()
                .map(|l| l.state_count)
                .sum::<u32>(),
            1
        );
    }
}

#[test]
fn unavailable_or_incomplete_manifests_keep_the_derived_fallback() {
    for (failure, omitted) in [(Some(b"action vtable unsupported".to_vec()), 0), (None, 2)] {
        let mut row = settled_row();
        row.failure = failure;
        row.omitted = omitted;
        let mut after = with_settlement(frame(101, 3, true, &[]), row);
        *after.tables[TableId::WarRelations.index()] =
            TableView::Complete(TableRows::WarRelations(vec![]));
        let result = scan(vec![frame(100, 3, false, &[9]), after]);
        assert!(result.deals[0].derived);
        assert!(result.deals[0].captured.is_none());
        assert_eq!(result.transfers.links[0].state_count, 1);
        assert_eq!(result.deals[0].winners, ["country/2"]);
        assert_eq!(result.deals[0].losers, ["country/1"]);
        assert_eq!(result.deals[0].end_hour, Some(101));
        assert!(result.deals[0].end_is_by);
        assert_eq!(result.deals[0].items[0].dates.before_hour, 100);
        assert_eq!(result.deals[0].items[0].dates.after_hour, 101);
        assert!(
            result.deals[0]
                .limitations
                .iter()
                .any(|s| s.contains("unavailable or truncated"))
        );
    }
}

#[test]
fn exact_action_tags_use_only_the_capture_hour_and_keep_unresolved_raw_operands() {
    let mut row = settled_row();
    row.actions[0].country = 1004;
    row.actions[0].country_operand = Some(1005);
    row.actions.push(SettledPeaceAction {
        country: 1006,
        taker: 1006,
        giver: 1006,
        negotiator: 1006,
        country_operand: Some(1006),
        ..Default::default()
    });
    let mut publication = frame(102, 6, true, &[]);
    if let TableView::Complete(TableRows::Countries(countries)) =
        &mut *publication.tables[TableId::Countries.index()]
    {
        for country in countries {
            country.tag = format!("later/{}", country.canonical_slot).into_bytes();
        }
    }
    let result = scan(vec![
        frame(101, 5, true, &[]),
        with_settlement(publication, row),
    ]);
    let captured = result
        .deals
        .iter()
        .find_map(|deal| deal.captured.as_ref())
        .unwrap();
    assert_eq!((captured.hour, captured.publication_hour), (101, 102));
    let action = &captured.actions[0];
    assert_eq!(action.country_tag.as_deref(), Some("country/4"));
    assert_eq!(action.taker_tag.as_deref(), Some("country/2"));
    assert_eq!(action.giver_tag.as_deref(), Some("country/1"));
    assert_eq!(action.negotiator_tag.as_deref(), Some("country/3"));
    assert_eq!(action.country_operand_tag.as_deref(), Some("country/5"));
    let missing = &captured.actions[1];
    assert_eq!(missing.country_operand, Some(1006));
    assert_eq!(
        (
            missing.country,
            missing.taker,
            missing.giver,
            missing.negotiator
        ),
        (1006, 1006, 1006, 1006)
    );
    assert!(missing.country_tag.is_none());
    assert!(missing.taker_tag.is_none());
    assert!(missing.giver_tag.is_none());
    assert!(missing.negotiator_tag.is_none());
    assert!(missing.country_operand_tag.is_none());
}

#[test]
fn busy_drain_keeps_event_hour_tags_without_redating_victory_points() {
    let event = frame(101, 5, true, &[]);
    let mut busy = frame(102, 5, true, &[]);
    *busy.tables[TableId::PeaceSettlements.index()] = TableView::Unavailable {
        reason: replay::rows::PEACE_DRAIN_BUSY.into(),
    };
    let mut publication = frame(103, 5, true, &[]);
    for sample in [&mut busy, &mut publication] {
        let hour = sample.hour;
        let TableView::Complete(TableRows::Countries(countries)) =
            &mut *sample.tables[TableId::Countries.index()]
        else {
            panic!()
        };
        for country in countries {
            country.tag = format!("later-{hour}/{}", country.canonical_slot).into_bytes();
        }
    }
    let mut row = settled_row();
    row.actions[0].country = 1004;
    row.actions[0].country_operand = Some(1005);
    let result = scan(vec![event, busy, with_settlement(publication, row)]);
    let deal = result
        .deals
        .iter()
        .find(|deal| deal.captured.is_some())
        .unwrap();
    let captured = deal.captured.as_ref().unwrap();
    assert_eq!((captured.hour, captured.publication_hour), (101, 103));
    let action = &captured.actions[0];
    assert_eq!(action.country_tag.as_deref(), Some("country/4"));
    assert_eq!(action.taker_tag.as_deref(), Some("country/2"));
    assert_eq!(action.giver_tag.as_deref(), Some("country/1"));
    assert_eq!(action.negotiator_tag.as_deref(), Some("country/3"));
    assert_eq!(action.country_operand_tag.as_deref(), Some("country/5"));
    assert_eq!(deal.winners, ["country/2"]);
    assert_eq!(deal.losers, ["country/1"]);
    assert_eq!(captured.transfers[0].from.as_deref(), Some("country/1"));
    assert_eq!(captured.transfers[0].to.as_deref(), Some("country/2"));
    assert_eq!(
        captured.subjects[0].subject_tag.as_deref(),
        Some("country/3")
    );
    let PeaceChange::StateTransfer {
        victory_points,
        victory_points_read_hour,
        ..
    } = &deal.items[0].change
    else {
        panic!("exact state transfer")
    };
    assert_eq!(victory_points.as_deref(), Some("5"));
    assert_eq!(*victory_points_read_hour, Some(103));
}

#[test]
fn exact_tags_keep_all_observed_hours_without_a_recent_mapping_cap() {
    let mut frames = vec![frame(101, 5, true, &[])];
    for hour in 102..=110 {
        let mut later = frame(hour, 5, true, &[]);
        let TableView::Complete(TableRows::Countries(countries)) =
            &mut *later.tables[TableId::Countries.index()]
        else {
            panic!()
        };
        for country in countries {
            country.tag = format!("later-{hour}/{}", country.canonical_slot).into_bytes();
        }
        frames.push(later);
    }
    let publication = frames.pop().unwrap();
    let mut row = settled_row();
    row.actions[0].country_operand = Some(1005);
    frames.push(with_settlement(publication, row));
    let result = scan(frames);
    let captured = result
        .deals
        .iter()
        .find_map(|deal| deal.captured.as_ref())
        .unwrap();
    assert_eq!((captured.hour, captured.publication_hour), (101, 110));
    assert_eq!(captured.actions[0].taker_tag.as_deref(), Some("country/2"));
    assert_eq!(
        captured.actions[0].country_operand_tag.as_deref(),
        Some("country/5")
    );
    assert_eq!(
        captured.subjects[0].subject_tag.as_deref(),
        Some("country/3")
    );
}

#[test]
fn compressed_tag_history_never_fills_a_missing_event_hour() {
    let before = frame(100, 5, false, &[]);
    let mut missing = frame(101, 5, true, &[]);
    *missing.tables[TableId::Countries.index()] = TableView::Unavailable {
        reason: "country mapping not recorded".into(),
    };
    let after = with_settlement(frame(102, 5, true, &[]), settled_row());
    let result = scan(vec![before, missing, after]);
    let captured = result
        .deals
        .iter()
        .find_map(|deal| deal.captured.as_ref())
        .unwrap();
    assert!(captured.actions[0].taker_tag.is_none());
    assert!(captured.actions[0].giver_tag.is_none());
    assert!(captured.subjects[0].subject_tag.is_none());
}

#[test]
fn conference_scan_has_no_card_cap_and_transport_refuses_oversized_replies() {
    let conferences: Vec<_> = (1..=100_001).collect();
    let reply = scan(vec![frame(100, 0, false, &conferences)]);
    assert_eq!(reply.deals.len(), conferences.len());
    let cancelled = AtomicBool::new(false);
    assert!(crate::transport::json_len(&reply, crate::transport::MAX_REPLY_BYTES).is_none());
    let error = crate::transport::bounded_json(&reply, &cancelled).unwrap_err();
    assert!(error.contains("transport byte budget"), "{error}");
}

#[test]
fn exact_action_tags_survive_missing_derived_inputs_without_using_publication_hour_tags() {
    for missing in [TableId::States, TableId::WarRelations] {
        let mut event_hour = frame(101, 5, true, &[]);
        *event_hour.tables[missing.index()] = TableView::Unavailable {
            reason: "derived input not recorded".into(),
        };
        let mut row = settled_row();
        row.actions[0].country = 1004;
        row.actions[0].country_operand = Some(1005);
        let mut publication = frame(102, 6, true, &[]);
        if let TableView::Complete(TableRows::Countries(countries)) =
            &mut *publication.tables[TableId::Countries.index()]
        {
            for country in countries {
                country.tag = format!("later/{}", country.canonical_slot).into_bytes();
            }
        }
        for frames in [
            vec![with_settlement(event_hour.clone(), row.clone())],
            vec![event_hour, with_settlement(publication, row)],
        ] {
            let result = scan(frames);
            let deal = result
                .deals
                .iter()
                .find(|deal| deal.captured.is_some())
                .unwrap();
            let captured = deal.captured.as_ref().unwrap();
            let action = &captured.actions[0];
            assert_eq!(
                action.country_tag.as_deref(),
                Some("country/4"),
                "{missing:?}"
            );
            assert_eq!(
                action.taker_tag.as_deref(),
                Some("country/2"),
                "{missing:?}"
            );
            assert_eq!(
                action.giver_tag.as_deref(),
                Some("country/1"),
                "{missing:?}"
            );
            assert_eq!(
                action.negotiator_tag.as_deref(),
                Some("country/3"),
                "{missing:?}"
            );
            assert_eq!(
                action.country_operand_tag.as_deref(),
                Some("country/5"),
                "{missing:?}"
            );
            assert_eq!(deal.winners, ["country/2"], "{missing:?}");
            assert_eq!(deal.losers, ["country/1"], "{missing:?}");
            assert_eq!(
                captured.transfers[0].from.as_deref(),
                Some("country/1"),
                "{missing:?}"
            );
            assert_eq!(
                captured.transfers[0].to.as_deref(),
                Some("country/2"),
                "{missing:?}"
            );
            assert_eq!(
                captured.subjects[0].subject_tag.as_deref(),
                Some("country/3"),
                "{missing:?}"
            );
            let PeaceChange::StateTransfer {
                victory_points,
                victory_points_read_hour,
                ..
            } = &deal.items[0].change
            else {
                panic!("exact state transfer")
            };
            assert_eq!(victory_points.as_deref(), Some("5"), "{missing:?}");
            assert_eq!(
                *victory_points_read_hour,
                Some(captured.publication_hour),
                "{missing:?}"
            );
        }
    }
}

#[test]
fn exact_victory_points_require_their_own_complete_province_read_and_do_not_carry_back() {
    for provinces in [
        TableView::Unavailable {
            reason: "province read failed".into(),
        },
        TableView::Truncated {
            rows: TableRows::Provinces(vec![ProvinceRow {
                id: 2,
                state: 1,
                victory_points: 7,
                ..Default::default()
            }]),
            missing: 1,
        },
        TableView::Complete(TableRows::Provinces(vec![])),
    ] {
        let mut publication = frame(102, 3, true, &[]);
        *publication.tables[TableId::Provinces.index()] = provinces;
        *publication.tables[TableId::WarRelations.index()] = TableView::Unavailable {
            reason: "war input not recorded".into(),
        };
        let result = scan(vec![
            frame(101, 3, true, &[]),
            with_settlement(publication, settled_row()),
        ]);
        let deal = result
            .deals
            .iter()
            .find(|deal| deal.captured.is_some())
            .unwrap();
        let PeaceChange::StateTransfer {
            victory_points,
            victory_points_read_hour,
            ..
        } = &deal.items[0].change
        else {
            panic!("exact state transfer")
        };
        assert_eq!(victory_points, &None);
        assert_eq!(*victory_points_read_hour, None);
        assert_eq!(deal.transfers.links[0].victory_points, None);
    }
}

#[test]
fn peace_action_localisation_changes_invalidate_frontend_content_caches() {
    assert!(
        crate::products::find(PeaceDeals::KEY)
            .unwrap()
            .content_dependent
    );
    let contract = crate::typescript_contract();
    let declaration = contract
        .lines()
        .find(|line| line.starts_with("export const CONTENT_PRODUCTS"))
        .unwrap();
    assert!(declaration.contains("\"peace-deals\""));
}

#[test]
fn failed_capture_without_a_derived_card_has_an_explicit_reason() {
    let mut row = settled_row();
    row.failure = Some(b"guarded fault".to_vec());
    let result = scan(vec![with_settlement(frame(101, 3, false, &[]), row)]);
    let deal = &result.deals[0];
    assert!(deal.items.is_empty());
    assert!(deal.captured.is_none());
    assert!(
        deal.capture_unavailable
            .as_deref()
            .unwrap()
            .contains("guarded fault")
    );
    assert!(serde_json::to_value(deal).unwrap()["captureUnavailable"].is_string());
}

#[test]
fn delayed_failed_and_omitted_manifests_keep_their_supplying_hour() {
    for failure in [true, false] {
        let mut row = settled_row();
        row.hour = Some(112);
        if failure {
            row.failure = Some(b"guarded fault".to_vec());
        } else {
            row.omitted = 1;
        }
        let result = scan(vec![
            frame(100, 3, false, &[]),
            with_settlement(frame(136, 3, false, &[]), row),
        ]);
        let deal = &result.deals[0];
        assert_eq!(deal.first_observed_hour, 112);
        assert_eq!(deal.capture_observed_hour, Some(136));
        assert_eq!(deal.capture_unavailable_hour, Some(136));
        assert!(deal.captured.is_none());
    }
    let mut row = settled_row();
    row.hour = Some(112);
    row.failure = Some(b"guarded fault".to_vec());
    let id = row.conference.unwrap();
    let result = scan(vec![
        frame(100, 3, false, &[id]),
        with_settlement(frame(136, 3, false, &[]), row),
    ]);
    let deal = &result.deals[0];
    assert_eq!(deal.first_observed_hour, 100);
    assert_eq!(deal.capture_observed_hour, None);
    assert_eq!(deal.capture_unavailable_hour, Some(136));
}

#[test]
fn exact_transfer_reuses_the_recorded_state_action_binding() {
    let mut after = with_settlement(frame(101, 3, true, &[]), settled_row());
    *after.tables[TableId::LexicalTokens.index()] =
        TableView::Complete(TableRows::LexicalTokens(vec![LexicalTokenRow {
            token: 1234,
            literal: b"take_states".to_vec(),
        }]));
    let result = scan(vec![after]);
    assert_eq!(
        result.deals[0].items[0].action.key.as_deref(),
        Some("take_states")
    );
}

#[test]
fn token_spelling_does_not_guess_an_action_naming_key() {
    let mut result = scan(vec![with_settlement(
        frame(101, 3, true, &[]),
        settled_row(),
    )]);
    name_actions(
        &mut result,
        &installed_content::localisation::Localisation::default(),
        "english",
    );
    let action = &result.deals[0].captured.as_ref().unwrap().actions[0];
    assert!(action.name.is_none());
    assert!(
        result
            .warnings
            .iter()
            .any(|s| s.contains("no proven native naming operand"))
    );
}

#[test]
fn cached_native_type_name_keeps_saved_override_and_recording_language() {
    let mut row = settled_row();
    row.actions[0].cached_name = Some(b"Saved custom type".to_vec());
    row.actions[0].name_key = None;
    let result = scan(vec![with_settlement(frame(101, 3, true, &[]), row)]);
    let mut same = result.clone();
    let mut localisation = installed_content::localisation::Localisation::default();
    localisation.language = "english".into();
    name_actions(&mut same, &localisation, "english");
    assert_eq!(
        same.deals[0].captured.as_ref().unwrap().actions[0]
            .name
            .as_deref(),
        Some("Saved custom type")
    );
    localisation.language = "synthetic_other_language".into();
    let mut other = result;
    name_actions(&mut other, &localisation, "english");
    assert!(
        other.deals[0].captured.as_ref().unwrap().actions[0]
            .name
            .is_none()
    );
    assert!(
        other
            .warnings
            .iter()
            .any(|why| why.contains("stored in english"))
    );
}

#[test]
fn automatic_transfers_have_no_invented_giver_or_action() {
    let mut row = settled_row();
    row.actions.clear();
    row.transfers[0].automatic = true;
    let result = scan(vec![
        frame(100, 3, false, &[]),
        with_settlement(frame(101, 3, true, &[]), row),
    ]);
    let deal = result.deals.iter().find(|d| !d.derived).unwrap();
    let captured = deal.captured.as_ref().unwrap();
    assert!(captured.actions.is_empty());
    assert_eq!(captured.transfers.len(), 1);
    assert_eq!(captured.transfers[0].from, None);
    assert_eq!(captured.transfers[0].giver, None);
    assert_eq!(captured.transfers[0].to.as_deref(), Some("country/2"));
    assert_eq!(result.transfers.links[0].state_count, 1);
    let derived = result.deals.iter().find(|d| d.derived).unwrap();
    assert!(derived.items[0].derived);
    assert_eq!(derived.items[0].dates.before_hour, 100);
    assert_eq!(derived.items[0].dates.after_hour, 101);
}

#[test]
fn sampled_automatic_transfer_keeps_the_same_cards_dated_fallback_item() {
    let mut row = settled_row();
    row.actions.clear();
    row.transfers[0].automatic = true;
    let result = scan(vec![
        frame(100, 3, false, &[9]),
        with_settlement(frame(101, 3, true, &[]), row),
    ]);
    let deal = result
        .deals
        .iter()
        .find(|d| d.conference_id.as_deref() == Some("9"))
        .unwrap();
    assert!(!deal.derived);
    assert!(result.derived);
    assert!(deal.items[0].derived);
    assert_eq!(deal.items[0].dates.before_hour, 100);
    assert_eq!(deal.items[0].dates.after_hour, 101);
    assert_eq!(result.transfers.links[0].state_count, 1);
}

#[test]
fn truncated_event_tables_preserve_intact_conferences_and_report_lost_events() {
    let mut after = with_settlement(frame(101, 3, true, &[]), settled_row());
    after.tables[TableId::PeaceSettlements.index()] = TableView::Truncated {
        rows: TableRows::PeaceSettlements(vec![settled_row()]),
        missing: 4,
    }
    .into();
    let result = scan(vec![frame(100, 3, false, &[9]), after]);
    assert!(!result.deals[0].derived);
    assert_eq!(
        result.unobserved.settlements,
        [HourSpan {
            first_hour: 101,
            last_hour: 101
        }]
    );
    assert!(
        result
            .warnings
            .iter()
            .any(|s| s.contains("4 lost conferences"))
    );
}

#[test]
fn exact_mod_action_names_follow_recorded_content_order() {
    let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../.cargo-target/w4-peace-fixtures")
        .join(format!("exact-mod-{}", std::process::id()));
    let base = root.join("base");
    let modded = root.join("mod");
    for (folder, text) in [(&base, "Base demand"), (&modded, "Mod frontier treaty")] {
        let path = folder.join("localisation/action_l_english.yml");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(
            path,
            format!("\u{feff}l_english:\n custom_frontier_demand:0 \"{text}\"\n"),
        )
        .unwrap();
    }
    let layers = installed_content::ResolvedLayers::from_folders(&[(&base, &[]), (&modded, &[])]).unwrap();
    let localisation = installed_content::localisation::read(&layers, "english").unwrap();
    let mut row = settled_row();
    row.actions[0].name_key = Some(b"custom_frontier_demand".to_vec());
    let mut result = scan(vec![with_settlement(frame(101, 3, true, &[]), row)]);
    name_actions(&mut result, &localisation, "english");
    let captured = result
        .deals
        .iter()
        .find_map(|d| d.captured.as_ref())
        .unwrap();
    assert_eq!(
        captured.actions[0].name.as_deref(),
        Some("Mod frontier treaty")
    );
    assert!(!captured.actions[0].derived);
}

#[test]
fn exact_projection_scales_to_design_and_stress_country_populations() {
    for count in [1500, 5000] {
        let mut row = settled_row();
        row.actions.clear();
        row.transfers.clear();
        row.winners.clear();
        row.losers.clear();
        row.subjects.clear();
        for (ordinal, state) in (1..count).step_by(2).enumerate() {
            row.winners.push(1001 + state);
            row.losers.push(1000 + state);
            row.actions.push(SettledPeaceAction {
                token: 991,
                country: 1001 + state,
                turn: 0,
                ordinal: ordinal as u32,
                taker: 1001 + state,
                giver: 1000 + state,
                negotiator: 1001 + state,
                state: Some(state as i32),
                flags: Some([0; 4]),
                ..Default::default()
            });
            row.transfers.push(SettledPeaceTransfer {
                state: state as i32,
                taker: 1001 + state,
                automatic: false,
            });
        }
        let result = scan(vec![
            frame(100, count, false, &[9]),
            with_settlement(frame(101, count, true, &[]), row),
        ]);
        assert_eq!(result.deals.len(), 1);
        let captured = result.deals[0].captured.as_ref().unwrap();
        assert_eq!(captured.actions.len(), count as usize / 2);
        assert_eq!(
            result
                .transfers
                .links
                .iter()
                .map(|l| l.state_count)
                .sum::<u32>(),
            count / 2
        );
    }
}

#[test]
fn recorded_conference_has_dated_derived_items_exact_vps_and_flow_nodes() {
    let result = scan(vec![
        frame(100, 2, false, &[1_u64 << 40]),
        frame(101, 2, true, &[]),
    ]);
    assert_eq!(result.deals.len(), 1);
    let deal = &result.deals[0];
    assert!(result.derived && deal.derived && deal.participants_derived);
    assert_eq!(deal.conference_id.as_deref(), Some("1099511627776"));
    assert_eq!(deal.end_hour, Some(100));
    assert!(!deal.end_is_by);
    assert_eq!(deal.winners, ["country/2"]);
    assert_eq!(deal.losers, ["country/1"]);
    assert!(deal.war_name.is_none());
    assert_eq!(deal.items.len(), 1);
    assert!(deal.items[0].derived);
    assert_eq!(
        deal.items[0].dates,
        PeaceDates {
            before_hour: 100,
            after_hour: 101
        }
    );
    assert_eq!(deal.transfers.links[0].state_count, 1);
    assert_eq!(deal.transfers.links[0].victory_points.as_deref(), Some("5"));
    assert_eq!(deal.transfers.links[0].source, "source:1:10001");
    assert_eq!(deal.transfers.links[0].target, "target:2:10002");
    assert_eq!(result.transfers, deal.transfers);
}

#[test]
fn unsampled_conferences_need_consecutive_changes_and_a_war_end_or_removal() {
    let result = scan(vec![frame(100, 2, false, &[]), frame(101, 2, true, &[])]);
    assert_eq!(
        result.deals[0].observation,
        PeaceObservation::ConferenceNotRecorded
    );
    assert!(result.deals[0].conference_id.is_none());
    assert!(
        scan(vec![frame(100, 2, false, &[]), frame(104, 2, true, &[])])
            .deals
            .is_empty()
    );
    let mut ongoing = frame(101, 2, true, &[]);
    ongoing.tables[TableId::WarRelations.index()] =
        frame(100, 2, false, &[]).tables[TableId::WarRelations.index()].clone();
    assert!(
        scan(vec![frame(100, 2, false, &[]), ongoing])
            .deals
            .is_empty()
    );
    let mut deleted = frame(101, 2, true, &[]);
    *deleted.tables[TableId::WarRelations.index()] =
        TableView::Complete(TableRows::WarRelations(vec![]));
    let result = scan(vec![frame(100, 2, false, &[]), deleted]);
    assert_eq!(result.deals.len(), 1);
    assert!(result.deals[0].end_is_by && result.deals[0].derived);
    assert_eq!(result.deals[0].end_hour, Some(101));
    assert_eq!(result.transfers.links[0].state_count, 1);
    let mut gap = frame(104, 2, true, &[]);
    *gap.tables[TableId::WarRelations.index()] =
        TableView::Complete(TableRows::WarRelations(vec![]));
    assert!(scan(vec![frame(100, 2, false, &[]), gap]).deals.is_empty());
}

#[test]
fn unresolved_or_replaced_wars_do_not_masquerade_as_a_removed_pair() {
    for (start, second) in [(80, 999_999), (101, 1002)] {
        let mut after = frame(101, 2, true, &[]);
        *after.tables[TableId::WarRelations.index()] =
            TableView::Complete(TableRows::WarRelations(vec![WarRelationRow {
                first_country: 1001,
                second_country: second,
                start_hour: start,
                ..Default::default()
            }]));
        assert!(
            scan(vec![frame(100, 2, false, &[]), after])
                .deals
                .is_empty()
        );
    }
    let mut after = frame(101, 2, true, &[]);
    *after.tables[TableId::WarRelations.index()] = TableView::Complete(TableRows::WarRelations(
        [(1001, 1002, 80), (1002, 1001, 99)]
            .into_iter()
            .map(
                |(first_country, second_country, start_hour)| WarRelationRow {
                    first_country,
                    second_country,
                    start_hour,
                    ..Default::default()
                },
            )
            .collect(),
    ));
    assert!(
        scan(vec![frame(100, 2, false, &[]), after])
            .deals
            .is_empty()
    );
}

#[test]
fn peace_action_labels_participate_in_the_content_cache_contract() {
    assert!(
        crate::products::find(PeaceDeals::KEY)
            .unwrap()
            .content_dependent
    );
}

#[test]
fn scheduled_roster_reads_without_timings_end_the_recorded_conference() {
    let hour = (100..)
        .find(|hour| replay::cadence::is_refresh(TableId::PeaceConferences, *hour))
        .unwrap();
    let mut after = frame(hour, 0, false, &[]);
    after.metrics = Metrics::default();
    let reply = scan(vec![frame(hour - 1, 0, false, &[9]), after]);
    assert_eq!(reply.deals.len(), 1);
    let deal = &reply.deals[0];
    assert_eq!(deal.conference_id.as_deref(), Some("9"));
    assert_eq!(deal.first_absent_hour, Some(hour));
    assert_eq!(deal.end_hour, Some(hour - 1));
    assert!(!deal.end_is_by);
    assert!(reply.unobserved.conferences.is_empty());
}

#[test]
fn scheduled_roster_carries_are_not_new_reads_and_do_not_lose_earlier_settlement() {
    let mut carried = frame(101, 2, true, &[9]);
    carried.metrics.table_ns[TableId::PeaceConferences.index()] = 0;
    let result = scan(vec![
        frame(100, 2, false, &[9]),
        carried,
        frame(102, 2, true, &[]),
    ]);
    assert_eq!(result.deals.len(), 1);
    let deal = &result.deals[0];
    assert_eq!(deal.last_observed_hour, 100);
    assert_eq!(deal.end_hour, Some(102));
    assert!(deal.end_is_by);
    assert_eq!(
        deal.items[0].dates,
        PeaceDates {
            before_hour: 100,
            after_hour: 102
        }
    );
    assert_eq!(
        result.unobserved.conferences,
        [HourSpan {
            first_hour: 101,
            last_hour: 101
        }]
    );
}

#[test]
fn finalized_resident_is_processed_once_before_its_removal() {
    let mut finalized = frame(101, 2, true, &[9]);
    let TableView::Complete(TableRows::PeaceConferences(rows)) =
        &mut *finalized.tables[TableId::PeaceConferences.index()]
    else {
        panic!()
    };
    rows[0].finalized = 1;
    let result = scan(vec![
        frame(100, 2, false, &[9]),
        finalized,
        frame(102, 2, true, &[]),
    ]);
    assert_eq!(result.deals.len(), 1);
    assert_eq!(result.deals[0].first_finalized_hour, Some(101));
    assert_eq!(result.deals[0].items[0].dates.after_hour, 101);
    assert_eq!(result.transfers.links[0].state_count, 1);
}

#[test]
fn gaps_and_missing_tables_remain_unknown_and_do_not_turn_vps_into_zero() {
    let mut after = frame(104, 2, true, &[]);
    *after.tables[TableId::Provinces.index()] = TableView::Unavailable {
        reason: "lost provinces".into(),
    };
    *after.tables[TableId::Subjects.index()] = TableView::Unavailable {
        reason: "lost subjects".into(),
    };
    let result = scan(vec![frame(100, 2, false, &[9]), after]);
    let deal = &result.deals[0];
    assert_eq!(deal.end_hour, Some(104));
    assert!(deal.end_is_by);
    assert_eq!(deal.transfers.links[0].victory_points, None);
    assert_eq!(result.transfers.links[0].victory_points, None);
    assert!(
        deal.limitations
            .iter()
            .any(|line| line.contains("Subjects"))
    );
    assert_eq!(
        result.unobserved.ownership,
        [HourSpan {
            first_hour: 101,
            last_hour: 103
        }]
    );
}

#[test]
fn a_missing_post_read_waits_for_the_first_complete_read() {
    let mut missing = frame(101, 2, true, &[]);
    *missing.tables[TableId::States.index()] = TableView::Unavailable {
        reason: "lost states".into(),
    };
    let result = scan(vec![
        frame(100, 2, false, &[9]),
        missing,
        frame(102, 2, true, &[]),
    ]);
    assert_eq!(
        result.deals[0].items[0].dates,
        PeaceDates {
            before_hour: 100,
            after_hour: 102
        }
    );
    assert_eq!(
        result.unobserved.ownership,
        [HourSpan {
            first_hour: 101,
            last_hour: 101
        }]
    );
}

#[test]
fn concurrent_conferences_keep_ownership_outcomes_unassigned_and_count_them_once() {
    let result = scan(vec![
        frame(100, 2, false, &[9, 10]),
        frame(101, 2, true, &[9, 10]),
        frame(102, 2, true, &[]),
    ]);
    assert_eq!(result.deals.len(), 3);
    assert!(
        result
            .deals
            .iter()
            .filter(|deal| deal.conference_id.is_some())
            .all(|deal| deal.items.is_empty())
    );
    assert_eq!(
        result
            .deals
            .iter()
            .find(|deal| deal.conference_id.is_none())
            .unwrap()
            .observation,
        PeaceObservation::ConferenceUnknown
    );
    assert_eq!(result.transfers.links[0].state_count, 1);
}

#[test]
fn simultaneous_removals_and_unknown_rosters_never_claim_a_conference_identity() {
    let result = scan(vec![
        frame(100, 2, false, &[9, 10]),
        frame(101, 2, true, &[]),
    ]);
    assert_eq!(result.deals.len(), 3);
    assert_eq!(
        result
            .deals
            .iter()
            .find(|deal| deal.conference_id.is_none())
            .unwrap()
            .observation,
        PeaceObservation::ConferenceUnknown
    );
    let mut missing = frame(100, 2, false, &[]);
    *missing.tables[TableId::PeaceConferences.index()] = TableView::Unavailable {
        reason: "missing roster".into(),
    };
    let result = scan(vec![missing, frame(101, 2, true, &[])]);
    assert_eq!(
        result.deals[0].observation,
        PeaceObservation::ConferenceUnknown
    );
}

#[test]
fn staggered_removals_cannot_assign_outcomes_to_overlapping_conferences() {
    let result = scan(vec![
        frame(100, 2, false, &[9, 10]),
        frame(101, 2, true, &[10]),
        frame(102, 2, true, &[]),
    ]);
    assert_eq!(result.deals.len(), 3);
    assert!(
        result
            .deals
            .iter()
            .filter(|deal| deal.conference_id.is_some())
            .all(|deal| deal.items.is_empty())
    );
    let outcome = result
        .deals
        .iter()
        .find(|deal| deal.conference_id.is_none())
        .unwrap();
    assert_eq!(outcome.observation, PeaceObservation::ConferenceUnknown);
    assert_eq!(result.transfers.links[0].state_count, 1);
}

#[test]
fn a_finalized_resident_does_not_hide_a_later_unassigned_outcome() {
    let mut finalized = frame(101, 4, true, &[9]);
    let TableView::Complete(TableRows::PeaceConferences(rows)) =
        &mut *finalized.tables[TableId::PeaceConferences.index()]
    else {
        panic!()
    };
    rows[0].finalized = 1;
    let mut later = finalized.clone();
    later.hour = 102;
    for sample in [&mut finalized, &mut later] {
        if sample.hour == 101 {
            let TableView::Complete(TableRows::States(states)) =
                &mut *sample.tables[TableId::States.index()]
            else {
                panic!()
            };
            states[2].owner = 1003;
        }
        let TableView::Complete(TableRows::WarRelations(wars)) =
            &mut *sample.tables[TableId::WarRelations.index()]
        else {
            panic!()
        };
        if sample.hour == 101 {
            wars[1].end_hour = 0;
        } else {
            wars[1].end_hour = 102;
        }
    }
    let result = scan(vec![frame(100, 4, false, &[9]), finalized, later]);
    assert_eq!(result.deals.len(), 2);
    assert_eq!(
        result
            .deals
            .iter()
            .find(|deal| deal.conference_id.is_some())
            .unwrap()
            .items
            .len(),
        1
    );
    assert_eq!(
        result
            .deals
            .iter()
            .find(|deal| deal.conference_id.is_none())
            .unwrap()
            .observation,
        PeaceObservation::ConferenceUnknown
    );
    assert_eq!(result.transfers.links.len(), 2);
}

#[test]
fn truncated_tables_and_missing_after_reads_do_not_prove_no_outcome() {
    let mut missing = frame(101, 2, true, &[]);
    *missing.tables[TableId::States.index()] = TableView::Truncated {
        rows: TableRows::States(vec![]),
        missing: 2,
    };
    let result = scan(vec![frame(100, 2, false, &[9]), missing]);
    assert!(result.deals[0].items.is_empty());
    assert!(
        result.deals[0]
            .limitations
            .iter()
            .any(|line| line.contains("after settlement"))
    );
    assert_eq!(
        result.unobserved.ownership,
        [HourSpan {
            first_hour: 101,
            last_hour: 101
        }]
    );
}

#[test]
fn unresolved_owners_withhold_their_state_without_erasing_other_transfers() {
    let mut before = frame(100, 3, false, &[9]);
    let mut after = frame(101, 3, true, &[]);
    for sample in [&mut before, &mut after] {
        let TableView::Complete(TableRows::States(states)) =
            &mut *sample.tables[TableId::States.index()]
        else {
            panic!()
        };
        states[2].owner = 999_999;
    }
    let result = scan(vec![before, after]);
    assert_eq!(result.transfers.links[0].state_count, 1);
    assert_eq!(
        result.unobserved.ownership,
        [HourSpan {
            first_hour: 100,
            last_hour: 101
        }]
    );
    assert!(
        result.deals[0]
            .limitations
            .iter()
            .any(|line| line.contains("999999"))
    );
}

#[test]
fn new_subject_released_country_and_government_changes_are_neutral_dated_evidence() {
    let mut before = frame(100, 3, false, &[9]);
    let TableView::Complete(TableRows::States(states)) =
        &mut *before.tables[TableId::States.index()]
    else {
        panic!()
    };
    states[2].owner = 1001;
    let TableView::Complete(TableRows::CountryCounterInputs(counters)) =
        &mut *before.tables[TableId::CountryCounterInputs.index()]
    else {
        panic!()
    };
    counters[2].owned_state_count = 0;
    let mut after = frame(101, 3, true, &[]);
    *after.tables[TableId::Subjects.index()] =
        TableView::Complete(TableRows::Subjects(vec![SubjectRow {
            overlord: 1002,
            subject: 1003,
            start_hour: 101,
            state: b"mod_subject_level".to_vec(),
            ..Default::default()
        }]));
    let TableView::Complete(TableRows::CountryAppearance(appearances)) =
        &mut *after.tables[TableId::CountryAppearance.index()]
    else {
        panic!()
    };
    appearances[0].leader_ideology = b"another_mod_ideology".to_vec();
    let result = scan(vec![before, after]);
    let items = &result.deals[0].items;
    assert_eq!(items.len(), 4);
    assert!(
        items
            .iter()
            .all(|item| item.derived && item.dates.after_hour == 101 && item.action.name.is_none())
    );
    assert!(items.iter().any(|item| matches!(&item.change, PeaceChange::CountryReleased { country, states, .. } if country == "country/3" && states == &[3])));
    assert!(items.iter().any(|item| matches!(&item.change, PeaceChange::NewSubject { subject, .. } if subject == "country/3")));
    assert!(items.iter().any(|item| matches!(&item.change, PeaceChange::GovernmentChange { country, .. } if country == "country/1") && item.winner.is_none()));
}

/// Before the settlement country/1 owns state 3 and country/3 does not exist;
/// the shared settled frame then hands state 3 to country/3.
fn release_before(roster: &[u64]) -> DecodedFrame {
    let mut before = frame(100, 3, false, roster);
    let TableView::Complete(TableRows::States(states)) =
        &mut *before.tables[TableId::States.index()]
    else {
        panic!()
    };
    states[2].owner = 1001;
    let TableView::Complete(TableRows::CountryCounterInputs(counters)) =
        &mut *before.tables[TableId::CountryCounterInputs.index()]
    else {
        panic!()
    };
    counters[2].owned_state_count = 0;
    before
}

fn occurrences(graph: &PeaceTransferGraph, state: u32) -> Vec<&PeaceTransferredState> {
    graph
        .links
        .iter()
        .flat_map(|link| &link.states)
        .filter(|occurrence| occurrence.state == state)
        .collect()
}

#[test]
fn released_country_states_join_both_graphs_with_their_former_owner() {
    for roster in [vec![9], vec![]] {
        let result = scan(vec![release_before(&roster), frame(101, 3, true, &[])]);
        let is_release =
            |item: &&PeaceItem| matches!(item.change, PeaceChange::CountryReleased { .. });
        let deal = result
            .deals
            .iter()
            .find(|deal| deal.items.iter().any(|item| is_release(&item)))
            .unwrap();
        let PeaceChange::CountryReleased {
            to_country,
            transfers,
            states,
            ..
        } = &deal.items.iter().find(is_release).unwrap().change
        else {
            panic!()
        };
        assert_eq!(states, &[3]);
        assert_eq!(
            (
                to_country.slot,
                to_country.country_id,
                to_country.tag.as_str(),
                to_country.hour
            ),
            (3, 10_003, "country/3", 101)
        );
        assert_eq!(transfers.len(), 1);
        assert_eq!(transfers[0].from_country.tag, "country/1");
        for graph in [&deal.transfers, &result.transfers] {
            let released = occurrences(graph, 3);
            assert_eq!(released.len(), 1);
            let released = released[0];
            assert!(released.released);
            let from = &released.from_country;
            assert_eq!(
                (from.slot, from.country_id, from.tag.as_str(), from.hour),
                (1, 10_001, "country/1", 100)
            );
            assert_eq!(released.to_country, *to_country);
            assert_eq!(
                (released.dates.before_hour, released.dates.after_hour),
                (100, 101)
            );
            assert_eq!(released.victory_points.as_deref(), Some("5"));
            assert_eq!(released.victory_points_read_hour, Some(101));
            let plain = occurrences(graph, 1);
            assert_eq!(plain.len(), 1);
            assert!(!plain[0].released);
        }
        let link = result
            .transfers
            .links
            .iter()
            .find(|link| link.target == "target:3:10003")
            .unwrap();
        assert_eq!(link.source, "source:1:10001");
        assert_eq!(link.victory_points.as_deref(), Some("5"));
    }
}

#[test]
fn a_finalizer_outcome_without_a_giver_keeps_the_released_states_former_owner() {
    for roster in [vec![9], vec![]] {
        let mut row = settled_row();
        row.transfers.push(SettledPeaceTransfer {
            state: 3,
            taker: 1003,
            automatic: true,
        });
        let result = scan(vec![
            release_before(&roster),
            with_settlement(frame(101, 3, true, &[]), row),
        ]);
        let released = occurrences(&result.transfers, 3);
        assert_eq!(released.len(), 1);
        assert!(released[0].released);
        assert_eq!(released[0].from_country.tag, "country/1");
        assert_eq!(
            (released[0].dates.before_hour, released[0].dates.after_hour),
            (100, 101)
        );
        assert!(!occurrences(&result.transfers, 1)[0].released);
        // A sampled conference owns its derived release; otherwise the war-end card does.
        let exact = result
            .deals
            .iter()
            .find(|deal| deal.captured.is_some())
            .unwrap();
        assert_eq!(
            occurrences(&exact.transfers, 3).len(),
            usize::from(roster.len() == 1)
        );
        assert!(
            exact
                .items
                .iter()
                .all(|item| !matches!(item.change, PeaceChange::CountryReleased { .. }))
        );
    }
}

#[test]
fn an_exact_transfer_to_a_released_country_is_marked_and_counted_once() {
    for roster in [vec![9], vec![]] {
        let mut row = settled_row();
        row.actions.push(SettledPeaceAction {
            token: 991,
            country: 1002,
            turn: 1,
            ordinal: 3,
            taker: 1003,
            giver: 1001,
            negotiator: 1002,
            state: Some(3),
            flags: Some([0; 4]),
            ..Default::default()
        });
        row.transfers.push(SettledPeaceTransfer {
            state: 3,
            taker: 1003,
            automatic: false,
        });
        let result = scan(vec![
            release_before(&roster),
            with_settlement(frame(101, 3, true, &[]), row),
        ]);
        let released = occurrences(&result.transfers, 3);
        assert_eq!(released.len(), 1);
        assert!(released[0].released);
        assert_eq!(released[0].to_country.tag, "country/3");
        assert_eq!(
            (released[0].dates.before_hour, released[0].dates.after_hour),
            (101, 101)
        );
        assert!(!occurrences(&result.transfers, 1)[0].released);
        let exact = result
            .deals
            .iter()
            .find(|deal| deal.captured.is_some())
            .unwrap();
        assert!(occurrences(&exact.transfers, 3)[0].released);
    }
}

#[test]
fn cancelled_subjects_do_not_become_derived_peace_outcomes() {
    for cancel in [0, 1] {
        let before = frame(100, 2, false, &[]);
        let mut after = frame(101, 2, true, &[]);
        *after.tables[TableId::Subjects.index()] =
            TableView::Complete(TableRows::Subjects(vec![SubjectRow {
                overlord: 1002,
                subject: 1001,
                start_hour: 101,
                cancel,
                ..Default::default()
            }]));
        let result = scan(vec![before, after]);
        assert_eq!(
            result.deals[0]
                .items
                .iter()
                .filter(|item| matches!(item.change, PeaceChange::NewSubject { .. }))
                .count(),
            usize::from(cancel == 0)
        );
    }
}

#[test]
fn replaced_native_countries_do_not_inherit_derived_war_end_evidence() {
    for (slot, id) in [(1, 20_001), (3, 10_001)] {
        let before = frame(100, 2, false, &[]);
        let mut after = frame(101, 2, true, &[]);
        let TableView::Complete(TableRows::Countries(countries)) =
            &mut *after.tables[TableId::Countries.index()]
        else {
            panic!()
        };
        countries[0].canonical_slot = slot;
        countries[0].id = id;
        countries.sort_by_key(|row| row.id);
        *after.tables[TableId::CountryLinks.index()] =
            TableView::Complete(TableRows::CountryLinks(vec![
                CountryLinkRow {
                    id: 1001,
                    canonical_slot: slot,
                },
                CountryLinkRow {
                    id,
                    canonical_slot: slot,
                },
                CountryLinkRow {
                    id: 1002,
                    canonical_slot: 2,
                },
                CountryLinkRow {
                    id: 10002,
                    canonical_slot: 2,
                },
            ]));
        let TableView::Complete(TableRows::CountryLinks(links)) =
            &mut *after.tables[TableId::CountryLinks.index()]
        else {
            panic!()
        };
        links.sort_by_key(|row| row.id);
        assert!(scan(vec![before, after]).deals.is_empty());
    }
}

#[test]
fn replacement_holders_are_not_derived_as_released_countries() {
    let mut before = frame(100, 3, false, &[9]);
    let TableView::Complete(TableRows::States(states)) =
        &mut *before.tables[TableId::States.index()]
    else {
        panic!()
    };
    states[2].owner = 1001;
    let TableView::Complete(TableRows::CountryCounterInputs(counters)) =
        &mut *before.tables[TableId::CountryCounterInputs.index()]
    else {
        panic!()
    };
    counters[2].owned_state_count = 0;
    let mut after = frame(101, 3, true, &[]);
    let TableView::Complete(TableRows::Countries(countries)) =
        &mut *after.tables[TableId::Countries.index()]
    else {
        panic!()
    };
    countries[2].id = 20_003;
    let result = scan(vec![before, after]);
    assert!(
        result.deals[0]
            .items
            .iter()
            .any(|item| { matches!(item.change, PeaceChange::StateTransfer { .. }) })
    );
    assert!(
        result.deals[0]
            .items
            .iter()
            .all(|item| { !matches!(item.change, PeaceChange::CountryReleased { .. }) })
    );
}

#[test]
fn events_identity_changes_and_active_conferences_are_not_invented_deals() {
    let mut before = frame(100, 2, false, &[]);
    *before.tables[TableId::WarRelations.index()] =
        TableView::Complete(TableRows::WarRelations(vec![]));
    let mut event = frame(101, 2, true, &[]);
    *event.tables[TableId::WarRelations.index()] =
        TableView::Complete(TableRows::WarRelations(vec![]));
    assert!(scan(vec![before, event]).deals.is_empty());
    let mut renamed = frame(101, 2, true, &[]);
    *renamed.tables[TableId::WarRelations.index()] =
        TableView::Complete(TableRows::WarRelations(vec![]));
    let TableView::Complete(TableRows::Countries(countries)) =
        &mut *renamed.tables[TableId::Countries.index()]
    else {
        panic!()
    };
    countries[0].tag = b"opaque changed tag".to_vec();
    assert!(
        scan(vec![frame(100, 2, false, &[]), renamed])
            .deals
            .is_empty()
    );
    let active = scan(vec![frame(100, 2, false, &[9])]);
    assert_eq!(active.deals[0].end_hour, None);
    assert!(active.deals[0].items.is_empty());
}

#[test]
fn synthetic_replays_at_1500_and_5000_countries_have_no_country_cross_product() {
    for count in [1_500, 5_000] {
        let result = scan(vec![
            frame(100, count, false, &[]),
            frame(101, count, true, &[]),
        ]);
        assert_eq!(result.deals.len(), count as usize / 2);
        assert_eq!(result.transfers.links.len(), count as usize / 2);
        assert_eq!(result.transfers.nodes.len(), count as usize);
        assert!(
            result
                .transfers
                .links
                .iter()
                .all(|link| link.state_count == 1 && link.victory_points.as_deref() == Some("5"))
        );
    }
}

#[test]
fn campaign_graph_counts_dated_occurrences_and_cards_sort_newest_first() {
    let mut second_before = frame(102, 2, true, &[12]);
    let mut second_after = frame(103, 2, false, &[]);
    for (sample, end) in [(&mut second_before, 0), (&mut second_after, 103)] {
        let TableView::Complete(TableRows::WarRelations(wars)) =
            &mut *sample.tables[TableId::WarRelations.index()]
        else {
            panic!()
        };
        wars[0].start_hour = 102;
        wars[0].end_hour = end;
    }
    let result = scan(vec![
        frame(100, 2, false, &[9]),
        frame(101, 2, true, &[]),
        second_before,
        second_after,
    ]);
    assert_eq!(result.deals[0].conference_id.as_deref(), Some("12"));
    assert_eq!(result.deals[1].conference_id.as_deref(), Some("9"));
    assert_eq!(
        result
            .transfers
            .links
            .iter()
            .map(|link| link.state_count)
            .sum::<u32>(),
        2
    );
    assert!(
        result
            .transfers
            .links
            .iter()
            .all(|link| link.states[0].state == 1)
    );
    assert_eq!(result.transfers.nodes.len(), 4);
}

#[test]
fn branch_validation_cancellation_and_the_per_source_cache_are_explicit() {
    let data = Arc::new(replay(vec![
        frame(100, 2, false, &[]),
        frame(101, 2, true, &[]),
    ]));
    assert!(scan_branch(&data, 2, &AtomicBool::new(false)).is_err());
    assert_eq!(
        scan_branch(&data, 1, &AtomicBool::new(true))
            .unwrap_err()
            .kind(),
        io::ErrorKind::Interrupted
    );
    let source = Source::from_replay(
        crate::SourceKey {
            source_id: "fixture".into(),
            revision: "1".into(),
        },
        "fixture.battleplan".into(),
        data,
        &AtomicBool::new(false),
    )
    .unwrap();
    let params = PeaceDealsParams {
        branch_id: "1".into(),
    };
    let context = LoadContext { content: None };
    let first = PeaceDeals::load(&source, &params, &context, &AtomicBool::new(false)).unwrap();
    let second = PeaceDeals::load(&source, &params, &context, &AtomicBool::new(false)).unwrap();
    assert_eq!(first.transfers, second.transfers);
    assert_eq!(source.state::<PeaceCache>().lock().unwrap().0.len(), 1);
}

#[test]
fn mod_localisation_renames_the_discovered_state_action_and_refuses_unsupported_text() {
    // Fixture files stay inside this worktree's ignored build directory.
    let scratch = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../.cargo-target")
        .canonicalize()
        .unwrap();
    let root = scratch.join(format!(
        "peace-action-fixture-{}-{}",
        std::process::id(),
        uuid::Uuid::new_v4()
    ));
    let vanilla = root.join("vanilla");
    let modded = root.join("mod");
    for (folder, relative, text) in [
        (
            &vanilla,
            "localisation/base_l_english.yml",
            "\u{feff}l_english:\n take_states:0 \"Take state\"\n",
        ),
        (
            &modded,
            "localisation/replace/rename_l_english.yml",
            "\u{feff}l_english:\n take_states:0 \"Claim territory by treaty\"\n",
        ),
    ] {
        let file = folder.join(relative);
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        std::fs::write(file, text).unwrap();
    }
    let layers =
        installed_content::ResolvedLayers::from_folders(&[(&vanilla, &[]), (&modded, &[])]).unwrap();
    let localisation = installed_content::localisation::read(&layers, "english").unwrap();
    let mut result = scan(vec![frame(100, 2, false, &[9]), frame(101, 2, true, &[])]);
    name_actions(&mut result, &localisation, "english");
    assert_eq!(
        result.deals[0].items[0].action.key.as_deref(),
        Some("take_states")
    );
    assert_eq!(
        result.deals[0].items[0].action.name.as_deref(),
        Some("Claim territory by treaty")
    );

    let file = modded.join("localisation/replace/rename_l_english.yml");
    std::fs::write(
        &file,
        "\u{feff}l_english:\n take_states:0 \"[ROOT.GetName]\"\n",
    )
    .unwrap();
    let layers =
        installed_content::ResolvedLayers::from_folders(&[(&vanilla, &[]), (&modded, &[])]).unwrap();
    let localisation = installed_content::localisation::read(&layers, "english").unwrap();
    name_actions(&mut result, &localisation, "english");
    assert!(result.deals[0].items[0].action.name.is_none());
    assert!(
        result
            .warnings
            .iter()
            .any(|line| line.contains("take_states") && line.contains("ROOT.GetName"))
    );
    assert!(root.canonicalize().unwrap().starts_with(&scratch));
    std::fs::remove_dir_all(&root).unwrap();
}

#[test]
fn missing_conflicting_or_unsupported_action_names_stay_neutral() {
    let mut missing = frame(101, 2, true, &[]);
    *missing.tables[TableId::LexicalTokens.index()] = TableView::Unavailable {
        reason: "dictionary lost".into(),
    };
    let result = scan(vec![frame(100, 2, false, &[9]), missing]);
    assert!(result.deals[0].items[0].action.key.is_none());
    assert!(
        result
            .warnings
            .iter()
            .any(|line| line.contains("LexicalTokens"))
    );
    let mut duplicate = frame(101, 2, true, &[]);
    let TableView::Complete(TableRows::LexicalTokens(rows)) =
        &mut *duplicate.tables[TableId::LexicalTokens.index()]
    else {
        panic!()
    };
    rows.push(LexicalTokenRow {
        token: 78,
        literal: b"take_states".to_vec(),
    });
    let result = scan(vec![frame(100, 2, false, &[9]), duplicate]);
    assert!(result.deals[0].items[0].action.key.is_none());
    assert!(
        result
            .warnings
            .iter()
            .any(|line| line.contains("conflicting"))
    );
    let mut result = scan(vec![frame(100, 2, false, &[9]), frame(101, 2, true, &[])]);
    name_actions(
        &mut result,
        &installed_content::localisation::Localisation::default(),
        "english",
    );
    assert!(result.deals[0].items[0].action.name.is_none());
    assert!(
        result.warnings.iter().any(
            |line| line.contains("take_states") && line.contains("not in the recorded content")
        )
    );
}

#[test]
fn earlier_existence_is_kept_even_when_war_inputs_are_missing() {
    let mut early = frame(99, 3, false, &[]);
    *early.tables[TableId::WarRelations.index()] = TableView::Unavailable {
        reason: "not read".into(),
    };
    let mut before = frame(100, 3, false, &[9]);
    let TableView::Complete(TableRows::States(states)) =
        &mut *before.tables[TableId::States.index()]
    else {
        panic!()
    };
    states[2].owner = 1001;
    let TableView::Complete(TableRows::CountryCounterInputs(counters)) =
        &mut *before.tables[TableId::CountryCounterInputs.index()]
    else {
        panic!()
    };
    counters[2].owned_state_count = 0;
    let result = scan(vec![early, before, frame(101, 3, true, &[])]);
    assert!(
        result.deals[0]
            .items
            .iter()
            .all(|item| !matches!(item.change, PeaceChange::CountryReleased { .. }))
    );
}

#[test]
fn recording_past_former_card_cap_keeps_every_card_before_budget_refusal() {
    let conferences: Vec<_> = (1..=100_001).collect();
    let reply = scan(vec![frame(100, 0, false, &conferences)]);
    assert_eq!(reply.deals.len(), conferences.len());
    assert_eq!(
        reply
            .deals
            .iter()
            .filter(|deal| deal.conference_id.is_some())
            .count(),
        conferences.len()
    );
    let error = admitted(reply).unwrap_err();
    assert!(error.to_string().contains("transport budget"));
    assert!(error.to_string().contains("no entries were dropped"));
}

#[test]
fn complete_reply_uses_shared_transport_budget_without_shortening_payloads() {
    let mut row = settled_row();
    row.actions = vec![row.actions[0].clone(); 50_000];
    let reply = scan(vec![with_settlement(frame(101, 3, false, &[]), row)]);
    let source = Source::from_replay(
        crate::SourceKey {
            source_id: "fixture".into(),
            revision: "1".into(),
        },
        "fixture.battleplan".into(),
        Arc::new(replay(vec![frame(101, 3, false, &[])])),
        &AtomicBool::new(false),
    )
    .unwrap();
    let cache = source.state::<PeaceCache>();
    cache.lock().unwrap().0.insert(1, Arc::new(reply));
    let params = PeaceDealsParams {
        branch_id: "1".into(),
    };
    let context = LoadContext { content: None };
    let reply = PeaceDeals::load(&source, &params, &context, &AtomicBool::new(false)).unwrap();
    assert_eq!(
        reply.deals[0].captured.as_ref().unwrap().actions.len(),
        50_000
    );
    let mut oversized = reply.clone();
    oversized.warnings.push("x".repeat(MAX_PRODUCT_REPLY_BYTES));
    cache.lock().unwrap().0.insert(1, Arc::new(oversized));
    let error = PeaceDeals::load(&source, &params, &context, &AtomicBool::new(false)).unwrap_err();
    assert!(error.to_string().contains("transport budget"));
    assert_eq!(
        cache.lock().unwrap().0[&1].deals[0]
            .captured
            .as_ref()
            .unwrap()
            .actions
            .len(),
        50_000
    );
}

#[test]
fn transfer_graph_joins_renames_and_separates_tag_and_slot_replacements() {
    let country = |slot, country_id, tag: &str, hour| PeaceTransferCountry {
        slot,
        country_id,
        tag: tag.into(),
        hour,
    };
    let item = |state, from: PeaceTransferCountry| PeaceItem {
        derived: true,
        dates: PeaceDates {
            before_hour: from.hour,
            after_hour: from.hour + 1,
        },
        winner: Some("T".into()),
        action: neutral(PeaceActionKind::StateTransfer),
        change: PeaceChange::StateTransfer {
            state,
            from: from.tag.clone(),
            to: "T".into(),
            to_country: country(90, 901, "T", from.hour + 1),
            from_country: from,
            victory_points: Some("1".into()),
            victory_points_read_hour: None,
        },
    };
    let items = [
        item(1, country(80, 800, "X", 10)),
        item(2, country(80, 800, "Y", 20)),
        item(3, country(81, 801, "X", 30)),
        item(4, country(80, 802, "Y", 40)),
    ];
    let mut campaign = GraphBuilder::default();
    let mut conferences = Vec::new();
    for item in &items {
        campaign.add(item);
        let mut graph = GraphBuilder::default();
        graph.add(item);
        conferences.push(graph.finish());
    }
    let graph = campaign.finish();
    assert_eq!(graph.links.len(), 3);
    let original = graph
        .links
        .iter()
        .find(|link| link.source == "source:80:800")
        .unwrap();
    assert_eq!(original.state_count, 2);
    assert_eq!(
        original
            .states
            .iter()
            .map(|state| state.from_country.tag.as_str())
            .collect::<Vec<_>>(),
        ["X", "Y"]
    );
    assert_eq!(conferences[0].links[0].id, conferences[1].links[0].id);
    assert_ne!(conferences[0].links[0].id, conferences[2].links[0].id);
    assert_ne!(conferences[1].links[0].id, conferences[3].links[0].id);
    assert_eq!(
        graph
            .nodes
            .iter()
            .find(|node| node.id == "source:80:800")
            .unwrap()
            .country
            .hour,
        20
    );
}

#[test]
fn exact_transfer_identity_comes_from_the_event_not_a_replacement_at_publication() {
    let event = frame(101, 3, true, &[]);
    let mut publication = frame(102, 3, true, &[]);
    let TableView::Complete(TableRows::Countries(countries)) =
        &mut *publication.tables[TableId::Countries.index()]
    else {
        panic!()
    };
    countries[0].id = 20_001;
    countries[0].tag = b"Y".to_vec();
    countries[2].tag = b"country/1".to_vec();
    countries.sort_by_key(|country| country.id);
    let result = scan(vec![event, with_settlement(publication, settled_row())]);
    let deal = result
        .deals
        .iter()
        .find(|deal| deal.captured.is_some())
        .unwrap();
    let captured = &deal.captured.as_ref().unwrap().transfers[0];
    let from = captured.from_country.as_ref().unwrap();
    assert_eq!((from.slot, from.country_id, from.hour), (1, 10_001, 101));
    assert_eq!(from.tag, "country/1");
    assert_eq!(deal.transfers.links[0].source, "source:1:10001");
    let unavailable = scan(vec![with_settlement(
        frame(102, 3, true, &[]),
        settled_row(),
    )]);
    let deal = unavailable
        .deals
        .iter()
        .find(|deal| deal.captured.is_some())
        .unwrap();
    assert!(
        deal.captured.as_ref().unwrap().transfers[0]
            .from_country
            .is_none()
    );
    assert!(deal.transfers.links.is_empty());
    assert!(deal.limitations.iter().any(|reason| {
        reason.contains("1 transfers with unrecorded event-hour endpoint identities")
    }));
}
