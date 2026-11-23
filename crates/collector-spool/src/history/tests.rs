use super::*;
use replay_capture_spool::spool::Record;
use replay_data::{identity::CaptureMode, identity::SampleBoundary, identity::Verification};
use std::io::Read as _;

#[test]
fn event_feed_history_seeds_sequence_generation_and_counts_from_retained_rows() {
    use replay_data::{
        event_feed_rows::EventFeedCountry, event_feed_rows::EventFeedCounts,
        event_feed_rows::EventFeedCoverage, event_feed_rows::EventFeedEvent,
        event_feed_rows::EventFeedHistoryRow, event_feed_rows::EventFeedSource, rows::CampaignRow,
        rows::TableId, rows::TableRows,
    };
    use replay_frame::{
        branches::BranchReason, branches::BranchStart, branches::Parentage, frame::DecodedFrame,
        frame::FrameEncoder, frame::Metrics, frame::TableUpdate, frame::TableView,
    };
    let id = b"01234567-89ab-cdef-0123-456789abcdef";
    let mut records = vec![Record::SessionStart(session())];
    for (branch, first, sequence, truncated) in [(1, 100, 99, true), (2, 50, 3, false)] {
        records.push(Record::BranchStart(BranchStart {
            id: branch,
            session: 1,
            loaded_hour: None,
            first_hour: first - 1,
            parent: Parentage::Unknown,
            reason: if branch == 1 {
                BranchReason::SessionStart
            } else {
                BranchReason::HourDiscontinuity
            },
        }));
        let mut encoder = FrameEncoder::new();
        for hour in [first - 1, first, first + 1] {
            let country = EventFeedCountry {
                reference: 5000,
                id: 4999,
                instance: 7,
            };
            let mut row = EventFeedHistoryRow {
                script_refusals: 0,
                script_tokens: Vec::new(),
                script_scopes: Vec::new(),
                generation: branch,
                read_hour: hour,
                coverage: EventFeedCoverage::Complete,
                counts: Some(EventFeedCounts {
                    acquired: 1,
                    settled: 1,
                    ..Default::default()
                }),
                outstanding: 0,
                failure: None,
                events: if hour == first {
                    vec![EventFeedEvent {
                        script: None,
                        sequence,
                        generation: branch,
                        hour,
                        source: EventFeedSource::Annex,
                        actor: country,
                        target: country,
                        cause: Default::default(),
                    }]
                } else {
                    Vec::new()
                },
            };
            if branch == 2 && hour > first {
                row.coverage = EventFeedCoverage::Unavailable;
                row.counts = None;
                row.failure = Some(b"fixture unknown accounting".to_vec());
            }
            let mut tables = std::array::from_fn(|_| TableView::Unavailable {
                reason: "fixture".into(),
            });
            tables[TableId::Campaign.index()] =
                TableView::Complete(TableRows::Campaign(vec![CampaignRow { id: id.to_vec() }]));
            let rows = TableRows::EventFeedHistory(vec![row]);
            tables[TableId::EventFeedHistory.index()] = if truncated && hour == first {
                TableView::Truncated { rows, missing: 1 }
            } else {
                TableView::Complete(rows)
            };
            let frame = encoder
                .encode(
                    DecodedFrame {
                        branch,
                        hour,
                        tables: tables.map(Into::into),
                        metrics: Metrics::default(),
                    },
                    hour == first - 1,
                )
                .unwrap();
            if truncated && hour == first {
                assert!(!frame.keyframe);
                assert!(matches!(
                    &frame.tables[TableId::EventFeedHistory.index()],
                    TableUpdate::Truncated { rows, missing: 1 } if rows.len() == 1
                ));
            }
            records.push(Record::Frame(frame));
        }
    }
    let replay = Replay::from_records(recorder_derive::writers(), id.to_vec(), &[records]).unwrap();
    let expected = (
        Some(99),
        Some(2),
        EventFeedCounts {
            acquired: 1,
            settled: 1,
            ..Default::default()
        },
    );
    assert_eq!(event_feed_history(&replay).unwrap(), expected);
    let reopened = Replay::open(replay.bytes().to_vec()).unwrap();
    assert_eq!(event_feed_history(&reopened).unwrap(), expected);
}

#[test]
fn an_older_layout_recording_is_left_as_it_is_and_the_campaign_records_anew() {
    let id = "01234567-89ab-cdef-0123-456789abcdef";
    let root =
        std::env::temp_dir().join(format!("hoi4_history_compatibility_{}", std::process::id()));
    fs::create_dir_all(&root).unwrap();
    let session = session();
    let replay =
        Replay::from_records(recorder_derive::writers(), id.into(), &[vec![Record::SessionStart(session.clone())]]).unwrap();
    let session = SessionStart { id: 2, ..session };
    let path = root.join(format!("{id}.battleplan"));
    let older = [b"H4RP".as_slice(), &94_u16.to_le_bytes(), id.as_bytes()].concat();
    fs::write(&path, &older).unwrap();
    let mut prepared = PreparedCampaigns::load(&root, &session, |_| Ok(())).unwrap();
    assert!(prepared.take(id, &session).unwrap().1.is_none());
    assert_eq!(fs::read(&path).unwrap(), older);
    drop(prepared);
    fs::remove_file(&path).unwrap();
    let mut prepared = PreparedCampaigns::load(&root, &session, |_| Ok(())).unwrap();
    assert!(prepared.take(id, &session).unwrap().1.is_none());
    fs::write(&path, replay.bytes()).unwrap();
    let mut prepared = PreparedCampaigns::load(&root, &session, |_| Ok(())).unwrap();
    assert!(prepared.take(id, &session).unwrap().1.is_some());
    drop(prepared);
    let header = &replay.bytes()[..14 + id.len()];
    fs::write(&path, [header, b"not a recording"].concat()).unwrap();
    let mut prepared = PreparedCampaigns::load(&root, &session, |_| Ok(())).unwrap();
    prepared.without_history(id);
    let (tracker, lease) = prepared.take(id, &session).unwrap();
    assert!(lease.is_none() && tracker.last_observation().is_none());
    drop(prepared);
    fs::remove_dir_all(&root).unwrap();
}

#[test]
fn a_campaigns_recording_is_found_by_its_header_whatever_its_name() {
    let id = "01234567-89ab-cdef-0123-456789abcdef";
    let root = std::env::temp_dir().join(format!("hoi4_history_named_{}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root).unwrap();
    let session = session();
    let replay =
        Replay::from_records(recorder_derive::writers(), id.into(), &[vec![Record::SessionStart(session.clone())]]).unwrap();
    let session = SessionStart { id: 2, ..session };
    let mut prepared = PreparedCampaigns::load(&root, &session, |_| Ok(())).unwrap();
    fs::write(root.join("Germany 1936.battleplan"), replay.bytes()).unwrap();
    let error = prepared.take(id, &session).err().unwrap().to_string();
    assert!(
        error.contains("appeared after history preparation"),
        "{error}"
    );
    let mut prepared = PreparedCampaigns::load(&root, &session, |_| Ok(())).unwrap();
    let (_, lease) = prepared.take(id, &session).unwrap();
    assert!(lease.is_some());
    drop((lease, prepared));
    fs::write(root.join("Germany 1936 - Copy.battleplan"), replay.bytes()).unwrap();
    let mut prepared = PreparedCampaigns::load(&root, &session, |_| Ok(())).unwrap();
    let error = prepared.take(id, &session).err().unwrap().to_string();
    assert!(error.contains("more than one recording"), "{error}");
    drop(prepared);
    fs::remove_file(root.join("Germany 1936 - Copy.battleplan")).unwrap();
    fs::write(root.join("Damaged.battleplan"), b"junk").unwrap();
    assert!(PreparedCampaigns::load(&root, &session, |_| Ok(())).is_err());
    fs::remove_dir_all(&root).unwrap();
}

#[test]
fn only_a_campaigns_own_unambiguous_unsaved_recording_stands_in_for_its_saved_one() {
    let id = "01234567-89ab-cdef-0123-456789abcdef";
    let other = "11234567-89ab-cdef-0123-456789abcdef";
    let root = std::env::temp_dir().join(format!("hoi4_history_unsaved_{}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root).unwrap();
    let session = session();
    let recording = |campaign: &str| {
        Replay::from_records(
            recorder_derive::writers(),
            campaign.into(),
            &[vec![Record::SessionStart(session.clone())]],
        )
        .unwrap()
    };
    let replay = recording(id);
    let session = SessionStart {
        id: 2,
        ..session.clone()
    };
    fs::write(root.join("Germany 1936.battleplan"), replay.bytes()).unwrap();
    let unsaved = replay_settings::publication::unsaved_path(&root, id);
    fs::create_dir_all(unsaved.parent().unwrap()).unwrap();
    fs::write(&unsaved, replay.bytes()).unwrap();
    let mut prepared = PreparedCampaigns::load(&root, &session, |_| Ok(())).unwrap();
    assert_eq!(prepared.history_path(id).unwrap(), Some(unsaved.clone()));
    assert_eq!(
        recording_files::unsaved_recording(&root, id.as_bytes()).unwrap(),
        Some(unsaved.clone())
    );
    let (_, lease) = prepared.take(id, &session).unwrap();
    assert!(lease.is_some());
    drop((lease, prepared));
    let copy = unsaved.with_file_name("Copy.battleplan");
    fs::write(&copy, replay.bytes()).unwrap();
    let refused = |campaign: &str| {
        let mut prepared = PreparedCampaigns::load(&root, &session, |_| Ok(())).unwrap();
        let error = prepared.take(campaign, &session).err().unwrap().to_string();
        assert!(error.contains("is ambiguous"), "{error}");
        assert!(prepared.history_path(campaign).is_err());
        assert!(recording_files::unsaved_recording(&root, campaign.as_bytes()).is_err());
    };
    refused(id);
    fs::remove_file(&unsaved).unwrap();
    refused(id);
    fs::remove_file(&copy).unwrap();
    fs::write(&unsaved, recording(other).bytes()).unwrap();
    refused(id);
    refused(other);
    fs::remove_file(&unsaved).unwrap();
    let fresh = replay_settings::publication::unsaved_path(&root, other);
    let bytes = recording(other).bytes().to_vec();
    fs::write(
        &fresh,
        &bytes[..bytes.len() - replay_columnar::envelope::FOOTER_BYTES],
    )
    .unwrap();
    let mut prepared = PreparedCampaigns::load(&root, &session, |_| Ok(())).unwrap();
    assert!(prepared.take(id, &session).is_ok());
    let error = prepared.take(other, &session).err().unwrap().to_string();
    assert!(error.contains(&fresh.display().to_string()), "{error}");
    fs::remove_dir_all(&root).unwrap();
}

fn session() -> SessionStart {
    SessionStart {
        id: 1,
        process_id: 1,
        process_start_time: 1,
        qpc_frequency: 1,
        sample_boundary: SampleBoundary::EndOfHour,
        capture_mode: CaptureMode::Full,
        identity: Identity {
            game_version: "synthetic".into(),
            exe_hash: [1; 32],
            exe_size: 1,
            vanilla_hash: [2; 32],
            dlc: Vec::new(),
            mods: Vec::new(),
            coverage_policy: replay_data::identity::COVERAGE_POLICY_VERSION,
            language: "l_english".into(),
            recorder_version: "synthetic".into(),
            collector_hash: [3; 32],
            verification: Verification::Unchecked,
        },
    }
}

#[test]
fn completion_sequence_seed_keeps_finalized_keys_across_reattach_and_branches() {
    use replay_data::{
        rows::CampaignRow, rows::OperationCompletionRow, rows::TableId, rows::TableRows,
    };
    use replay_frame::{
        branches::BranchReason, branches::BranchStart, branches::Parentage, frame::DecodedFrame,
        frame::FrameEncoder, frame::Metrics, frame::TableView,
    };
    let id = b"01234567-89ab-cdef-0123-456789abcdef";
    let frame = |branch, hour, sequence| {
        let mut tables = std::array::from_fn(|_| TableView::Unavailable {
            reason: "fixture".into(),
        });
        tables[TableId::Campaign.index()] =
            TableView::Complete(TableRows::Campaign(vec![CampaignRow { id: id.to_vec() }]));
        tables[TableId::OperationCompletions.index()] =
            TableView::Complete(TableRows::OperationCompletions(vec![
                OperationCompletionRow {
                    owner: 5000,
                    sequence,
                    id: 7,
                    target: 4999,
                    definition_token: 42,
                    completion_hour: hour,
                    terminal: 1,
                    ..Default::default()
                },
            ]));
        DecodedFrame {
            branch,
            hour,
            tables: tables.map(Into::into),
            metrics: Metrics::default(),
        }
    };
    let branch = |id, first_hour| {
        Record::BranchStart(BranchStart {
            id,
            session: 1,
            loaded_hour: None,
            first_hour,
            parent: Parentage::Unknown,
            reason: if id == 1 {
                BranchReason::SessionStart
            } else {
                BranchReason::HourDiscontinuity
            },
        })
    };
    let mut first = FrameEncoder::new();
    let mut second = FrameEncoder::new();
    let replay = Replay::from_records(
        recorder_derive::writers(),
        id.to_vec(),
        &[
            vec![
                Record::SessionStart(session()),
                branch(1, 100),
                Record::Frame(first.encode(frame(1, 100, 99), false).unwrap()),
            ],
            vec![
                Record::SessionStart(session()),
                branch(2, 50),
                Record::Frame(second.encode(frame(2, 50, 3), false).unwrap()),
            ],
        ],
    )
    .unwrap();
    assert_eq!(completion_sequence(&replay).unwrap(), 99);
    let reopened = Replay::open(replay.bytes().to_vec()).unwrap();
    assert_eq!(completion_sequence(&reopened).unwrap(), 99);
}

#[test]
fn resumed_politics_epoch_includes_observed_rows_when_coverage_is_unavailable() {
    use replay_data::{
        rows::CampaignRow, rows::ClockRow, rows::CountryPowerChangeRow, rows::TableId,
        rows::TableRows,
    };
    use replay_frame::{
        branches::BranchReason, branches::BranchStart, branches::Parentage, frame::DecodedFrame,
        frame::FrameEncoder, frame::Metrics, frame::TableView,
    };
    let session = session();
    let mut tables = std::array::from_fn(|_| TableView::Unavailable {
        reason: "synthetic unavailable census".into(),
    });
    tables[TableId::Clock.index()] = TableView::Complete(TableRows::Clock(vec![ClockRow {
        hour: 1,
        ..Default::default()
    }]));
    tables[TableId::Campaign.index()] =
        TableView::Complete(TableRows::Campaign(vec![CampaignRow {
            id: b"politics-history".to_vec(),
        }]));
    tables[TableId::CountryPowerChanges.index()] =
        TableView::Complete(TableRows::CountryPowerChanges(vec![
            CountryPowerChangeRow {
                country: 5_000,
                instance: 7,
                sequence: 3,
                epoch: 99,
                hour: 1,
                ..Default::default()
            },
        ]));
    let frame = FrameEncoder::new()
        .encode(
            DecodedFrame {
                branch: 1,
                hour: 1,
                metrics: Metrics::default(),
                tables: tables.map(Into::into),
            },
            true,
        )
        .unwrap();
    let replay = Replay::from_records(
        recorder_derive::writers(),
        b"politics-history".to_vec(),
        &[vec![
            Record::SessionStart(session),
            Record::BranchStart(BranchStart {
                id: 1,
                session: 1,
                loaded_hour: Some(0),
                first_hour: 1,
                parent: Parentage::Unknown,
                reason: BranchReason::SessionStart,
            }),
            Record::Frame(frame),
        ]],
    )
    .unwrap();
    assert_eq!(politics_epoch(&replay).unwrap(), 99);
}

#[test]
fn naval_sequence_prepares_every_retained_branch_before_resuming_same_process() {
    use replay_data::{
        rows::CampaignRow, rows::ClockRow, rows::NavalResultSourceRow, rows::TableId,
        rows::TableRows,
    };
    use replay_frame::{
        branches::BranchReason, branches::BranchStart, branches::Parentage, frame::Frame,
        frame::Metrics, frame::TableUpdate,
    };
    let id = "01234567-89ab-cdef-0123-456789abcdef";
    let other_id = "11234567-89ab-cdef-0123-456789abcdef";
    let root = std::env::temp_dir().join(format!(
        "hoi4_history_naval_sequence_{}",
        std::process::id()
    ));
    fs::create_dir_all(&root).unwrap();
    let session = session();
    let replay = |campaign: &str, branches: &[(u64, i32, &[u64])]| {
        let mut records = vec![Record::SessionStart(session.clone())];
        for (branch, first, ids) in branches {
            records.push(Record::BranchStart(BranchStart {
                id: *branch,
                session: session.id,
                loaded_hour: Some(*first - 1),
                first_hour: *first,
                parent: Parentage::Unknown,
                reason: if *branch == 1 {
                    BranchReason::SessionStart
                } else {
                    BranchReason::LoadSignal
                },
            }));
            for hour in [*first, *first + 1] {
                records.push(Record::Frame(Frame {
                    branch: *branch,
                    hour,
                    keyframe: true,
                    metrics: Metrics::default(),
                    tables: std::array::from_fn(|index| {
                        let table = TableId::ALL[index];
                        if !matches!(
                            table,
                            TableId::Clock | TableId::Campaign | TableId::NavalResultSources
                        ) {
                            return TableUpdate::Unavailable {
                                reason: "outside naval history fixture".into(),
                            };
                        }
                        TableUpdate::Snapshot(match table {
                            TableId::Clock => TableRows::Clock(vec![ClockRow {
                                hour,
                                play_session_started_at: *branch as i64,
                                ..Default::default()
                            }]),
                            TableId::Campaign => TableRows::Campaign(vec![CampaignRow {
                                id: campaign.as_bytes().to_vec(),
                            }]),
                            TableId::NavalResultSources => {
                                TableRows::NavalResultSources(if hour == *first {
                                    ids.iter()
                                        .map(|id| NavalResultSourceRow {
                                            id: *id,
                                            game_hour: hour,
                                            result: *id << 32 | 0x46,
                                            combat: *id << 32 | 0x45,
                                            ..Default::default()
                                        })
                                        .collect()
                                } else {
                                    vec![]
                                })
                            }
                            table => TableRows::empty(table),
                        })
                    }),
                }));
            }
        }
        Replay::from_records(recorder_derive::writers(), campaign.as_bytes().to_vec(), &[records]).unwrap()
    };
    let retained = replay(id, &[(1, 10, &[3, 80]), (2, 5, &[7])]);
    assert_eq!(naval_result_sequence(&retained).unwrap(), 80);
    let older = replay(other_id, &[(1, 10, &[150])]);
    fs::write(root.join(format!("{id}.battleplan")), retained.bytes()).unwrap();
    fs::write(root.join(format!("{other_id}.battleplan")), older.bytes()).unwrap();
    let mut prepared = PreparedCampaigns::load(&root, &session, |_| Ok(())).unwrap();
    assert_eq!(prepared.naval_result_sequence(), 150);
    let (mut tracker, lease) = prepared.take(id, &session).unwrap();
    assert!(lease.is_some());
    let continued = tracker.observe(Some(6), 7, Some(2), &[]).unwrap();
    assert_eq!(continued.branch, 2);
    assert!(
        continued.start.is_none(),
        "reattachment resumes the existing branch"
    );
    assert_eq!(
        prepared.naval_result_sequence(),
        150,
        "taking a campaign cannot rewind the bootstrap seed"
    );
    drop(lease);
    drop(prepared);
    fs::remove_dir_all(&root).unwrap();
}

#[test]
fn market_and_other_event_ledger_sequences_restore_all_retained_prefixes_before_same_process_resume()
 {
    use collector_observers::event_ledger::EventLedger;
    use replay_data::{
        rows::AirLossEventRow, rows::BuildingBombingDamageRow, rows::CampaignRow, rows::ClockRow,
        rows::MarketEventRow, rows::NuclearStrikeRow, rows::TableId, rows::TableRows,
    };
    use replay_frame::{
        branches::BranchReason, branches::BranchStart, branches::Parentage, frame::Frame,
        frame::Metrics, frame::TableUpdate,
    };
    let id = "21234567-89ab-cdef-0123-456789abcdef";
    let other_id = "31234567-89ab-cdef-0123-456789abcdef";
    let root =
        std::env::temp_dir().join(format!("hoi4_history_air_sequence_{}", std::process::id()));
    fs::create_dir_all(&root).unwrap();
    let session = session();
    let replay = |campaign: &str, branches: &[(u64, i32, u64, u64, u64, bool)]| {
        let segments: Vec<_> = branches
            .iter()
            .map(
                |&(branch, first, loss_id, bombing_id, nuclear_id, partial)| {
                    let mut records = vec![
                        Record::SessionStart(session.clone()),
                        Record::BranchStart(BranchStart {
                            id: branch,
                            session: session.id,
                            loaded_hour: Some(first - 2),
                            first_hour: first - 1,
                            parent: Parentage::Unknown,
                            reason: if branch == 1 {
                                BranchReason::SessionStart
                            } else {
                                BranchReason::LoadSignal
                            },
                        }),
                    ];
                    for hour in [first - 1, first, first + 1] {
                        records.push(Record::Frame(Frame {
                            branch,
                            hour,
                            keyframe: hour == first - 1,
                            metrics: Metrics::default(),
                            tables: std::array::from_fn(|index| {
                                let table = TableId::ALL[index];
                                if !matches!(
                                    table,
                                    TableId::Clock
                                        | TableId::Campaign
                                        | TableId::AirLossEvents
                                        | TableId::BuildingBombingDamage
                                        | TableId::NuclearStrikes
                                        | TableId::MarketEvents
                                ) {
                                    return TableUpdate::Unavailable {
                                        reason: "outside air history fixture".into(),
                                    };
                                }
                                let rows = match table {
                                    TableId::Clock => TableRows::Clock(vec![ClockRow {
                                        hour,
                                        play_session_started_at: branch as i64,
                                        ..Default::default()
                                    }]),
                                    TableId::Campaign => TableRows::Campaign(vec![CampaignRow {
                                        id: campaign.as_bytes().to_vec(),
                                    }]),
                                    TableId::AirLossEvents if hour == first => {
                                        TableRows::AirLossEvents(vec![AirLossEventRow {
                                            id: loss_id,
                                            game_hour: hour,
                                            country: 5000,
                                            region: 71,
                                            family_base: 3,
                                            native_cause: 1,
                                            amount_raw: -100_000,
                                            ..Default::default()
                                        }])
                                    }
                                    TableId::BuildingBombingDamage if hour == first => {
                                        TableRows::BuildingBombingDamage(vec![
                                            BuildingBombingDamageRow {
                                                id: bombing_id,
                                                game_hour: hour,
                                                amount_raw: 10_000_000,
                                                ..Default::default()
                                            },
                                        ])
                                    }
                                    TableId::NuclearStrikes if hour == first => {
                                        TableRows::NuclearStrikes(vec![NuclearStrikeRow {
                                            id: nuclear_id,
                                            hour,
                                            ..Default::default()
                                        }])
                                    }
                                    TableId::MarketEvents if hour == first => {
                                        TableRows::MarketEvents(vec![MarketEventRow {
                                            id: loss_id + 100,
                                            hour,
                                            kind: 6,
                                            ..Default::default()
                                        }])
                                    }
                                    table => TableRows::empty(table),
                                };
                                if partial
                                    && hour == first
                                    && matches!(
                                        table,
                                        TableId::AirLossEvents
                                            | TableId::BuildingBombingDamage
                                            | TableId::NuclearStrikes
                                            | TableId::MarketEvents
                                    )
                                {
                                    TableUpdate::Truncated { rows, missing: 1 }
                                } else {
                                    TableUpdate::Snapshot(rows)
                                }
                            }),
                        }));
                    }
                    records
                },
            )
            .collect();
        Replay::from_records(recorder_derive::writers(), campaign.as_bytes().to_vec(), &segments).unwrap()
    };
    let retained = replay(id, &[(1, 10, 0, 5, 3, false), (2, 5, 80, 9, 0, true)]);
    assert!(retained.index.len() >= 2);
    assert_eq!(
        event_ledger_sequences(&retained).unwrap(),
        [Some(80), Some(9), Some(3), Some(180)]
    );
    let older = replay(other_id, &[(1, 2, 5, 150, 40, true)]);
    fs::write(root.join(format!("{id}.battleplan")), retained.bytes()).unwrap();
    fs::write(root.join(format!("{other_id}.battleplan")), older.bytes()).unwrap();
    let mut prepared = PreparedCampaigns::load(&root, &session, |_| Ok(())).unwrap();
    let seeds = prepared.event_ledger_sequences();
    assert_eq!(seeds, [Some(80), Some(150), Some(40), Some(180)]);
    let (mut tracker, lease) = prepared.take(id, &session).unwrap();
    let continued = tracker.observe(Some(6), 7, Some(2), &[]).unwrap();
    assert_eq!(continued.branch, 2);
    assert!(continued.start.is_none());
    assert_eq!(prepared.event_ledger_sequences(), seeds);

    let losses = EventLedger::<collector_observers::air_losses::Entry>::new();
    let bombing = EventLedger::<collector_observers::building_bombing::Entry>::new();
    let market = EventLedger::<MarketEventRow>::new();
    market.restore_sequence(seeds[3]).unwrap();
    losses.restore_sequence(seeds[0]).unwrap();
    bombing.restore_sequence(seeds[1]).unwrap();
    market.prepare(1);
    assert!(unsafe { market.drain(&mut Vec::new()) }.is_err());
    losses.prepare(1);
    bombing.prepare(1);
    assert!(unsafe { losses.drain(&mut Vec::new()) }.is_err());
    assert!(unsafe { bombing.drain(&mut Vec::new()) }.is_err());
    let clock = |hour| {
        Some(engine_native::cgame_state::Clock {
            hour,
            play_session_started_at: 2,
            game_state: 0,
            play_time: 0,
            seed: 0,
            campaign_id: Default::default(),
            speed: 0,
            tick_busy: 0,
        })
    };
    for hour in [6, 7] {
        unsafe {
            market.boundary(clock(hour));
            losses.boundary(clock(hour));
            bombing.boundary(clock(hour));
        }
    }
    let token = losses.claim((5000, 71)).unwrap();
    losses
        .complete(
            token,
            Ok(Some(collector_observers::air_losses::Entry {
                amount_raw: -100_000,
                ..Default::default()
            })),
        )
        .unwrap();
    let token = bombing.claim((5000, 72)).unwrap();
    bombing
        .complete(
            token,
            Ok(Some(collector_observers::building_bombing::Entry {
                amount_raw: 10_000_000,
                ..Default::default()
            })),
        )
        .unwrap();
    let token = market.claim((5000, 17)).unwrap();
    market
        .complete(
            token,
            Ok(Some(MarketEventRow {
                kind: 6,
                ..Default::default()
            })),
        )
        .unwrap();
    let mut market_rows = Vec::with_capacity(1);
    unsafe { market.drain(&mut market_rows) }.unwrap();
    assert_eq!(market_rows[0].id, 181);
    let mut loss_rows = Vec::with_capacity(1);
    let mut damage_rows = Vec::with_capacity(1);
    unsafe { losses.drain(&mut loss_rows) }.unwrap();
    unsafe { bombing.drain(&mut damage_rows) }.unwrap();
    assert_eq!(loss_rows[0].id, 81);
    assert_eq!(damage_rows[0].id, 151);
    assert_eq!(
        PreparedCampaigns::default().event_ledger_sequences(),
        [None, None, None, None]
    );
    drop(lease);
    drop(prepared);
    fs::remove_dir_all(&root).unwrap();
}

#[test]
fn resident_history_lease_never_blocks_recorder_append_or_rollback() {
    use std::io::Write;
    let root = std::env::temp_dir().join(format!("hoi4_history_shared_{}", std::process::id()));
    fs::create_dir_all(&root).unwrap();
    let path = root.join("history.bin");
    fs::write(&path, b"resident snapshot").unwrap();
    let mut resident = lease(&path).unwrap();
    let mut writer = OpenOptions::new().append(true).open(&path).unwrap();
    writer.write_all(b" appended").unwrap();
    writer.sync_data().unwrap();
    drop(writer);
    let replacement = root.join("replacement.bin");
    fs::write(&replacement, b"rolled back").unwrap();
    replay_settings::publication::replace_file(&replacement, &path).unwrap();
    let mut bytes = Vec::new();
    resident.read_to_end(&mut bytes).unwrap();
    assert_eq!(bytes, b"resident snapshot appended");
    assert_eq!(fs::read(&path).unwrap(), b"rolled back");
    drop(resident);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn the_history_load_leaves_no_thread_it_started_running() {
    use replay_data::{rows::CampaignRow, rows::TableId, rows::TableRows};
    use replay_frame::{
        branches::BranchReason, branches::BranchStart, branches::Parentage, frame::DecodedFrame,
        frame::FrameEncoder, frame::Metrics, frame::TableView,
    };
    const CHILD: &str = "BATTLEPLAN_HISTORY_LOAD_THREADS";
    if std::env::var_os(CHILD).is_none() {
        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "history::tests::the_history_load_leaves_no_thread_it_started_running",
                "--nocapture",
            ])
            .env(CHILD, "1")
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "history load child failed: {}\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        return;
    }
    let module =
        unsafe { windows_sys::Win32::System::LibraryLoader::GetModuleHandleW(std::ptr::null()) };
    let census = || collector_workers::module_threads::started_in(module).unwrap();
    let (started, id) = std::sync::mpsc::channel();
    let (release, released) = std::sync::mpsc::channel::<()>();
    let thread = std::thread::spawn(move || {
        started
            .send(unsafe { windows_sys::Win32::System::Threading::GetCurrentThreadId() })
            .unwrap();
        let _ = released.recv();
    });
    let id = id.recv().unwrap();
    assert!(census().contains(&id));
    drop(release);
    thread.join().unwrap();
    assert!(!census().contains(&id));

    let campaign = "01234567-89ab-cdef-0123-456789abcdef";
    let root = std::env::temp_dir().join(format!("hoi4_history_threads_{}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root).unwrap();
    let mut tables = std::array::from_fn(|_| TableView::Unavailable {
        reason: "fixture".into(),
    });
    tables[TableId::Campaign.index()] =
        TableView::Complete(TableRows::Campaign(vec![CampaignRow {
            id: campaign.into(),
        }]));
    let frame = DecodedFrame {
        branch: 1,
        hour: 100,
        tables: tables.map(Into::into),
        metrics: Metrics::default(),
    };
    let replay = Replay::from_records(
        recorder_derive::writers(),
        campaign.into(),
        &[vec![
            Record::SessionStart(session()),
            Record::BranchStart(BranchStart {
                id: 1,
                session: 1,
                loaded_hour: None,
                first_hour: 100,
                parent: Parentage::Unknown,
                reason: BranchReason::SessionStart,
            }),
            Record::Frame(FrameEncoder::new().encode(frame, false).unwrap()),
        ]],
    )
    .unwrap();
    assert!(!replay.index.is_empty());
    let unsaved = replay_settings::publication::unsaved_path(&root, campaign);
    fs::create_dir_all(unsaved.parent().unwrap()).unwrap();
    fs::write(&unsaved, replay.bytes()).unwrap();
    let session = SessionStart { id: 2, ..session() };

    let before = census();
    let mut prepared = PreparedCampaigns::load(&root, &session, |_| Ok(())).unwrap();
    let left: Vec<_> = census()
        .into_iter()
        .filter(|id| !before.contains(id))
        .collect();
    assert!(
        left.is_empty(),
        "{} threads the load started are still running",
        left.len()
    );
    let (_, lease) = prepared.take(campaign, &session).unwrap();
    assert!(lease.is_some());
    drop((lease, prepared));
    fs::remove_dir_all(&root).unwrap();
}
