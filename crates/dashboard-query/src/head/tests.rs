use super::*;
use crate::{
    fixture::*,
    timeline::{Join, Stretch, Timeline},
};
use replay::campaign::Replay;
use replay_data::{
    identity::{CaptureMode, Identity, Roster, SampleBoundary, SessionStart, Verification},
    rows::{
        ArmyTotalRow, CampaignRow, ClockRow, CountryLinkRow, CountryRow, RowCodec, SupplyHealthRow, TableId,
    },
};
use replay_frame::branches::{BranchReason, BranchStart, Parentage};
use replay_journal::synthetic::{Input, Sample, Synthetic};
use serde_json::json;

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

fn branch_start(branch: u64, first: i32) -> BranchStart {
    BranchStart { id: branch, session: branch, loaded_hour: None, first_hour: first, parent: Parentage::Unknown, reason: BranchReason::SessionStart }
}

fn sample<R: RowCodec>(frame: &mut Synthetic, table: TableId, rows: &[R]) -> io::Result<()> {
    frame.table(table, Input::Snapshot { rows, partial: None, read: true, sample: Sample::FromReadBit })
}

#[test]
fn parse_params_rejects_reads_and_invalid_combinations() {
    let fixture = Fixture::new(Data::Power, FIRST..=FIRST + 48);
    let query = fixture.query();
    let with_reads = json!({
        "branchId": "1",
        "family": { "kind": "country-measure", "measure": "politicalPower" },
        "hour": FIRST,
        "binHours": 6,
        "countries": ["GER"],
        "reads": [{ "country": "GER", "hour": FIRST }]
    });
    assert!(parse_params(&query, &with_reads, true).is_err());

    let ranked_with_countries = json!({
        "branchId": "1",
        "family": { "kind": "country-measure", "measure": "politicalPower" },
        "hour": FIRST,
        "binHours": 6,
        "countries": ["GER"],
        "top": 2
    });
    assert!(parse_params(&query, &ranked_with_countries, true).is_err());

    let ranked_zero = json!({
        "branchId": "1",
        "family": { "kind": "country-measure", "measure": "politicalPower" },
        "hour": FIRST,
        "binHours": 6,
        "top": 0
    });
    assert!(parse_params(&query, &ranked_zero, true).is_err());

    let bad_bin = json!({
        "branchId": "1",
        "family": { "kind": "country-measure", "measure": "politicalPower" },
        "hour": FIRST,
        "binHours": 7,
        "countries": ["GER"]
    });
    assert!(parse_params(&query, &bad_bin, true).is_err());
}

#[test]
fn bin_bounds_and_open_bin_acc_handle_i32_min_and_max() {
    assert_eq!(bin_bounds(i32::MIN, 1).unwrap(), (i32::MIN, i32::MIN, None));
    assert!(bin_bounds(i32::MIN, 6).is_err());
    assert_eq!(bin_bounds(i32::MAX, 1).unwrap(), (i32::MAX, i32::MAX, Some(i32::MAX - 1)));
    let (bf, bl, ct) = bin_bounds(i32::MAX, 6).unwrap();
    assert_eq!(bl, i32::MAX);
    assert_eq!(ct, Some(bf - 1));

    let mut min_acc = OpenBinAcc::new(i32::MIN, 1, false);
    min_acc.push_span(i32::MIN, i32::MIN, Some(100_000), true, 100_000.0);
    let min_bin = min_acc.bin.unwrap();
    assert_eq!(min_bin.first_hour, i32::MIN);
    assert_eq!(min_bin.last_hour, i32::MIN);
    assert!(min_bin.first_break);
    assert!(!min_bin.has_break);

    let mut max_acc = OpenBinAcc::new(bf, 6, true);
    max_acc.push_span(bf, i32::MAX, Some(200_000), false, 100_000.0);
    let max_bin = max_acc.bin.unwrap();
    assert_eq!(max_bin.first_hour, bf);
    assert_eq!(max_bin.last_hour, i32::MAX);
    assert!(!max_bin.first_break);
}

#[test]
fn open_bin_handles_shared_slot_distinct_country_ids_on_slot_series() {
    let first = 120;
    let last = 131;
    let campaign = CampaignRow { id: b"slot-swap".to_vec() };
    let replay = Replay::from_synthetic(recorder_derive::writers(), campaign.id.clone(), session(1), branch_start(1, first), last, |frame, hour| {
        sample(frame, TableId::Clock, &[ClockRow { hour, ..ClockRow::default() }])?;
        sample(frame, TableId::Campaign, std::slice::from_ref(&campaign))?;
        let id = if hour < 126 { 9001 } else { 9099 };
        let countries = [CountryRow { id, canonical_slot: 7, tag: b"GER".to_vec(), ..CountryRow::default() }];
        let links = [CountryLinkRow { id, canonical_slot: 7 }];
        let army = [ArmyTotalRow { slot: 7, owner_divisions: if hour < 126 { 10 } else { 25 }, ..ArmyTotalRow::default() }];
        sample(frame, TableId::Countries, &countries)?;
        sample(frame, TableId::CountryLinks, &links)?;
        sample(frame, TableId::ArmyTotals, &army)
    }).unwrap();
    let timeline = Arc::new(Timeline { id: 1, first, last, stretches: vec![Stretch { run: 0, first, last, join: Join::Start }], unheld: Vec::new() });
    let holders = Arc::new(crate::identity::holders(replay.recording(), &timeline).unwrap());
    let query = Query { replay: &replay, timeline, holders };
    let reply = answer(&query, &json!({
        "branchId": "1",
        "family": { "kind": "army-totals", "measure": "ownerDivisions" },
        "hour": 127,
        "binHours": 6,
        "countries": ["GER"]
    })).unwrap();
    let open = reply.lines[0].open_bin.as_ref().unwrap();
    assert_eq!(open.first_hour, 126);
    assert_eq!(open.last_hour, 127);
    assert!(open.first_break);
    assert_eq!(open.first_value, 25.0);
}

#[test]
fn cold_series_head_distinguishes_not_applicable_and_refuses_non_complete_ranked() {
    let first = 120;
    let last = 125;
    let campaign = CampaignRow { id: b"supply-na".to_vec() };
    let replay = Replay::from_synthetic(recorder_derive::writers(), campaign.id.clone(), session(1), branch_start(1, first), last, |frame, hour| {
        sample(frame, TableId::Clock, &[ClockRow { hour, ..ClockRow::default() }])?;
        sample(frame, TableId::Campaign, std::slice::from_ref(&campaign))?;
        let countries = [CountryRow { id: 9001, canonical_slot: 7, tag: b"GER".to_vec(), ..CountryRow::default() }];
        let links = [CountryLinkRow { id: 9001, canonical_slot: 7 }];
        sample(frame, TableId::Countries, &countries)?;
        sample(frame, TableId::CountryLinks, &links)?;
        match hour {
            120 => sample(frame, TableId::SupplyHealth, &[SupplyHealthRow { slot: 7, no_supply: 0, strained: 0, in_supply: 0 }]),
            121 => frame.table::<SupplyHealthRow>(TableId::SupplyHealth, Input::NotRecorded { read: false, sample: Sample::FromReadBit }),
            122 => frame.table(TableId::SupplyHealth, Input::Snapshot { rows: &[SupplyHealthRow { slot: 7, no_supply: 1, strained: 0, in_supply: 1 }], partial: Some(1), read: true, sample: Sample::FromReadBit }),
            _ => sample(frame, TableId::SupplyHealth, &[SupplyHealthRow { slot: 7, no_supply: 1, strained: 0, in_supply: 3 }]),
        }
    }).unwrap();
    let timeline = Arc::new(Timeline { id: 1, first, last, stretches: vec![Stretch { run: 0, first, last, join: Join::Start }], unheld: Vec::new() });
    let holders = Arc::new(crate::identity::holders(replay.recording(), &timeline).unwrap());
    let query = Query { replay: &replay, timeline, holders };

    let na = answer(&query, &json!({
        "branchId": "1",
        "family": { "kind": "supply-health", "measure": "noSupplyShare" },
        "hour": 120,
        "binHours": 6,
        "countries": ["GER"]
    })).unwrap();
    assert_eq!(na.lines[0].value.value_raw, None);
    assert_eq!(na.lines[0].value.not_applicable, Some(true));
    assert_eq!(na.lines[0].value.missing.as_deref(), Some("No divisions"));

    let ranked_not_recorded = answer(&query, &json!({
        "branchId": "1",
        "family": { "kind": "supply-health", "measure": "noSupplyShare" },
        "hour": 121,
        "binHours": 6,
        "top": 2
    }));
    assert!(ranked_not_recorded.is_err());

    let ranked_truncated = answer(&query, &json!({
        "branchId": "1",
        "family": { "kind": "supply-health", "measure": "noSupplyShare" },
        "hour": 122,
        "binHours": 6,
        "top": 2
    }));
    assert!(ranked_truncated.is_err());

    let ranked_complete = answer(&query, &json!({
        "branchId": "1",
        "family": { "kind": "supply-health", "measure": "noSupplyShare" },
        "hour": 123,
        "binHours": 6,
        "top": 2
    })).unwrap();
    assert_eq!(ranked_complete.roster.as_ref().unwrap().entries.len(), 1);
    assert_eq!(ranked_complete.lines[0].value.value_raw.as_deref(), Some("250000"));
}

#[test]
fn mark_prefix_index_counts_intervals_and_skips_unrecorded_countries() {
    let fixture = Fixture::new(Data::Power, FIRST..=FIRST + 300);
    let query = fixture.query();
    let series = dashboard_declare::series("country-measure").unwrap();
    let measure = series.measure("politicalPower").unwrap();
    let line = Line::new(&query, series, measure).unwrap();
    let cache = MarkPrefixIndex::default();
    let ger = Country::from("GER");

    let at_100 = cache.prefix_through(&line, &ger, FIRST + 100).unwrap();
    let at_250 = cache.prefix_through(&line, &ger, FIRST + 250).unwrap();
    let direct_250 = line.drop_interval_count(&ger, FIRST, FIRST + 250).unwrap();
    assert_eq!(at_250, direct_250);
    assert!(at_250 >= at_100);
    assert_eq!(cache.len(), 1);

    let bogus = Country::from("ZZZ");
    assert_eq!(cache.prefix_through(&line, &bogus, FIRST + 250).unwrap(), 0);
    assert_eq!(cache.len(), 1);

    cache.clear();
    assert!(cache.is_empty());
}

#[test]
fn real_two_run_branch_preserves_continuity_across_load_and_breaks_on_gap() {
    let make_segment = |branch: u64, first: i32, hours: i32| -> Vec<replay_capture_spool::spool::Record> {
        let mut out = vec![
            replay_capture_spool::spool::Record::SessionStart(session(branch)),
            replay_capture_spool::spool::Record::BranchStart(branch_start(branch, first)),
        ];
        let mut encoder = replay_frame::frame::FrameEncoder::new();
        for hour in first..first + hours {
            let mut tables = std::array::from_fn(|_| replay_frame::frame::TableView::Unavailable { reason: "outside the fixture".into() });
            tables[TableId::Campaign.index()] = replay_frame::frame::TableView::Complete(replay_data::rows::TableRows::Campaign(vec![CampaignRow { id: b"two-runs".to_vec() }]));
            tables[TableId::Clock.index()] = replay_frame::frame::TableView::Complete(replay_data::rows::TableRows::Clock(vec![ClockRow { hour, ..ClockRow::default() }]));
            tables[TableId::Countries.index()] = replay_frame::frame::TableView::Complete(replay_data::rows::TableRows::Countries(vec![CountryRow {
                id: 9001,
                canonical_slot: 7,
                tag: b"GER".to_vec(),
                political_power_raw: Some(i64::from(hour) * 100_000),
                ..CountryRow::default()
            }]));
            tables[TableId::CountryLinks.index()] = replay_frame::frame::TableView::Complete(replay_data::rows::TableRows::CountryLinks(vec![CountryLinkRow { id: 9001, canonical_slot: 7 }]));
            let mut metrics = replay_frame::frame::Metrics::default();
            metrics.table_ns[TableId::Campaign.index()] = 1;
            let mut input = replay_frame::frame::FrameInput::from(replay_frame::frame::DecodedFrame { branch, hour, tables: tables.map(Into::into), metrics });
            input.scope_to(CaptureMode::Full);
            out.push(replay_capture_spool::spool::Record::Frame(encoder.encode_input(input, false).unwrap()));
        }
        out
    };
    let records = [make_segment(1, 120, 3), make_segment(2, 123, 3)];
    let replay = Replay::from_records(recorder_derive::writers(), b"two-runs".to_vec(), &records)
        .unwrap()
        .into_timelines(&Default::default())
        .unwrap();
    assert_eq!(replay.recording().run_count(), 2);
    let timeline = Arc::new(crate::timeline::resolve(&replay, 1).unwrap());
    let holders = Arc::new(crate::identity::holders(replay.recording(), &timeline).unwrap());
    let query = Query { replay: &replay, timeline, holders };
    let reply = answer(&query, &json!({
        "branchId": "1",
        "family": { "kind": "country-measure", "measure": "politicalPower" },
        "hour": 125,
        "binHours": 6,
        "countries": ["GER"]
    })).unwrap();
    let open = reply.lines[0].open_bin.as_ref().unwrap();
    assert_eq!(open.first_hour, 120);
    assert_eq!(open.last_hour, 125);
    assert!(!open.has_break);
    assert_eq!(open.min_value, 120.0);
    assert_eq!(open.max_value, 125.0);
}
