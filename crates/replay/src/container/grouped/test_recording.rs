//! Shared synthetic recording setup for grouped reader tests.
use super::*;
use crate::{
    branches::{BranchReason, BranchStart, Parentage},
    frame::FrameEncoder,
    identity::{Identity, Roster, SessionStart, Verification},
    rows::{CampaignRow, ClockRow},
    spool::Record,
};

pub(super) fn build(
    campaign: &[u8],
    hours: &[Hour],
    mut fill: impl FnMut(Hour, &mut [TableView; TABLE_COUNT]) -> Metrics,
) -> Replay {
    let first_hour = *hours.first().expect("a synthetic recording needs an hour");
    let mut records = vec![
        Record::SessionStart(SessionStart {
            id: 1,
            process_id: 1,
            process_start_time: 1,
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
                recorder_version: "test".into(),
                collector_hash: [3; 32],
                verification: Verification::Verified {
                    launcher: Roster::default(),
                    memory: Roster::default(),
                },
            },
        }),
        Record::BranchStart(BranchStart {
            id: 1,
            session: 1,
            loaded_hour: Some(first_hour - 1),
            first_hour,
            parent: Parentage::Unknown,
            reason: BranchReason::SessionStart,
        }),
    ];
    let mut encoder = FrameEncoder::new();
    for &hour in hours {
        let mut tables = std::array::from_fn(|_| TableView::Unavailable {
            reason: "outside the synthetic fixture".into(),
        });
        tables[TableId::Campaign.index()] =
            TableView::Complete(TableRows::Campaign(vec![CampaignRow {
                id: campaign.to_vec(),
            }]));
        tables[TableId::Clock.index()] = TableView::Complete(TableRows::Clock(vec![ClockRow {
            hour,
            ..Default::default()
        }]));
        let metrics = fill(hour, &mut tables);
        let frame = crate::frame::retire_absent(DecodedFrame {
            branch: 1,
            hour,
            tables: tables.map(Into::into),
            metrics,
        });
        records.push(Record::Frame(encoder.encode(frame, false).unwrap()));
    }
    Replay::from_records(campaign.to_vec(), &[records]).unwrap()
}
