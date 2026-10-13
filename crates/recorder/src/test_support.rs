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
        identity::{Identity, SampleBoundary, SessionStart, Verification},
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
        records.push(Record::Frame(allocation_frame(9, hour, 19)));
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

/// One complete keyframe of the synthetic campaign on `branch`, loaded at `load_signal`.
pub(crate) fn allocation_frame(branch: u64, hour: i32, load_signal: i64) -> replay::frame::Frame {
    use replay::{
        frame::{Frame, TableUpdate},
        rows::{CampaignRow, ClockRow},
    };
    Frame {
        branch,
        hour,
        keyframe: true,
        metrics: Metrics::default(),
        tables: std::array::from_fn(|index| {
            let rows = match TableId::ALL[index] {
                TableId::Clock => TableRows::Clock(vec![ClockRow {
                    hour,
                    play_session_started_at: load_signal,
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
                TableId::GameScoreMultipliers => {
                    TableRows::GameScoreMultipliers(vec![replay::rows::GameScoreMultiplierRow {
                        total_raw: Some(0),
                        army_raw: Some(0),
                        navy_raw: Some(0),
                        air_raw: Some(0),
                        industry_raw: Some(0),
                        land_raw: Some(0),
                        ..Default::default()
                    }])
                }
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
    }
}

/// The first load signal (19) of the synthetic campaign's session.
pub(crate) const FIRST_LOAD: i64 = 19;
/// A second load of the same campaign later in the session.
pub(crate) const RELOAD: i64 = 23;
/// A third load of the same campaign after that.
pub(crate) const THIRD_LOAD: i64 = 29;

/// Replace the closing detach of `records` with another load of the synthetic
/// campaign, as the collector writes it (a load-signal branch, then each
/// hour's clock witness before its frame), and detach after it.
fn push_load(
    records: &mut Vec<replay::spool::Record>,
    branch: u64,
    hours: std::ops::RangeInclusive<i32>,
    load_signal: i64,
) {
    use replay::{
        branches::{BranchReason, BranchStart, Parentage},
        spool::{ControlRecord, Record},
    };
    records.pop();
    let last_hour = *hours.end();
    records.push(Record::BranchStart(BranchStart {
        id: branch,
        session: 7,
        loaded_hour: Some(*hours.start() - 1),
        first_hour: *hours.start(),
        parent: Parentage::Unknown,
        reason: BranchReason::LoadSignal,
    }));
    for hour in hours {
        records.push(Record::Control(ControlRecord::ClockWitness {
            session: 7,
            qpc: branch * 1_000 + hour as u64 * 100,
            hour,
            load_signal: Some(load_signal),
        }));
        records.push(Record::Frame(allocation_frame(branch, hour, load_signal)));
    }
    records.push(Record::Control(ControlRecord::Detach {
        session: 7,
        last_hour: Some(last_hour),
    }));
}

/// [`allocation_records`] (branch 9, hours 2 to 4) followed by a second load
/// of the same campaign (branch 10, hours 3 and 4).
pub(crate) fn two_load_records() -> Vec<replay::spool::Record> {
    let mut records = allocation_records();
    push_load(&mut records, 10, 3..=4, RELOAD);
    records
}

/// [`two_load_records`] followed by a third load (branch 11, hours 5 and 6).
pub(crate) fn three_load_records() -> Vec<replay::spool::Record> {
    let mut records = two_load_records();
    push_load(&mut records, 11, 5..=6, THIRD_LOAD);
    records
}

/// Write `records` as one spool segment of the synthetic campaign in `session`.
pub(crate) fn write_campaign_spool(
    session: &std::path::Path,
    records: &[replay::spool::Record],
) -> std::path::PathBuf {
    let campaign = session.join(std::str::from_utf8(ALLOCATION_CAMPAIGN).unwrap());
    std::fs::create_dir_all(&campaign).unwrap();
    let spool: Vec<u8> = records
        .iter()
        .flat_map(|record| replay::spool::encode_record(record).unwrap())
        .collect();
    std::fs::write(campaign.join("spool.0000.h4s"), spool).unwrap();
    campaign
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
    let path = std::env::temp_dir().join(format!("battle-plan-recorder-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&path);
    std::fs::create_dir_all(&path).unwrap();
    path
}
