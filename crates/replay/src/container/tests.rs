use super::*;
use crate::{
    branches::Parentage,
    frame::{FrameEncoder, Metrics, TableUpdate},
    identity::Roster,
    rows::{CampaignRow, ClockRow, SunkShipsRow, WorldTensionRow},
    spool::{encode_record, recover_prefix},
};

const CAMPAIGN: &[u8] = b"container-test-campaign";

#[test]
fn partial_capture_trailer_retains_prefix_gap_and_list_metadata() {
    for failed_hour in [Some(3), None] {
        let mut captured = records(1, &[1, 2]);
        if let Some(hour) = failed_hour {
            captured.push(Record::Gap(Gap {
                branch: 1,
                first_hour: hour,
                last_hour: hour,
                reason: "collector allocation failed".into(),
            }));
        }
        let partial = ControlRecord::Partial {
            session: 1,
            reason: "collector allocation failed (phase 2, code 1)".into(),
            failed_hour,
        };
        captured.push(Record::Control(partial.clone()));
        captured.push(Record::Control(ControlRecord::Detach {
            session: 1,
            last_hour: Some(2),
        }));
        let replay = Replay::from_records(CAMPAIGN.into(), &[captured]).unwrap();
        assert!(replay.is_partial());
        assert_eq!(replay.partial(), Some(&partial));
        assert_eq!(replay.gaps.len(), usize::from(failed_hour.is_some()));
        assert!(matches!(replay.seek(1, 2).unwrap(), SeekResult::Frame(_)));
        if let Some(hour) = failed_hour {
            assert!(
                matches!(replay.seek(1, hour).unwrap(), SeekResult::Gap { reason, .. } if reason == "collector allocation failed")
            );
        }
        let reopened = Replay::open(replay.bytes().to_vec()).unwrap();
        assert_eq!(reopened.partial(), Some(&partial));
        let mut source = std::io::Cursor::new(replay.bytes());
        assert_eq!(
            Replay::read_partial_from(&mut source, replay.bytes().len() as u64).unwrap(),
            Some(partial)
        );
        let mut damaged = replay.bytes().to_vec();
        damaged[replay.trailer_start + 4] ^= 1;
        let length = damaged.len() as u64;
        assert!(Replay::read_partial_from(&mut std::io::Cursor::new(damaged), length).is_err());
    }
}

#[test]
fn partial_capture_can_end_a_loaded_branch_before_its_first_frame() {
    let prefix = Replay::from_records(CAMPAIGN.into(), &[records(1, &[1, 2])]).unwrap();
    let history = prefix.into_history().unwrap();
    let mut tracker = history.tracker_for(&history.sessions[0]).unwrap();
    let failed_hour = -7;
    let start = tracker
        .observe(None, failed_hour, Some(123), &[])
        .unwrap()
        .start
        .unwrap();
    assert_eq!(start.reason, BranchReason::LoadSignal);
    assert_eq!(start.loaded_hour, None);
    assert_eq!(start.parent, Parentage::Unknown);
    let lost_branch = start.id;
    let mut captured = records(1, &[1, 2]);
    captured.extend([
        Record::BranchStart(start),
        Record::Gap(Gap {
            branch: lost_branch,
            first_hour: failed_hour,
            last_hour: failed_hour,
            reason: "collector allocation failed".into(),
        }),
        Record::Control(ControlRecord::Partial {
            session: 1,
            reason: "collector allocation failed".into(),
            failed_hour: Some(failed_hour),
        }),
        Record::Control(ControlRecord::Detach {
            session: 1,
            last_hour: None,
        }),
    ]);
    let replay = Replay::from_records(CAMPAIGN.into(), &[captured]).unwrap();
    assert_eq!(replay.index.len(), 1);
    assert_eq!(replay.branches.len(), 2);
    assert!(matches!(replay.seek(1, 2).unwrap(), SeekResult::Frame(_)));
    assert!(
        matches!(replay.seek(lost_branch, failed_hour).unwrap(), SeekResult::Gap { reason, .. } if reason == "collector allocation failed")
    );
    assert!(Replay::open(replay.bytes().to_vec()).unwrap().is_partial());
}

fn session(id: u64) -> SessionStart {
    SessionStart {
        id,
        process_id: id as u32,
        process_start_time: id,
        qpc_frequency: 10_000_000,
        sample_boundary: crate::identity::SampleBoundary::EndOfHour,
        identity: Identity {
            game_version: "test".into(),
            exe_hash: [1; 32],
            exe_size: 1,
            vanilla_hash: [2; 32],
            dlc: Vec::new(),
            mods: Vec::new(),
            coverage_policy: 1,
            language: "l_english".into(),
            recorder_version: format!("test-{id}"),
            collector_hash: [id as u8; 32],
            verification: Verification::Verified {
                launcher: Roster::default(),
                memory: Roster::default(),
            },
        },
    }
}

fn branch(id: u64, hour: i32) -> BranchStart {
    BranchStart {
        id,
        session: id,
        loaded_hour: Some(hour - 1),
        first_hour: hour,
        parent: Parentage::Unknown,
        reason: BranchReason::SessionStart,
    }
}

fn source(branch: u64, hour: i32) -> DecodedFrame {
    let mut tables =
        std::array::from_fn(|index| TableView::Complete(TableRows::empty(TableId::ALL[index])));
    tables[TableId::Clock.index()] = TableView::Complete(TableRows::Clock(vec![ClockRow {
        hour,
        ..ClockRow::default()
    }]));
    tables[TableId::Campaign.index()] =
        TableView::Complete(TableRows::Campaign(vec![CampaignRow {
            id: CAMPAIGN.into(),
        }]));
    tables[TableId::SunkShips.index()] =
        TableView::Complete(TableRows::SunkShips(vec![SunkShipsRow::default()]));
    tables[TableId::WorldTension.index()] =
        TableView::Complete(TableRows::WorldTension(vec![WorldTensionRow::default()]));
    tables[TableId::LocalPlayer.index()] =
        TableView::Complete(TableRows::LocalPlayer(vec![Default::default()]));
    tables[TableId::SimulationFeatures.index()] =
        TableView::Complete(TableRows::SimulationFeatures(vec![
            crate::rows::SimulationFeatureRow::default(),
        ]));
    tables[TableId::CombatLogState.index()] = TableView::Complete(TableRows::CombatLogState(vec![
        crate::rows::CombatLogStateRow::default(),
    ]));
    tables[TableId::AirSortieModifierMetadata.index()] =
        TableView::Complete(TableRows::AirSortieModifierMetadata(vec![
            crate::rows::AirSortieModifierMetadataRow::default(),
        ]));
    tables[TableId::ReservedCountryNaming.index()] =
        TableView::Complete(TableRows::ReservedCountryNaming(vec![Default::default()]));
    tables[TableId::GameScoreMultipliers.index()] =
        TableView::Complete(TableRows::GameScoreMultipliers(vec![
            crate::rows::GameScoreMultiplierRow {
                total_raw: Some(0),
                army_raw: Some(0),
                navy_raw: Some(0),
                air_raw: Some(0),
                industry_raw: Some(0),
                land_raw: Some(0),
                ..Default::default()
            },
        ]));
    tables[TableId::EventFeedHistory.index()] =
        TableView::Complete(TableRows::EventFeedHistory(vec![
            crate::rows::EventFeedHistoryRow {
                read_hour: hour,
                coverage: crate::event_feed_rows::EventFeedCoverage::Complete,
                counts: Some(Default::default()),
                ..Default::default()
            },
        ]));
    DecodedFrame {
        branch,
        hour,
        metrics: Metrics::default(),
        tables: tables.map(Into::into),
    }
}

#[test]
fn destruction_hours_roundtrip_signed_empty_unavailable_and_selected() {
    use crate::rows::{
        DivisionDeletionDisposition, DivisionEquipmentDeletionRow, EquipmentDestructionCause,
        EquipmentDestructionCoverage, EquipmentDestructionRow,
    };
    let event = EquipmentDestructionRow {
        country: 4999,
        cause: EquipmentDestructionCause::DivisionDestroyed,
        variant: 0xf00100000046,
        amount_raw: 900001,
        ..Default::default()
    };
    let deletion = DivisionEquipmentDeletionRow {
        division_handle: (781_u64 << 32) | 0x1234,
        country: event.country,
        disposition: DivisionDeletionDisposition::Loss,
        cause: Some(event.cause),
        ..Default::default()
    };
    let views = [
        (
            TableView::Unavailable {
                reason: "first interval partial".into(),
            },
            TableView::Unavailable {
                reason: "first interval partial".into(),
            },
        ),
        (
            TableView::Complete(TableRows::EquipmentDestruction(vec![event.clone()])),
            TableView::Complete(TableRows::DivisionEquipmentDeletions(vec![
                deletion.clone(),
            ])),
        ),
        (
            TableView::Complete(TableRows::EquipmentDestruction(vec![
                EquipmentDestructionRow {
                    cause: EquipmentDestructionCause::LandCombat,
                    amount_raw: -30_000,
                    ..event.clone()
                },
            ])),
            TableView::Complete(TableRows::DivisionEquipmentDeletions(vec![])),
        ),
        (
            TableView::Complete(TableRows::EquipmentDestruction(vec![])),
            TableView::Complete(TableRows::DivisionEquipmentDeletions(vec![
                deletion.clone(),
            ])),
        ),
        (
            TableView::Complete(TableRows::EquipmentDestruction(vec![])),
            TableView::Complete(TableRows::DivisionEquipmentDeletions(vec![])),
        ),
        (
            TableView::Complete(TableRows::EquipmentDestruction(vec![event])),
            TableView::Complete(TableRows::DivisionEquipmentDeletions(vec![deletion])),
        ),
        (
            TableView::Complete(TableRows::EquipmentDestruction(vec![
                EquipmentDestructionRow {
                    coverage: EquipmentDestructionCoverage::SupplementalUnavailable.into(),
                    unavailable_reason: "delete hook sentinel differs".to_owned().into(),
                    ..Default::default()
                },
            ])),
            TableView::Unavailable {
                reason: "delete hook sentinel differs".into(),
            },
        ),
        (
            TableView::Unavailable {
                reason: "event log overflow".into(),
            },
            TableView::Unavailable {
                reason: "event log overflow".into(),
            },
        ),
    ];
    let mut encoder = FrameEncoder::new();
    let mut input = vec![
        Record::SessionStart(session(1)),
        Record::BranchStart(branch(1, 1)),
    ];
    for (i, (amounts, deletions)) in views.iter().enumerate() {
        let mut frame = source(1, i as i32 + 1);
        *frame.tables[TableId::EquipmentDestruction.index()] = amounts.clone();
        *frame.tables[TableId::DivisionEquipmentDeletions.index()] = deletions.clone();
        frame.metrics.table_ns[TableId::EquipmentDestruction.index()] = 1;
        frame.metrics.table_ns[TableId::DivisionEquipmentDeletions.index()] = 1;
        input.push(Record::Frame(encoder.encode(frame, false).unwrap()));
    }
    let replay = Replay::from_records(CAMPAIGN.into(), &[input]).unwrap();
    for (i, (amounts, deletions)) in views.iter().enumerate() {
        let SeekResult::Frame(frame) = replay.seek(1, i as i32 + 1).unwrap() else {
            panic!("missing hour");
        };
        assert_eq!(
            &*frame.tables[TableId::EquipmentDestruction.index()],
            amounts
        );
        assert_eq!(
            &*frame.tables[TableId::DivisionEquipmentDeletions.index()],
            deletions
        );
    }
    for block in &replay.index {
        let full = replay.read_block(block).unwrap();
        let selected = replay
            .read_block_selected(
                block,
                &[
                    TableId::EquipmentDestruction,
                    TableId::DivisionEquipmentDeletions,
                ],
            )
            .unwrap();
        for (a, b) in full.iter().zip(&selected) {
            assert_eq!(
                a.tables[TableId::EquipmentDestruction.index()],
                b.tables[TableId::EquipmentDestruction.index()]
            );
            assert_eq!(
                a.tables[TableId::DivisionEquipmentDeletions.index()],
                b.tables[TableId::DivisionEquipmentDeletions.index()]
            );
        }
    }
    assert_eq!(
        crate::cadence::cadence(TableId::EquipmentDestruction),
        crate::cadence::Cadence::Hourly
    );
    assert_eq!(
        crate::cadence::cadence(TableId::DivisionEquipmentDeletions),
        crate::cadence::Cadence::Hourly
    );
}

fn records(id: u64, hours: &[i32]) -> Vec<Record> {
    let mut encoder = FrameEncoder::new();
    let mut records = vec![
        Record::SessionStart(session(id)),
        Record::BranchStart(branch(id, hours[0])),
    ];
    for &hour in hours {
        records.push(Record::Frame(
            encoder.encode(source(id, hour).clone(), false).unwrap(),
        ));
    }
    records
}

fn rebuild(replay: &Replay, prefix: Vec<u8>, index: &[BlockIndex]) -> Vec<u8> {
    let mut bytes = prefix;
    finish_file(
        &mut bytes,
        &replay.sessions,
        &replay.branches,
        &replay.gaps,
        &replay.controls,
        &replay.recovery_notes,
        index,
    )
    .unwrap();
    bytes
}

#[test]
fn size_driven_blocks_preserve_every_hour_value_origin_and_metric() {
    use crate::rows::{CountryRow, LexicalTokenRow, SliceReadRow};

    let mut encoder = FrameEncoder::new();
    let mut input = vec![
        Record::SessionStart(session(1)),
        Record::BranchStart(branch(1, 1)),
    ];
    let mut expected = Vec::new();
    let mut original_raw_bytes = 4;
    for hour in 1..=20 {
        let mut decoded = source(1, hour);
        *decoded.tables[TableId::Countries.index()] = TableView::Complete(TableRows::Countries(
            (0..5_000)
                .map(|id| CountryRow {
                    id,
                    canonical_slot: id,
                    command_power_raw: i64::from(hour) * -100_001,
                    ..Default::default()
                })
                .collect(),
        ));
        *decoded.tables[TableId::LexicalTokens.index()] =
            TableView::Complete(TableRows::LexicalTokens(vec![LexicalTokenRow {
                token: 0,
                literal: vec![hour as u8; 64 * 1024],
            }]));
        *decoded.tables[TableId::SliceReads.index()] =
            TableView::Complete(TableRows::SliceReads(vec![SliceReadRow {
                table: TableId::ResearchInputs.index() as u32,
                owner: 4_999,
                sample_hour: 1,
            }]));
        *decoded.tables[TableId::Provinces.index()] = TableView::Unavailable {
            reason: "synthetic guarded read failure".into(),
        };
        decoded.metrics.total_ns = hour as u64;
        decoded.metrics.table_ns[TableId::Countries.index()] = hour as u64 * 2;
        let frame = encoder.encode(decoded.clone(), false).unwrap();
        let mut payload = Vec::new();
        frame.encode(&mut payload).unwrap();
        original_raw_bytes += payload.len() + 4;
        assert_eq!(frame.keyframe, hour == 1);
        expected.push(decoded);
        input.push(Record::Frame(frame));
    }
    // Exercise the production overflow path with a small bound instead of
    // allocating a gigabyte in a unit test.
    let limit = 512 * 1024;
    assert!(original_raw_bytes > limit);
    let mut observed = Vec::new();
    let replay = Replay::from_record_slices(
        CAMPAIGN.into(),
        &[&input],
        &mut |frame, decoded, sizes| {
            assert_eq!(sizes, frame.table_encoded_sizes()?);
            observed.push(decoded.clone());
            Ok(())
        },
        None,
        limit,
    )
    .unwrap();
    assert!(replay.index.len() > 1);
    assert!(
        replay
            .index
            .iter()
            .all(|block| block.raw_length <= limit as u64)
    );
    assert_eq!(observed, expected);
    let reopened = Replay::open(replay.bytes().to_vec()).unwrap();
    let mut count = 0;
    for block in &reopened.index {
        let frames = reopened.read_block(block).unwrap();
        assert!(frames[0].keyframe);
        assert!(frames.iter().skip(1).all(|frame| !frame.keyframe));
        let mut decoder = FrameDecoder::new();
        for frame in frames {
            assert_eq!(decoder.advance(&frame).unwrap(), &expected[count]);
            count += 1;
        }
    }
    assert_eq!(count, expected.len());
    for decoded in &expected {
        assert!(
            matches!(reopened.seek(1, decoded.hour).unwrap(), SeekResult::Frame(actual) if *actual == *decoded)
        );
    }
}

#[test]
fn size_driven_blocks_check_framing_and_refuse_unrepresentable_hours() {
    let input = records(1, &[1, 2]);
    let Record::Frame(first) = &input[2] else {
        unreachable!()
    };
    let mut payload = Vec::new();
    first.encode(&mut payload).unwrap();
    let mut block = RawBlock::new(first);
    let exact = payload.len() + 8;
    assert!(!block.fits(payload.len(), exact - 1));
    assert!(block.fits(payload.len(), exact));
    block.push_payload(first, &payload, exact).unwrap();
    assert!(!block.fits(usize::MAX, MAX_BLOCK_RAW_BYTES));
    assert!(
        Replay::from_record_slices(
            CAMPAIGN.into(),
            &[&input],
            &mut |_, _, _| Ok(()),
            None,
            exact - 1
        )
        .err()
        .unwrap()
        .to_string()
        .contains("single replay frame")
    );

    let mut input = input;
    let Record::Frame(last) = &mut input[3] else {
        unreachable!()
    };
    last.tables[TableId::Provinces.index()] = TableUpdate::Truncated {
        rows: TableRows::Provinces(vec![]),
        missing: 7,
    };
    let error = Replay::from_record_slices(
        CAMPAIGN.into(),
        &[&input],
        &mut |_, _, _| Ok(()),
        None,
        exact,
    )
    .err()
    .unwrap()
    .to_string();
    assert!(error.contains("Provinces"), "{error}");
    assert!(error.contains("7 missing rows"), "{error}");
}

#[test]
fn recovered_frame_reader_matches_in_memory_finalize_and_append() {
    let segments = [(
        3,
        Recovery {
            format: Some(FormatVersion::CURRENT),
            records: records(1, &[1, 2, 3]),
            valid_bytes: 99,
            damage: None,
        },
    )];
    let mut restored = Vec::new();
    let replay = Replay::from_recovered_with_reader(
        CAMPAIGN.into(),
        &segments,
        |number, frame| {
            restored.push((number, frame.hour));
            Ok(frame.clone())
        },
        |_, _, _| Ok(()),
    )
    .unwrap();
    assert_eq!(restored, [(3, 1), (3, 2), (3, 3)]);
    assert_eq!(
        replay.bytes(),
        Replay::from_recovered(CAMPAIGN.into(), &segments)
            .unwrap()
            .bytes()
    );
    let next = [(
        8,
        Recovery {
            format: Some(FormatVersion::CURRENT),
            records: records(2, &[4, 5]),
            valid_bytes: 100,
            damage: Some("synthetic tail damage".into()),
        },
    )];
    let appended = replay
        .append_recovered_with_reader(&next, |_, frame| Ok(frame.clone()), |_, _, _| Ok(()))
        .unwrap();
    assert_eq!(
        appended.bytes(),
        replay.append_recovered(&next).unwrap().bytes()
    );
    assert!(
        Replay::from_recovered_with_reader(
            CAMPAIGN.into(),
            &segments,
            |_, _| { Err(io::Error::from(io::ErrorKind::OutOfMemory)) },
            |_, _, _| Ok(())
        )
        .is_err()
    );
}

#[test]
fn history_preserves_exact_hours_and_parentage_across_blocks_and_sessions() {
    let mut first = records(1, &[1, 3, 721]);
    first.push(Record::Gap(Gap {
        branch: 1,
        first_hour: 2,
        last_hour: 2,
        reason: "queue full".into(),
    }));
    let history = Replay::from_records(CAMPAIGN.into(), &[first, records(2, &[3, 4])])
        .unwrap()
        .into_history()
        .unwrap();
    assert_eq!(history.campaign_id, CAMPAIGN);
    assert_eq!(history.sessions, [session(1), session(2)]);
    assert_eq!(history.branches[0].hours, [1, 3, 721]);
    assert_eq!(history.branches[1].hours, [3, 4]);
    for (loaded_hour, expected) in [
        (1, Parentage::Probable { branch: 1, hour: 1 }),
        (2, Parentage::Unknown),
        (3, Parentage::Unknown),
    ] {
        let mut tracker = history.tracker_for(&session(3)).unwrap();
        let start = tracker
            .observe(Some(loaded_hour), loaded_hour + 1, Some(0), &[])
            .unwrap()
            .start
            .unwrap();
        assert_eq!(start.id, 3);
        assert_eq!(start.reason, BranchReason::SessionStart);
        assert_eq!(start.parent, expected);
    }
}

#[test]
fn history_resumes_last_written_branch_and_checks_actual_session_metadata() {
    let mut input = records(1, &[1, 3]);
    input.push(Record::BranchStart(BranchStart {
        id: 2,
        session: 1,
        loaded_hour: Some(1),
        first_hour: 2,
        parent: Parentage::Probable { branch: 1, hour: 1 },
        reason: BranchReason::LoadSignal,
    }));
    let mut reloaded = source(2, 2);
    if let TableView::Complete(TableRows::Clock(rows)) =
        &mut *reloaded.tables[TableId::Clock.index()]
    {
        rows[0].play_session_started_at = 42;
    }
    input.push(Record::Frame(
        FrameEncoder::new().encode(reloaded.clone(), true).unwrap(),
    ));
    let history = Replay::from_records(CAMPAIGN.into(), &[input])
        .unwrap()
        .into_history()
        .unwrap();
    assert_eq!(
        history.session_tail(1),
        Some(SessionTail {
            branch: 2,
            hour: 2,
            load_signal: Some(42),
        })
    );
    let mut tracker = history.tracker_for(&session(1)).unwrap();
    assert_eq!(tracker.current_branch(), Some(2));
    assert_eq!(
        tracker.observe(Some(2), 3, Some(42), &[]).unwrap().start,
        None
    );
    let mut tracker = history.tracker_for(&session(1)).unwrap();
    assert_eq!(
        tracker
            .observe(Some(2), 3, Some(43), &[])
            .unwrap()
            .start
            .unwrap()
            .reason,
        BranchReason::LoadSignal
    );

    let mut pending = session(1);
    pending.identity.verification = Verification::Unchecked;
    assert!(
        history
            .tracker_for(&pending)
            .unwrap_err()
            .to_string()
            .contains("metadata")
    );
    assert!(history.tracker_for(&history.sessions[0]).is_ok());
    let mut changed = session(2);
    changed.identity.vanilla_hash[0] ^= 1;
    assert!(
        history
            .tracker_for(&changed)
            .unwrap_err()
            .to_string()
            .contains("different content")
    );
}

#[test]
fn history_uses_matching_witness_only_when_tail_clock_is_unavailable() {
    for clock_available in [false, true] {
        let mut input = records(1, &[1, 2]);
        if !clock_available && let Record::Frame(frame) = &mut input[3] {
            frame.tables[TableId::Clock.index()] = TableUpdate::Unavailable {
                reason: "hour changed during read".into(),
            };
        }
        for (hour, load_signal) in [(2, Some(41)), (2, Some(42)), (2, None), (3, Some(99))] {
            input.push(Record::Control(ControlRecord::ClockWitness {
                session: 1,
                qpc: 100,
                hour,
                load_signal,
            }));
        }
        let history = Replay::from_records(CAMPAIGN.into(), &[input])
            .unwrap()
            .into_history()
            .unwrap();
        assert_eq!(history.branches[0].hours, [1, 2]);
        assert_eq!(
            history.session_tail(1),
            Some(SessionTail {
                branch: 1,
                hour: 2,
                load_signal: Some(if clock_available { 0 } else { 42 }),
            })
        );
    }
}

#[test]
fn exact_seek_reports_holes_and_append_preserves_blocks_and_session_provenance() {
    let mut first = records(1, &[1, 3, 721]);
    first.insert(
        3,
        Record::Gap(Gap {
            branch: 1,
            first_hour: 2,
            last_hour: 2,
            reason: "queue full".into(),
        }),
    );
    let old = Replay::from_records(CAMPAIGN.into(), &[first]).unwrap();
    assert_eq!(old.index.len(), 2);
    assert!(
        matches!(old.seek(1, 2).unwrap(), SeekResult::Gap { reason, .. } if reason == "queue full")
    );
    assert!(matches!(old.seek(1, 720).unwrap(), SeekResult::Gap { .. }));
    let mut next = records(2, &[722, 723]);
    if let Record::SessionStart(session) = &mut next[0] {
        session.identity.verification = Verification::Unchecked;
    }
    let appended = old.append(&[next]).unwrap();
    assert!(old.content_verified());
    assert!(!appended.content_verified());
    assert_eq!(appended.sessions.len(), 2);
    assert_eq!(appended.sessions[0], old.sessions[0]);
    assert_eq!(
        &appended.bytes()[..old.trailer_start],
        &old.bytes()[..old.trailer_start]
    );
    for entry in &old.index {
        let range = entry.offset as usize..(entry.offset + entry.length) as usize;
        assert_eq!(&old.bytes()[range.clone()], &appended.bytes()[range]);
    }
    assert!(
        matches!(appended.seek(2, 723).unwrap(), SeekResult::Frame(frame) if *frame == source(2, 723))
    );
}

#[test]
fn compact_trailer_controls_preserve_order_duplicates_and_session_context() {
    let witness = ControlRecord::ClockWitness {
        session: 1,
        qpc: 200,
        hour: 2,
        load_signal: Some(-1),
    };
    let controls = vec![
        witness.clone(),
        witness,
        ControlRecord::GapSummary {
            session: 1,
            dropped_frames: 3,
            reason: "loss range unavailable".into(),
        },
        ControlRecord::Detach {
            session: 1,
            last_hour: Some(2),
        },
    ];
    let mut first = records(1, &[1, 2]);
    first.extend(controls.iter().cloned().map(Record::Control));
    let replay = Replay::from_records(CAMPAIGN.into(), &[first]).unwrap();
    assert_eq!(replay.controls, controls);
    let reopened = Replay::open(replay.bytes().to_vec()).unwrap();
    assert_eq!(reopened.controls, controls);
    let next_control = ControlRecord::Detach {
        session: 2,
        last_hour: None,
    };
    let mut next = records(2, &[3]);
    next.push(Record::Control(next_control.clone()));
    let appended = reopened.append(&[next]).unwrap();
    assert_eq!(appended.controls[..controls.len()], controls);
    assert_eq!(appended.controls.last(), Some(&next_control));
    assert_eq!(
        appended.bytes()[..reopened.trailer_start],
        reopened.bytes()[..reopened.trailer_start]
    );

    let mut invalid_context = records(1, &[1]);
    invalid_context.push(Record::Control(ControlRecord::ClockWitness {
        session: 99,
        qpc: 1,
        hour: 1,
        load_signal: None,
    }));
    assert!(Replay::from_records(CAMPAIGN.into(), &[invalid_context]).is_err());
    let mut invalid_trailer = replay.bytes()[..replay.trailer_start].to_vec();
    finish_file(
        &mut invalid_trailer,
        &replay.sessions,
        &replay.branches,
        &replay.gaps,
        &[ControlRecord::Detach {
            session: 99,
            last_hour: None,
        }],
        &replay.recovery_notes,
        &replay.index,
    )
    .unwrap();
    assert!(
        Replay::open(invalid_trailer)
            .err()
            .unwrap()
            .to_string()
            .contains("unknown session")
    );
}

#[test]
fn incompatible_campaign_or_content_cannot_append() {
    let replay = Replay::from_records(CAMPAIGN.into(), &[records(1, &[1])]).unwrap();
    let mut wrong_campaign = records(2, &[2]);
    if let Record::Frame(frame) = &mut wrong_campaign[2] {
        frame.tables[TableId::Campaign.index()] =
            crate::frame::TableUpdate::Snapshot(TableRows::Campaign(vec![CampaignRow {
                id: b"other".into(),
            }]));
    }
    assert!(replay.append(&[wrong_campaign]).is_err());
    let mut wrong_content = records(2, &[2]);
    if let Record::SessionStart(session) = &mut wrong_content[0] {
        session.identity.vanilla_hash[0] ^= 1;
    }
    assert!(replay.append(&[wrong_content]).is_err());
    assert!(matches!(replay.seek(1, 1).unwrap(), SeekResult::Frame(_)));
}

#[test]
fn seek_rejects_an_intermediate_campaign_change_even_when_target_is_valid() {
    let replay = Replay::from_records(CAMPAIGN.into(), &[records(1, &[1, 2, 3])]).unwrap();
    let mut frames = replay.read_block(&replay.index[0]).unwrap();
    frames[1].tables[TableId::Campaign.index()] =
        TableUpdate::Snapshot(TableRows::Campaign(vec![CampaignRow {
            id: b"different-campaign".to_vec(),
        }]));
    frames[2].tables[TableId::Campaign.index()] =
        TableUpdate::Snapshot(TableRows::Campaign(vec![CampaignRow {
            id: CAMPAIGN.to_vec(),
        }]));

    // The malformed identity is inside a structurally valid compressed block.
    let mut raw = RawBlock::new(&frames[0]);
    let mut sequential = FrameDecoder::new();
    for frame in &frames {
        raw.push(frame).unwrap();
        let decoded = sequential.apply(frame).unwrap();
        assert_eq!(
            validate_campaign(&decoded, CAMPAIGN).is_err(),
            frame.hour == 2
        );
    }
    let mut prefix = replay.bytes[..replay.header_end].to_vec();
    let mut index = Vec::new();
    append_blocks(&mut prefix, &mut index, &[raw.compress().unwrap()]).unwrap();
    let malformed = Replay::open(rebuild(&replay, prefix, &index)).unwrap();
    assert!(matches!(
        malformed.seek(1, 1).unwrap(),
        SeekResult::Frame(_)
    ));
    for hour in [2, 3] {
        assert!(matches!(
            malformed.seek(1, hour).unwrap(),
            SeekResult::Gap { reason, .. }
                if reason == "frame campaign identity differs from replay header"
        ));
    }
    assert!(malformed.into_history().is_err());
}

#[test]
fn selected_current_blocks_preserve_late_tables_and_their_updates() {
    let mut encoder = FrameEncoder::new();
    let mut records = vec![
        Record::SessionStart(session(1)),
        Record::BranchStart(branch(1, 1)),
    ];
    for hour in [1, 2, 721] {
        let mut frame = source(1, hour);
        for table in TableId::ALL {
            if let Some(rows) = crate::rows::sample_extended_table(table, hour as usize) {
                *frame.tables[table.index()] = TableView::Complete(rows);
            }
            frame.metrics.table_ns[table.index()] = hour as u64 + table.index() as u64;
        }
        records.push(Record::Frame(encoder.encode(frame, false).unwrap()));
    }
    let replay = Replay::from_records(CAMPAIGN.into(), &[records]).unwrap();
    let selected = [
        TableId::Campaign,
        TableId::DivisionEtaInputs,
        TableId::RaidTargets,
        TableId::NavalBases,
        TableId::FactionTheatres,
    ];
    for block in &replay.index {
        let full = replay.read_block(block).unwrap();
        let projected = replay.read_block_selected(block, &selected).unwrap();
        assert_eq!(full.len(), projected.len());
        for (full, projected) in full.iter().zip(&projected) {
            assert_eq!(full.metrics, projected.metrics);
            for table in selected {
                assert_eq!(full.tables[table.index()], projected.tables[table.index()]);
            }
            assert!(matches!(
                &projected.tables[TableId::Clock.index()],
                TableUpdate::Unavailable { .. }
            ));
        }
    }
}

#[test]
fn unversioned_spool_cannot_be_finalized_as_a_current_replay() {
    let mut bytes = Vec::new();
    for record in records(1, &[1, 2]) {
        let mut encoded = encode_record(&record).unwrap();
        encoded.drain(4..6);
        encoded[..4].copy_from_slice(b"H4SP");
        bytes.extend(encoded);
    }
    let recovered = recover_prefix(&bytes);
    assert!(recovered.records.is_empty());
    assert_eq!(recovered.valid_bytes, 0);
    assert!(
        recovered
            .damage
            .as_deref()
            .unwrap()
            .contains("unsupported spool record magic")
    );
    assert!(Replay::from_recovered(CAMPAIGN.into(), &[(0, recovered)]).is_err());
}

#[test]
fn unsupported_versions_are_refused_before_decoding_frames() {
    let replay = Replay::from_records(CAMPAIGN.into(), &[records(1, &[1])]).unwrap();
    for version in [0, 90, 95, FORMAT_VERSION + 1, u16::MAX] {
        let mut bytes = replay.bytes().to_vec();
        bytes[4..6].copy_from_slice(&version.to_le_bytes());
        let error = Replay::open(bytes).err().unwrap().to_string();
        assert_eq!(error, FormatVersion::read(version).unwrap_err().to_string());
    }
}

#[test]
fn corrupt_block_does_not_hide_an_unrelated_block() {
    let replay = Replay::from_records(CAMPAIGN.into(), &[records(1, &[1, 2, 721])]).unwrap();
    let first = &replay.index[0];
    let mut bytes = replay.bytes().to_vec();
    bytes[(first.offset + first.length - 1) as usize] ^= 0x40;
    let damaged = Replay::open(bytes).unwrap();
    assert!(
        matches!(damaged.seek(1, 1).unwrap(), SeekResult::Gap { reason, .. } if reason.contains("unreadable block"))
    );
    assert!(
        matches!(damaged.seek(1, 721).unwrap(), SeekResult::Frame(frame) if *frame == source(1, 721))
    );
    assert!(damaged.into_history().is_err());
}

#[test]
fn a_large_block_decodes_in_parallel_to_the_frames_it_holds() {
    let hours: Vec<i32> = (1..=200).collect();
    let replay = Replay::from_records(CAMPAIGN.into(), &[records(1, &hours)]).unwrap();
    assert_eq!(replay.index.len(), 1);
    let started = std::sync::atomic::AtomicUsize::new(0);
    let on_worker = || {
        started.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    };
    let parallel = Parallel {
        workers: 4,
        on_worker: &on_worker,
    };
    let frames = replay
        .read_block_parallel(&replay.index[0], &parallel)
        .unwrap();
    assert_eq!(frames, replay.read_block(&replay.index[0]).unwrap());
    assert_eq!(started.load(std::sync::atomic::Ordering::SeqCst), 4);
    assert_eq!(
        frames.iter().map(|frame| frame.hour).collect::<Vec<_>>(),
        hours
    );
    let mut decoder = FrameDecoder::new();
    for (frame, hour) in frames.iter().zip(&hours) {
        assert_eq!(*decoder.advance(frame).unwrap(), source(1, *hour));
    }
}

#[test]
fn parallel_map_keeps_input_order() {
    let parallel = Parallel {
        workers: 8,
        on_worker: &|| {},
    };
    let items: Vec<u32> = (0..5_000).collect();
    let out = parallel_map(&items, &parallel, |item| u64::from(*item) * 7);
    assert!(
        out.iter()
            .enumerate()
            .all(|(at, value)| *value == at as u64 * 7)
    );
    assert!(parallel_map(&[] as &[u32], &parallel, |item| *item).is_empty());
}

#[test]
fn parallel_payloads_report_the_first_bad_frame_in_position_order() {
    let hours: Vec<i32> = (1..=64).collect();
    let mut encoder = FrameEncoder::new();
    let mut payloads: Vec<Vec<u8>> = hours
        .iter()
        .map(|hour| {
            let mut bytes = Vec::new();
            encoder
                .encode(source(1, *hour), false)
                .unwrap()
                .encode(&mut bytes)
                .unwrap();
            bytes
        })
        .collect();
    // Two damaged frames, the later one first in any thread's run.
    payloads[40].truncate(3);
    payloads[9].push(0);
    let slices: Vec<&[u8]> = payloads.iter().map(Vec::as_slice).collect();
    let sequential = decode_payloads(&slices, None, None, FormatVersion::CURRENT);
    let parallel = decode_payloads(
        &slices,
        None,
        Some(&Parallel {
            workers: 4,
            on_worker: &|| {},
        }),
        FormatVersion::CURRENT,
    );
    let first_error = |results: &[io::Result<Frame>]| {
        results
            .iter()
            .position(Result::is_err)
            .map(|at| (at, results[at].as_ref().unwrap_err().to_string()))
    };
    assert_eq!(first_error(&parallel), first_error(&sequential));
    assert_eq!(first_error(&parallel).map(|(at, _)| at), Some(9));
}

/// A block whose raw bytes are rewritten by `edit`, recompressed in place of
/// the original.
fn edited_block(replay: &Replay, edit: impl FnOnce(&mut Vec<u8>)) -> Replay {
    let block = &replay.index[0];
    let compressed = &replay.bytes()[block.offset as usize..(block.offset + block.length) as usize];
    let mut raw = zstd::decode_all(compressed).unwrap();
    edit(&mut raw);
    let recompressed = zstd::encode_all(raw.as_slice(), 3).unwrap();
    let mut prefix = replay.bytes()[..replay.header_end].to_vec();
    let mut index = replay.index.clone();
    index[0].offset = prefix.len() as u64;
    index[0].length = recompressed.len() as u64;
    index[0].raw_length = raw.len() as u64;
    prefix.extend_from_slice(&recompressed);
    Replay::open(rebuild(replay, prefix, &index)).unwrap()
}

#[test]
fn a_broken_payload_boundary_is_the_same_error_read_in_parallel() {
    let hours: Vec<i32> = (1..=64).collect();
    let replay = Replay::from_records(CAMPAIGN.into(), &[records(1, &hours)]).unwrap();
    let parallel = Parallel {
        workers: 4,
        on_worker: &|| {},
    };
    let both = |replay: &Replay| {
        let sequential = replay.read_block(&replay.index[0]).map(|_| ());
        let parallel = replay
            .read_block_parallel(&replay.index[0], &parallel)
            .map(|_| ());
        (
            sequential.map_err(|error| error.to_string()),
            parallel.map_err(|error| error.to_string()),
        )
    };
    // One more frame counted than the block holds: the boundary read runs
    // off the end after 64 good frames.
    let overcounted = edited_block(&replay, |raw| {
        let count = u32::from_le_bytes(raw[..4].try_into().unwrap()) + 1;
        raw[..4].copy_from_slice(&count.to_le_bytes());
    });
    let (sequential, parallel_read) = both(&overcounted);
    assert!(sequential.is_err());
    assert_eq!(sequential, parallel_read);
    // A payload length past the end of the block, in the middle of it.
    let overlong = edited_block(&replay, |raw| {
        let mut at = 4;
        for _ in 0..40 {
            let length = u32::from_le_bytes(raw[at..at + 4].try_into().unwrap()) as usize;
            at += 4 + length;
        }
        raw[at..at + 4].copy_from_slice(&u32::MAX.to_le_bytes());
    });
    let (sequential, parallel_read) = both(&overlong);
    assert!(sequential.is_err());
    assert_eq!(sequential, parallel_read);
}

#[test]
fn hostile_extents_raw_lengths_and_concatenated_frames_are_refused() {
    let replay = Replay::from_records(CAMPAIGN.into(), &[records(1, &[1, 2])]).unwrap();
    let mut index = replay.index.clone();
    index[0].offset = u64::MAX;
    assert!(
        Replay::open(rebuild(
            &replay,
            replay.bytes()[..replay.trailer_start].into(),
            &index
        ))
        .is_err()
    );
    index = replay.index.clone();
    index[0].raw_length = MAX_BLOCK_RAW_BYTES as u64 + 1;
    assert!(
        Replay::open(rebuild(
            &replay,
            replay.bytes()[..replay.trailer_start].into(),
            &index
        ))
        .is_err()
    );
    index = replay.index.clone();
    index[0].raw_length = 4;
    let short = Replay::open(rebuild(
        &replay,
        replay.bytes()[..replay.trailer_start].into(),
        &index,
    ))
    .unwrap();
    assert!(
        matches!(short.seek(1, 1).unwrap(), SeekResult::Gap { reason, .. } if reason.contains("raw length"))
    );
    index = replay.index.clone();
    let mut prefix = replay.bytes()[..replay.trailer_start].to_vec();
    prefix.extend_from_slice(&replay.bytes()[replay.header_end..replay.trailer_start]);
    index[0].length *= 2;
    let concatenated = Replay::open(rebuild(&replay, prefix, &index)).unwrap();
    assert!(
        matches!(concatenated.seek(1, 1).unwrap(), SeekResult::Gap { reason, .. } if reason.contains("concatenated"))
    );
    index = replay.index.clone();
    index[0].last_hour = 3;
    let wrong_range = Replay::open(rebuild(
        &replay,
        replay.bytes()[..replay.trailer_start].into(),
        &index,
    ))
    .unwrap();
    assert!(
        matches!(wrong_range.seek(1, 3).unwrap(), SeekResult::Gap { reason, .. } if reason.contains("range mismatch"))
    );
    let mut bytes = replay.bytes().to_vec();
    bytes[replay.trailer_start + 5] ^= 1;
    assert!(Replay::open(bytes).is_err());
}

#[test]
fn recovery_retains_tail_damage_without_inventing_an_hour() {
    let records = records(1, &[1, 2]);
    let mut bytes: Vec<_> = records
        .iter()
        .flat_map(|record| encode_record(record).unwrap())
        .collect();
    bytes.pop();
    let recovery = recover_prefix(&bytes);
    let expected = recovery.damage.clone().unwrap();
    let replay = Replay::from_recovered(CAMPAIGN.into(), &[(17, recovery)]).unwrap();
    assert_eq!(replay.recovery_notes.len(), 1);
    assert_eq!(replay.recovery_notes[0].segment, 17);
    assert_eq!(replay.recovery_notes[0].reason, expected);
    assert!(matches!(replay.seek(1, 1).unwrap(), SeekResult::Frame(_)));
    assert!(matches!(replay.seek(1, 2).unwrap(), SeekResult::Gap { .. }));
}

#[test]
fn recovered_frame_wins_over_uncertain_flush_gap_and_parent_claim_survives_damage() {
    let mut first = records(1, &[1, 2]);
    first.push(Record::Gap(Gap {
        branch: 1,
        first_hour: 2,
        last_hour: 2,
        reason: "flush failed after write".into(),
    }));
    let mut second = records(2, &[11]);
    if let Record::BranchStart(branch) = &mut second[1] {
        branch.loaded_hour = Some(10);
        branch.parent = Parentage::Probable {
            branch: 999,
            hour: 10,
        };
        branch.reason = BranchReason::LoadSignal;
    }
    let replay = Replay::from_records(CAMPAIGN.into(), &[first, second]).unwrap();
    assert!(
        matches!(replay.seek(1, 2).unwrap(), SeekResult::Frame(frame) if *frame == source(1, 2))
    );
    assert_eq!(replay.gaps[0].reason, "flush failed after write");
    assert_eq!(
        replay.branches[1].parent,
        Parentage::Probable {
            branch: 999,
            hour: 10
        }
    );
    assert!(matches!(replay.seek(2, 11).unwrap(), SeekResult::Frame(_)));
}

#[test]
fn recovery_orders_segments_numerically_and_rejects_duplicate_numbers() {
    let first = records(1, &[1]);
    let mut later = vec![Record::BranchStart(branch(1, 1))];
    let mut encoder = FrameEncoder::new();
    later.push(Record::Frame(
        encoder.encode(source(1, 2).clone(), true).unwrap(),
    ));
    let recovery = |records| Recovery {
        format: Some(FormatVersion::CURRENT),
        records,
        valid_bytes: 0,
        damage: None,
    };
    let replay = Replay::from_recovered(
        CAMPAIGN.into(),
        &[(8, recovery(later)), (2, recovery(first.clone()))],
    )
    .unwrap();
    assert!(matches!(replay.seek(1, 2).unwrap(), SeekResult::Frame(_)));
    assert!(
        Replay::from_recovered(
            CAMPAIGN.into(),
            &[(2, recovery(first.clone())), (2, recovery(first))]
        )
        .is_err()
    );
}

#[test]
fn finalization_observer_sees_decoded_failures_and_identical_wire_sizes() {
    let mut input = records(1, &[1, 2, 3, 4]);
    if let Record::Frame(frame) = &mut input[3] {
        frame.tables[TableId::Provinces.index()] =
            crate::frame::TableUpdate::Unchanged { baseline: 999 };
    }
    if let Record::Frame(frame) = &mut input[5] {
        frame.tables[TableId::Provinces.index()] =
            TableUpdate::Snapshot(TableRows::Provinces(Vec::new()));
    }
    let mut sequential = FrameDecoder::new();
    let expected: Vec<_> = input
        .iter()
        .filter_map(|record| match record {
            Record::Frame(frame) => Some(sequential.apply(frame).unwrap()),
            _ => None,
        })
        .collect();
    let segments = [(
        0,
        Recovery {
            format: Some(FormatVersion::CURRENT),
            records: input,
            valid_bytes: 0,
            damage: None,
        },
    )];
    let mut observed = Vec::new();
    let replay =
        Replay::from_recovered_with(CAMPAIGN.into(), &segments, |frame, decoded, sizes| {
            assert_eq!(sizes, frame.table_encoded_sizes()?);
            observed.push(decoded.clone());
            Ok(())
        })
        .unwrap();
    assert_eq!(observed, expected);
    assert_eq!(
        observed.iter().map(|frame| frame.hour).collect::<Vec<_>>(),
        [1, 2, 3, 4]
    );
    assert!(
        matches!(&*observed[1].tables[TableId::Provinces.index()], TableView::Unavailable { reason } if reason.contains("999"))
    );
    assert!(
        matches!(&*observed[2].tables[TableId::Provinces.index()], TableView::Unavailable { reason } if reason.contains("2"))
    );
    assert_eq!(
        observed[3].tables[TableId::Provinces.index()],
        TableView::Complete(TableRows::Provinces(Vec::new()))
    );
    for frame in &observed {
        assert!(
            matches!(replay.seek(frame.branch, frame.hour).unwrap(), SeekResult::Frame(decoded) if *decoded == *frame)
        );
    }
    assert_eq!(
        replay.bytes(),
        Replay::from_recovered(CAMPAIGN.into(), &segments)
            .unwrap()
            .bytes()
    );

    let next = [(
        1,
        Recovery {
            format: Some(FormatVersion::CURRENT),
            records: records(2, &[4, 5]),
            valid_bytes: 0,
            damage: None,
        },
    )];
    let mut appended_hours = Vec::new();
    let appended = replay
        .append_recovered_with(&next, |frame, _, _| {
            appended_hours.push((frame.branch, frame.hour));
            Ok(())
        })
        .unwrap();
    assert_eq!(appended_hours, [(2, 4), (2, 5)]);
    assert_eq!(
        appended.bytes(),
        replay.append_recovered(&next).unwrap().bytes()
    );
    assert!(
        replay
            .append_recovered_with(&next, |_, _, _| Err(io::Error::other("report aborted")))
            .is_err()
    );
    // Failed table baselines affect table visibility, not the recorded hour
    // used as exact evidence when resolving a later branch's parentage.
    let history = replay.into_history().unwrap();
    assert_eq!(history.branches[0].hours, [1, 2, 3, 4]);
    let mut tracker = history.tracker_for(&session(2)).unwrap();
    assert_eq!(
        tracker
            .observe(Some(2), 3, Some(0), &[])
            .unwrap()
            .start
            .unwrap()
            .parent,
        Parentage::Probable { branch: 1, hour: 2 }
    );
}

struct MemoryWriter {
    bytes: Vec<u8>,
    fail_after: Option<usize>,
}
impl Write for MemoryWriter {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        if self.fail_after == Some(0) {
            return Err(io::Error::other("partial write failure"));
        }
        let count = self
            .fail_after
            .map_or(bytes.len(), |remaining| remaining.min(bytes.len()));
        if let Some(remaining) = &mut self.fail_after {
            *remaining -= count;
        }
        self.bytes.extend_from_slice(&bytes[..count]);
        Ok(count)
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}
struct MemoryPublisher {
    existing: Vec<u8>,
    fail_after: Option<usize>,
    fail_create: bool,
    fail_sync: bool,
    fail_replace: bool,
    synced: bool,
}
impl MemoryPublisher {
    fn new(existing: &[u8]) -> Self {
        Self {
            existing: existing.into(),
            fail_after: None,
            fail_create: false,
            fail_sync: false,
            fail_replace: false,
            synced: false,
        }
    }
}
impl AtomicPublisher for MemoryPublisher {
    type Writer = MemoryWriter;
    fn create(&mut self) -> io::Result<MemoryWriter> {
        if self.fail_create {
            return Err(io::Error::other("create failure"));
        }
        Ok(MemoryWriter {
            bytes: Vec::new(),
            fail_after: self.fail_after,
        })
    }
    fn sync(&mut self, _: &mut MemoryWriter) -> io::Result<()> {
        if self.fail_sync {
            return Err(io::Error::other("sync failure"));
        }
        self.synced = true;
        Ok(())
    }
    fn replace(&mut self, writer: MemoryWriter) -> io::Result<()> {
        assert!(self.synced);
        if self.fail_replace {
            return Err(io::Error::other("replace failure"));
        }
        self.existing = writer.bytes;
        Ok(())
    }
}

#[test]
fn every_interrupted_append_preserves_previous_file_byte_for_byte() {
    let old = Replay::from_records(CAMPAIGN.into(), &[records(1, &[1, 2])]).unwrap();
    let appended = old.append(&[records(2, &[3, 4])]).unwrap();
    let mut stages = Vec::new();
    let mut success = MemoryPublisher::new(old.bytes());
    appended
        .publish_with(&mut success, |stage| {
            stages.push(stage);
            Ok(())
        })
        .unwrap();
    assert_eq!(success.existing, appended.bytes());
    for fail_stage in stages {
        let mut disk = MemoryPublisher::new(old.bytes());
        assert!(
            appended
                .publish_with(&mut disk, |stage| if stage == fail_stage {
                    Err(io::Error::other("interrupted"))
                } else {
                    Ok(())
                })
                .is_err()
        );
        assert_eq!(disk.existing, old.bytes());
    }
    for fail_after in 0..appended.bytes().len() {
        let mut disk = MemoryPublisher::new(old.bytes());
        disk.fail_after = Some(fail_after);
        assert!(appended.publish_with(&mut disk, |_| Ok(())).is_err());
        assert_eq!(disk.existing, old.bytes());
    }
    for failure in 0..3 {
        let mut disk = MemoryPublisher::new(old.bytes());
        disk.fail_create = failure == 0;
        disk.fail_sync = failure == 1;
        disk.fail_replace = failure == 2;
        assert!(appended.publish_with(&mut disk, |_| Ok(())).is_err());
        assert_eq!(disk.existing, old.bytes());
    }
}

#[test]
fn format97_replay_opens_seeks_and_decodes_selected_and_parallel_blocks() {
    let hours = (1..=64).collect::<Vec<_>>();
    let current = Replay::from_records(CAMPAIGN.into(), &[records(1, &hours)]).unwrap();
    let legacy = edited_block(&current, |raw| {
        raw.clear();
        raw.extend_from_slice(&64_u32.to_le_bytes());
        for hour in 1..=64 {
            let frame = crate::rows::format98_tests::empty_frame97(hour, hour == 1);
            raw.extend_from_slice(&(frame.len() as u32).to_le_bytes());
            raw.extend(frame);
        }
    });
    let mut bytes = legacy.bytes().to_vec();
    bytes[4..6].copy_from_slice(&97_u16.to_le_bytes());
    let legacy = Replay::open(bytes).unwrap();
    assert_eq!(legacy.format.version(), 97);
    assert!(
        legacy
            .append(&[])
            .err()
            .unwrap()
            .to_string()
            .contains(&format!("Replay append requires format {FORMAT_VERSION}"))
    );
    let frames = legacy.read_block(&legacy.index[0]).unwrap();
    assert_eq!(frames.len(), 64);
    assert_eq!(
        legacy
            .read_block_parallel(
                &legacy.index[0],
                &Parallel {
                    workers: 4,
                    on_worker: &|| {},
                },
            )
            .unwrap(),
        frames
    );
    let selected = legacy
        .read_block_selected(&legacy.index[0], &[TableId::AvailableAdvisors])
        .unwrap();
    assert!(
        matches!(&selected[0].tables[TableId::AvailableAdvisors.index()], TableUpdate::Unavailable { reason } if reason.contains("not recorded"))
    );
    assert!(matches!(legacy.seek(1, 64).unwrap(), SeekResult::Frame(frame) if frame.hour == 64));
}
#[path = "compatibility_tests.rs"]
mod compatibility_tests;
