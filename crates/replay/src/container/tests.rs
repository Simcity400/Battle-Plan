#[path = "aliases_tests.rs"]
mod aliases_tests;
use super::*;
#[path = "file_backed_tests.rs"]
mod file_backed_tests;
use crate::{
    branches::Parentage,
    frame::{FrameEncoder, Metrics, TableUpdate},
    identity::Roster,
    rows::{CampaignRow, ClockRow, SunkShipsRow, WorldTensionRow},
    spool::{encode_record, recover_prefix},
};

pub(super) const CAMPAIGN: &[u8] = b"container-test-campaign";

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

pub(super) fn source(branch: u64, hour: i32) -> DecodedFrame {
    let mut tables = std::array::from_fn(|index| {
        let table = TableId::ALL[index];
        TableView::Complete(if table.key_bits() == 0 {
            TableRows::one_default(table)
        } else {
            TableRows::empty(table)
        })
    });
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
    tables[TableId::DesignObserverContext.index()] =
        TableView::Complete(TableRows::DesignObserverContext(vec![Default::default()]));
    tables[TableId::MapEvents.index()] =
        TableView::Complete(TableRows::MapEvents(vec![Default::default()]));
    crate::frame::retire_absent(DecodedFrame {
        branch,
        hour,
        metrics: Metrics::default(),
        tables: tables.map(Into::into),
    })
}

#[test]
fn source_completes_catalogued_singletons_without_admitting_empty_design_context() {
    let expected = source(1, 42);
    for table in TableId::ALL
        .into_iter()
        .filter(|table| table.key_bits() == 0)
    {
        let TableView::Complete(rows) = &*expected.tables[table.index()] else {
            panic!("fixture singleton {table:?} is not complete");
        };
        assert_eq!(rows.len(), 1, "fixture singleton {table:?}");
        assert_eq!(
            rows.first().unwrap().key(),
            0,
            "fixture singleton {table:?}"
        );
    }
    FrameEncoder::new().encode(expected.clone(), false).unwrap();
    let mut invalid = expected;
    *invalid.tables[TableId::DesignObserverContext.index()] =
        TableView::Complete(TableRows::DesignObserverContext(vec![]));
    let error = FrameEncoder::new().encode(invalid, false).unwrap_err();
    assert!(
        error
            .to_string()
            .contains("complete singleton table must contain exactly one row")
    );
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
    // A selected seek stops at its hour, certifies its block only once it has read through
    // the block's last hour, and rebuilds only what was asked for (with the campaign every
    // frame is checked against).
    let tables = [TableId::EquipmentDestruction];
    let fresh = Replay::open(replay.bytes().to_vec()).unwrap();
    for (i, (amounts, _)) in views.iter().enumerate() {
        let hour = i as i32 + 1;
        let SeekResult::Frame(frame) = fresh.seek_selected(1, hour, Some(&tables)).unwrap() else {
            panic!("missing hour");
        };
        assert_eq!(
            fresh.is_verified(&fresh.index[0]),
            hour == fresh.index[0].last_hour
        );
        assert_eq!(
            &*frame.tables[TableId::EquipmentDestruction.index()],
            amounts
        );
        assert!(matches!(
            &*frame.tables[TableId::Campaign.index()],
            TableView::Complete(_)
        ));
        assert!(matches!(
            &*frame.tables[TableId::DivisionEquipmentDeletions.index()],
            TableView::Unavailable { .. }
        ));
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

pub(super) fn records(id: u64, hours: &[i32]) -> Vec<Record> {
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

/// The same recording with every block rewritten in the interleaved layout.
pub(super) fn interleaved_copy(replay: &Replay) -> Replay {
    let mut prefix = replay.bytes()[..replay.header_end].to_vec();
    let mut blocks = Vec::new();
    for block in &replay.index {
        let frames = replay.read_block(block).unwrap();
        let mut raw = RawBlock::new(&frames[0]);
        for frame in &frames {
            raw.push(frame).unwrap();
        }
        blocks.push(raw.compress().unwrap());
    }
    let mut index = Vec::new();
    append_blocks(&mut prefix, &mut index, &blocks).unwrap();
    Replay::open(rebuild(replay, prefix, &index)).unwrap()
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
    let mut table_sizes = Vec::new();
    let replay = Replay::from_record_slices(
        CAMPAIGN.into(),
        &[&input],
        &mut |frame, decoded, sizes| {
            assert_eq!(sizes, frame.table_encoded_sizes()?);
            observed.push(decoded.clone());
            table_sizes.push((frame.hour, sizes));
            Ok(())
        },
        None,
        limit,
    )
    .unwrap();
    // Each block keeps every table's plain updates within the bound; tables
    // under it share blocks with the table that splits them.
    assert!(replay.index.len() > 1);
    assert!(replay.index.len() < 20);
    for block in &replay.index {
        for table in TableId::ALL {
            let plain: usize = table_sizes
                .iter()
                .filter(|(hour, _)| (block.first_hour..=block.last_hour).contains(hour))
                .map(|(_, sizes)| sizes[table.index()])
                .sum();
            assert!(plain <= limit, "{table:?} {plain}");
        }
    }
    assert_eq!(observed, expected);
    // Blocks store which tables were read, not their read times.
    let expected: Vec<DecodedFrame> = expected
        .into_iter()
        .map(|mut frame| {
            frame.metrics = frame.metrics.stored();
            frame
        })
        .collect();
    assert!(
        expected
            .iter()
            .all(|frame| frame.metrics.table_ns[TableId::Countries.index()]
                == crate::frame::READ_TIME_NOT_RECORDED)
    );
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
fn blocks_keep_the_keyframe_cadence_while_each_table_stays_within_the_bound() {
    use crate::rows::{ArmyOrderRow, LexicalTokenRow};

    // Two tables re-send 5 KB each hour: together their ten hours exceed
    // the bound, and neither does alone, so the block is not split and its
    // hours need no second keyframe.
    let mut encoder = FrameEncoder::new();
    let mut input = vec![
        Record::SessionStart(session(1)),
        Record::BranchStart(branch(1, 1)),
    ];
    let mut expected = Vec::new();
    for hour in 1..=10 {
        let mut decoded = source(1, hour);
        *decoded.tables[TableId::LexicalTokens.index()] =
            TableView::Complete(TableRows::LexicalTokens(vec![LexicalTokenRow {
                token: 0,
                literal: (0..5_000)
                    .map(|at| (at * 7 + hour as usize * 13) as u8)
                    .collect(),
            }]));
        *decoded.tables[TableId::ArmyOrders.index()] =
            TableView::Complete(TableRows::ArmyOrders(vec![ArmyOrderRow {
                owner_group: 9,
                instance: 1,
                path: (0..1_250)
                    .map(|step| step * 31 + hour as u32 * 977)
                    .collect(),
                ..Default::default()
            }]));
        expected.push(decoded.clone());
        input.push(Record::Frame(encoder.encode(decoded, false).unwrap()));
    }
    let limit = 64 * 1024;
    let mut plain = [0_usize; TABLE_COUNT];
    let replay = Replay::from_record_slices(
        CAMPAIGN.into(),
        &[&input],
        &mut |_, _, sizes| {
            for (total, size) in plain.iter_mut().zip(sizes) {
                *total += size;
            }
            Ok(())
        },
        None,
        limit,
    )
    .unwrap();
    assert!(plain[TableId::LexicalTokens.index()] + plain[TableId::ArmyOrders.index()] > limit);
    assert!(plain.iter().all(|plain| *plain <= limit));
    assert_eq!(replay.index.len(), 1);
    let frames = replay.read_block(&replay.index[0]).unwrap();
    let mut decoder = FrameDecoder::new();
    for (frame, expected) in frames.iter().zip(&expected) {
        let mut expected = expected.clone();
        expected.metrics = expected.metrics.stored();
        assert_eq!(decoder.advance(frame).unwrap(), &expected);
    }
}

#[test]
fn size_driven_blocks_check_framing_and_refuse_unrepresentable_hours() {
    let input = records(1, &[1, 2]);
    let Record::Frame(first) = &input[2] else {
        unreachable!()
    };
    let mut payload = Vec::new();
    let sizes = first.encode_with_sizes(&mut payload).unwrap();
    // Grouped blocks bound each table's plain updates and the plain frame
    // headers (count, length and header of each frame); interleaved blocks
    // bound their whole plain frames.
    let block = grouped::BlockWriter::new(first);
    let header = payload.len() - sizes.iter().sum::<usize>() + 8;
    let exact = header.max(*sizes.iter().max().unwrap());
    assert!(!block.fits(payload.len(), &sizes, exact - 1));
    assert!(block.fits(payload.len(), &sizes, exact));
    assert!(!block.fits(usize::MAX, &sizes, MAX_BLOCK_RAW_BYTES));
    let mut legacy = RawBlock::new(first);
    let whole = payload.len() + 8;
    assert!(!legacy.fits(payload.len(), whole - 1));
    legacy.push_payload(first, &payload, whole).unwrap();
    assert!(!legacy.fits(usize::MAX, MAX_BLOCK_RAW_BYTES));
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
fn trailer_hour_span_covers_every_branch_without_reading_blocks() {
    let replay = Replay::from_records(
        CAMPAIGN.into(),
        &[records(1, &[5, 9, 721]), records(2, &[3, 4])],
    )
    .unwrap();
    let length = replay.bytes().len() as u64;
    let (partial, span) =
        Replay::read_trailer_summary(&mut std::io::Cursor::new(replay.bytes()), length).unwrap();
    assert!(partial.is_none());
    assert_eq!(span, Some((3, 721)));
    // Block payloads are not read: damaging one leaves the span readable.
    let mut damaged = replay.bytes().to_vec();
    let block = replay.index[0].offset as usize;
    damaged[block + 8] ^= 0xff;
    assert_eq!(
        Replay::read_trailer_summary(&mut std::io::Cursor::new(&damaged), length)
            .unwrap()
            .1,
        Some((3, 721))
    );
    // The checksum is left to full opening: damage the index entries do not
    // read leaves the span readable, as the trailer's terminal notice is.
    let trailer_byte = length as usize - FOOTER_SIZE - 1;
    damaged[trailer_byte] ^= 0xff;
    assert_eq!(
        Replay::read_trailer_summary(&mut std::io::Cursor::new(&damaged), length)
            .unwrap()
            .1,
        Some((3, 721))
    );
    // A trailer that cannot be read is refused rather than guessed.
    let footer = length as usize - FOOTER_SIZE;
    let trailer = u64::from_le_bytes(damaged[footer + 4..footer + 12].try_into().unwrap()) as usize;
    damaged[trailer] ^= 0xff;
    assert!(Replay::read_trailer_summary(&mut std::io::Cursor::new(&damaged), length).is_err());
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
    let damaged = Replay::open(invalid_trailer).unwrap();
    assert!(damaged.controls.is_empty());
    assert_eq!(damaged.damaged_parts, 1);
    assert_eq!(
        damaged.warnings,
        ["Unreadable controls: replay control refers to an unknown session"]
    );
    assert_eq!(damaged.index, replay.index);
    assert_eq!(
        damaged.read_block(&damaged.index[0]).unwrap(),
        replay.read_block(&replay.index[0]).unwrap()
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
    let grouped = Replay::from_records(CAMPAIGN.into(), &[records(1, &[1, 2, 721])]).unwrap();
    // An interleaved block's checksum, and a grouped block's later entries,
    // which no other block's keyframes anchor to.
    let interleaved = interleaved_copy(&grouped);
    let first = &interleaved.index[0];
    let mut checksum = interleaved.bytes().to_vec();
    checksum[(first.offset + first.length - 1) as usize] ^= 0x40;
    for bytes in [checksum, damage_later_entries(&grouped, 0)] {
        let damaged = Replay::open(bytes).unwrap();
        assert!(
            matches!(damaged.seek(1, 1).unwrap(), SeekResult::Gap { reason, .. } if reason.contains("unreadable block"))
        );
        assert!(
            matches!(damaged.seek(1, 721).unwrap(), SeekResult::Frame(frame) if *frame == source(1, 721))
        );
        assert!(damaged.into_history().is_err());
    }
}

/// The recording's bytes with the last byte of a grouped block's last section
/// of later entries flipped: that section's checksum.
fn damage_later_entries(replay: &Replay, position: usize) -> Vec<u8> {
    let block = &replay.index[position];
    let sections = replay.block_sections(block).unwrap().unwrap();
    let last = sections
        .iter()
        .rposition(|section| matches!(section.section, Section::Later(_)))
        .unwrap();
    let after: u64 = sections[last + 1..]
        .iter()
        .map(|section| section.compressed)
        .sum();
    let mut bytes = replay.bytes().to_vec();
    bytes[(block.offset + block.length - after - 1) as usize] ^= 0x40;
    bytes
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
fn parallel_decode_workers_keep_the_callers_job_before_setup() {
    use job_priority::JobPriority;
    use std::sync::{
        Arc,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    };

    let background = Arc::new(AtomicBool::new(true));
    JobPriority::run(Arc::clone(&background), || {
        for expected in [true, false] {
            background.store(expected, Ordering::Release);
            let started = AtomicUsize::new(0);
            let setup = || {
                assert_eq!(JobPriority::current().background(), expected);
                started.fetch_add(1, Ordering::Relaxed);
            };
            let items: Vec<u32> = (0..128).collect();
            let result = parallel_map(
                &items,
                &Parallel {
                    workers: 4,
                    on_worker: &setup,
                },
                |item| {
                    assert_eq!(JobPriority::current().background(), expected);
                    *item
                },
            )
            .unwrap();
            assert_eq!(result, items);
            assert_eq!(started.load(Ordering::Relaxed), 4);
        }
    });
}

#[test]
fn parallel_map_keeps_input_order() {
    let parallel = Parallel {
        workers: 8,
        on_worker: &|| {},
    };
    let items: Vec<u32> = (0..5_000).collect();
    let out = parallel_map(&items, &parallel, |item| u64::from(*item) * 7).unwrap();
    assert!(
        out.iter()
            .enumerate()
            .all(|(at, value)| *value == at as u64 * 7)
    );
    assert!(
        parallel_map(&[] as &[u32], &parallel, |item| *item)
            .unwrap()
            .is_empty()
    );
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
    let sequential = decode_payloads(&slices, None, FormatVersion::CURRENT).unwrap();
    let parallel = decode_payloads(
        &slices,
        Some(&Parallel {
            workers: 4,
            on_worker: &|| {},
        }),
        FormatVersion::CURRENT,
    )
    .unwrap();
    let first_error = |results: &[io::Result<Frame>]| {
        results
            .iter()
            .position(Result::is_err)
            .map(|at| (at, results[at].as_ref().unwrap_err().to_string()))
    };
    assert_eq!(first_error(&parallel), first_error(&sequential));
    assert_eq!(first_error(&parallel).map(|(at, _)| at), Some(9));
}

/// The interleaved raw bytes of a block: its frame count, then each frame
/// prefixed by its length, as older recordings store them.
fn interleaved_raw(frames: &[Frame]) -> Vec<u8> {
    let mut block = RawBlock::new(&frames[0]);
    for frame in frames {
        block.push(frame).unwrap();
    }
    let mut raw = block.count.to_le_bytes().to_vec();
    raw.extend_from_slice(&block.frames);
    raw
}

/// The first block replaced by an interleaved block of its frames whose raw
/// bytes are then rewritten by `edit`.
fn edited_block(replay: &Replay, edit: impl FnOnce(&mut Vec<u8>)) -> Replay {
    let block = &replay.index[0];
    let mut raw = interleaved_raw(&replay.read_block(block).unwrap());
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
fn a_seek_that_stops_early_still_refuses_a_block_broken_after_its_hour() {
    let hours: Vec<i32> = (1..=64).collect();
    let replay = Replay::from_records(CAMPAIGN.into(), &[records(1, &hours)]).unwrap();
    let unreadable = |replay: &Replay, hour| matches!(replay.seek(1, hour).unwrap(), SeekResult::Gap { reason, .. } if reason.contains("unreadable block"));
    // One more frame counted than the block holds: the frame at hour 1 decodes, but
    // the block as a whole does not, so no hour of it is trusted.
    let overcounted = edited_block(&replay, |raw| {
        let count = u32::from_le_bytes(raw[..4].try_into().unwrap()) + 1;
        raw[..4].copy_from_slice(&count.to_le_bytes());
    });
    assert!(unreadable(&overcounted, 1));
    assert!(
        unreadable(&overcounted, 1),
        "a refused block is never marked verified"
    );
    // Blocks whose frames end before the hour range the index claims: the
    // early hours decode, the block's end check fails, even after a whole read.
    let mut index = replay.index.clone();
    index[0].last_hour = 65;
    let wrong_range = Replay::open(rebuild(
        &replay,
        replay.bytes()[..replay.trailer_start].into(),
        &index,
    ))
    .unwrap();
    assert!(wrong_range.read_block(&wrong_range.index[0]).is_err());
    assert!(unreadable(&wrong_range, 1));
    // An intact block, once read whole, may be left partway on later seeks.
    assert!(replay.read_block(&replay.index[0]).is_ok());
    assert!(matches!(replay.seek(1, 1).unwrap(), SeekResult::Frame(_)));
}

#[test]
fn hostile_extents_raw_lengths_and_concatenated_frames_become_gaps() {
    let replay = Replay::from_records(CAMPAIGN.into(), &[records(1, &[1, 2])]).unwrap();
    let mut index;
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
fn recovered_recordings_refuse_every_rewrite_and_append_without_touching_the_file() {
    let healthy = Replay::from_records(CAMPAIGN.into(), &[records(1, &[1, 2])]).unwrap();
    assert!(!healthy.needed_recovery());
    let directory =
        std::env::temp_dir().join(format!("battle-plan-refuse-rewrite-{}", std::process::id()));
    std::fs::create_dir_all(&directory).unwrap();
    let destination = directory.join("existing.battleplan");
    let next = records(2, &[3]);
    let segments = [(
        0,
        Recovery {
            records: next.clone(),
            format: Some(FormatVersion::CURRENT),
            valid_bytes: 0,
            damage: None,
        },
    )];
    let message = "This recording is damaged, so it can't be added to; start a new file.";
    for case in ["skipped-index", "unindexed", "checksum", "withheld-control"] {
        let mut bytes = healthy.bytes().to_vec();
        match case {
            "skipped-index" => {
                let mut index = healthy.index.clone();
                index[0].raw_length = 3;
                bytes = rebuild(&healthy, bytes[..healthy.trailer_start].into(), &index);
            }
            "unindexed" => {
                let mut prefix = bytes[..healthy.trailer_start].to_vec();
                prefix.extend_from_slice(b"unindexed");
                bytes = rebuild(&healthy, prefix, &healthy.index);
            }
            "checksum" => *bytes.last_mut().unwrap() ^= 1,
            "withheld-control" => {
                bytes.truncate(healthy.trailer_start);
                finish_file(
                    &mut bytes,
                    &healthy.sessions,
                    &healthy.branches,
                    &healthy.gaps,
                    &[ControlRecord::Detach {
                        session: 99,
                        last_hour: None,
                    }],
                    &[],
                    &healthy.index,
                )
                .unwrap();
            }
            _ => unreachable!(),
        }
        let recovered = Replay::open(bytes.clone()).unwrap();
        assert!(recovered.needed_recovery(), "{case}");
        assert_eq!(
            recovered
                .append(std::slice::from_ref(&next))
                .err()
                .unwrap()
                .to_string(),
            message,
            "{case}"
        );
        assert_eq!(
            recovered
                .append_recovered(&segments)
                .err()
                .unwrap()
                .to_string(),
            message,
            "{case}"
        );
        assert_eq!(
            recovered
                .append_recovered_with(&segments, |_, _, _| panic!("must not observe new frames"))
                .err()
                .unwrap()
                .to_string(),
            message,
            "{case}"
        );
        assert_eq!(
            recovered
                .append_recovered_with_reader(
                    &segments,
                    |_, _| panic!("must not read new frames"),
                    |_, _, _| panic!("must not observe new frames")
                )
                .err()
                .unwrap()
                .to_string(),
            message,
            "{case}"
        );
        let mut publisher = MemoryPublisher::new(&bytes);
        // Returning the damage error proves refusal happens before create.
        publisher.fail_create = true;
        assert_eq!(
            recovered
                .publish_with(&mut publisher, |_| panic!("must not publish"))
                .unwrap_err()
                .to_string(),
            message,
            "{case}"
        );
        assert_eq!(publisher.existing, bytes, "{case}");
        std::fs::write(&destination, &bytes).unwrap();
        assert_eq!(
            recovered
                .write_atomic(&destination)
                .unwrap_err()
                .to_string(),
            message,
            "{case}"
        );
        assert_eq!(std::fs::read(&destination).unwrap(), bytes, "{case}");
        assert!(!temporary_path(&destination).exists(), "{case}");
        assert_eq!(
            Replay::open(bytes.clone())
                .unwrap()
                .add_recovery_notes(&[])
                .err()
                .unwrap()
                .to_string(),
            message,
            "{case}"
        );
        assert_eq!(
            recovered.into_history().err().unwrap().to_string(),
            message,
            "{case}"
        );
    }
    std::fs::remove_file(destination).unwrap();
    std::fs::remove_dir(directory).unwrap();
}

#[test]
fn recorded_gap_and_table_diagnostics_are_not_shortened() {
    let reason = "recorded message: ".to_owned() + &"\u{e9}".repeat(2048);
    let mut input = records(1, &[1]);
    let Record::Frame(frame) = &mut input[2] else {
        unreachable!()
    };
    frame.tables[TableId::WorldTension.index()] = TableUpdate::Unavailable {
        reason: reason.clone(),
    };
    input.push(Record::Gap(Gap {
        branch: 1,
        first_hour: 2,
        last_hour: 3,
        reason: reason.clone(),
    }));
    let replay = Replay::from_records(CAMPAIGN.into(), &[input]).unwrap();
    let SeekResult::Gap { reason: actual, .. } = replay.seek(1, 2).unwrap() else {
        panic!("expected recorded gap")
    };
    assert_eq!(actual, reason);
    let SeekResult::Frame(frame) = replay.seek(1, 1).unwrap() else {
        panic!("expected readable frame")
    };
    assert_eq!(
        frame
            .complete::<crate::rows::tables::WorldTension>()
            .unwrap_err()
            .to_string(),
        format!("hour 1 WorldTension is unavailable: {reason}")
    );
}

#[test]
fn format97_replay_opens_seeks_and_decodes_selected_and_parallel_blocks() {
    let hours = (1..=64).collect::<Vec<_>>();
    let current = Replay::from_records(CAMPAIGN.into(), &[records(1, &hours)]).unwrap();
    let legacy = edited_block(&current, |raw| {
        raw.clear();
        raw.extend_from_slice(&64_u32.to_le_bytes());
        for hour in 1..=64 {
            let frame = replay_frame::test_support::empty_frame97(hour, hour == 1);
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
#[path = "grouped_tests.rs"]
mod grouped_tests;
#[path = "pack_campaign_tests.rs"]
mod pack_campaign_tests;

#[path = "security_tests.rs"]
mod security_tests;

#[test]
fn invalid_index_entries_are_skipped_without_changing_readable_blocks() {
    let replay = Replay::from_records(
        CAMPAIGN.into(),
        &[records(1, &[1, 2]), records(2, &[100, 101])],
    )
    .unwrap();
    for case in [
        "offset",
        "overflow",
        "zero-length",
        "overlap",
        "raw-short",
        "raw-large",
        "hours",
        "hour-span",
        "unknown-branch",
        "overlapping-hours",
    ] {
        let mut index = replay.index.clone();
        match case {
            "offset" => index[1].offset = u64::MAX,
            "overflow" => index[1].length = u64::MAX,
            "zero-length" => index[1].length = 0,
            "overlap" => index[1].offset = index[0].offset,
            "raw-short" => index[1].raw_length = 3,
            "raw-large" => index[1].raw_length = MAX_BLOCK_RAW_BYTES as u64 + 1,
            "hours" => index[1].last_hour = index[1].first_hour - 1,
            "hour-span" => index[1].last_hour = index[1].first_hour + KEYFRAME_INTERVAL as i32,
            "unknown-branch" => index[1].branch = 99,
            "overlapping-hours" => {
                index[1].branch = index[0].branch;
                index[1].first_hour = index[0].first_hour;
                index[1].last_hour = index[0].last_hour;
            }
            _ => unreachable!(),
        }
        let damaged = Replay::open(rebuild(
            &replay,
            replay.bytes()[..replay.trailer_start].into(),
            &index,
        ))
        .unwrap();
        assert_eq!(damaged.index, replay.index[..1], "{case}");
        assert_eq!(
            damaged.read_block(&damaged.index[0]).unwrap(),
            replay.read_block(&replay.index[0]).unwrap(),
            "{case}"
        );
        assert!(
            matches!(damaged.seek(2, 100).unwrap(), SeekResult::Gap { .. }),
            "{case}"
        );
        assert_eq!(damaged.damaged_parts, 2, "{case}");
        assert!(
            damaged
                .warnings
                .iter()
                .any(|reason| reason.contains("Skipped block")),
            "{case}"
        );
        if !matches!(
            case,
            "hours" | "hour-span" | "unknown-branch" | "overlapping-hours"
        ) {
            assert_eq!(damaged.gaps.last().unwrap().first_hour, 100);
            assert_eq!(damaged.damaged_range, Some((100, 101)));
        }
    }
}

#[test]
fn readable_trailers_warn_for_checksum_and_trailing_bytes_and_preserve_partial_status() {
    let mut input = records(1, &[1, 2]);
    input.push(Record::Control(ControlRecord::Partial {
        session: 1,
        reason: "synthetic terminal failure".into(),
        failed_hour: None,
    }));
    let replay = Replay::from_records(CAMPAIGN.into(), &[input]).unwrap();
    for case in ["checksum", "trailing"] {
        let mut bytes = replay.bytes().to_vec();
        if case == "checksum" {
            *bytes.last_mut().unwrap() ^= 1;
        } else {
            let footer = bytes.len() - FOOTER_SIZE;
            let mut tail = bytes.split_off(footer);
            bytes.extend_from_slice(b"extra");
            tail[12..20]
                .copy_from_slice(&((footer - replay.trailer_start + 5) as u64).to_le_bytes());
            tail[20..24]
                .copy_from_slice(&crc32fast::hash(&bytes[replay.trailer_start..]).to_le_bytes());
            bytes.extend(tail);
        }
        let length = bytes.len() as u64;
        assert_eq!(
            Replay::read_partial_from(&mut std::io::Cursor::new(&bytes), length)
                .unwrap()
                .as_ref(),
            replay.partial()
        );
        let damaged = Replay::open(bytes).unwrap();
        assert_eq!(damaged.partial(), replay.partial());
        assert_eq!(damaged.index, replay.index);
        assert_eq!(
            damaged.read_block(&damaged.index[0]).unwrap(),
            replay.read_block(&replay.index[0]).unwrap()
        );
        assert_eq!(damaged.damaged_parts, 0);
        assert_eq!(damaged.warnings.len(), 1);
    }
}

#[test]
fn a_skipped_first_index_entry_preserves_later_blocks() {
    // Interleaved blocks stand alone; grouped ones may anchor to the skipped
    // block (a_withheld_index_entry_never_lends_its_place_to_an_anchor).
    let healthy = interleaved_copy(
        &Replay::from_records(CAMPAIGN.into(), &[records(1, &[1, 721, 1441])]).unwrap(),
    );
    let mut index = healthy.index.clone();
    index[0].raw_length = 3;
    let recovered = Replay::open(rebuild(
        &healthy,
        healthy.bytes()[..healthy.trailer_start].to_vec(),
        &index,
    ))
    .unwrap();
    assert_eq!(recovered.index, healthy.index[1..]);
    assert_eq!(recovered.damaged_parts, 2);
    assert_eq!(recovered.damaged_range, Some((1, 1)));
    for (actual, expected) in recovered.index.iter().zip(&healthy.index[1..]) {
        assert_eq!(
            recovered.read_block(actual).unwrap(),
            healthy.read_block(expected).unwrap()
        );
    }
}

#[test]
fn a_withheld_index_entry_never_lends_its_place_to_an_anchor() {
    let healthy = Replay::from_records(CAMPAIGN.into(), &[records(1, &[1, 721, 1441])]).unwrap();
    assert_eq!(healthy.index.len(), 3);
    // Some table of the last block anchors its keyframe to the middle block.
    assert!(TableId::ALL.into_iter().any(|table| {
        grouped::keyframe_chain(&healthy, 2, table)
            .is_ok_and(|chain| chain.get(1).is_some_and(|&(at, _)| at == 1))
    }));
    let mut index = healthy.index.clone();
    index[1].raw_length = 3;
    let recovered = Replay::open(rebuild(
        &healthy,
        healthy.bytes()[..healthy.trailer_start].to_vec(),
        &index,
    ))
    .unwrap();
    assert_eq!(
        recovered.index,
        [healthy.index[0].clone(), healthy.index[2].clone()]
    );
    assert_eq!(
        recovered.read_block(&recovered.index[0]).unwrap(),
        healthy.read_block(&healthy.index[0]).unwrap()
    );
    // Counting back from the last block reaches the withheld block, not the first.
    let error = recovered.read_block(&recovered.index[1]).unwrap_err();
    assert!(
        error
            .to_string()
            .contains("keyframe anchor names no earlier block"),
        "{error}"
    );
}

#[test]
fn readable_index_survives_unindexed_bytes_and_conflicting_controls() {
    let replay = Replay::from_records(CAMPAIGN.into(), &[records(1, &[1, 2])]).unwrap();
    let expected = replay.read_block(&replay.index[0]).unwrap();
    let mut prefix = replay.bytes()[..replay.trailer_start].to_vec();
    prefix.extend_from_slice(b"unindexed");
    let damaged = Replay::open(rebuild(&replay, prefix, &replay.index)).unwrap();
    assert_eq!(damaged.read_block(&damaged.index[0]).unwrap(), expected);
    assert_eq!(damaged.damaged_parts, 1);
    assert_eq!(damaged.damaged_range, None);
    assert!(
        damaged
            .warnings
            .iter()
            .any(|reason| reason.contains("unindexed"))
    );
    let mut bytes = replay.bytes()[..replay.trailer_start].to_vec();
    finish_file(
        &mut bytes,
        &replay.sessions,
        &replay.branches,
        &[Gap {
            branch: 99,
            first_hour: 100,
            last_hour: 100,
            reason: "synthetic missing context".into(),
        }],
        &[],
        &[],
        &replay.index,
    )
    .unwrap();
    let damaged = Replay::open(bytes).unwrap();
    assert!(damaged.gaps.is_empty());
    assert_eq!(damaged.damaged_parts, 1);
    assert!(
        damaged
            .warnings
            .iter()
            .any(|reason| reason.contains("Unreadable gap metadata"))
    );
    assert_eq!(damaged.read_block(&damaged.index[0]).unwrap(), expected);
    for session in [1, 99] {
        let partial = ControlRecord::Partial {
            session,
            reason: "synthetic failure".into(),
            failed_hour: None,
        };
        let controls = if session == 1 {
            vec![partial.clone(), partial]
        } else {
            vec![partial]
        };
        let mut bytes = replay.bytes()[..replay.trailer_start].to_vec();
        finish_file(
            &mut bytes,
            &replay.sessions,
            &replay.branches,
            &replay.gaps,
            &controls,
            &[],
            &replay.index,
        )
        .unwrap();
        let length = bytes.len() as u64;
        assert_eq!(
            Replay::read_partial_from(&mut std::io::Cursor::new(&bytes), length).unwrap(),
            None
        );
        let damaged = Replay::open(bytes).unwrap();
        assert!(damaged.controls.is_empty());
        assert_eq!(damaged.damaged_parts, 1);
        assert!(
            damaged
                .warnings
                .iter()
                .any(|reason| reason.contains("Unreadable controls"))
        );
        assert_eq!(damaged.read_block(&damaged.index[0]).unwrap(), expected);
    }
}

#[test]
fn each_unindexed_region_counts_once_without_inventing_dates() {
    let healthy = Replay::from_records(CAMPAIGN.into(), &[records(1, &[1, 721])]).unwrap();
    let mut prefix = healthy.bytes()[..healthy.header_end].to_vec();
    let mut index = healthy.index.clone();
    for block in &mut index {
        prefix.extend_from_slice(b"unindexed");
        let compressed =
            &healthy.bytes()[block.offset as usize..(block.offset + block.length) as usize];
        block.offset = prefix.len() as u64;
        prefix.extend_from_slice(compressed);
    }
    prefix.extend_from_slice(b"unindexed tail");
    let recovered = Replay::open(rebuild(&healthy, prefix, &index)).unwrap();
    assert_eq!(recovered.damaged_parts, 3);
    assert_eq!(recovered.damaged_range, None);
    assert!(recovered.gaps.is_empty());
    assert_eq!(recovered.warnings.len(), 3);
    for (actual, expected) in recovered.index.iter().zip(&healthy.index) {
        assert_eq!(
            recovered.read_block(actual).unwrap(),
            healthy.read_block(expected).unwrap()
        );
    }
}

#[test]
fn invalid_length_prefixed_trailer_records_are_skipped_and_framing_stays_strict() {
    let base = Replay::from_records(CAMPAIGN.into(), &[records(1, &[1, 2])]).unwrap();
    let gaps = [
        Gap {
            branch: 1,
            first_hour: 3,
            last_hour: 4,
            reason: "bad".into(),
        },
        Gap {
            branch: 1,
            first_hour: 5,
            last_hour: 6,
            reason: "good".into(),
        },
    ];
    let controls = [
        ControlRecord::GapSummary {
            session: 1,
            dropped_frames: 2,
            reason: "bad".into(),
        },
        ControlRecord::Partial {
            session: 1,
            reason: "bad".into(),
            failed_hour: None,
        },
        ControlRecord::Detach {
            session: 1,
            last_hour: Some(2),
        },
    ];
    let notes = [
        RecoveryNote {
            session: Some(1),
            segment: 0,
            valid_bytes: 10,
            reason: "bad".into(),
        },
        RecoveryNote {
            session: Some(1),
            segment: 1,
            valid_bytes: 20,
            reason: "good".into(),
        },
    ];
    let mut healthy = base.bytes()[..base.trailer_start].to_vec();
    finish_file(
        &mut healthy,
        &base.sessions,
        &base.branches,
        &gaps,
        &controls,
        &notes,
        &base.index,
    )
    .unwrap();
    let footer = healthy.len() - FOOTER_SIZE;
    let mut trailer = Decoder::new(&healthy[base.trailer_start..footer]);
    trailer.take(4).unwrap();
    let mut lists = Vec::new();
    for _ in 0..5 {
        let count = trailer.count(4).unwrap();
        let mut ranges = Vec::new();
        for _ in 0..count {
            let length = trailer.u32().unwrap() as usize;
            let start = footer - trailer.remaining();
            trailer.take(length).unwrap();
            ranges.push(start..start + length);
        }
        lists.push(ranges);
    }
    for case in [
        "reversed-gap",
        "blank-gap",
        "zero-summary",
        "blank-partial",
        "unknown-control",
        "blank-recovery",
    ] {
        let mut bytes = healthy.clone();
        let (list, record, warning) = match case {
            "reversed-gap" | "blank-gap" => (2, 0, "Unreadable gap record"),
            "zero-summary" | "unknown-control" => (3, 0, "Unreadable control record"),
            "blank-partial" => (3, 1, "Unreadable control record"),
            "blank-recovery" => (4, 0, "Unreadable recovery note record"),
            _ => unreachable!(),
        };
        let range = &lists[list][record];
        match case {
            "reversed-gap" => {
                bytes[range.start + 12..range.start + 16].copy_from_slice(&2_i32.to_le_bytes())
            }
            "zero-summary" => bytes[range.start + 9..range.start + 17].fill(0),
            "blank-partial" => bytes[range.start + 13..range.start + 16].fill(b' '),
            "unknown-control" => bytes[range.start] = 255,
            _ => bytes[range.end - 3..range.end].fill(b' '),
        }
        let crc = crc32fast::hash(&bytes[base.trailer_start..footer]);
        bytes[footer + 20..].copy_from_slice(&crc.to_le_bytes());
        let partial =
            Replay::read_partial_from(&mut std::io::Cursor::new(&bytes), bytes.len() as u64)
                .unwrap();
        assert_eq!(
            partial.as_ref(),
            if case == "blank-partial" {
                None
            } else {
                Some(&controls[1])
            },
            "{case}"
        );
        let recovered = Replay::open(bytes).unwrap();
        assert_eq!(recovered.damaged_parts, 1, "{case}");
        assert_eq!(recovered.damaged_range, None, "{case}");
        assert_eq!(recovered.warnings.len(), 1, "{case}");
        assert!(recovered.warnings[0].starts_with(warning), "{case}");
        assert_eq!(
            recovered.gaps,
            if list == 2 {
                gaps[1..].to_vec()
            } else {
                gaps.to_vec()
            },
            "{case}"
        );
        assert_eq!(
            recovered.controls,
            controls
                .iter()
                .enumerate()
                .filter(|(position, _)| list != 3 || *position != record)
                .map(|(_, control)| control.clone())
                .collect::<Vec<_>>(),
            "{case}"
        );
        assert_eq!(
            recovered.recovery_notes,
            if list == 4 {
                notes[1..].to_vec()
            } else {
                notes.to_vec()
            },
            "{case}"
        );
        assert_eq!(recovered.index, base.index, "{case}");
        assert_eq!(
            recovered.read_block(&recovered.index[0]).unwrap(),
            base.read_block(&base.index[0]).unwrap(),
            "{case}"
        );
    }
    for list in [0, 1, 2, 3, 4] {
        let mut bytes = healthy.clone();
        let range = &lists[list][0];
        bytes[range.start - 4..range.start].copy_from_slice(&u32::MAX.to_le_bytes());
        let crc = crc32fast::hash(&bytes[base.trailer_start..footer]);
        bytes[footer + 20..].copy_from_slice(&crc.to_le_bytes());
        assert!(
            Replay::read_partial_from(&mut std::io::Cursor::new(&bytes), bytes.len() as u64)
                .is_err()
        );
        assert!(Replay::open(bytes).is_err());
    }
}

#[test]
fn noncontiguous_branches_and_sessions_warn_without_losing_valid_blocks() {
    // Reordered blocks: interleaved ones, since grouped keyframes count back
    // through the file's blocks to their anchors.
    let replay = interleaved_copy(
        &Replay::from_records(
            CAMPAIGN.into(),
            &[
                records(1, &[1, 1 + KEYFRAME_INTERVAL as i32]),
                records(2, &[100]),
            ],
        )
        .unwrap(),
    );
    assert_eq!(replay.index.len(), 3);
    let mut prefix = replay.bytes()[..replay.header_end].to_vec();
    let order = [0, 2, 1];
    let mut index = Vec::new();
    for position in order {
        let mut block = replay.index[position].clone();
        let compressed =
            &replay.bytes()[block.offset as usize..(block.offset + block.length) as usize];
        block.offset = prefix.len() as u64;
        prefix.extend_from_slice(compressed);
        index.push(block);
    }
    let modified = Replay::open(rebuild(&replay, prefix, &index)).unwrap();
    assert_eq!(modified.damaged_parts, 0);
    assert!(
        modified
            .warnings
            .iter()
            .any(|reason| reason.contains("branch blocks are not contiguous"))
    );
    assert!(
        modified
            .warnings
            .iter()
            .any(|reason| reason.contains("session blocks are not contiguous"))
    );
    for (new, old) in order.into_iter().enumerate() {
        assert_eq!(
            modified.read_block(&modified.index[new]).unwrap(),
            replay.read_block(&replay.index[old]).unwrap()
        );
    }
}

#[cfg(windows)]
#[path = "publication_tests.rs"]
mod publication_tests;
