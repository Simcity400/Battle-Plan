use std::io;

use replay::campaign::Replay;
use replay_recording::control::ControlRecord;
use replay_capture_spool::spool::Record;
use replay_data::{
    identity::CaptureMode, identity::Identity, identity::Roster, identity::SampleBoundary,
    identity::SessionStart, identity::Verification, rows::CampaignRow, rows::CountryLinkRow,
    rows::CountryRow, rows::TableId, rows::TableRows,
};
use replay_frame::{
    branches::BranchReason, branches::BranchStart, branches::Parentage, frame::Frame,
    frame::Metrics, frame::TableUpdate,
};

pub fn recording(partial: bool) -> io::Result<Replay> {
    recording_sequence(partial, false)
}

pub fn recording_sequence(partial: bool, sequence: bool) -> io::Result<Replay> {
    let campaign = b"synthetic-fuzz-campaign";
    let session = SessionStart {
        id: 1,
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
            dlc: Vec::new(),
            mods: Vec::new(),
            coverage_policy: 1,
            language: "l_english".into(),
            recorder_version: "fuzz".into(),
            collector_hash: [3; 32],
            verification: Verification::Verified {
                launcher: Roster::default(),
                memory: Roster::default(),
            },
        },
    };
    let mut frame = Frame {
        branch: 1,
        hour: 0,
        keyframe: true,
        metrics: Metrics::default(),
        tables: std::array::from_fn(|_| TableUpdate::Unavailable {
            reason: "synthetic".into(),
        }),
    };
    frame.tables[TableId::Campaign.index()] =
        TableUpdate::Snapshot(TableRows::Campaign(vec![CampaignRow {
            id: campaign.to_vec(),
        }]));
    frame.tables[TableId::Countries.index()] =
        TableUpdate::Snapshot(TableRows::Countries(vec![CountryRow {
            id: 1,
            canonical_slot: 1,
            tag: b"SYN".to_vec(),
            ..Default::default()
        }]));
    let country_links = || {
        TableUpdate::Snapshot(TableRows::CountryLinks(vec![CountryLinkRow {
            id: 1,
            canonical_slot: 1,
        }]))
    };
    if !sequence {
        frame.tables[TableId::CountryLinks.index()] = country_links();
    }
    let mut records = vec![
        Record::SessionStart(session),
        Record::BranchStart(BranchStart {
            id: 1,
            session: 1,
            loaded_hour: None,
            first_hour: 0,
            parent: Parentage::Unknown,
            reason: BranchReason::SessionStart,
        }),
        Record::Frame(frame),
    ];
    if sequence {
        let previous = match records.last().unwrap() {
            Record::Frame(frame) => frame.clone(),
            _ => unreachable!(),
        };
        let mut next = previous;
        next.hour = 1;
        next.keyframe = false;
        next.tables[TableId::Campaign.index()] = TableUpdate::Unchanged { baseline: 0 };
        let mut row = CountryRow {
            id: 1,
            canonical_slot: 1,
            tag: b"SYN".to_vec(),
            ..Default::default()
        };
        let before = row.clone();
        row.command_power_raw = 1;
        use replay_data::rows::RowCodec;
        next.tables[TableId::Countries.index()] = TableUpdate::Delta {
            baseline: 0,
            changed: vec![row.delta(Some(&before))?.unwrap()],
            deleted: vec![],
        };
        next.tables[TableId::CountryLinks.index()] = country_links();
        records.push(Record::Frame(next));
    }
    if partial {
        records.push(Record::Control(ControlRecord::Partial {
            session: 1,
            reason: "synthetic capture failure".into(),
            failed_hour: None,
        }));
    }
    Replay::from_records(recorder_derive::writers(), campaign.to_vec(), &[records])
}
