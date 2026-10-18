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

pub fn allocation_records() -> Vec<replay::spool::Record> {
    use replay::{spool::ControlRecord, spool::Record};
    use replay_data::{
        identity::Identity, identity::SampleBoundary, identity::SessionStart,
        identity::Verification,
    };
    use replay_frame::{branches::BranchReason, branches::BranchStart, branches::Parentage};
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
pub fn allocation_frame(branch: u64, hour: i32, load_signal: i64) -> replay_frame::frame::Frame {
    use replay_data::{rows::CampaignRow, rows::ClockRow};
    use replay_frame::{frame::Frame, frame::TableUpdate};
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
                // Each hour's map-event interval runs from the previous hour to
                // this frame's hour, so consecutive frames continue it.
                TableId::MapEvents => TableRows::MapEvents(vec![replay_data::rows::MapEventsRow {
                    from_hour: hour - 1,
                    to_hour: hour,
                    ..Default::default()
                }]),
                // Every other singleton takes one default row, so a newly
                // registered singleton table keeps the frame complete.
                table if table.key_bits() == 0 => TableRows::one_default(table),
                table => TableRows::empty(table),
            };
            // A current spool holds no retired table; decoding names it.
            match replay_data::format::FormatVersion::CURRENT.table_refusal(rows.table()) {
                Some(reason) => TableUpdate::Unavailable { reason },
                None => TableUpdate::Snapshot(rows),
            }
        }),
    }
}

/// The first load signal (19) of the synthetic campaign's session.
pub const FIRST_LOAD: i64 = 19;
/// A second load of the same campaign later in the session.
pub const RELOAD: i64 = 23;
/// A third load of the same campaign after that.
pub const THIRD_LOAD: i64 = 29;

/// Replace the closing detach of `records` with another load of the synthetic
/// campaign, as the collector writes it (a load-signal branch, then each
/// hour's clock witness before its frame), and detach after it.
fn push_load(
    records: &mut Vec<replay::spool::Record>,
    branch: u64,
    hours: std::ops::RangeInclusive<i32>,
    load_signal: i64,
) {
    use replay::{spool::ControlRecord, spool::Record};
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

/// [`allocation_records`] (branch 9, hours 2 to 4) followed by a second load
/// of the same campaign (branch 10, hours 3 and 4).
pub fn two_load_records() -> Vec<replay::spool::Record> {
    let mut records = allocation_records();
    push_load(&mut records, 10, 3..=4, RELOAD);
    records
}

/// [`two_load_records`] followed by a third load (branch 11, hours 5 and 6).
pub fn three_load_records() -> Vec<replay::spool::Record> {
    let mut records = two_load_records();
    push_load(&mut records, 11, 5..=6, THIRD_LOAD);
    records
}

/// Write `records` as one spool segment of the synthetic campaign in `session`.
pub fn write_campaign_spool(
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
        phase: 2,
        code: 1,
        drain_complete: false,
    }
}

pub fn allocation_test_directory(name: &str) -> std::path::PathBuf {
    let path = std::env::temp_dir().join(format!("battle-plan-recorder-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&path);
    std::fs::create_dir_all(&path).unwrap();
    path
}

pub fn replay(records: Vec<replay::spool::Record>) -> replay::container::Replay {
    let encoded =
        replay::container::Replay::from_records(ALLOCATION_CAMPAIGN.to_vec(), &[records]).unwrap();
    replay::container::Replay::open(encoded.bytes().to_vec()).unwrap()
}

#[cfg(test)]
mod tests {
    use super::*;
    use replay_frame::frame::TableUpdate;

    /// Every singleton table in the registry is complete with one row (or names
    /// its retirement), so a newly registered singleton cannot leave the shared
    /// fixture's frames unencodable.
    #[test]
    fn allocation_frames_complete_every_singleton_table() {
        let frame = allocation_frame(9, 2, FIRST_LOAD);
        for table in TableId::ALL
            .into_iter()
            .filter(|table| table.key_bits() == 0)
        {
            match &frame.tables[table.index()] {
                TableUpdate::Snapshot(rows) => assert_eq!(rows.len(), 1, "{table:?}"),
                TableUpdate::Unavailable { .. } => {}
                other => panic!("{table:?} is {other:?}"),
            }
        }
        for record in allocation_records() {
            replay::spool::encode_record(&record).unwrap();
        }
    }
}
