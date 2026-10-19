use super::*;
use replay_data::rows::TABLE_COUNT;
use replay_data::{
    rows::CampaignRow, rows::ClockRow, rows::CountryRow, rows::DivisionStatusRow, rows::Recorded,
    rows::TableRows,
};
use replay_frame::{frame::Metrics, frame::SharedView};

const SELECTED: &[TableId] = &[
    TableId::Campaign,
    TableId::Clock,
    TableId::Countries,
    TableId::DivisionStatus,
];

fn frame() -> DecodedFrame {
    let mut frame = DecodedFrame {
        branch: 71,
        hour: 300,
        metrics: Metrics::default(),
        tables: std::array::from_fn(|_| {
            TableView::Unavailable {
                reason: "Table not selected for reconstruction".into(),
            }
            .into()
        }),
    };
    frame.tables[TableId::Campaign.index()] =
        TableView::Complete(TableRows::Campaign(vec![CampaignRow { id: vec![0, 255] }])).into();
    frame.tables[TableId::Clock.index()] = TableView::Complete(TableRows::Clock(vec![ClockRow {
        hour: 300,
        ..Default::default()
    }]))
    .into();
    frame.tables[TableId::Countries.index()] = TableView::Complete(TableRows::Countries(
        (1..=5_000)
            .map(|id| CountryRow {
                id,
                canonical_slot: id,
                tag: id.to_string().into_bytes(),
                political_power_raw: Some(i64::MAX - i64::from(id)),
                ..Default::default()
            })
            .collect(),
    ))
    .into();
    frame.tables[TableId::DivisionStatus.index()] = TableView::Truncated {
        rows: TableRows::DivisionStatus(vec![DivisionStatusRow {
            division: u64::MAX,
            maximum_hit_points: i64::MAX,
            maximum_organisation: i64::MIN,
            border_combat_stats: Recorded::NotRecorded,
            name: Recorded::Value(Some(vec![0, 255, 1])),
            ..Default::default()
        }]),
        missing: 50_001,
    }
    .into();
    let mut samples = [None; TABLE_COUNT];
    samples[TableId::Countries.index()] = Some(273);
    frame.restore_samples(samples).unwrap();
    frame
}

#[test]
fn checkpoint_retains_partial_census_old_fields_bytes_and_five_thousand_countries() {
    let source = frame();
    let position = Position {
        block: 91,
        next_frame: 7,
    };
    let bytes = encode_checkpoint(&source, position, SELECTED).unwrap();
    assert_eq!(header(bytes.as_slice()).unwrap(), (71, 300));
    let restored = decode_checkpoint(bytes.as_slice(), SELECTED).unwrap();
    assert_eq!(restored.frame, source);
    assert_eq!(restored.position, position);
    assert_eq!(
        restored.frame.tables[TableId::Countries.index()].sampled_at(),
        Some(273)
    );
    assert!(matches!(
        &*restored.frame.tables[TableId::DivisionStatus.index()],
        TableView::Truncated {
            missing: 50_001,
            ..
        }
    ));
    // A recording-style keyframe would turn this partial census into a refusal;
    // the prepared snapshot must retain every reached row and its count instead.
    assert_eq!(
        restored
            .frame
            .complete::<replay_data::rows::tables::Countries>()
            .unwrap()
            .len(),
        5_000
    );
}

#[test]
fn production_map_loader_retains_country_template_configuration_from_recording_and_checkpoint() {
    use crate::test_fixtures::{CAMPAIGN, records, scale_frame};
    use replay::container::{Replay, ReplayStream};
    use replay_data::rows::{CountryTemplateStateRow, GarrisonTemplateSelection, tables};
    let mut input = scale_frame(1, 100, 1);
    let row = CountryTemplateStateRow {
        country: 11,
        selected_staff: 52,
        lock_all: 1,
        next_hq_order: 7,
        garrison_priority: Recorded::Value(Some(-3)),
        garrisons: Recorded::Value(Some(
            vec![GarrisonTemplateSelection {
                occupied: Some(21),
                state: Some(0),
                template: Some(52),
            }]
            .into(),
        )),
    };
    input.tables[TableId::CountryTemplateState.index()] =
        TableView::Complete(TableRows::CountryTemplateState(vec![row.clone()])).into();
    let replay =
        Arc::new(Replay::from_records(CAMPAIGN.into(), &[records(1, vec![input])]).unwrap());
    let fixture = crate::prepared::series::tests::Fixture::new();
    // Both producer and playback use the production table selection.
    let selected = selected_tables();
    let mut builder =
        Builder::new(NonZeroU32::new(720).unwrap(), selected, Progress::default()).unwrap();
    builder.set_ranges(&replay);
    let mut stream = ReplayStream::new(&[TableId::Campaign]).unwrap();
    let mut writer = fixture.writer();
    stream
        .next_block_with(
            &replay,
            |raw, frame| {
                builder.accept_sparse(
                    &mut writer,
                    &replay,
                    &raw,
                    frame,
                    Position {
                        block: 0,
                        next_frame: 1,
                    },
                    || Ok(()),
                )
            },
            || Ok(()),
        )
        .unwrap();
    writer.commit().unwrap();
    let mut reader = Reader::new(
        Store::local(
            fixture
                .directory
                .file_name()
                .unwrap()
                .to_str()
                .unwrap()
                .parse()
                .unwrap(),
        )
        .unwrap(),
        fixture.key.clone(),
        builder.progress().clone(),
        replay,
        selected,
    )
    .unwrap();
    let Some(Current::Frame(frame)) = reader.seek(1, 100).unwrap() else {
        panic!("prepared frame");
    };
    assert_eq!(
        frame.complete::<tables::CountryTemplateState>().unwrap(),
        &[row]
    );
}

#[test]
fn checkpoint_preserves_unavailable_and_known_empty_without_inventing_reads() {
    let mut source = frame();
    source.tables[TableId::DivisionStatus.index()] = TableView::Unavailable {
        reason: "division status read failed twice".into(),
    }
    .into();
    let bytes = encode_checkpoint(
        &source,
        Position {
            block: 0,
            next_frame: 1,
        },
        SELECTED,
    )
    .unwrap();
    assert_eq!(
        decode_checkpoint(bytes.as_slice(), SELECTED).unwrap().frame,
        source
    );
    source.tables[TableId::DivisionStatus.index()] =
        TableView::Complete(TableRows::DivisionStatus(Vec::new())).into();
    let bytes = encode_checkpoint(
        &source,
        Position {
            block: 0,
            next_frame: 1,
        },
        SELECTED,
    )
    .unwrap();
    let restored = decode_checkpoint(bytes.as_slice(), SELECTED).unwrap();
    assert_eq!(restored.frame, source);
    assert_eq!(
        restored.frame.tables[TableId::DivisionStatus.index()].sampled_at(),
        None
    );
    assert!(!SharedView::ptr_eq(
        &source.tables[TableId::Countries.index()],
        &restored.frame.tables[TableId::Countries.index()]
    ));
}

#[test]
fn checkpoints_refuse_changed_selection_contract_and_trailing_payload() {
    let source = frame();
    let bytes = encode_checkpoint(
        &source,
        Position {
            block: 0,
            next_frame: 1,
        },
        SELECTED,
    )
    .unwrap();
    assert!(decode_checkpoint(bytes.as_slice(), &[TableId::Campaign]).is_err());
    let mut changed = bytes.clone();
    changed[36..38].copy_from_slice(&0u16.to_le_bytes());
    assert!(decode_checkpoint(changed.as_slice(), SELECTED).is_err());
    let mut trailing = bytes;
    trailing.push(0);
    assert!(decode_checkpoint(trailing.as_slice(), SELECTED).is_err());
    assert!(validate_selection(&[TableId::DivisionStatus]).is_err());
    assert!(validate_selection(&[TableId::Campaign, TableId::Campaign]).is_err());
}

#[test]
fn resume_reader_uses_and_validates_the_stored_selection() {
    let source = frame();
    let position = Position {
        block: 0,
        next_frame: 1,
    };
    let bytes = encode_checkpoint(&source, position, SELECTED).unwrap();
    let restored = decode_resume(bytes.as_slice()).unwrap();
    assert_eq!(restored.frame, source);
    assert_eq!(restored.position, position);
    assert!(restored.selects(TableId::DivisionStatus));
    assert!(!restored.selects(TableId::AirAces));
    let mut changed = bytes.clone();
    changed[38 + TableId::Countries.index()] = 0;
    assert!(decode_resume(changed.as_slice()).is_err());
    let mut trailing = bytes;
    trailing.push(0);
    assert!(decode_resume(trailing.as_slice()).is_err());
}

#[test]
fn checkpoint_compresses_native_state_and_rejects_corrupt_or_concatenated_frames() {
    let source = frame();
    let mut raw = Vec::new();
    source.encode_checkpoint(&mut raw).unwrap();
    let bytes = encode_checkpoint(
        &source,
        Position {
            block: 0,
            next_frame: 1,
        },
        SELECTED,
    )
    .unwrap();
    assert!(
        bytes.len() < raw.len() / 2,
        "the 5,000-country checkpoint must be compressed"
    );
    let payload = 38 + TableId::ALL.len();
    let mut corrupt = bytes.clone();
    let end = corrupt.len() - 1;
    corrupt[end] ^= 1;
    assert!(decode_checkpoint(corrupt.as_slice(), SELECTED).is_err());
    let mut concatenated = bytes.clone();
    concatenated.extend_from_slice(&bytes[payload + 8..]);
    assert!(decode_checkpoint(concatenated.as_slice(), SELECTED).is_err());
    let mut wrong_length = bytes;
    wrong_length[payload..payload + 8].copy_from_slice(&1u64.to_le_bytes());
    assert!(decode_checkpoint(wrong_length.as_slice(), SELECTED).is_err());
}

#[test]
fn naval_management_integration_prepared_map_retains_hover_inputs() {
    use replay_data::rows::{
        NavalRepairFleetRow, NavalRepairTaskForceRow, NavalTaskForceCompositionRow,
    };
    let selected = selected_tables();
    for table in [
        TableId::NavalRepairFleets,
        TableId::NavalRepairBases,
        TableId::NavalRepairQueue,
        TableId::NavalRepairTaskForces,
        TableId::NavalTaskForceComposition,
    ] {
        assert!(
            selected.contains(&table),
            "naval hover input {table:?} must be prepared"
        );
    }
    let mut source = frame();
    *source.tables[TableId::NavalRepairFleets.index()] =
        TableView::Complete(TableRows::NavalRepairFleets(vec![NavalRepairFleetRow {
            fleet: 9,
            automated_homebase: 1,
        }]));
    *source.tables[TableId::NavalRepairTaskForces.index()] =
        TableView::Complete(TableRows::NavalRepairTaskForces(vec![
            NavalRepairTaskForceRow {
                task_force: 7,
                repair_mode: -2,
                repair_split: 1,
                auto_reinforcement: 1,
                ..Default::default()
            },
        ]));
    *source.tables[TableId::NavalTaskForceComposition.index()] =
        TableView::Complete(TableRows::NavalTaskForceComposition(vec![
            NavalTaskForceCompositionRow {
                task_force: 7,
                kind: 0,
                ordinal: 0,
                sub_unit: 3,
                role: -1,
                definition: -1,
                amount: 12,
            },
        ]));
    *source.tables[TableId::NavalRepairBases.index()] =
        TableView::Complete(TableRows::NavalRepairBases(vec![
            replay_data::rows::NavalRepairBaseRow {
                province: 3,
                allocated_dockyards: 4,
                ..Default::default()
            },
        ]));
    *source.tables[TableId::NavalRepairQueue.index()] =
        TableView::Complete(TableRows::NavalRepairQueue(vec![
            replay_data::rows::NavalRepairQueueRow {
                province: 3,
                ordinal: 0,
                ship: 99,
            },
        ]));
    let encoded = encode_checkpoint(
        &source,
        Position {
            block: 0,
            next_frame: 1,
        },
        selected,
    )
    .unwrap();
    let restored = decode_checkpoint(encoded.as_slice(), selected).unwrap();
    for table in [
        TableId::NavalRepairFleets,
        TableId::NavalRepairBases,
        TableId::NavalRepairQueue,
        TableId::NavalRepairTaskForces,
        TableId::NavalTaskForceComposition,
    ] {
        assert_eq!(
            restored.frame.tables[table.index()],
            source.tables[table.index()]
        );
    }
}

#[test]
fn map_selection_keeps_every_authored_trigger_operand_and_omits_product_only_inputs() {
    let selected = selected_tables();
    validate_selection(selected).unwrap();
    for &table in crate::triggers::RECORDED_TABLES {
        assert!(
            selected.contains(&table),
            "authored trigger input {table:?} is required"
        );
    }
    for table in [
        TableId::BattleSounds,
        TableId::Clock,
        TableId::DivisionMovement,
        TableId::DivisionHistory,
        TableId::DivisionPanelStats,
        TableId::DivisionLandInputs,
        TableId::MapEvents,
        TableId::NuclearStrikes,
        TableId::TaskForceMapMotion,
        // The Operatives map mode's native icon inputs.
        TableId::OperativeMissions,
        TableId::StrategicOperatives,
        TableId::AgencyOperationBuffers,
        TableId::Operations,
        TableId::CountryStabilityInputs,
        // Movement inputs remain selected beside resources and trade routes.
        TableId::MovementStateTerms,
        TableId::CoreSupplyAccess,
        TableId::CountryUnitSpeedEntries,
        TableId::RiverRelations,
        TableId::DivisionTransportSpeeds,
        // The resources map mode and the trade routes.
        TableId::ConvoyClients,
        TableId::CountryOilFactors,
        TableId::RegionalConvoyEfficiency,
        TableId::ResourceExchanges,
        TableId::ResourceOrigins,
        TableId::ResourceRights,
        TableId::StateTemporaryResources,
        TableId::TradeRoutes,
    ] {
        assert!(selected.contains(&table));
    }
    // The deployed division XP view normalizes stored XP with the already recorded
    // complete manpower contributors; the career view needs its retained entries.
    assert!(selected.contains(&TableId::DivisionHistory));
    assert!(selected.contains(&TableId::DivisionLandInputs));
    assert!(selected.len() < TableId::ALL.len());
}

#[test]
fn gap_intervals_keep_overlap_order_duplicates_and_maximum_hour() {
    use replay_frame::branches::Gap;
    let gaps = [
        Gap {
            branch: 1,
            first_hour: 20,
            last_hour: 40,
            reason: "first".into(),
        },
        Gap {
            branch: 1,
            first_hour: 10,
            last_hour: 30,
            reason: "second".into(),
        },
        Gap {
            branch: 1,
            first_hour: 20,
            last_hour: 20,
            reason: "first".into(),
        },
        Gap {
            branch: 1,
            first_hour: Hour::MAX,
            last_hour: Hour::MAX,
            reason: "end".into(),
        },
    ];
    assert_eq!(
        gap_intervals(&gaps.iter().collect::<Vec<_>>()).unwrap(),
        vec![
            (10, 19, "second".into()),
            (20, 20, "first; second; first".into()),
            (21, 30, "first; second".into()),
            (31, 40, "first".into()),
            (Hour::MAX, Hour::MAX, "end".into()),
        ]
    );
}

#[test]
fn sparse_map_checkpoints_equal_hourly_bytes_across_short_blocks_gaps_branches_and_resume() {
    use crate::test_fixtures::{CAMPAIGN, continuation_records, records, scale_frame};
    use replay::{container::Replay, container::ReplayStream, container::test_work::table_decodes};
    let observed = |branch, hour| {
        let mut frame = scale_frame(branch, hour, 2);
        frame.metrics.table_ns[TableId::Countries.index()] = 1;
        frame.tables[TableId::DivisionStatus.index()] = match hour {
            104 => TableView::Truncated {
                rows: TableRows::DivisionStatus(vec![DivisionStatusRow {
                    division: 1,
                    ..Default::default()
                }]),
                missing: 7,
            },
            105 => TableView::Unavailable {
                reason: "synthetic failed census".into(),
            },
            _ => TableView::Complete(TableRows::DivisionStatus(vec![DivisionStatusRow {
                division: 1,
                maximum_hit_points: i64::from(hour) * 1000,
                ..Default::default()
            }])),
        }
        .into();
        frame
    };
    let frames = |branch, hours: &[i32]| hours.iter().map(|&hour| observed(branch, hour)).collect();
    let first =
        Replay::from_records(CAMPAIGN.into(), &[records(1, frames(1, &[100, 101]))]).unwrap();
    assert!(
        first.warnings.is_empty(),
        "synthetic recording warnings: {:?}",
        first.warnings
    );
    let mut replay = first
        .append(&[continuation_records(
            &first,
            1,
            frames(1, &[102, 104, 105, 106]),
        )])
        .unwrap();
    replay = replay.append(&[records(2, frames(2, &[20, 21]))]).unwrap();
    // A new session starts a new branch instead of re-entering session 1.
    replay = replay
        .append(&[records(3, frames(3, &[108, 109, 110]))])
        .unwrap();
    let appended = replay
        .append(&[continuation_records(
            &replay,
            3,
            frames(3, &[111, 112, 113]),
        )])
        .unwrap();
    let control = crate::prepared::series::tests::Fixture::new();
    let sparse = crate::prepared::series::tests::Fixture::new();
    let stride = NonZeroU32::new(3).unwrap();
    let mut full_map = Builder::new(stride, SELECTED, Progress::default()).unwrap();
    let mut sparse_map = Builder::new(stride, SELECTED, Progress::default()).unwrap();
    let mut full = ReplayStream::new(SELECTED).unwrap();
    let mut narrow = ReplayStream::new(&[TableId::Campaign]).unwrap();
    for block in 0..appended.index.len() {
        // The appended tail is first opened after accepting the earlier revision.
        let replay = if block < replay.index.len() {
            &replay
        } else {
            &appended
        };
        let mut writer = control.writer();
        let mut count = 0;
        full.next_block_with(
            replay,
            |raw, frame| {
                count += 1;
                full_map.accept(
                    &mut writer,
                    &raw,
                    frame,
                    Position {
                        block,
                        next_frame: count,
                    },
                )
            },
            || Ok(()),
        )
        .unwrap();
        writer.commit().unwrap();
        let mut writer = sparse.writer();
        let mut count = 0;
        let (_, decodes) = table_decodes(|| {
            narrow
                .next_block_with(
                    replay,
                    |raw, frame| {
                        count += 1;
                        sparse_map.accept_sparse(
                            &mut writer,
                            replay,
                            &raw,
                            frame,
                            Position {
                                block,
                                next_frame: count,
                            },
                            || Ok(()),
                        )
                    },
                    || Ok(()),
                )
                .unwrap()
        });
        writer.commit().unwrap();
        assert_eq!(
            sparse.payloads(),
            control.payloads(),
            "block {block}: every compressed checkpoint byte"
        );
        assert_eq!(
            serde_json::to_vec(sparse_map.progress()).unwrap(),
            serde_json::to_vec(full_map.progress()).unwrap()
        );
        let hours: Vec<_> = decodes
            .iter()
            .filter(|(_, _, table, _)| *table == TableId::DivisionStatus)
            .map(|(_, _, _, hours)| *hours)
            .collect();
        assert_eq!(
            hours,
            if block == 1 { vec![2] } else { vec![1] },
            "block {block}: only checkpoint prefixes decoded"
        );
        let progress =
            serde_json::from_slice(&serde_json::to_vec(sparse_map.progress()).unwrap()).unwrap();
        sparse_map = Builder::new(stride, SELECTED, progress).unwrap();
        narrow = ReplayStream::resume(
            &[TableId::Campaign],
            narrow.cursor().clone(),
            narrow.accumulator().cloned(),
        )
        .unwrap();
    }
}

#[test]
fn production_prepared_seek_and_advance_preserve_movement_speed_inputs() {
    use crate::test_fixtures::{CAMPAIGN, records, scale_frame};
    use replay::container::{Replay, ReplayStream};
    use replay_data::rows::*;
    use replay_derive::eta_derive::{
        self, Defines,
        frame_terms::{FrameTerms, Geography, SubUnitContent},
    };
    struct Map;
    impl SubUnitContent for Map {
        fn special_forces(&self, _: &[u8]) -> Result<bool, String> {
            Ok(false)
        }
        fn sub_unit_types(&self, _: &[u8]) -> Result<u64, String> {
            Ok(0)
        }
    }
    impl Geography for Map {
        fn terrain_index(&self, _: u32) -> Option<i32> {
            Some(1)
        }
        fn terrain_flags(&self, _: u32) -> Option<u8> {
            Some(1)
        }
        fn terrain_movement(&self, _: i32) -> Option<i64> {
            Some(0)
        }
        fn crossing_kind(&self, _: u32, _: u32) -> Option<u8> {
            Some(0)
        }
        fn region(&self, _: u32) -> Option<u32> {
            Some(7)
        }
    }
    let observed = |hour| {
        let mut frame = scale_frame(1, hour, 1);
        let value = i64::from(hour);
        for rows in [
            TableRows::MovementStateTerms(vec![MovementStateTermRow {
                state: 1,
                roster: 1,
                owner: 1,
                terms: Some(StateMovementTerms {
                    army_speed: Some(value),
                    ..Default::default()
                }),
            }]),
            TableRows::CoreSupplyAccess(vec![CoreSupplyAccessRow {
                owner: 1,
                province: 1,
                access: Some(hour % 2 == 0),
            }]),
            TableRows::CountryUnitSpeedEntries(vec![CountryUnitSpeedEntryRow {
                country: 1,
                key: 7,
                entry: Some(Some(value)),
            }]),
            TableRows::RiverRelations(vec![RiverRelationRow {
                owner: 1,
                controller: 2,
                entry: Some(Some(value)),
            }]),
            TableRows::DivisionTransportSpeeds(vec![OwnerTransportSpeedRow {
                owner: 1,
                region: 7,
                speed: Some(value),
            }]),
        ] {
            let table = rows.table();
            frame.tables[table.index()] = TableView::Complete(rows).into();
        }
        frame
    };
    let replay = Replay::from_records(
        CAMPAIGN.into(),
        &[records(
            1,
            vec![observed(100), observed(101), observed(102)],
        )],
    )
    .unwrap();
    let fixture = crate::prepared::series::tests::Fixture::new();
    // Use the production selection for both preparation and playback.
    let selected = selected_tables();
    let mut builder =
        Builder::new(NonZeroU32::new(3).unwrap(), selected, Progress::default()).unwrap();
    builder.set_ranges(&replay);
    let mut stream = ReplayStream::new(selected).unwrap();
    for block in 0..replay.index.len() {
        let mut writer = fixture.writer();
        let mut count = 0;
        stream
            .next_block_with(
                &replay,
                |raw, frame| {
                    count += 1;
                    builder.accept(
                        &mut writer,
                        &raw,
                        frame,
                        Position {
                            block,
                            next_frame: count,
                        },
                    )
                },
                || Ok(()),
            )
            .unwrap();
        writer.commit().unwrap();
    }
    let mut reader = Reader::new(
        store::Store::local(
            uuid::Uuid::parse_str(fixture.directory.file_name().unwrap().to_str().unwrap())
                .unwrap(),
        )
        .unwrap(),
        fixture.key.clone(),
        builder.progress().clone(),
        Arc::new(replay),
        selected,
    )
    .unwrap();
    for (hour, current) in [
        (100, reader.seek(1, 100).unwrap().unwrap()),
        (101, reader.advance(1, 101, |_| {}).unwrap().unwrap()),
        (102, reader.advance(1, 102, |_| {}).unwrap().unwrap()),
    ] {
        let Current::Frame(frame) = current else {
            panic!("expected prepared frame")
        };
        let expected = observed(hour);
        for table in [
            TableId::MovementStateTerms,
            TableId::CoreSupplyAccess,
            TableId::CountryUnitSpeedEntries,
            TableId::RiverRelations,
            TableId::DivisionTransportSpeeds,
        ] {
            assert_eq!(
                &*frame.tables[table.index()],
                &*expected.tables[table.index()],
                "{table:?} at {hour}"
            );
        }
        let movement = DivisionMovementRow {
            path: vec![2],
            path_total: 1,
            naval_transfer: 1,
            base_speed: Recorded::Value(Some(100_000)),
            ..Default::default()
        };
        let division = DivisionRow {
            province: 1,
            logical_owner: 1,
            ..Default::default()
        };
        let defines = Defines {
            transport_scale: 100_000,
            ..Default::default()
        };
        let speed = FrameTerms::new(&frame)
            .with_terms(&division, &movement, &Map, &defines, |terms| {
                eta_derive::edge_speed(100_000, 1, 2, terms, &defines)
            })
            .unwrap();
        assert_eq!(speed, i64::from(hour));
    }
}

#[test]
fn map_checkpoints_preserve_complete_sparse_mines_and_retained_incidents() {
    use replay_data::rows::{
        NavalAccidentReportRow, NavalMissionTaskForceRow, NavalPatrolMissionInputRow,
        NavalRegionalMineRow,
    };
    let selected = selected_tables();
    let naval_tables = [
        TableId::NavalRegionalMines,
        TableId::NavalAccidentReports,
        TableId::NavalDominanceTaskForceRanges,
        TableId::NavalPatrolMissionInputs,
    ];
    for table in naval_tables {
        assert!(selected.contains(&table));
    }
    assert!(!selected.contains(&TableId::NavalMineLevels));
    let mut source = frame();
    *source.tables[TableId::NavalRegionalMines.index()] =
        TableView::Complete(TableRows::NavalRegionalMines(vec![
            NavalRegionalMineRow {
                country: 0xfedc_ba98,
                kind: 0,
                region: 9001,
                amount: 0,
            },
            NavalRegionalMineRow {
                country: 0xfedc_ba98,
                kind: 1,
                region: 8001,
                amount: -17,
            },
        ]));
    *source.tables[TableId::NavalAccidentReports.index()] =
        TableView::Complete(TableRows::NavalAccidentReports(vec![
            NavalAccidentReportRow {
                country: 0xfedc_ba98,
                kind: 1,
                ordinal: 0,
                region: 8001,
                equipment: 0xfedc_ba98_0123_4567,
                ship: 0x0123_4567_fedc_ba98,
                damage: -9,
                sunk: 7,
                owner: 0x9876_5432,
                mine_owner: 0x2345_6789,
                expire_hour: 123,
            },
        ]));
    *source.tables[TableId::NavalDominanceTaskForceRanges.index()] =
        TableView::Complete(TableRows::NavalDominanceTaskForceRanges(vec![
            NavalMissionTaskForceRow {
                task_force: 0xfedc_ba98_0123_4567,
                range_base: -17,
                ..Default::default()
            },
        ]));
    *source.tables[TableId::NavalPatrolMissionInputs.index()] =
        TableView::Complete(TableRows::NavalPatrolMissionInputs(vec![
            NavalPatrolMissionInputRow {
                task_force: 0xfedc_ba98_0123_4567,
                target: Some(0x0123_4567_fedc_ba98),
                target_live: Some(0),
                progress: Some(-9),
                chance: None,
                ..Default::default()
            },
        ]));
    let position = Position {
        block: 91,
        next_frame: 7,
    };
    let bytes = encode_checkpoint(&source, position, selected).unwrap();
    let restored = decode_checkpoint(bytes.as_slice(), selected).unwrap();
    for table in naval_tables {
        assert_eq!(
            restored.frame.tables[table.index()],
            source.tables[table.index()]
        );
    }
}
