use replay_data::{rows::TableId, rows::TableRows};
use replay_frame::{frame::DecodedFrame, frame::Metrics, frame::TableView};

pub fn frame(rows: impl IntoIterator<Item = TableRows>) -> DecodedFrame {
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

pub const ALLOCATION_CAMPAIGN: &[u8; 36] = b"21b7cc84-3028-4f65-8e0d-adf2f37ca1a6";

pub fn allocation_records() -> Vec<replay_capture_spool::spool::Record> {
    use replay_recording::control::ControlRecord;
    use replay_capture_spool::spool::Record;
    use replay_data::{
        identity::CaptureMode, identity::Identity, identity::SampleBoundary,
        identity::SessionStart, identity::Verification,
    };
    use replay_frame::{branches::BranchReason, branches::BranchStart, branches::Parentage};
    let mut records = vec![
        Record::SessionStart(SessionStart {
            id: 7,
            process_id: 2,
            process_start_time: 13,
            qpc_frequency: 100,
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

const NOT_ACQUIRED: &str = "table was not acquired";

pub fn allocation_frame(branch: u64, hour: i32, load_signal: i64) -> replay_frame::frame::Frame {
    use replay_data::{rows::CampaignRow, rows::ClockRow};
    use replay_frame::{frame::Frame, frame::TableUpdate};
    Frame {
        branch,
        hour,
        keyframe: true,
        metrics: Metrics::default(),
        tables: std::array::from_fn(|index| {
            let table = TableId::ALL[index];
            match replay_frame::frame::current_absence(table) {
                Some(replay_frame::frame::TableView::NotRecorded { mode }) => {
                    return TableUpdate::NotRecorded { mode };
                }
                Some(replay_frame::frame::TableView::Unavailable { reason }) => {
                    return TableUpdate::Unavailable { reason };
                }
                None if table.is_derived() => {
                    return TableUpdate::Unavailable { reason: NOT_ACQUIRED.into() };
                }
                None => {}
                _ => unreachable!("absence is an omission or a named refusal"),
            }
            let rows = match table {
                TableId::Clock => TableRows::Clock(vec![ClockRow {
                    hour,
                    play_session_started_at: load_signal,
                    ..Default::default()
                }]),
                TableId::Campaign => TableRows::Campaign(vec![CampaignRow {
                    id: ALLOCATION_CAMPAIGN.to_vec(),
                }]),
                TableId::GameScoreMultipliers => TableRows::GameScoreMultipliers(vec![
                    replay_data::rows::GameScoreMultiplierRow {
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
                    TableRows::EventFeedHistory(vec![replay_data::rows::EventFeedHistoryRow {
                        read_hour: hour,
                        coverage: replay_data::event_feed_rows::EventFeedCoverage::Complete,
                        counts: Some(Default::default()),
                        ..Default::default()
                    }])
                }
                TableId::MapEvents => TableRows::MapEvents(vec![replay_data::rows::MapEventsRow {
                    from_hour: hour - 1,
                    to_hour: hour,
                    ..Default::default()
                }]),
                table if table.key_bits() == 0 => TableRows::one_default(table),
                table => TableRows::empty(table),
            };
            TableUpdate::Snapshot(rows)
        }),
    }
}

pub const FIRST_LOAD: i64 = 19;
pub const RELOAD: i64 = 23;
pub const THIRD_LOAD: i64 = 29;

pub fn named_frame(
    branch: u64,
    hour: i32,
    group: &str,
    players: usize,
) -> replay_frame::frame::Frame {
    use replay_data::rows::{
        CountryAppearanceRow, CountryLinkRow, CountryRow, LocalPlayerRow, PlayerRow,
        PoliticalPartyRow, PoliticsRow,
    };
    let mut frame = allocation_frame(branch, hour, FIRST_LOAD);
    for rows in [
        TableRows::LocalPlayer(vec![LocalPlayerRow {
            country: 400,
            observer_country: 0,
        }]),
        TableRows::CountryLinks(vec![CountryLinkRow {
            id: 400,
            canonical_slot: 30,
        }]),
        TableRows::Countries(vec![CountryRow {
            id: 400,
            canonical_slot: 30,
            tag: b"Z01".to_vec(),
            ..Default::default()
        }]),
        TableRows::CountryAppearance(vec![CountryAppearanceRow {
            country: 400,
            ..Default::default()
        }]),
        TableRows::Politics(vec![PoliticsRow {
            country: 400,
            ruling_party: Some(7),
            ..Default::default()
        }]),
        TableRows::PoliticalParties(vec![PoliticalPartyRow {
            country: 400,
            id: 7,
            token: group.as_bytes().to_vec(),
            ..Default::default()
        }]),
        TableRows::Players(
            (0..players)
                .map(|id| PlayerRow {
                    id: id as i32,
                    country: 400,
                    ..Default::default()
                })
                .collect(),
        ),
    ] {
        let index = rows.table().index();
        frame.tables[index] = replay_frame::frame::TableUpdate::Snapshot(rows);
    }
    frame
}

pub fn named_session_records(
    session: u64,
    branch: u64,
    hours: &[(i32, &str, usize)],
) -> Vec<replay_capture_spool::spool::Record> {
    use replay_recording::control::ControlRecord;
    use replay_capture_spool::spool::Record;
    let mut records = allocation_records();
    records.truncate(2);
    if let Record::SessionStart(start) = &mut records[0] {
        start.id = session;
    }
    if let Record::BranchStart(start) = &mut records[1] {
        start.id = branch;
        start.session = session;
        start.first_hour = hours[0].0;
        start.loaded_hour = Some(hours[0].0 - 1);
    }
    for (at, &(hour, group, players)) in hours.iter().enumerate() {
        records.push(Record::Frame(named_frame(branch, hour, group, players)));
        records.push(Record::Control(ControlRecord::ClockWitness {
            session,
            qpc: 100 * (at as u64 + 1),
            hour,
            load_signal: Some(FIRST_LOAD),
        }));
    }
    records.push(Record::Control(ControlRecord::Detach {
        session,
        last_hour: Some(hours.last().unwrap().0),
    }));
    records
}

fn push_load(
    records: &mut Vec<replay_capture_spool::spool::Record>,
    branch: u64,
    hours: std::ops::RangeInclusive<i32>,
    load_signal: i64,
) {
    use replay_recording::control::ControlRecord;
    use replay_capture_spool::spool::Record;
    use replay_frame::{branches::BranchReason, branches::BranchStart, branches::Parentage};
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

pub fn two_load_records() -> Vec<replay_capture_spool::spool::Record> {
    let mut records = allocation_records();
    push_load(&mut records, 10, 3..=4, RELOAD);
    records
}

pub fn three_load_records() -> Vec<replay_capture_spool::spool::Record> {
    let mut records = two_load_records();
    push_load(&mut records, 11, 5..=6, THIRD_LOAD);
    records
}

pub fn write_campaign_journal(
    session: &std::path::Path,
    records: &[replay_capture_spool::spool::Record],
) -> std::path::PathBuf {
    let campaign = session.join(std::str::from_utf8(ALLOCATION_CAMPAIGN).unwrap());
    write_capture(&campaign, 0, records, true);
    campaign
}

pub fn write_capture(
    campaign: &std::path::Path,
    number: u32,
    records: &[replay_capture_spool::spool::Record],
    finish: bool,
) {
    let id = campaign.file_name().and_then(|name| name.to_str()).unwrap();
    replay::campaign::fixtures::write_journal(campaign, number, id.as_bytes(), &[records.to_vec()], finish)
        .unwrap();
}

pub fn load_segment_records(
    branch: u64,
    hours: std::ops::RangeInclusive<i32>,
    load: i64,
) -> Vec<replay_capture_spool::spool::Record> {
    use replay_recording::control::ControlRecord;
    use replay_capture_spool::spool::Record;
    use replay_frame::{branches::BranchReason, branches::BranchStart, branches::Parentage};
    let mut records = vec![allocation_records().remove(0)];
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
            qpc: branch * 1_000 + hour as u64,
            hour,
            load_signal: Some(load),
        }));
        records.push(Record::Frame(allocation_frame(branch, hour, load)));
    }
    records
}

pub fn write_load_capture(
    campaign: &std::path::Path,
    number: u32,
    branch: u64,
    hours: std::ops::RangeInclusive<i32>,
    load: i64,
) {
    write_capture(campaign, number, &load_segment_records(branch, hours, load), true);
}

pub fn recorded_frames(path: &std::path::Path) -> Vec<(u64, i32)> {
    let replay = replay::campaign::Replay::open(std::fs::read(path).unwrap()).unwrap();
    replay
        .index
        .iter()
        .flat_map(|block| replay.read_block(block).unwrap())
        .map(|frame| (frame.branch, frame.hour))
        .collect()
}

pub fn allocation_witness(committed_bytes: u64) -> replay_data::witness::TerminalWitness {
    replay_data::witness::TerminalWitness {
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
        uncommitted: None,
        phase: 2,
        code: 1,
        drain_complete: false,
    }
}

pub fn allocation_range(
    branch: u64,
    load_signal: Option<i64>,
    hours: std::ops::RangeInclusive<i32>,
) -> replay_data::witness::UncommittedRange {
    replay_data::witness::UncommittedRange {
        campaign_id: *ALLOCATION_CAMPAIGN,
        branch,
        load_signal,
        first_hour: *hours.start(),
        last_hour: *hours.end(),
    }
}

pub fn allocation_test_directory(name: &str) -> std::path::PathBuf {
    let path = std::env::temp_dir().join(format!("battleplan-recorder-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&path);
    std::fs::create_dir_all(&path).unwrap();
    path
}

pub fn replay(records: Vec<replay_capture_spool::spool::Record>) -> replay::campaign::Replay {
    let encoded =
        replay::campaign::Replay::from_records(recorder_derive::writers(), ALLOCATION_CAMPAIGN.to_vec(), &[records]).unwrap();
    replay::campaign::Replay::open(encoded.bytes().to_vec()).unwrap()
}

#[cfg(test)]
mod tests {
    use super::*;
    use replay_frame::frame::TableUpdate;

    #[test]
    fn allocation_frames_complete_every_recorded_singleton_and_hold_no_derived_table() {
        let frame = allocation_frame(9, 2, FIRST_LOAD);
        for table in TableId::ALL {
            match (&frame.tables[table.index()], table.is_derived()) {
                (TableUpdate::Unavailable { reason }, true) => {
                    assert_eq!(reason, NOT_ACQUIRED, "{table:?}");
                }
                (TableUpdate::Snapshot(rows), false) if table.key_bits() == 0 => {
                    assert_eq!(rows.len(), 1, "{table:?}");
                }
                (TableUpdate::Unavailable { .. }, false) => {}
                (other, _) if table.is_derived() || table.key_bits() == 0 => {
                    panic!("{table:?} is {other:?}")
                }
                _ => {}
            }
        }
        for record in allocation_records() {
            replay_capture_spool::spool::encode_record(&record).unwrap();
        }
        let recording = replay(allocation_records());
        let hours: Vec<i32> = recording
            .index
            .iter()
            .flat_map(|block| recording.read_block(block).unwrap())
            .map(|frame| frame.hour)
            .collect();
        assert_eq!(hours, [2, 3, 4]);
    }
}
