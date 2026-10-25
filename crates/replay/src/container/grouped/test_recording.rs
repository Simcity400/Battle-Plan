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
    build_branches(campaign, &[(1, hours, &[])], |_, hour, tables| {
        fill(hour, tables)
    })
}

/// One session's branches in order, each with its hours and the hours whose
/// frames are forced keyframes; a later branch starts as a load.
pub(super) fn build_branches(
    campaign: &[u8],
    branches: &[(BranchId, &[Hour], &[Hour])],
    fill: impl FnMut(BranchId, Hour, &mut [TableView; TABLE_COUNT]) -> Metrics,
) -> Replay {
    build_branches_limit(campaign, branches, fill, MAX_BLOCK_RAW_BYTES)
}

pub(super) fn build_branches_limit(
    campaign: &[u8],
    branches: &[(BranchId, &[Hour], &[Hour])],
    fill: impl FnMut(BranchId, Hour, &mut [TableView; TABLE_COUNT]) -> Metrics,
    limit: usize,
) -> Replay {
    build_branches_mode(campaign, branches, fill, limit, CaptureMode::Full)
}

pub(super) fn build_branches_mode(
    campaign: &[u8],
    branches: &[(BranchId, &[Hour], &[Hour])],
    mut fill: impl FnMut(BranchId, Hour, &mut [TableView; TABLE_COUNT]) -> Metrics,
    limit: usize,
    mode: CaptureMode,
) -> Replay {
    let mut records = vec![Record::SessionStart(SessionStart {
        id: 1,
        process_id: 1,
        process_start_time: 1,
        qpc_frequency: 10_000_000,
        sample_boundary: crate::identity::SampleBoundary::EndOfHour,
        capture_mode: mode,
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
    })];
    let mut encoder = FrameEncoder::new();
    for (index, &(branch, hours, keyframes)) in branches.iter().enumerate() {
        let first_hour = *hours.first().expect("a synthetic branch needs an hour");
        records.push(Record::BranchStart(BranchStart {
            id: branch,
            session: 1,
            loaded_hour: Some(first_hour - 1),
            first_hour,
            parent: Parentage::Unknown,
            reason: if index == 0 {
                BranchReason::SessionStart
            } else {
                BranchReason::LoadSignal
            },
        }));
        for &hour in hours {
            let mut tables = std::array::from_fn(|_| TableView::Unavailable {
                reason: "outside the synthetic fixture".into(),
            });
            tables[TableId::Campaign.index()] =
                TableView::Complete(TableRows::Campaign(vec![CampaignRow {
                    id: campaign.to_vec(),
                }]));
            tables[TableId::Clock.index()] =
                TableView::Complete(TableRows::Clock(vec![ClockRow {
                    hour,
                    ..Default::default()
                }]));
            let metrics = fill(branch, hour, &mut tables);
            let mut frame = crate::frame::retire_absent(DecodedFrame {
                branch,
                hour,
                tables: tables.map(Into::into),
                metrics,
            });
            for table in crate::scope::omitted(mode) {
                frame.tables[table.index()] = TableView::NotRecorded { mode }.into();
            }
            let keyframe = keyframes.contains(&hour);
            records.push(Record::Frame(encoder.encode(frame, keyframe).unwrap()));
        }
    }
    Replay::from_record_slices(
        campaign.to_vec(),
        &[records.as_slice()],
        super::super::Observer::Nothing,
        super::super::Frames::Kept,
        limit,
        None,
    )
    .unwrap()
}
