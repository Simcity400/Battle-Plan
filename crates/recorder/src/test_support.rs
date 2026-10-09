use replay::{
    frame::{DecodedFrame, Metrics, TableView},
    rows::{TableId, TableRows},
};

pub(crate) fn frame(rows: impl IntoIterator<Item = TableRows>) -> DecodedFrame {
    let mut tables =
        std::array::from_fn(|index| TableView::Complete(TableRows::empty(TableId::ALL[index])));
    for rows in rows {
        let index = rows.table().index();
        tables[index] = TableView::Complete(rows);
    }
    DecodedFrame {
        branch: 1,
        hour: 7,
        tables: tables.map(Into::into),
        metrics: Metrics::default(),
    }
}

pub(crate) const ALLOCATION_CAMPAIGN: &[u8; 36] = b"21b7cc84-3028-4f65-8e0d-adf2f37ca1a6";

pub(crate) fn allocation_records() -> Vec<replay::spool::Record> {
    use replay::{
        branches::{BranchReason, BranchStart, Parentage},
        frame::{Frame, TableUpdate},
        identity::{Identity, SampleBoundary, SessionStart, Verification},
        rows::{CampaignRow, ClockRow},
        spool::{ControlRecord, Record},
    };
    let mut records = vec![
        Record::SessionStart(SessionStart {
            id: 7,
            process_id: 2,
            process_start_time: 13,
            qpc_frequency: 100,
            sample_boundary: SampleBoundary::EndOfHour,
            identity: Identity {
                game_version: "synthetic".into(),
                exe_hash: [1; 32],
                exe_size: 1,
                vanilla_hash: [2; 32],
                dlc: vec![],
                mods: vec![],
                coverage_policy: 1,
                language: "english".into(),
                recorder_version: "test".into(),
                collector_hash: [3; 32],
                verification: Verification::Unchecked,
            },
        }),
        Record::BranchStart(BranchStart {
            id: 9,
            session: 7,
            loaded_hour: Some(1),
            first_hour: 2,
            parent: Parentage::Unknown,
            reason: BranchReason::SessionStart,
        }),
    ];
    for hour in 2..=4 {
        records.push(Record::Frame(Frame {
            branch: 9,
            hour,
            keyframe: true,
            metrics: Metrics::default(),
            tables: std::array::from_fn(|index| {
                let rows = match TableId::ALL[index] {
                    TableId::Clock => TableRows::Clock(vec![ClockRow {
                        hour,
                        play_session_started_at: 19,
                        ..Default::default()
                    }]),
                    TableId::Campaign => TableRows::Campaign(vec![CampaignRow {
                        id: ALLOCATION_CAMPAIGN.to_vec(),
                    }]),
                    TableId::SunkShips => TableRows::SunkShips(vec![Default::default()]),
                    TableId::WorldTension => TableRows::WorldTension(vec![Default::default()]),
                    TableId::LocalPlayer => TableRows::LocalPlayer(vec![Default::default()]),
                    TableId::SimulationFeatures => {
                        TableRows::SimulationFeatures(vec![Default::default()])
                    }
                    TableId::CombatLogState => TableRows::CombatLogState(vec![Default::default()]),
                    TableId::AirSortieModifierMetadata => {
                        TableRows::AirSortieModifierMetadata(vec![Default::default()])
                    }
                    TableId::ReservedCountryNaming => {
                        TableRows::ReservedCountryNaming(vec![Default::default()])
                    }
                    TableId::GameScoreMultipliers => TableRows::GameScoreMultipliers(vec![
                        replay::rows::GameScoreMultiplierRow {
                            total_raw: Some(0),
                            army_raw: Some(0),
                            navy_raw: Some(0),
                            air_raw: Some(0),
                            industry_raw: Some(0),
                            land_raw: Some(0),
                            ..Default::default()
                        },
                    ]),
                    TableId::EventFeedHistory => {
                        TableRows::EventFeedHistory(vec![replay::rows::EventFeedHistoryRow {
                            read_hour: hour,
                            coverage: replay::event_feed_rows::EventFeedCoverage::Complete,
                            counts: Some(Default::default()),
                            ..Default::default()
                        }])
                    }
                    table => TableRows::empty(table),
                };
                TableUpdate::Snapshot(rows)
            }),
        }));
        records.push(Record::Control(ControlRecord::ClockWitness {
            session: 7,
            qpc: hour as u64 * 100,
            hour,
            load_signal: Some(19),
        }));
    }
    records.push(Record::Control(ControlRecord::Detach {
        session: 7,
        last_hour: Some(4),
    }));
    records
}

pub(crate) fn allocation_witness(committed_bytes: u64) -> replay::witness::TerminalWitness {
    replay::witness::TerminalWitness {
        session: 7,
        process_start_time: 13,
        campaign_id: Some(*ALLOCATION_CAMPAIGN),
        branch: Some(9),
        load_signal: Some(19),
        failed_hour: Some(99),
        committed_campaign_id: Some(*ALLOCATION_CAMPAIGN),
        committed_segment: Some(0),
        committed_bytes,
        pending_campaign_id: None,
        pending_segment: None,
        pending_bytes: 0,
        phase: 2,
        code: 1,
        drain_complete: false,
    }
}

pub(crate) fn allocation_test_directory(name: &str) -> std::path::PathBuf {
    let path = std::path::PathBuf::from(r"captures\astra\lead-h-collector-oom")
        .join(format!("recorder-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&path);
    std::fs::create_dir_all(&path).unwrap();
    path
}
