use super::*;
use crate::{SourceKey, test_fixtures::source as test_frame};
use dashboard_query::head::SeriesHeadReply;
use playhead::{
    Piece, Playhead, Settings, Step, Timeline as PlayheadTimeline,
    pages::{Pages, Span, WorldPages},
};
use replay::campaign::Replay;
use replay_capture_spool::spool::Record;
use replay_data::{
    identity::{CaptureMode, Identity, Roster, SampleBoundary, SessionStart, Verification},
    rows::{
        ArmyTotalRow, CampaignRow, ClockRow, CountryLinkRow, CountryRow, RowCodec, SupplyHealthRow, TableId, TableRows,
    },
};
use replay_frame::{
    branches::{BranchReason, BranchStart, Parentage},
    frame::{FrameEncoder, FrameInput, TableView},
};
use replay_journal::synthetic::{Input, Sample, Synthetic};
use serde_json::json;
use std::sync::mpsc;

fn session(branch: u64) -> SessionStart {
    SessionStart {
        id: branch,
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
            dlc: vec![],
            mods: vec![],
            coverage_policy: 1,
            language: "l_english".into(),
            recorder_version: "test".into(),
            collector_hash: [3; 32],
            verification: Verification::Verified { launcher: Roster::default(), memory: Roster::default() },
        },
    }
}

fn branch_start(branch: u64, first: i32, reason: BranchReason, parent: Parentage) -> BranchStart {
    BranchStart { id: branch, session: branch, loaded_hour: None, first_hour: first, parent, reason }
}

fn sample<R: RowCodec>(frame: &mut Synthetic, table: TableId, rows: &[R]) -> io::Result<()> {
    frame.table(table, Input::Snapshot { rows, partial: None, read: true, sample: Sample::FromReadBit })
}

fn make_source(replay: Replay) -> Source {
    Source::from_replay(
        SourceKey { source_id: format!("series-head-test-{}", uuid::Uuid::new_v4()), revision: "1".into() },
        "synthetic.battleplan".into(),
        Arc::new(replay),
        &AtomicBool::new(false),
    )
    .unwrap()
}

fn drive_differential(
    source: &Source,
    branch: u64,
    params: &Value,
    schedule: &[(i32, bool)],
) -> Vec<(i32, u64, Result<SeriesHeadReply, String>)> {
    let query = crate::queries::query(source, branch).unwrap();
    let (parsed, _) = parse_params(&query, params, false).unwrap();
    let parsed = Arc::new(parsed);
    let index = branch_index(source, &query);
    let marks = marks_cache(source);
    let engine = Arc::new(Mutex::new(Engine::new(Arc::clone(&parsed), index, Arc::clone(&marks))));
    let initial_lanes = engine.lock().unwrap().lanes.clone();
    let world = WorldPages::over(
        replay_recording::columnar_frames::Recording::open(source.replay_ref().bytes().to_vec()).unwrap(),
        Span::Whole,
    )
    .unwrap();
    let pieces: Vec<Piece> = query
        .timeline
        .stretches
        .iter()
        .map(|s| Piece { run: world.run_of(s.run).expect("run"), first: s.first, last: s.last })
        .collect();
    let pages: Arc<dyn Pages> = Arc::new(world);
    let timeline = PlayheadTimeline::new(&*pages, pieces).unwrap();

    let (send, recv) = mpsc::channel::<(i32, u64, Result<SeriesHeadReply, String>)>();
    let hook_engine = Arc::clone(&engine);
    let hook_replay = source.replay();
    let hook_timeline = Arc::clone(&query.timeline);
    let hook_holders = Arc::clone(&query.holders);
    let hook_parsed = Arc::clone(&parsed);
    let hook_marks = Arc::clone(&marks);

    let hook: playhead::Hook = Box::new(move |step: &Step, lanes: &playhead::Lanes<'_>| {
        let hour = step.last();
        let q = Query { replay: &hook_replay, timeline: Arc::clone(&hook_timeline), holders: Arc::clone(&hook_holders) };
        let read = hook_engine.lock().unwrap().read(&q, hour, Some(ProductStep { step, lanes, held: None, wake: None }));
        let inc: Result<SeriesHeadReply, String> = match &*read.value {
            Ok(v) => Ok(serde_json::from_value(v.clone()).unwrap()),
            Err(e) => Err(e.clone()),
        };
        let line = Line::new(&q, hook_parsed.series, hook_parsed.measure).unwrap();
        let cold = compute_cold_counted(&line, &hook_parsed, hour, Some(&hook_marks)).map(|(r, _)| r).map_err(|e| e.to_string());
        assert_eq!(inc, cold, "incremental vs cold mismatch at hour {hour} (step {}..={}, reset={})", step.first(), step.last(), step.reset());
        let _ = send.send((hour, read.rows, inc));
    });

    let settings = Settings { step_threads: 1, fetch_threads: 1, ahead: 1, ring: 3, speed: 1.0, rate: 1.0, ..Settings::default() };
    let playhead = Playhead::new(Arc::clone(&pages), timeline, settings, Some(hook)).unwrap();
    let reader_id = playhead.declare(initial_lanes, None).unwrap();
    let mut consumer = playhead.consumer(reader_id);
    let mut out = Vec::new();
    let mut last_hour: Option<i32> = None;

    for &(target, seek) in schedule {
        if seek {
            playhead.seek(target);
        } else {
            let delta = last_hour.map_or(1, |prev| (target - prev).max(1));
            playhead.pace(f64::from(delta));
        }
        loop {
            let mut got = None;
            playhead
                .drive_until(&mut consumer, target, || match recv.try_recv() {
                    Ok(item) => {
                        got = Some(item);
                        std::ops::ControlFlow::Break(Ok(()))
                    }
                    Err(mpsc::TryRecvError::Empty) => std::ops::ControlFlow::Continue(()),
                    Err(mpsc::TryRecvError::Disconnected) => std::ops::ControlFlow::Break(Err("hook disconnected".into())),
                })
                .unwrap_or_else(|error| panic!("drive_until {target} failed: {error}"));
            let item = got.unwrap();
            let reached = item.0;
            out.push(item);
            last_hour = Some(reached);
            if reached >= target {
                break;
            }
        }
    }
    drop(consumer);
    playhead.finish();
    out
}

fn synthetic_power_replay(first: i32, last: i32) -> Replay {
    let campaign = CampaignRow { id: b"diff-power".to_vec() };
    Replay::from_synthetic(
        recorder_derive::writers(),
        campaign.id.clone(),
        session(1),
        branch_start(1, first, BranchReason::SessionStart, Parentage::Unknown),
        last,
        |frame, hour| {
            sample(frame, TableId::Clock, &[ClockRow { hour, ..ClockRow::default() }])?;
            sample(frame, TableId::Campaign, std::slice::from_ref(&campaign))?;
            let ger_pp = match hour % 37 {
                15..=17 => None,
                rem => Some(if hour % 29 == 0 { 100_000 } else { 500_000 + i64::from(rem) * 10_000 }),
            };
            let ita_pp = Some(300_000 + i64::from((hour * 7) % 50) * 12_000);
            let mut countries = vec![
                CountryRow { id: 9001, canonical_slot: 7, tag: b"GER".to_vec(), political_power_raw: ger_pp, ..CountryRow::default() },
                CountryRow { id: 9002, canonical_slot: 8, tag: b"ITA".to_vec(), political_power_raw: ita_pp, ..CountryRow::default() },
            ];
            if hour >= first + 120 {
                countries.push(CountryRow { id: 9003, canonical_slot: 9, tag: b"SOV".to_vec(), political_power_raw: Some(800_000 - i64::from((hour - first) % 40) * 15_000), ..CountryRow::default() });
            }
            let links: Vec<_> = countries.iter().map(|c| CountryLinkRow { id: c.id, canonical_slot: c.canonical_slot }).collect();
            sample(frame, TableId::Countries, &countries)?;
            sample(frame, TableId::CountryLinks, &links)
        },
    )
    .unwrap()
}

#[test]
fn selected_single_24h_multiday_and_random_steps_match_cold() {
    let first = 120;
    let last = first + 360;
    let source = make_source(synthetic_power_replay(first, last));
    for bin_hours in [1, 6, 24] {
        for with_marks in [false, true] {
            let params = json!({
                "branchId": "1",
                "family": { "kind": "country-measure", "measure": "politicalPower" },
                "binHours": bin_hours,
                "countries": ["GER", "ITA"],
                "withMarks": with_marks
            });
            let mut schedule = vec![(first, true)];
            for step in [1, 1, 1, 6, 24, 24, 72, 13, 31, 5, 48, 19, 24] {
                let next = (schedule.last().unwrap().0 + step).min(last);
                schedule.push((next, false));
                if next == last {
                    break;
                }
            }
            let out = drive_differential(&source, 1, &params, &schedule);
            assert!(!out.is_empty());
        }
    }
}

#[test]
fn within_membership_entry_exit_reentry_mid_bin_and_spends_outside_group() {
    let first = 120;
    let last = first + 240;
    let source = make_source(synthetic_power_replay(first, last));
    let params = json!({
        "branchId": "1",
        "family": { "kind": "country-measure", "measure": "politicalPower" },
        "binHours": 24,
        "countries": ["GER"],
        "within": [["GER", [
            { "firstHour": first, "lastHour": first + 10 },
            { "firstHour": first + 20, "lastHour": first + 55 },
            { "firstHour": first + 75, "lastHour": last }
        ]]],
        "withMarks": true
    });
    let mut schedule = vec![(first, true)];
    for h in (first + 1..=first + 96).step_by(1) {
        schedule.push((h, false));
    }
    schedule.push((first + 120, false));
    schedule.push((first + 144, false));
    schedule.push((first + 168, false));
    let out = drive_differential(&source, 1, &params, &schedule);
    assert!(!out.is_empty());
}

#[test]
fn holder_changes_shared_slot_distinct_ids_and_unrelated_holder_edges() {
    let first = 120;
    let last = first + 180;
    let campaign = CampaignRow { id: b"holder-edges".to_vec() };
    let replay = Replay::from_synthetic(
        recorder_derive::writers(),
        campaign.id.clone(),
        session(1),
        branch_start(1, first, BranchReason::SessionStart, Parentage::Unknown),
        last,
        |frame, hour| {
            sample(frame, TableId::Clock, &[ClockRow { hour, ..ClockRow::default() }])?;
            sample(frame, TableId::Campaign, std::slice::from_ref(&campaign))?;
            let ger_id = if hour < first + 60 { 9001 } else { 9091 };
            let mut countries = vec![
                CountryRow { id: ger_id, canonical_slot: 7, tag: b"GER".to_vec(), political_power_raw: Some(200_000), ..CountryRow::default() },
            ];
            if (hour / 12) % 2 == 0 {
                countries.push(CountryRow { id: 9050 + ((hour / 24) as u32), canonical_slot: 12, tag: b"POL".to_vec(), political_power_raw: Some(50_000), ..CountryRow::default() });
            }
            let links: Vec<_> = countries.iter().map(|c| CountryLinkRow { id: c.id, canonical_slot: c.canonical_slot }).collect();
            let army = [ArmyTotalRow { slot: 7, owner_divisions: if hour < first + 60 { 10 } else { 30 }, ..ArmyTotalRow::default() }];
            sample(frame, TableId::Countries, &countries)?;
            sample(frame, TableId::CountryLinks, &links)?;
            sample(frame, TableId::ArmyTotals, &army)
        },
    )
    .unwrap();
    let source = make_source(replay);
    let params = json!({
        "branchId": "1",
        "family": { "kind": "army-totals", "measure": "ownerDivisions" },
        "binHours": 6,
        "countries": ["GER"]
    });
    let mut schedule = vec![(first, true)];
    for h in (first + 1..=first + 96).step_by(3) {
        schedule.push((h, false));
    }
    let out = drive_differential(&source, 1, &params, &schedule);
    assert!(!out.is_empty());
}

#[test]
fn ranked_top_n_swaps_and_least_match_cold() {
    let first = 120;
    let last = first + 240;
    let source = make_source(synthetic_power_replay(first, last));
    for least in [false, true] {
        let params = json!({
            "branchId": "1",
            "family": { "kind": "country-measure", "measure": "politicalPower" },
            "binHours": 6,
            "top": 2,
            "least": least,
            "withMarks": true
        });
        let mut schedule = vec![(first, true)];
        for step in [1, 1, 6, 24, 24, 48, 24, 24, 24] {
            let next = (schedule.last().unwrap().0 + step).min(last);
            schedule.push((next, false));
        }
        let out = drive_differential(&source, 1, &params, &schedule);
        assert!(!out.is_empty());
    }
}

#[test]
fn truncation_not_recorded_and_not_applicable_match_cold() {
    let first = 120;
    let last = first + 48;
    let campaign = CampaignRow { id: b"status-diff".to_vec() };
    let replay = Replay::from_synthetic(
        recorder_derive::writers(),
        campaign.id.clone(),
        session(1),
        branch_start(1, first, BranchReason::SessionStart, Parentage::Unknown),
        last,
        |frame, hour| {
            sample(frame, TableId::Clock, &[ClockRow { hour, ..ClockRow::default() }])?;
            sample(frame, TableId::Campaign, std::slice::from_ref(&campaign))?;
            let countries = [CountryRow { id: 9001, canonical_slot: 7, tag: b"GER".to_vec(), ..CountryRow::default() }];
            let links = [CountryLinkRow { id: 9001, canonical_slot: 7 }];
            sample(frame, TableId::Countries, &countries)?;
            sample(frame, TableId::CountryLinks, &links)?;
            match hour - first {
                0..=5 => sample(frame, TableId::SupplyHealth, &[SupplyHealthRow { slot: 7, no_supply: 0, strained: 0, in_supply: 0 }]),
                6..=11 => frame.table::<SupplyHealthRow>(TableId::SupplyHealth, Input::NotRecorded { read: false, sample: Sample::FromReadBit }),
                12..=17 => frame.table(TableId::SupplyHealth, Input::Snapshot { rows: &[SupplyHealthRow { slot: 7, no_supply: 1, strained: 0, in_supply: 1 }], partial: Some(1), read: true, sample: Sample::FromReadBit }),
                _ => sample(frame, TableId::SupplyHealth, &[SupplyHealthRow { slot: 7, no_supply: 1, strained: 1, in_supply: 2 }]),
            }
        },
    )
    .unwrap();
    let source = make_source(replay);
    let selected = json!({
        "branchId": "1",
        "family": { "kind": "supply-health", "measure": "noSupplyShare" },
        "binHours": 6,
        "countries": ["GER"]
    });
    let ranked = json!({
        "branchId": "1",
        "family": { "kind": "supply-health", "measure": "noSupplyShare" },
        "binHours": 6,
        "top": 2
    });
    let schedule: Vec<(i32, bool)> = std::iter::once((first, true)).chain((first + 1..=last).step_by(3).map(|h| (h, false))).collect();
    drive_differential(&source, 1, &selected, &schedule);
    drive_differential(&source, 1, &ranked, &schedule);
}

#[test]
fn real_two_run_branch_switch_and_backward_seeks_match_cold() {
    let make_segment = |branch: u64, first: i32, hours: i32, base_pp: i64| -> Vec<Record> {
        let mut out = vec![
            Record::SessionStart(session(branch)),
            Record::BranchStart(branch_start(branch, first, BranchReason::SessionStart, Parentage::Unknown)),
        ];
        let mut encoder = FrameEncoder::new();
        for hour in first..first + hours {
            let mut frame = test_frame(branch, hour, None);
            *frame.tables[TableId::Countries.index()] = TableView::Complete(TableRows::Countries(vec![
                CountryRow { id: 9001, canonical_slot: 7, tag: b"GER".to_vec(), political_power_raw: Some(base_pp + i64::from(hour) * 1000), ..CountryRow::default() },
            ]));
            *frame.tables[TableId::CountryLinks.index()] = TableView::Complete(TableRows::CountryLinks(vec![
                CountryLinkRow { id: 9001, canonical_slot: 7 },
            ]));
            let mut input = FrameInput::from(frame);
            input.scope_to(CaptureMode::Full);
            out.push(Record::Frame(encoder.encode_input(input, false).unwrap()));
        }
        out
    };
    let records = [make_segment(1, 120, 48, 100_000), make_segment(2, 168, 48, 200_000)];
    let replay = Replay::from_records(recorder_derive::writers(), b"two-run-diff".to_vec(), &records)
        .unwrap()
        .into_timelines(&Default::default())
        .unwrap();
    assert_eq!(replay.recording().run_count(), 2);
    let source = make_source(replay);
    let params = json!({
        "branchId": "1",
        "family": { "kind": "country-measure", "measure": "politicalPower" },
        "binHours": 6,
        "countries": ["GER"]
    });
    let schedule = vec![(120, true), (144, false), (168, false), (192, false), (132, true), (156, false), (215, false)];
    drive_differential(&source, 1, &params, &schedule);
}

#[test]
fn scale_1500_countries_24h_step_row_visits_bounded() {
    let first = 120;
    let last = first + 72;
    let count = 1500usize;
    let tags: Vec<Vec<u8>> = (0..count)
        .map(|i| {
            let a = b'A' + ((i / (26 * 26)) % 26) as u8;
            let b = b'A' + ((i / 26) % 26) as u8;
            let c = b'A' + (i % 26) as u8;
            vec![a, b, c]
        })
        .collect();
    let campaign = CampaignRow { id: b"scale-1500".to_vec() };
    let replay = Replay::from_synthetic(
        recorder_derive::writers(),
        campaign.id.clone(),
        session(1),
        branch_start(1, first, BranchReason::SessionStart, Parentage::Unknown),
        last,
        |frame, hour| {
            sample(frame, TableId::Clock, &[ClockRow { hour, ..ClockRow::default() }])?;
            sample(frame, TableId::Campaign, std::slice::from_ref(&campaign))?;
            let day = (hour - first) / 24;
            let countries: Vec<CountryRow> = (0..count)
                .map(|i| {
                    let id = 10_000 + i as u32;
                    let slot = i as u32 + 1;
                    let base = i64::from(day) * 100_000 + (i as i64) * 100;
                    let swap = if i == 1490 || i == 1491 { i64::from((day + (i as i32)) % 2) * 500 } else { 0 };
                    CountryRow {
                        id,
                        canonical_slot: slot,
                        tag: tags[i].clone(),
                        political_power_raw: Some(base + swap),
                        ..CountryRow::default()
                    }
                })
                .collect();
            let links: Vec<CountryLinkRow> = countries.iter().map(|c| CountryLinkRow { id: c.id, canonical_slot: c.canonical_slot }).collect();
            sample(frame, TableId::Countries, &countries)?;
            sample(frame, TableId::CountryLinks, &links)
        },
    )
    .unwrap();
    let source = make_source(replay);
    let first_tag = std::str::from_utf8(&tags[0]).unwrap();
    let selected_params = json!({
        "branchId": "1",
        "family": { "kind": "country-measure", "measure": "politicalPower" },
        "binHours": 6,
        "countries": [first_tag]
    });
    let schedule = vec![(first, true), (first + 24, false), (first + 48, false), (first + 72, false)];
    let selected_out = drive_differential(&source, 1, &selected_params, &schedule);
    for &(_, rows, _) in &selected_out[1..] {
        assert!(rows <= 2, "1 selected country over 1500 should visit O(1) rows per 24h step, got {rows}");
    }

    let ranked_params = json!({
        "branchId": "1",
        "family": { "kind": "country-measure", "measure": "politicalPower" },
        "binHours": 6,
        "top": 10
    });
    let ranked_out = drive_differential(&source, 1, &ranked_params, &schedule);
    for &(_, rows, _) in &ranked_out[1..] {
        assert!(rows <= 1600, "top-10 over 1500 should visit at most one daily tick per 24h step, got {rows}");
    }
}
