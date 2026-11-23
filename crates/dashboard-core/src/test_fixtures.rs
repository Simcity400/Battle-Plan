
use replay_capture_spool::spool::Record;
use replay_data::{
    identity::CaptureMode, identity::Identity, identity::Roster, identity::SampleBoundary,
    identity::SessionStart, identity::Verification, rows::CampaignRow, rows::CountryLinkRow,
    rows::CountryRow, rows::TableId, rows::TableRows,
};
use replay_frame::{
    branches::BranchReason, branches::BranchStart, branches::Parentage, frame::DecodedFrame,
    frame::Frame, frame::FrameEncoder, frame::FrameInput, frame::Metrics, frame::TableInput,
    frame::TableUpdate, frame::TableView,
};

pub const CAMPAIGN: &[u8] = b"analytics-test";

pub fn ownership_frame(branch: u64, hour: i32, owner: u32) -> DecodedFrame {
    let mut frame = source(branch, hour, Some(1));
    if let TableView::Complete(TableRows::Countries(rows)) =
        &mut *frame.tables[TableId::Countries.index()]
    {
        for (id, slot, tag) in [(44, 8, b"FRA"), (55, 9, b"CTL"), (66, 10, b"MOD")] {
            rows.push(CountryRow {
                id,
                canonical_slot: slot,
                tag: tag.to_vec(),
                ..CountryRow::default()
            });
        }
        rows.sort_by_key(|row| row.id);
    }
    if let TableView::Complete(TableRows::CountryLinks(rows)) =
        &mut *frame.tables[TableId::CountryLinks.index()]
    {
        rows.extend(
            [(44, 8), (55, 9), (66, 10)]
                .map(|(id, canonical_slot)| CountryLinkRow { id, canonical_slot }),
        );
        rows.sort_by_key(|row| row.id);
    }
    *frame.tables[TableId::States.index()] =
        TableView::Complete(TableRows::States(vec![replay_data::rows::StateRow {
            id: 1,
            owner,
            controller: 55,
            movement_infrastructure: 0,
            ..Default::default()
        }]));
    frame
}

pub fn session(id: u64) -> SessionStart {
    SessionStart {
        id,
        process_id: 1,
        process_start_time: 1,
        qpc_frequency: 10_000_000,
        sample_boundary: SampleBoundary::EndOfHour,
        capture_mode: CaptureMode::Full,
        identity: Identity {
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
        },
    }
}

pub fn source(branch: u64, hour: i32, power: Option<i64>) -> DecodedFrame {
    let mut tables = std::array::from_fn(|_| TableView::Unavailable {
        reason: "outside country fixture".into(),
    });
    tables[TableId::Campaign.index()] =
        TableView::Complete(TableRows::Campaign(vec![CampaignRow {
            id: CAMPAIGN.into(),
        }]));
    tables[TableId::Countries.index()] =
        TableView::Complete(TableRows::Countries(vec![CountryRow {
            border_land_doctrine_level: replay_data::rows::Recorded::Value(None),
            id: 9001,
            canonical_slot: 7,
            tag: b"GER".to_vec(),
            political_power_raw: power,
            command_power_raw: i64::MAX,
            stability_raw: i64::MIN,
            war_support_raw: 70_001,
            convoys_raw: Some(99_001),
            peace_conference_participant: 0,
            manpower_residual: 0,
        }]));
    tables[TableId::CountryLinks.index()] = TableView::Complete(TableRows::CountryLinks(vec![
        CountryLinkRow {
            id: 11,
            canonical_slot: 7,
        },
        CountryLinkRow {
            id: 9001,
            canonical_slot: 7,
        },
    ]));
    DecodedFrame {
        branch,
        hour,
        tables: tables.map(Into::into),
        metrics: Metrics::default(),
    }
}

pub fn records(branch: u64, frames: Vec<DecodedFrame>) -> Vec<Record> {
    let mut encoder = FrameEncoder::new();
    framed(branch, frames, |frame| encoder.encode_input(frame, false).unwrap())
}

pub fn snapshot_records(branch: u64, frames: Vec<DecodedFrame>) -> Vec<Record> {
    framed(branch, frames, |frame| Frame {
        branch: frame.branch,
        hour: frame.hour,
        keyframe: true,
        metrics: frame.metrics,
        tables: frame.tables.map(|table| match table {
            TableInput::Sampled(TableView::Complete(rows)) => TableUpdate::Snapshot(rows),
            TableInput::Sampled(TableView::Truncated { rows, missing }) => TableUpdate::Truncated { rows, missing },
            TableInput::Sampled(TableView::Unavailable { reason }) => TableUpdate::Unavailable { reason },
            TableInput::Sampled(TableView::NotRecorded { mode }) => TableUpdate::NotRecorded { mode },
            _ => unreachable!("a decoded frame's tables are sampled"),
        }),
    })
}

fn framed(branch: u64, frames: Vec<DecodedFrame>, mut frame_of: impl FnMut(FrameInput) -> Frame) -> Vec<Record> {
    let session = session(branch);
    let mode = session.capture_mode;
    let mut out = vec![
        Record::SessionStart(session),
        Record::BranchStart(BranchStart {
            id: branch,
            session: branch,
            loaded_hour: None,
            first_hour: frames[0].hour,
            parent: Parentage::Unknown,
            reason: BranchReason::SessionStart,
        }),
    ];
    out.extend(
        frames
            .into_iter()
            .map(|mut frame| {
                if matches!(&*frame.tables[TableId::ResearchInputs.index()], TableView::Unavailable { reason } if reason == "fixture hourly research") {
                    fixture_research_inputs(&mut frame);
                }
                let mut frame = FrameInput::from(frame);
                frame.scope_to(mode);
                Record::Frame(frame_of(frame))
            }),
    );
    out
}

pub fn continuation_records(
    replay: &replay::campaign::Replay,
    branch: u64,
    frames: Vec<DecodedFrame>,
) -> Vec<Record> {
    let start = replay
        .branches
        .iter()
        .find(|start| start.id == branch)
        .unwrap();
    let mut out = records(branch, frames);
    out[0] = Record::SessionStart(
        replay
            .sessions
            .iter()
            .find(|session| session.id == start.session)
            .unwrap()
            .clone(),
    );
    out[1] = Record::BranchStart(start.clone());
    out
}

pub fn fixture_research_inputs(frame: &mut DecodedFrame) {
    use replay_data::{rows::ResearchInputRow, rows::tables};
    let input = (|| {
        let slots = frame.complete::<tables::ResearchSlots>().ok()?;
        let techs = frame.complete::<tables::ResearchTechnologies>().ok()?;
        let definitions = frame.complete::<tables::TechnologyDefinitions>().ok()?;
        slots
            .iter()
            .map(|s| {
                let mut row = ResearchInputRow {
                    database_index: s.database_index,
                    country: s.country,
                    index: s.index,
                    technology_handle: s.technology_handle,
                    technology_present: s.technology_present,
                    research_speed_raw: s.research_speed_raw,
                    banked_days_raw: s.banked_days_raw,
                    applied_days_raw: s.applied_days_raw,
                    ..Default::default()
                };
                if s.technology_present != 0 {
                    let t = techs.iter().find(|t| {
                        t.country == s.country
                            && s.database_index
                                == replay_data::rows::Recorded::Value(t.database_index)
                    })?;
                    row.token = definitions
                        .iter()
                        .find(|d| {
                            s.database_index == replay_data::rows::Recorded::Value(d.database_index)
                        })?
                        .id
                        .clone();
                    row.level = t.level;
                    row.classifier = t.classifier;
                    row.progress_raw = t.progress_raw;
                    row.ahead_reduction_raw = t.ahead_reduction_years_raw;
                }
                Some(row)
            })
            .collect::<Option<Vec<_>>>()
    })();
    *frame.tables[TableId::ResearchInputs.index()] = input.map_or_else(
        || TableView::Unavailable {
            reason: "fixture research unavailable".into(),
        },
        |rows| TableView::Complete(TableRows::ResearchInputs(rows)),
    );
    *frame.tables[TableId::ResearchAppliedBonuses.index()] =
        TableView::Complete(TableRows::ResearchAppliedBonuses(vec![]));
    let mut reads: Vec<_> = frame
        .complete::<tables::ResearchSlots>()
        .map(|slots| slots.iter().map(|slot| slot.country).collect::<Vec<_>>())
        .unwrap_or_default()
        .into_iter()
        .flat_map(|country| {
            [
                TableId::ResearchInputs,
                TableId::ResearchAppliedBonuses,
                TableId::ResearchModifiers,
                TableId::ResearchTechnologies,
            ]
            .map(|table| replay_data::rows::SliceReadRow {
                table: table.index() as u32,
                owner: country,
                sample_hour: frame.hour,
            })
        })
        .collect();
    reads.sort_by_key(|row| (row.table, row.owner));
    reads.dedup();
    *frame.tables[TableId::SliceReads.index()] = TableView::Complete(TableRows::SliceReads(reads));
}

pub fn scale_tag(index: u32) -> Vec<u8> {
    const DIGITS: &[u8] = b"0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz";
    let digit = |place: u32| DIGITS[(index / place % 62) as usize];
    vec![digit(62 * 62), digit(62), digit(1)]
}

pub fn scale_frame(branch: u64, hour: i32, count: u32) -> DecodedFrame {
    let mut frame = source(branch, hour, Some(1));
    *frame.tables[TableId::Countries.index()] = TableView::Complete(TableRows::Countries(
        (0..count)
            .map(|index| CountryRow {
                id: 100_000 + index,
                canonical_slot: index,
                tag: scale_tag(index),
                ..CountryRow::default()
            })
            .collect(),
    ));
    *frame.tables[TableId::CountryLinks.index()] = TableView::Complete(TableRows::CountryLinks(
        (0..count)
            .map(|index| CountryLinkRow {
                id: 100_000 + index,
                canonical_slot: index,
            })
            .collect(),
    ));
    *frame.tables[TableId::States.index()] = TableView::Complete(TableRows::States(
        (0..count)
            .map(|index| replay_data::rows::StateRow {
                id: index + 1,
                owner: 100_000 + index,
                controller: 100_000 + index,
                movement_infrastructure: 0,
                ..Default::default()
            })
            .collect(),
    ));
    frame
}

pub fn stability_inputs() -> replay_data::rows::CountryStabilityInputRow {
    replay_data::rows::CountryStabilityInputRow {
        country: 9001,
        stability_raw: 50_000,
        war_support_raw: 40_000,
        ruling_party_popularity_raw: 4_500_000,
        propaganda_stability_q15: -3277,
        coastal_protection_stability_raw: 2_500,
        interception_sum_raw: 10_000,
        world_tension_raw: 50_000,
        base_stability_war_factor: -20_000,
        base_stability_party_popularity_factor: 15_000,
        min_stability: 0,
        max_stability: 100_000,
        war_support_offensive_war: -20_000,
        war_support_defensive_war: 20_000,
        war_support_tension_impact: 40_000,
        min_war_support: 0,
        max_war_support: 100_000,
        interception_war_support_scale_q15: 16_384,
        interception_bombing_war_support_impact: 30_000,
        base_max_command_power: 8_000_000,
        stability_factor: Some(30_000),
        party_popularity_stability_factor: Some(0),
        war_stability_factor: Some(-10_000),
        offensive_war_stability_factor: Some(5_000),
        defensive_war_stability_factor: Some(0),
        war_support_factor: Some(0),
        stability_weekly: Some(0),
        war_support_weekly: Some(0),
        keys: vec![
            0x8d, 0x90, 0x91, 0x17b, 0x17c, 0x95, 0x8e, 0x96, 0x14b, 0x14d,
        ],
        language: b"l_english".to_vec(),
        ..Default::default()
    }
}

pub fn stability_frame(
    hour: i32,
    inputs: replay_data::rows::CountryStabilityInputRow,
    read: i32,
    lines: Vec<replay_data::rows::CountryStabilityLineRow>,
) -> DecodedFrame {
    let mut frame = source(1, hour, Some(1));
    let reads = [
        TableId::CountryStabilityInputs,
        TableId::CountryStabilityLines,
    ]
    .into_iter()
    .filter(|table| *table == TableId::CountryStabilityInputs || !lines.is_empty())
    .map(|table| replay_data::rows::SliceReadRow {
        table: table.index() as u32,
        owner: 9001,
        sample_hour: read,
    })
    .collect();
    *frame.tables[TableId::SliceReads.index()] = TableView::Complete(TableRows::SliceReads(reads));
    *frame.tables[TableId::CountryStabilityInputs.index()] =
        TableView::Complete(TableRows::CountryStabilityInputs(vec![inputs]));
    *frame.tables[TableId::CountryStabilityLines.index()] =
        TableView::Complete(TableRows::CountryStabilityLines(lines));
    *frame.tables[TableId::ModifierDefinitions.index()] =
        TableView::Complete(TableRows::ModifierDefinitions(vec![
            replay_data::rows::ModifierDefinitionRow {
                index: 0x8d,
                token: 1,
                flags: 3,
                ..Default::default()
            },
        ]));
    frame
}

pub fn stockpile_frame(
    hour: i32,
    read: bool,
    stock: &[(u64, i64)],
    lines: Option<&[(u64, i64, i64, u8)]>,
) -> DecodedFrame {
    use replay_data::{
        rows::LexicalTokenRow, rows::ProductionLineRow, rows::StockpileRow,
        rows::VariantDefinitionRow,
    };
    let mut frame = source(1, hour, Some(1));
    *frame.tables[TableId::CountryLinks.index()] =
        TableView::Complete(TableRows::CountryLinks(vec![
            CountryLinkRow {
                id: 11,
                canonical_slot: 7,
            },
            CountryLinkRow {
                id: 21,
                canonical_slot: 8,
            },
            CountryLinkRow {
                id: 9001,
                canonical_slot: 7,
            },
        ]));
    let mut rows: Vec<StockpileRow> = stock
        .iter()
        .enumerate()
        .map(|(ordinal, &(variant_handle, amount_raw))| StockpileRow {
            country: 11,
            ordinal: ordinal as u32,
            variant_handle,
            amount_raw,
        })
        .collect();
    rows.push(StockpileRow {
        country: 21,
        ordinal: 0,
        variant_handle: 10,
        amount_raw: 999,
    });
    *frame.tables[TableId::Stockpiles.index()] = TableView::Complete(TableRows::Stockpiles(rows));
    *frame.tables[TableId::ProductionLines.index()] = match lines {
        Some(lines) => TableView::Complete(TableRows::ProductionLines(
            lines
                .iter()
                .enumerate()
                .map(
                    |(index, &(variant, speed_raw, cost_raw, kind))| ProductionLineRow {
                        handle: index as u64 + 1,
                        country: 9001,
                        kind,
                        variant,
                        speed_raw,
                        cost_raw,
                        ..Default::default()
                    },
                )
                .collect(),
        )),
        None => TableView::Unavailable {
            reason: "fixture".into(),
        },
    };
    *frame.tables[TableId::LexicalTokens.index()] =
        TableView::Complete(TableRows::LexicalTokens(vec![
            LexicalTokenRow {
                token: 1,
                literal: b"infantry_equipment_1".to_vec(),
            },
            LexicalTokenRow {
                token: 2,
                literal: b"destroyer_1".to_vec(),
            },
        ]));
    let variant = |id, equipment_token, name: &[u8]| VariantDefinitionRow {
        id,
        equipment_token,
        creator_link: 9001,
        display_name: name.to_vec(),
        ..Default::default()
    };
    *frame.tables[TableId::VariantDefinitions.index()] =
        TableView::Complete(TableRows::VariantDefinitions(vec![
            variant(10, 1, b""),
            variant(20, 1, b"Gewehr 43"),
            variant(30, 2, b""),
        ]));
    frame.metrics.table_ns[TableId::Countries.index()] = 1;
    if read {
        frame.metrics.table_ns[TableId::DeploymentLines.index()] = 1;
    }
    if hour == slot_hour(TableId::Stockpiles)
        || replay_data::cadence::is_refresh(TableId::VariantDefinitions, hour)
    {
        frame.metrics.table_ns[TableId::VariantModules.index()] = 1;
    }
    frame
}

pub fn slot_hour(table: TableId) -> i32 {
    let replay_data::cadence::Cadence::Daily(slot) = replay_data::cadence::cadence(table) else {
        panic!("{table:?} is read in a daily slot");
    };
    1000 * replay_data::cadence::STAGGER_PERIOD + slot as i32
}

pub fn named_frame(hour: i32, name: Option<&[u8]>) -> DecodedFrame {
    use replay_data::rows::VariantDefinitionRow;
    let mut frame = stockpile_frame(hour, false, &[(20, 100_000)], Some(&[]));
    frame.metrics.table_ns[TableId::VariantModules.index()] = 1;
    *frame.tables[TableId::VariantDefinitions.index()] = match name {
        Some(name) => {
            TableView::Complete(TableRows::VariantDefinitions(vec![VariantDefinitionRow {
                id: 20,
                equipment_token: 1,
                creator_link: 9001,
                display_name: name.to_vec(),
                ..Default::default()
            }]))
        }
        None => TableView::Unavailable {
            reason: "fixture".into(),
        },
    };
    frame
}

pub fn replay(frames: Vec<DecodedFrame>) -> replay::campaign::Replay {
    replay::campaign::Replay::from_records(recorder_derive::writers(), CAMPAIGN.into(), &[records(1, frames)]).unwrap()
}

pub fn equipment_types(texts: &[&[u8]]) -> std::io::Result<content_reader_core::equipment::Database> {
    let texts: Vec<String> = texts
        .iter()
        .map(|text| String::from_utf8(text.to_vec()).unwrap())
        .collect();
    let paths: Vec<String> = (0..texts.len())
        .map(|i| format!("common/units/equipment/{i:04}.txt"))
        .collect();
    let files: Vec<(&str, &str)> = paths
        .iter()
        .zip(&texts)
        .map(|(path, text)| (path.as_str(), text.as_str()))
        .collect();
    let (database, _) = content_reader_core::equipment::fixture::load(&files);
    match &database.lost {
        Some(reason) => Err(std::io::Error::other(reason.clone())),
        None => Ok(database),
    }
}

pub const PRODUCTION_HOUR: i32 = 4 * replay_data::cadence::STAGGER_PERIOD
    + match replay_data::cadence::cadence(TableId::ProductionLines) {
        replay_data::cadence::Cadence::Daily(slot) => slot as i32,
        _ => panic!("production lines have a daily slot"),
    };

pub type LineFixture = (u64, u64, i32, i64);

pub type OutputFixture = (u64, u64, i64, bool, bool);

pub fn production_frame(
    hour: i32,
    read: bool,
    variants: bool,
    germany: u32,
    lines: &[LineFixture],
    output: Option<&[OutputFixture]>,
) -> DecodedFrame {
    use replay_data::{
        rows::CountryAppearanceRow, rows::IndustrialOrganisationRow, rows::LexicalTokenRow,
        rows::OutputPurpose, rows::ProductionLineOutputRow, rows::ProductionLineRow,
        rows::Recorded, rows::ReservedCountryNamingRow, rows::VariantDefinitionRow,
    };
    let mut frame = source(1, hour, Some(1));
    let country = |id, canonical_slot, tag: &[u8]| CountryRow {
        id,
        canonical_slot,
        tag: tag.to_vec(),
        ..Default::default()
    };
    *frame.tables[TableId::Countries.index()] = TableView::Complete(TableRows::Countries(vec![
        country(21, 0, b"ITA"),
        country(germany, 7, b"GER"),
    ]));
    let link = |id, canonical_slot| CountryLinkRow { id, canonical_slot };
    *frame.tables[TableId::CountryLinks.index()] =
        TableView::Complete(TableRows::CountryLinks(vec![
            link(11, 7),
            link(21, 0),
            link(germany, 7),
        ]));
    *frame.tables[TableId::CountryAppearance.index()] =
        TableView::Complete(TableRows::CountryAppearance(vec![CountryAppearanceRow {
            country: 11,
            original_tag_reference: Recorded::Value(Some(0)),
            ..Default::default()
        }]));
    *frame.tables[TableId::ReservedCountryNaming.index()] =
        TableView::Complete(TableRows::ReservedCountryNaming(vec![
            ReservedCountryNamingRow {
                dynamic_tag_boundary: Recorded::Value(Some(50)),
                ..Default::default()
            },
        ]));
    *frame.tables[TableId::LexicalTokens.index()] =
        TableView::Complete(TableRows::LexicalTokens(vec![
            LexicalTokenRow {
                token: 601,
                literal: b"infantry_equipment_1".to_vec(),
            },
            LexicalTokenRow {
                token: 602,
                literal: b"GER_mauser_organization".to_vec(),
            },
        ]));
    let variant = |id, display: &[u8]| VariantDefinitionRow {
        id,
        equipment_token: 601,
        creator_link: 11,
        display_name: display.to_vec(),
        append_variant_name: 1,
        icon_active: 1,
        icon_kind: 1,
        icon: b".".to_vec(),
        ..Default::default()
    };
    *frame.tables[TableId::VariantDefinitions.index()] =
        TableView::Complete(TableRows::VariantDefinitions(vec![
            variant(70, b""),
            variant(71, b"Gewehr 98"),
        ]));
    let line =
        |&(handle, variant, active_factories, efficiency_raw): &LineFixture| ProductionLineRow {
            handle,
            country: 11,
            kind: 0,
            variant,
            mio: 79,
            active_factories,
            requested_factories: active_factories + 1,
            max_factories: 150,
            cost_raw: 50_000,
            speed_raw: 310_000,
            efficiency_raw,
            amount: -1,
            ..Default::default()
        };
    let mut rows: Vec<_> = lines.iter().map(line).collect();
    rows.push(ProductionLineRow {
        handle: 999,
        country: 21,
        variant: 70,
        cost_raw: 50_000,
        ..Default::default()
    });
    rows.sort_unstable_by_key(|row| row.handle);
    *frame.tables[TableId::ProductionLines.index()] =
        TableView::Complete(TableRows::ProductionLines(rows));
    *frame.tables[TableId::IndustrialOrganisations.index()] =
        TableView::Complete(TableRows::IndustrialOrganisations(vec![
            IndustrialOrganisationRow {
                organisation: 79,
                country: 11,
                owner: 11,
                token: 602,
                ..Default::default()
            },
        ]));
    *frame.tables[TableId::ProductionLineOutput.index()] = match output {
        Some(output) => TableView::Complete(TableRows::ProductionLineOutput(
            output
                .iter()
                .enumerate()
                .map(
                    |(ordinal, &(line, variant, units, exact, ended))| ProductionLineOutputRow {
                        line,
                        ordinal: ordinal as u32,
                        country: 11,
                        variant,
                        mio: 79,
                        units,
                        since_hour: PRODUCTION_HOUR,
                        observed_since_creation: line != 5,
                        ended,
                        exact,
                        purpose: Recorded::Value(OutputPurpose::New.raw()),
                        ..Default::default()
                    },
                )
                .collect(),
        )),
        None => TableView::Unavailable {
            reason: "a tick went unobserved".into(),
        },
    };
    frame.metrics.table_ns[TableId::Countries.index()] = 1;
    if read {
        frame.metrics.table_ns[TableId::Stockpiles.index()] = 1;
    }
    if variants {
        frame.metrics.table_ns[TableId::VariantModules.index()] = 1;
    }
    frame
}

pub fn resource_frame(
    hour: i32,
    steel: i64,
    germany: u32,
    deals: &[(u64, u32, u32, i64)],
) -> DecodedFrame {
    use replay_data::{
        rows::CountryResourceRow, rows::LexicalTokenRow, rows::ResourceDefinitionRow,
        rows::ResourceExchangeRow,
    };
    let mut frame = source(1, hour, Some(1));
    let country = |id, canonical_slot, tag: &[u8]| CountryRow {
        id,
        canonical_slot,
        tag: tag.to_vec(),
        ..Default::default()
    };
    *frame.tables[TableId::Countries.index()] = TableView::Complete(TableRows::Countries(vec![
        country(21, 8, b"ITA"),
        country(31, 9, b"SPR"),
        country(germany, 7, b"GER"),
    ]));
    let link = |id, canonical_slot| CountryLinkRow { id, canonical_slot };
    *frame.tables[TableId::CountryLinks.index()] =
        TableView::Complete(TableRows::CountryLinks(vec![
            link(11, 7),
            link(21, 8),
            link(31, 9),
            link(germany, 7),
        ]));
    *frame.tables[TableId::LexicalTokens.index()] =
        TableView::Complete(TableRows::LexicalTokens(vec![
            LexicalTokenRow {
                token: 501,
                literal: b"steel".to_vec(),
            },
            LexicalTokenRow {
                token: 502,
                literal: b"oil".to_vec(),
            },
        ]));
    *frame.tables[TableId::ResourceDefinitions.index()] =
        TableView::Complete(TableRows::ResourceDefinitions(vec![
            ResourceDefinitionRow {
                id: 1,
                token: 501,
                active: 1,
                factory_cost: 12_500,
            },
            ResourceDefinitionRow {
                id: 2,
                token: 502,
                active: 1,
                factory_cost: 12_500,
            },
        ]));
    let pools = |country, resource, produced| CountryResourceRow {
        country,
        resource,
        produced,
        imported: 800_000,
        to_export: 2_500_000,
        exported: 1_700_000,
        used: produced + 800_000 - 2_500_000,
        special_total: produced + 800_000 - 2_500_000 - 40_000,
        production_demand: -3_000_000,
        ..Default::default()
    };
    *frame.tables[TableId::CountryResources.index()] =
        TableView::Complete(TableRows::CountryResources(vec![
            pools(11, 1, steel),
            pools(11, 2, 0),
            pools(21, 1, 99_900_000),
        ]));
    *frame.tables[TableId::ResourceExchanges.index()] =
        TableView::Complete(TableRows::ResourceExchanges(
            deals
                .iter()
                .map(|&(id, exporter, importer, delivered)| ResourceExchangeRow {
                    id,
                    exporter,
                    importer,
                    resource: 1,
                    delivered,
                    assigned_factories: 1,
                    required_factories: 1,
                    ..Default::default()
                })
                .collect(),
        ));
    frame
}

pub fn world_frame(hour: i32, austria: bool) -> DecodedFrame {
    let mut frame = source(1, hour, Some(1));
    let country = |id, canonical_slot, tag: &[u8]| CountryRow {
        id,
        canonical_slot,
        tag: tag.to_vec(),
        ..Default::default()
    };
    let mut countries = vec![country(21, 8, b"ITA"), country(9001, 7, b"GER")];
    let link = |id, canonical_slot| CountryLinkRow { id, canonical_slot };
    let mut links = vec![link(11, 7), link(21, 8), link(9001, 7)];
    if austria {
        countries.insert(1, country(31, 9, b"D01 mod"));
        links.insert(2, link(31, 9));
    }
    *frame.tables[TableId::Countries.index()] =
        TableView::Complete(TableRows::Countries(countries));
    *frame.tables[TableId::CountryLinks.index()] =
        TableView::Complete(TableRows::CountryLinks(links));
    frame
}
