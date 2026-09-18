use std::{
    path::PathBuf,
    sync::atomic::{AtomicBool, Ordering},
};

use replay::{
    branches::{BranchReason, BranchStart, Gap, Parentage},
    container::Replay,
    frame::{DecodedFrame, FrameEncoder, Metrics, TableUpdate, TableView},
    identity::{Identity, Roster, SampleBoundary, SessionStart, Verification},
    rows::{CampaignRow, CountryLinkRow, CountryRow, TableId, TableRows},
    spool::Record,
};

use crate::{
    Analytics, CountrySeriesRequest, MAX_POINTS, PointStatus,
    reduce::{Reducer, Values},
};

const CAMPAIGN: &[u8] = b"analytics-test";

fn session(id: u64) -> SessionStart {
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

fn source(branch: u64, hour: i32, power: Option<i64>) -> DecodedFrame {
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
            name: b"Germany".to_vec(),
            political_power_raw: power,
            command_power_raw: i64::MAX,
            stability_raw: i64::MIN,
            war_support_raw: 70_001,
            convoys_raw: Some(99_001),
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
        tables,
        metrics: Metrics::default(),
    }
}

fn records(branch: u64, frames: Vec<DecodedFrame>) -> Vec<Record> {
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
            .map(|frame| Record::Frame(encoder.encode(frame, false).unwrap())),
    );
    out
}

fn open(segments: Vec<Vec<Record>>) -> Analytics {
    Analytics::from_replay(
        Replay::from_records(CAMPAIGN.into(), &segments).unwrap(),
        PathBuf::from("memory.battleplan"),
        &AtomicBool::new(false),
    )
    .unwrap()
}

fn request(branch: u64, max_points: Option<u32>) -> CountrySeriesRequest {
    CountrySeriesRequest {
        branch_id: branch.to_string(),
        country_id: "7".into(),
        max_points,
        first_hour: None,
        last_hour: None,
    }
}

#[test]
fn canonical_country_aliases_and_full_width_native_values_survive_json() {
    let branch = u64::MAX;
    let mut analytics = open(vec![records(
        branch,
        vec![source(branch, 100, Some(i64::MAX))],
    )]);
    let result = analytics
        .country_series(&request(branch, None), &AtomicBool::new(false))
        .unwrap();
    assert_eq!(result.branch_id, "18446744073709551615");
    assert_eq!(result.session_id, result.branch_id);
    assert_eq!(result.country.as_ref().unwrap().id, "7");
    assert_eq!(analytics.metadata().countries[0].id, "7");
    let values = &result.points[0].first.as_ref().unwrap().values;
    assert_eq!(
        values.political_power.as_deref(),
        Some("9223372036854775807")
    );
    assert_eq!(values.stability.as_deref(), Some("-9223372036854775808"));
    assert_eq!(values.convoys.as_deref(), Some("99001"));
    let json = serde_json::to_string(&result).unwrap();
    assert!(json.contains("\"politicalPower\":\"9223372036854775807\""));
    assert_eq!(
        serde_json::from_str::<crate::CountrySeries>(&json).unwrap(),
        result
    );
    let mut raw_reference = request(branch, None);
    raw_reference.country_id = "9001".into();
    let absent = analytics
        .country_series(&raw_reference, &AtomicBool::new(false))
        .unwrap();
    assert_eq!(absent.points[0].status, PointStatus::Absent);
}

#[test]
fn gaps_partial_tables_absence_and_optional_nulls_are_distinct() {
    let mut partial = source(1, 103, Some(3));
    partial.tables[TableId::Countries.index()] = TableView::Truncated {
        rows: TableRows::Countries(vec![]),
        missing: 1,
    };
    let mut absent = source(1, 104, Some(4));
    absent.tables[TableId::Countries.index()] = TableView::Complete(TableRows::Countries(vec![]));
    let mut input = records(
        1,
        vec![
            source(1, 100, Some(1)),
            source(1, 101, None),
            partial,
            absent,
            source(1, 105, Some(5)),
        ],
    );
    input.push(Record::Gap(Gap {
        branch: 1,
        first_hour: 102,
        last_hour: 102,
        reason: "queue full".into(),
    }));
    let mut analytics = open(vec![input]);
    let result = analytics
        .country_series(&request(1, None), &AtomicBool::new(false))
        .unwrap();
    assert_eq!(
        result
            .points
            .iter()
            .map(|point| point.status)
            .collect::<Vec<_>>(),
        vec![
            PointStatus::Recorded,
            PointStatus::Recorded,
            PointStatus::Gap,
            PointStatus::Unavailable,
            PointStatus::Absent,
            PointStatus::Recorded,
        ]
    );
    assert_eq!(result.coverage.recorded_hours, 3);
    assert_eq!(result.coverage.gap_hours, 1);
    assert_eq!(result.coverage.unavailable_hours, 1);
    assert_eq!(result.coverage.absent_hours, 1);
    assert_eq!(result.points[1].missing_metric_hours.political_power, 1);
    assert!(
        result.points[1]
            .first
            .as_ref()
            .unwrap()
            .values
            .political_power
            .is_none()
    );
    assert!(
        result.points[2]
            .reason
            .as_ref()
            .unwrap()
            .contains("queue full")
    );
    assert!(
        result.points[3]
            .reason
            .as_ref()
            .unwrap()
            .contains("truncated")
    );
}

#[test]
fn downsampling_keeps_exact_signed_extrema_and_every_loss_count() {
    let mut reducer = Reducer::new(0, 19_999, MAX_POINTS);
    for hour in 0..20_000 {
        if hour % 3 == 1 {
            reducer.missing(hour, hour, PointStatus::Gap, "lost hour");
        } else {
            let value = if hour == 777 {
                i64::MIN
            } else if hour == 9000 {
                i64::MAX
            } else {
                i64::from(hour)
            };
            reducer.record(
                hour,
                Values([Some(value), None, Some(value), Some(0), None]),
            );
        }
    }
    let (points, coverage) = reducer.finish();
    assert!(points.len() <= MAX_POINTS as usize);
    assert_eq!(coverage.recorded_hours + coverage.gap_hours, 20_000);
    assert_eq!(coverage.gap_hours, 6667);
    assert!(
        points
            .iter()
            .any(|point| point.min.political_power.as_deref() == Some("-9223372036854775808"))
    );
    assert!(
        points
            .iter()
            .any(|point| point.max.political_power.as_deref() == Some("9223372036854775807"))
    );
    assert!(
        points
            .iter()
            .all(|point| point.status == PointStatus::Mixed)
    );
    assert!(
        points
            .iter()
            .all(|point| point.missing_metric_hours.command_power == point.recorded_hours)
    );
}

#[test]
fn a_full_signed_hour_range_is_bounded_without_per_hour_expansion() {
    let mut reducer = Reducer::new(i32::MIN, i32::MAX, 1);
    reducer.missing(i32::MIN, i32::MAX, PointStatus::Gap, "no data");
    let (points, coverage) = reducer.finish();
    assert_eq!(points.len(), 1);
    assert_eq!(coverage.gap_hours, 4_294_967_296);
    assert_eq!(points[0].status, PointStatus::Gap);
    assert!(points[0].first.is_none());
}

#[test]
fn branch_requests_never_join_overlapping_dates_or_probable_parents() {
    let first = records(1, vec![source(1, 100, Some(10)), source(1, 101, Some(11))]);
    let mut second = records(2, vec![source(2, 101, Some(20)), source(2, 102, Some(21))]);
    if let Record::BranchStart(branch) = &mut second[1] {
        branch.loaded_hour = Some(100);
        branch.parent = Parentage::Probable {
            branch: 1,
            hour: 100,
        };
    }
    let mut analytics = open(vec![first, second]);
    assert_eq!(
        analytics.metadata().branches[1].parent_id.as_deref(),
        Some("1")
    );
    let result = analytics
        .country_series(&request(2, None), &AtomicBool::new(false))
        .unwrap();
    assert_eq!((result.first_hour, result.last_hour), (101, 102));
    assert_eq!(
        result.points[0]
            .first
            .as_ref()
            .unwrap()
            .values
            .political_power
            .as_deref(),
        Some("20")
    );
    assert_eq!(result.coverage.recorded_hours, 2);
}

#[test]
fn recovered_frames_override_overlapping_loss_diagnostics() {
    let mut input = records(1, vec![source(1, 100, Some(1)), source(1, 101, Some(2))]);
    if let Record::BranchStart(branch) = &mut input[1] {
        branch.first_hour = 98;
    }
    input.push(Record::Gap(Gap {
        branch: 1,
        first_hour: 98,
        last_hour: 103,
        reason: "failed flush".into(),
    }));
    let mut analytics = open(vec![input]);
    assert_eq!(analytics.metadata().branches[0].first_hour, 98);
    assert_eq!(analytics.metadata().branches[0].last_hour, 103);
    let result = analytics
        .country_series(&request(1, None), &AtomicBool::new(false))
        .unwrap();
    assert_eq!(result.coverage.recorded_hours, 2);
    assert_eq!(result.coverage.gap_hours, 4);
    assert_eq!(result.points[2].status, PointStatus::Recorded);
    assert_eq!(result.points[3].status, PointStatus::Recorded);
}

#[test]
fn unreadable_block_is_a_gap_and_the_next_keyframe_recovers() {
    let replay = Replay::from_records(
        CAMPAIGN.into(),
        &[records(
            1,
            vec![source(1, 100, Some(1)), source(1, 820, Some(2))],
        )],
    )
    .unwrap();
    let block = &replay.index[0];
    let mut bytes = replay.bytes().to_vec();
    bytes[(block.offset + block.length - 1) as usize] ^= 0x40;
    let mut analytics = Analytics::from_replay(
        Replay::open(bytes).unwrap(),
        "memory.battleplan".into(),
        &AtomicBool::new(false),
    )
    .unwrap();
    let result = analytics
        .country_series(&request(1, None), &AtomicBool::new(false))
        .unwrap();
    assert_eq!(result.coverage.gap_hours, 720);
    assert_eq!(result.coverage.recorded_hours, 1);
    assert_eq!(result.points[0].status, PointStatus::Gap);
    assert!(
        result.points[0]
            .reason
            .as_ref()
            .unwrap()
            .contains("unreadable block")
    );
    assert_eq!(result.points.last().unwrap().status, PointStatus::Recorded);
}

#[test]
fn a_campaign_mismatch_cannot_lend_country_values_to_a_chart() {
    let mut replay = Replay::from_records(
        CAMPAIGN.into(),
        &[records(
            1,
            vec![source(1, 100, Some(1)), source(1, 101, Some(2))],
        )],
    )
    .unwrap();
    replay.campaign_id = b"different campaign".to_vec();
    let mut analytics =
        Analytics::from_replay(replay, "memory.battleplan".into(), &AtomicBool::new(false))
            .unwrap();
    let result = analytics
        .country_series(&request(1, None), &AtomicBool::new(false))
        .unwrap();
    assert_eq!(result.coverage.gap_hours, 2);
    assert!(result.points.iter().all(|point| point.first.is_none()));
    assert!(
        result.points[0]
            .reason
            .as_ref()
            .unwrap()
            .contains("campaign identity")
    );
}

#[test]
fn disagreeing_country_links_are_unavailable_rather_than_absent_or_guessed() {
    let mut frame = source(1, 100, Some(42));
    if let TableView::Complete(TableRows::Countries(rows)) =
        &mut frame.tables[TableId::Countries.index()]
    {
        rows[0].canonical_slot = 8;
    }
    let mut analytics = open(vec![records(1, vec![frame])]);
    let result = analytics
        .country_series(&request(1, None), &AtomicBool::new(false))
        .unwrap();
    assert_eq!(result.points[0].status, PointStatus::Unavailable);
    assert!(result.points[0].first.is_none());
}

#[test]
fn cancelled_queries_do_not_replace_the_last_completed_cache() {
    let mut analytics = open(vec![records(1, vec![source(1, 100, Some(42))])]);
    let cancel = AtomicBool::new(false);
    let first = analytics
        .country_series(&request(1, None), &cancel)
        .unwrap();
    assert_eq!(analytics.scans, 1);
    let repeat = analytics
        .country_series(&request(1, Some(MAX_POINTS)), &cancel)
        .unwrap();
    assert_eq!(first, repeat);
    assert_eq!(analytics.scans, 1);
    cancel.store(true, Ordering::Relaxed);
    assert_eq!(
        analytics
            .country_series(&request(1, Some(1)), &cancel)
            .unwrap_err()
            .kind(),
        std::io::ErrorKind::Interrupted
    );
    assert_eq!(
        analytics
            .country_series(&request(1, None), &cancel)
            .unwrap_err()
            .kind(),
        std::io::ErrorKind::Interrupted
    );
    cancel.store(false, Ordering::Relaxed);
    assert_eq!(
        analytics
            .country_series(&request(1, None), &cancel)
            .unwrap(),
        first
    );
    assert_eq!(analytics.scans, 1);
}

#[test]
fn invalid_identifiers_and_point_budgets_fail_before_scanning() {
    let mut analytics = open(vec![records(1, vec![source(1, 100, Some(42))])]);
    let cancel = AtomicBool::new(false);
    for branch in ["01", "1 ", "+1", "-1", "18446744073709551616", "999"] {
        let mut input = request(1, None);
        input.branch_id = branch.into();
        assert!(analytics.country_series(&input, &cancel).is_err());
    }
    for count in [0, 1201, u32::MAX] {
        assert!(
            analytics
                .country_series(&request(1, Some(count)), &cancel)
                .is_err()
        );
    }
    assert_eq!(analytics.scans, 0);
}

#[test]
fn a_lost_country_baseline_never_reuses_stale_values() {
    let mut input = records(1, vec![source(1, 100, Some(1)), source(1, 101, Some(2))]);
    if let Record::Frame(frame) = &mut input[3] {
        frame.tables[TableId::Countries.index()] = TableUpdate::Unchanged { baseline: 99 };
    }
    let mut analytics = open(vec![input]);
    let result = analytics
        .country_series(&request(1, None), &AtomicBool::new(false))
        .unwrap();
    assert_eq!(result.points[0].status, PointStatus::Recorded);
    assert_eq!(result.points[1].status, PointStatus::Unavailable);
    assert!(result.points[1].first.is_none());
}

#[test]
fn an_initial_failed_census_is_not_a_complete_empty_country_catalogue() {
    let mut failed = source(1, 100, Some(1));
    failed.tables[TableId::Countries.index()] = TableView::Unavailable {
        reason: "initial capacity".into(),
    };
    let analytics = open(vec![records(1, vec![failed, source(1, 101, Some(2))])]);
    assert_eq!(analytics.metadata().countries[0].observed_hour, 101);
}

#[test]
fn ranged_queries_decode_prefixes_but_return_only_requested_hours() {
    let mut analytics = open(vec![records(
        1,
        vec![
            source(1, 100, Some(10)),
            source(1, 103, Some(30)),
            source(1, 104, Some(40)),
        ],
    )]);
    let mut range = request(1, None);
    range.first_hour = Some(101);
    range.last_hour = Some(103);
    let result = analytics
        .country_series(&range, &AtomicBool::new(false))
        .unwrap();
    assert_eq!((result.first_hour, result.last_hour), (101, 103));
    assert_eq!(result.coverage.gap_hours, 2);
    assert_eq!(result.coverage.recorded_hours, 1);
    assert_eq!(
        result
            .points
            .last()
            .unwrap()
            .first
            .as_ref()
            .unwrap()
            .values
            .political_power
            .as_deref(),
        Some("30")
    );
    range.last_hour = Some(102);
    let result = analytics
        .country_series(&range, &AtomicBool::new(false))
        .unwrap();
    assert_eq!(result.coverage.gap_hours, 2);
    assert_eq!(result.coverage.recorded_hours, 0);
    assert_eq!(analytics.scans, 2);
    range.first_hour = Some(99);
    assert!(
        analytics
            .country_series(&range, &AtomicBool::new(false))
            .is_err()
    );
    range.first_hour = Some(104);
    assert!(
        analytics
            .country_series(&range, &AtomicBool::new(false))
            .is_err()
    );
    assert_eq!(analytics.scans, 2);
}
