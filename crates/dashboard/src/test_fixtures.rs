use replay::{
    branches::{BranchReason, BranchStart, Parentage},
    frame::{DecodedFrame, FrameEncoder, Metrics, TableView},
    identity::{Identity, Roster, SampleBoundary, SessionStart, Verification},
    rows::{CampaignRow, CountryLinkRow, CountryRow, TableId, TableRows},
    spool::Record,
};

/// Gather a synthetic content root whose executable contains `test`.
pub(crate) fn content_identity(
    root: &std::path::Path,
) -> (crate::products::ContentOptions, replay::identity::Identity) {
    let options = crate::products::ContentOptions {
        options: installed_content::ResolveOptions {
            game_dir: root.to_owned(),
            user_dir: root.join("user"),
            cache_dir: root.join("cache"),
        },
        allow_unverified: true,
    };
    std::fs::create_dir_all(root.join("dlc")).unwrap();
    let gathered = installed_content::gather(&installed_content::GatherOptions {
        game_dir: options.options.game_dir.clone(),
        user_dir: options.options.user_dir.clone(),
        cache_dir: options.options.cache_dir.clone(),
        dlc_order: Some(Vec::new()),
    })
    .unwrap();
    let identity = gathered
        .identity(
            "test".into(),
            [
                0x9f, 0x86, 0xd0, 0x81, 0x88, 0x4c, 0x7d, 0x65, 0x9a, 0x2f, 0xea, 0xa0, 0xc5, 0x5a,
                0xd0, 0x15, 0xa3, 0xbf, 0x4f, 0x1b, 0x2b, 0x0b, 0x82, 0x2c, 0xd1, 0x5d, 0x6c, 0x15,
                0xb0, 0xf0, 0x0a, 0x08,
            ],
            4,
            "test".into(),
            [1; 32],
        )
        .unwrap();
    (options, identity)
}

pub(crate) const CAMPAIGN: &[u8] = b"analytics-test";

/// Open frames against an English synthetic content root.
pub(crate) fn source_with_content(
    frames: Vec<DecodedFrame>,
    root: &std::path::Path,
) -> (crate::Source, crate::products::LoadContext) {
    std::fs::create_dir_all(root.join("user")).unwrap();
    for (path, text) in [
        ("hoi4.exe", "test"),
        (
            "user/settings.txt",
            "language=l_english force_pow2_textures=no",
        ),
        (
            "user/dlc_load.json",
            r#"{"enabled_mods":[],"disabled_dlcs":[]}"#,
        ),
    ] {
        std::fs::write(root.join(path), text).unwrap();
    }
    let (options, identity) = content_identity(root);
    let mut input = records(1, frames);
    let Record::SessionStart(session) = &mut input[0] else {
        unreachable!()
    };
    session.identity = identity;
    let source = crate::Source::from_replay(
        crate::SourceKey {
            source_id: "content-fixture".into(),
            revision: "1".into(),
        },
        "memory.battleplan".into(),
        std::sync::Arc::new(
            replay::container::Replay::from_records(CAMPAIGN.into(), &[input]).unwrap(),
        ),
        &std::sync::atomic::AtomicBool::new(false),
    )
    .unwrap();
    (
        source,
        crate::products::LoadContext {
            content: Some(options),
        },
    )
}

pub(crate) fn ownership_frame(branch: u64, hour: i32, owner: u32) -> DecodedFrame {
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
        TableView::Complete(TableRows::States(vec![replay::rows::StateRow {
            id: 1,
            owner,
            controller: 55,
            movement_infrastructure: 0,
            ..Default::default()
        }]));
    frame
}

pub(crate) fn session(id: u64) -> SessionStart {
    SessionStart {
        id,
        process_id: 1,
        process_start_time: 1,
        qpc_frequency: 10_000_000,
        sample_boundary: SampleBoundary::EndOfHour,
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

pub(crate) fn source(branch: u64, hour: i32, power: Option<i64>) -> DecodedFrame {
    let mut tables = std::array::from_fn(|_| TableView::Unavailable {
        reason: "outside country fixture".into(),
    });
    tables[TableId::Campaign.index()] =
        TableView::Complete(TableRows::Campaign(vec![CampaignRow {
            id: CAMPAIGN.into(),
        }]));
    tables[TableId::Countries.index()] =
        TableView::Complete(TableRows::Countries(vec![CountryRow {
            border_land_doctrine_level: replay::rows::Recorded::Value(None),
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

pub(crate) fn records(branch: u64, frames: Vec<DecodedFrame>) -> Vec<Record> {
    let mut out = vec![
        Record::SessionStart(session(branch)),
        Record::BranchStart(BranchStart {
            id: branch,
            session: branch,
            loaded_hour: None,
            first_hour: frames[0].hour,
            parent: Parentage::Unknown,
            reason: BranchReason::SessionStart,
        }),
    ];
    let mut encoder = FrameEncoder::new();
    out.extend(
        frames
            .into_iter()
            .map(|mut frame| {
                // Existing research scenarios describe a logical observation.
                // Populate the hourly companion from those same fixture values.
                if matches!(&*frame.tables[TableId::ResearchInputs.index()], TableView::Unavailable { reason } if reason == "fixture hourly research") {
                    fixture_research_inputs(&mut frame);
                }
                Record::Frame(encoder.encode(frame, false).unwrap())
            }),
    );
    out
}

/// A new segment of an existing branch repeats its original context before a
/// fresh keyframe; its first hour does not redefine when the branch began.
pub(crate) fn continuation_records(
    replay: &replay::container::Replay,
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

pub(crate) fn fixture_research_inputs(frame: &mut DecodedFrame) {
    use replay::rows::{ResearchInputRow, tables};
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
                            && s.database_index == replay::rows::Recorded::Value(t.database_index)
                    })?;
                    row.token = definitions
                        .iter()
                        .find(|d| {
                            s.database_index == replay::rows::Recorded::Value(d.database_index)
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
    // Every fixture hour reads every country.
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
            .map(|table| replay::rows::SliceReadRow {
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

/// A unique three-byte tag per country, as the engine's tag loader allows.
pub(crate) fn scale_tag(index: u32) -> Vec<u8> {
    const DIGITS: &[u8] = b"0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz";
    let digit = |place: u32| DIGITS[(index / place % 62) as usize];
    vec![digit(62 * 62), digit(62), digit(1)]
}

/// Every province its own country: `count` countries,
/// each owning one state.
pub(crate) fn scale_frame(branch: u64, hour: i32, count: u32) -> DecodedFrame {
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
            .map(|index| replay::rows::StateRow {
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

pub(crate) fn stability_inputs() -> replay::rows::CountryStabilityInputRow {
    replay::rows::CountryStabilityInputRow {
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

pub(crate) fn stability_frame(
    hour: i32,
    inputs: replay::rows::CountryStabilityInputRow,
    read: i32,
    lines: Vec<replay::rows::CountryStabilityLineRow>,
) -> DecodedFrame {
    let mut frame = source(1, hour, Some(1));
    let reads = [
        TableId::CountryStabilityInputs,
        TableId::CountryStabilityLines,
    ]
    .into_iter()
    .filter(|table| *table == TableId::CountryStabilityInputs || !lines.is_empty())
    .map(|table| replay::rows::SliceReadRow {
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
            replay::rows::ModifierDefinitionRow {
                index: 0x8d,
                token: 1,
                flags: 3,
                ..Default::default()
            },
        ]));
    frame
}

/// Germany's (native id 9001, also linked as 11) and Italy's stockpiles and
/// production lines. `read` times a walk of another table of the stockpile slot
/// only, as a shared read times only its anchor; `lines` None records the
/// production lines as unavailable. Variants 10 and 20 are infantry equipment
/// (token 1), 30 a ship (token 2); variant 20 has its own name.
pub(crate) fn stockpile_frame(
    hour: i32,
    read: bool,
    stock: &[(u64, i64)],
    lines: Option<&[(u64, i64, i64, u8)]>,
) -> DecodedFrame {
    use replay::rows::{LexicalTokenRow, ProductionLineRow, StockpileRow, VariantDefinitionRow};
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
    // Italy's stock is not Germany's.
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
    // The initial stockpile frame reads variants too; later reads follow their cadence.
    if hour == slot_hour(TableId::Stockpiles)
        || replay::cadence::is_refresh(TableId::VariantDefinitions, hour)
    {
        frame.metrics.table_ns[TableId::VariantModules.index()] = 1;
    }
    frame
}

/// The hour of a daily table's slot on day 1000.
pub(crate) fn slot_hour(table: TableId) -> i32 {
    let replay::cadence::Cadence::Daily(slot) = replay::cadence::cadence(table) else {
        panic!("{table:?} is read in a daily slot");
    };
    1000 * replay::cadence::STAGGER_PERIOD + slot as i32
}

/// A stockpile frame whose variant 20 is named `name`, or whose variant table is
/// unavailable (`None`) though its read was due.
pub(crate) fn named_frame(hour: i32, name: Option<&[u8]>) -> DecodedFrame {
    use replay::rows::VariantDefinitionRow;
    let mut frame = stockpile_frame(hour, false, &[(20, 100_000)], Some(&[]));
    // The variant table's slot-mate walked: its read was due at this hour.
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

pub(crate) fn replay(frames: Vec<DecodedFrame>) -> replay::container::Replay {
    replay::container::Replay::from_records(CAMPAIGN.into(), &[records(1, frames)]).unwrap()
}

/// Equipment types from authored texts, as `installed_content::equipment` reads a folder's files.
pub(crate) fn equipment_types(texts: &[&[u8]]) -> std::io::Result<installed_content::equipment::Database> {
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
    let (database, _) = installed_content::equipment::fixture::load(&files);
    match &database.lost {
        Some(reason) => Err(std::io::Error::other(reason.clone())),
        None => Ok(database),
    }
}

// Start these scenarios on the production slot, as the shared reader dates it.
pub(crate) const PRODUCTION_HOUR: i32 = 4 * replay::cadence::STAGGER_PERIOD
    + match replay::cadence::cadence(TableId::ProductionLines) {
        replay::cadence::Cadence::Daily(slot) => slot as i32,
        _ => panic!("production lines have a daily slot"),
    };

/// One line of the production fixture: (handle, variant, active factories,
/// efficiency Q15).
pub(crate) type LineFixture = (u64, u64, i32, i64);

/// One output row of the production fixture: (line, variant, units, exact, ended).
pub(crate) type OutputFixture = (u64, u64, i64, bool, bool);

/// A frame with Germany (native id `germany`, linked through id 11 too) and
/// Italy (slot 0), Germany's `lines` and output, one Italian line, and Germany's
/// organisation 79 (Mauser). `read` times a walk of another table of the
/// production slot only, as a shared read times only its anchor; `variants` does
/// the same for the variant slot; `output` is None where the output table is
/// unavailable. Variants 70 and 71 are infantry equipment (71 named "Gewehr 98" by
/// its designer); every variant's icon entry is the auto-design ".".
pub(crate) fn production_frame(
    hour: i32,
    read: bool,
    variants: bool,
    germany: u32,
    lines: &[LineFixture],
    output: Option<&[OutputFixture]>,
) -> DecodedFrame {
    use replay::rows::{
        CountryAppearanceRow, IndustrialOrganisationRow, LexicalTokenRow, ProductionLineOutputRow,
        ProductionLineRow, VariantDefinitionRow,
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
            ..Default::default()
        }]));
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
            // Germany's lines name it by its other link.
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
                        mio_credit: units,
                        since_hour: PRODUCTION_HOUR,
                        observed_since_creation: line != 5,
                        ended,
                        exact,
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

/// Germany (raw links 11 and `germany`, slot 7), Italy (21, slot 8) and Spain
/// (31, slot 9), with steel and oil. Deals are (handle, exporter, importer,
/// delivered).
pub(crate) fn resource_frame(
    hour: i32,
    steel: i64,
    germany: u32,
    deals: &[(u64, u32, u32, i64)],
) -> DecodedFrame {
    use replay::rows::{
        CountryResourceRow, LexicalTokenRow, ResourceDefinitionRow, ResourceExchangeRow,
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
        // Special projects take 0.4; the production lines need 30.
        special_total: produced + 800_000 - 2_500_000 - 40_000,
        production_demand: -3_000_000,
        ..Default::default()
    };
    *frame.tables[TableId::CountryResources.index()] =
        TableView::Complete(TableRows::CountryResources(vec![
            // Germany's rows name it by its other link.
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

/// A frame with Germany (slot 7, links 11 and 9001), Italy (slot 8, link 21) and,
/// when `austria`, a dynamic country with a space in its tag (`D01 mod`, slot
/// 9, link 31), as a mod's tags may be. Other tables are unavailable.
pub(crate) fn world_frame(hour: i32, austria: bool) -> DecodedFrame {
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
