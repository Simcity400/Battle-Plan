use replay::{
    branches::{BranchReason, BranchStart, Parentage},
    frame::{DecodedFrame, FrameEncoder, Metrics, TableView},
    identity::{Identity, Roster, SampleBoundary, SessionStart, Verification},
    rows::{CampaignRow, CountryLinkRow, CountryRow, TableId, TableRows},
    spool::Record,
};

pub(crate) const CAMPAIGN: &[u8] = b"analytics-test";

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
                        t.country == s.country && t.technology_handle == s.technology_handle
                    })?;
                    row.token = definitions
                        .iter()
                        .find(|d| d.technology_handle == s.technology_handle)?
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
