use super::*;
use crate::{
    identity::{Country, Holders},
    series::board,
    tile::{self, BINS, HAS_BREAK, PRESENT},
    timeline::{Join, Stretch, Timeline},
};
use dashboard_declare::{Carry, Column, Key, Measure, Series};
use replay::campaign::Replay;
use replay_columnar::summary::Cells;
use replay_data::{
    identity::{CaptureMode, Identity, Roster, SampleBoundary, SessionStart, Verification},
    rows::{CampaignRow, ClockRow, CountryExperienceRow, CountryFactoriesRow, CountryLinkRow, CountryResourceRow, CountryRow, TableId},
};
use replay_frame::branches::{BranchReason, BranchStart, Parentage};
use replay_journal::synthetic::{Input, Sample, Synthetic};
use std::sync::Arc;

const BASE: i32 = 120;
const LAST_HOUR: i32 = 149;
const T1: i32 = 128;
const T2: i32 = 129;
const T3: i32 = 134;
const UNAVAIL_FIRST: i32 = 139;
const UNAVAIL_LAST: i32 = 140;
const NOTREC_FIRST: i32 = 141;
const NOTREC_LAST: i32 = 142;
const FRA_MID_FIRST: i32 = 146;
const FRA_MID_LAST: i32 = 147;

const EXP_TRUNC: &str = "Country experience read only in part at this hour (1 rows not read)";
const EXP_UNAVAIL: &str = "Country experience not available at this hour: reader fault";
const EXP_NOTREC: &str = "Country experience not recorded at this hour";
const FAC_TRUNC: &str = "Country factories read only in part at this hour (1 rows not read)";
const FAC_UNAVAIL: &str = "Country factories not available at this hour: reader fault";
const FAC_NOTREC: &str = "Country factories not recorded at this hour";
const RES_TRUNC: &str = "Country resources read only in part at this hour (1 rows not read)";

const ARMY_COL: usize = 0;
const NAVY_COL: usize = 3;

fn exp_ratio(cells: &dyn Cells, slot: u32) -> Computed {
    let Some(army) = cells.number(ARMY_COL, slot) else {
        return Computed::Unrecorded;
    };
    let Some(navy) = cells.number(NAVY_COL, slot) else {
        return Computed::Unrecorded;
    };
    if navy <= 0 {
        return Computed::NotApplicable("No navy experience");
    }
    Computed::Value(army / navy)
}

static RATIO_EXP: Series = Series {
    kind: "test-exp-ratio",
    table: TableId::CountryExperience,
    key: Key::Country,
    parts: None,
    measures: &[Measure {
        name: "ratio",
        column: Column::Computed { id: 910, inputs: &["army_raw", "navy_raw"], value: exp_ratio },
        decimals: 0,
        aggregates: 0,
        carry: Carry::None,
        rank: 0,
    }],
    drops: false,
    absent_none: false,
};

static ABSENT_ZERO_EXP: Series = Series {
    kind: "test-exp-zero",
    table: TableId::CountryExperience,
    key: Key::Country,
    parts: None,
    measures: &[Measure {
        name: "army",
        column: Column::Cell("army_raw"),
        decimals: 5,
        aggregates: 0,
        carry: Carry::None,
        rank: 0,
    }],
    drops: false,
    absent_none: true,
};

struct TruncatedFixture {
    replay: Replay,
    timeline: Arc<Timeline>,
    holders: Arc<Holders>,
}

impl TruncatedFixture {
    fn new() -> Self {
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
                dlc: vec![],
                mods: vec![],
                coverage_policy: 1,
                language: "l_english".into(),
                recorder_version: "test".into(),
                collector_hash: [3; 32],
                verification: Verification::Verified { launcher: Roster::default(), memory: Roster::default() },
            },
        };
        let branch = BranchStart { id: 1, session: 1, loaded_hour: None, first_hour: BASE, parent: Parentage::Unknown, reason: BranchReason::SessionStart };
        let campaign = CampaignRow { id: b"truncated-witness".to_vec() };
        let countries = [
            CountryRow { id: 9001, canonical_slot: 7, tag: b"GER".to_vec(), ..CountryRow::default() },
            CountryRow { id: 9002, canonical_slot: 8, tag: b"ITA".to_vec(), ..CountryRow::default() },
            CountryRow { id: 9003, canonical_slot: 9, tag: b"FRA".to_vec(), ..CountryRow::default() },
        ];
        let links = [
            CountryLinkRow { id: 9001, canonical_slot: 7 },
            CountryLinkRow { id: 9002, canonical_slot: 8 },
            CountryLinkRow { id: 9003, canonical_slot: 9 },
        ];
        let exp_ger = CountryExperienceRow { country: 9001, army_raw: 100, navy_raw: 0, ..CountryExperienceRow::default() };
        let exp_ita = CountryExperienceRow { country: 9002, army_raw: 200, navy_raw: 10, ..CountryExperienceRow::default() };
        let exp_fra = CountryExperienceRow { country: 9003, army_raw: 50, navy_raw: 5, ..CountryExperienceRow::default() };
        let fac_ger = CountryFactoriesRow { country: 9001, military_total_raw: 1_000_000, ..CountryFactoriesRow::default() };
        let fac_ita = CountryFactoriesRow { country: 9002, military_total_raw: 2_000_000, ..CountryFactoriesRow::default() };
        let fac_fra = CountryFactoriesRow { country: 9003, military_total_raw: 500_000, ..CountryFactoriesRow::default() };
        let res_ger1 = CountryResourceRow { country: 9001, resource: 1, produced: 110_000, ..CountryResourceRow::default() };
        let res_ger2 = CountryResourceRow { country: 9001, resource: 2, produced: 220_000, ..CountryResourceRow::default() };
        let res_ita1 = CountryResourceRow { country: 9002, resource: 1, produced: 330_000, ..CountryResourceRow::default() };

        let write = |frame: &mut Synthetic, hour: i32| -> io::Result<()> {
            frame.table(TableId::Clock, Input::Snapshot { rows: &[ClockRow { hour, ..ClockRow::default() }], partial: None, read: true, sample: Sample::FromReadBit })?;
            frame.table(TableId::Campaign, Input::Snapshot { rows: std::slice::from_ref(&campaign), partial: None, read: true, sample: Sample::FromReadBit })?;
            frame.table(TableId::Countries, Input::Snapshot { rows: &countries, partial: None, read: true, sample: Sample::FromReadBit })?;
            frame.table(TableId::CountryLinks, Input::Snapshot { rows: &links, partial: None, read: true, sample: Sample::FromReadBit })?;
            match hour {
                T1 => {
                    frame.table(TableId::CountryExperience, Input::Snapshot { rows: std::slice::from_ref(&exp_ger), partial: Some(1), read: true, sample: Sample::FromReadBit })?;
                    frame.table(TableId::CountryFactories, Input::Snapshot { rows: std::slice::from_ref(&fac_ger), partial: Some(1), read: true, sample: Sample::FromReadBit })?;
                    frame.table(TableId::CountryResources, Input::Snapshot { rows: std::slice::from_ref(&res_ger1), partial: Some(1), read: true, sample: Sample::FromReadBit })?;
                }
                T2 => {
                    frame.table(TableId::CountryExperience, Input::Snapshot { rows: std::slice::from_ref(&exp_ita), partial: Some(1), read: true, sample: Sample::FromReadBit })?;
                    frame.table(TableId::CountryFactories, Input::Snapshot { rows: std::slice::from_ref(&fac_ita), partial: Some(1), read: true, sample: Sample::FromReadBit })?;
                    frame.table(TableId::CountryResources, Input::Snapshot { rows: std::slice::from_ref(&res_ita1), partial: Some(1), read: true, sample: Sample::FromReadBit })?;
                }
                T3 => {
                    frame.table(TableId::CountryExperience, Input::Snapshot { rows: &[exp_ger.clone(), exp_ita.clone()], partial: Some(1), read: true, sample: Sample::FromReadBit })?;
                    frame.table(TableId::CountryFactories, Input::Snapshot { rows: &[fac_ger.clone(), fac_ita.clone()], partial: Some(1), read: true, sample: Sample::FromReadBit })?;
                    frame.table(TableId::CountryResources, Input::Snapshot { rows: &[res_ger1.clone(), res_ger2.clone(), res_ita1.clone()], partial: None, read: true, sample: Sample::FromReadBit })?;
                }
                h if (UNAVAIL_FIRST..=UNAVAIL_LAST).contains(&h) => {
                    frame.table::<CountryExperienceRow>(TableId::CountryExperience, Input::Unavailable { reason: "reader fault", read: false, sample: Sample::FromReadBit })?;
                    frame.table::<CountryFactoriesRow>(TableId::CountryFactories, Input::Unavailable { reason: "reader fault", read: false, sample: Sample::FromReadBit })?;
                    frame.table::<CountryResourceRow>(TableId::CountryResources, Input::Unavailable { reason: "reader fault", read: false, sample: Sample::FromReadBit })?;
                }
                h if (NOTREC_FIRST..=NOTREC_LAST).contains(&h) => {
                    frame.table::<CountryExperienceRow>(TableId::CountryExperience, Input::NotRecorded { read: false, sample: Sample::FromReadBit })?;
                    frame.table::<CountryFactoriesRow>(TableId::CountryFactories, Input::NotRecorded { read: false, sample: Sample::FromReadBit })?;
                    frame.table::<CountryResourceRow>(TableId::CountryResources, Input::NotRecorded { read: false, sample: Sample::FromReadBit })?;
                }
                h if (FRA_MID_FIRST..=FRA_MID_LAST).contains(&h) => {
                    frame.table(TableId::CountryExperience, Input::Snapshot { rows: &[exp_ger.clone(), exp_ita.clone(), exp_fra.clone()], partial: None, read: true, sample: Sample::FromReadBit })?;
                    frame.table(TableId::CountryFactories, Input::Snapshot { rows: &[fac_ger.clone(), fac_ita.clone(), fac_fra.clone()], partial: None, read: true, sample: Sample::FromReadBit })?;
                    frame.table(TableId::CountryResources, Input::Snapshot { rows: &[res_ger1.clone(), res_ger2.clone(), res_ita1.clone()], partial: None, read: true, sample: Sample::FromReadBit })?;
                }
                _ => {
                    frame.table(TableId::CountryExperience, Input::Snapshot { rows: &[exp_ger.clone(), exp_ita.clone()], partial: None, read: true, sample: Sample::FromReadBit })?;
                    frame.table(TableId::CountryFactories, Input::Snapshot { rows: &[fac_ger.clone(), fac_ita.clone()], partial: None, read: true, sample: Sample::FromReadBit })?;
                    frame.table(TableId::CountryResources, Input::Snapshot { rows: &[res_ger1.clone(), res_ger2.clone(), res_ita1.clone()], partial: None, read: true, sample: Sample::FromReadBit })?;
                }
            }
            Ok(())
        };

        let replay = Replay::from_synthetic(recorder_derive::writers(), campaign.id.clone(), session, branch, LAST_HOUR, write).unwrap();
        let timeline = Arc::new(Timeline { id: 1, first: BASE, last: LAST_HOUR, stretches: vec![Stretch { run: 0, first: BASE, last: LAST_HOUR, join: Join::Start }], unheld: Vec::new() });
        let holders = Arc::new(crate::identity::holders(replay.recording(), &timeline).unwrap());
        Self { replay, timeline, holders }
    }

    fn query(&self) -> Query<'_> {
        Query { replay: &self.replay, timeline: Arc::clone(&self.timeline), holders: Arc::clone(&self.holders) }
    }
}

fn read(tag: &str, hour: i32, part: Option<&str>) -> SeriesRead {
    SeriesRead { country: tag.into(), hour, part: part.map(str::to_owned) }
}

#[test]
fn exact_states_apply_witness_rule_for_scalar_computed_parts_and_ranked() {
    assert_eq!(dashboard_declare::field(TableId::CountryExperience, "army_raw").unwrap(), ARMY_COL);
    assert_eq!(dashboard_declare::field(TableId::CountryExperience, "navy_raw").unwrap(), NAVY_COL);
    let fixture = TruncatedFixture::new();
    let query = fixture.query();

    let exp_series = dashboard_declare::series("country-experience").unwrap();
    let exp = Line::new(&query, exp_series, exp_series.measure("army").unwrap()).unwrap();
    let states = exp
        .states(
            &[
                read("GER", BASE, None),
                read("FRA", BASE, None),
                read("GER", T1, None),
                read("ITA", T1, None),
                read("FRA", T1, None),
                read("GER", T2, None),
                read("ITA", T2, None),
                read("GER", UNAVAIL_FIRST, None),
                read("GER", NOTREC_FIRST, None),
            ],
            None,
        )
        .unwrap();
    assert_eq!(states[0].run.as_ref().map(|r| r.value_raw.as_str()), Some("100"));
    assert_eq!(states[0].missing, None);
    assert_eq!(states[1].run, None);
    assert_eq!(states[1].missing.as_deref(), Some("FRA's army was not recorded at this hour"));
    assert_eq!(states[2].run.as_ref().map(|r| r.value_raw.as_str()), Some("100"));
    assert_eq!(states[2].missing, None);
    assert_eq!(states[3].run, None);
    assert_eq!(states[3].missing.as_deref(), Some(EXP_TRUNC));
    assert_eq!(states[4].run, None);
    assert_eq!(states[4].missing.as_deref(), Some(EXP_TRUNC));
    assert_eq!(states[5].run, None);
    assert_eq!(states[5].missing.as_deref(), Some(EXP_TRUNC));
    assert_eq!(states[6].run.as_ref().map(|r| r.value_raw.as_str()), Some("200"));
    assert_eq!(states[6].missing, None);
    assert_eq!(states[7].missing.as_deref(), Some(EXP_UNAVAIL));
    assert_eq!(states[8].missing.as_deref(), Some(EXP_NOTREC));

    let ratio = Line::new(&query, &RATIO_EXP, &RATIO_EXP.measures[0]).unwrap();
    let ratio_states = ratio.states(&[read("GER", BASE, None), read("GER", T1, None), read("ITA", T1, None), read("ITA", T2, None)], None).unwrap();
    assert_eq!(ratio_states[0].missing.as_deref(), Some("No navy experience"));
    assert_eq!(ratio_states[0].not_applicable, Some(true));
    assert_eq!(ratio_states[1].missing.as_deref(), Some("No navy experience"), "witnessed truncated computed NotApplicable matches complete");
    assert_eq!(ratio_states[1].not_applicable, Some(true));
    assert_eq!(ratio_states[2].missing.as_deref(), Some(EXP_TRUNC), "unwitnessed truncated computed row uses truncation reason");
    assert_eq!(ratio_states[2].not_applicable, None);
    assert_eq!(ratio_states[3].run.as_ref().map(|r| r.value_raw.as_str()), Some("20"));

    let res_series = dashboard_declare::series("country-resources").unwrap();
    let res = Line::new(&query, res_series, res_series.measure("produced").unwrap()).unwrap();
    let part_states = res.states(&[read("GER", T1, Some("1")), read("GER", T1, Some("2")), read("GER", T1, None)], None).unwrap();
    assert_eq!(part_states[0].run.as_ref().map(|r| r.value_raw.as_str()), Some("110000"));
    assert_eq!(part_states[1].run, None);
    assert_eq!(part_states[1].missing.as_deref(), Some(RES_TRUNC));
    assert_eq!(part_states[2].run, None);
    assert_eq!(part_states[2].missing.as_deref(), Some(RES_TRUNC));

    let ranked_reply = answer(
        &query,
        "series-state",
        &serde_json::json!({ "branchId": "1", "family": { "kind": "country-factories", "measure": "built1" }, "top": 2, "hour": T1 }),
    )
    .unwrap()
    .unwrap();
    assert_eq!(ranked_reply["states"], serde_json::json!([]));
    assert_eq!(ranked_reply["missing"], serde_json::json!(FAC_TRUNC));
}

#[test]
fn summaries_and_tiles_apply_witness_rule_and_preserve_complete_mid_bin_buckets() {
    let fixture = TruncatedFixture::new();
    let query = fixture.query();
    let exp_series = dashboard_declare::series("country-experience").unwrap();
    let exp = Line::new(&query, exp_series, exp_series.measure("army").unwrap()).unwrap();

    let hourly = exp.summaries(&["GER", "ITA", "FRA"], BASE, LAST_HOUR, 30).unwrap();
    assert_eq!(
        hourly[0].1,
        vec![
            MissingSpan { first_hour: T2, last_hour: T2, reason: EXP_TRUNC.into() },
            MissingSpan { first_hour: UNAVAIL_FIRST, last_hour: UNAVAIL_LAST, reason: EXP_UNAVAIL.into() },
            MissingSpan { first_hour: NOTREC_FIRST, last_hour: NOTREC_LAST, reason: EXP_NOTREC.into() },
        ],
        "GER is witnessed at T1 and T3 so only T2, unavailable, and not-recorded are missing"
    );
    assert_eq!(
        hourly[1].1,
        vec![
            MissingSpan { first_hour: T1, last_hour: T1, reason: EXP_TRUNC.into() },
            MissingSpan { first_hour: UNAVAIL_FIRST, last_hour: UNAVAIL_LAST, reason: EXP_UNAVAIL.into() },
            MissingSpan { first_hour: NOTREC_FIRST, last_hour: NOTREC_LAST, reason: EXP_NOTREC.into() },
        ],
        "ITA is witnessed at T2 and T3 so only T1, unavailable, and not-recorded are missing"
    );

    let binned = exp.summaries(&["GER", "ITA", "FRA"], BASE, LAST_HOUR, 5).unwrap();
    assert_eq!(binned[0].1, hourly[0].1);
    assert_eq!(binned[1].1, hourly[1].1);
    assert_eq!(binned[0].0.buckets.len(), 5);
    assert!(!binned[0].0.buckets[0].unwrap().has_break, "complete bin 0 has no break");
    assert!(binned[0].0.buckets[1].unwrap().has_break, "GER unwitnessed at T2 breaks bin 1");
    assert!(!binned[0].0.buckets[2].unwrap().has_break, "GER witnessed at T3 keeps bin 2 unbroken");
    assert!(binned[1].0.buckets[1].unwrap().has_break, "ITA unwitnessed at T1 breaks bin 1");
    assert!(!binned[1].0.buckets[2].unwrap().has_break, "ITA witnessed at T3 keeps bin 2 unbroken");
    let fra_bin4 = binned[2].0.buckets[4].expect("FRA enters and leaves mid-bin 4 on complete hours");
    assert!(!fra_bin4.has_break, "mid-bin start and end on complete hours must match main with has_break == false");
    assert_eq!((fra_bin4.first_value, fra_bin4.last_value, fra_bin4.min_value, fra_bin4.max_value), (50.0, 50.0, 50.0, 50.0));

    let tile_reply = tile::tile(&exp, "1", &[Ask::whole("GER"), Ask::whole("ITA"), Ask::whole("FRA")], None, 6, 0, false).unwrap();
    let bytes = crate::fixture::unbase64(&tile_reply.bins);
    let bin1_slot = ((BASE - tile_reply.first_hour) / 6) as usize + 1;
    let bin2_slot = bin1_slot + 1;
    let bin4_slot = bin1_slot + 3;
    assert_ne!(bytes[bin1_slot] & (PRESENT | HAS_BREAK), PRESENT, "GER tile bin 1 carries HAS_BREAK");
    assert_eq!(bytes[bin2_slot] & (PRESENT | HAS_BREAK), PRESENT, "GER tile bin 2 is present without HAS_BREAK");
    assert_eq!(bytes[2 * BINS + bin4_slot] & (PRESENT | HAS_BREAK), PRESENT, "FRA complete mid-bin tile has no HAS_BREAK");
    assert!(
        tile_reply.lines[0].missing.iter().any(|span| span.first_hour == T2 && span.last_hour == T2 && span.reason == EXP_TRUNC)
            && !tile_reply.lines[0].missing.iter().any(|span| span.first_hour <= T1 && T1 <= span.last_hour),
        "GER tile missing matches Line::missing"
    );
}

#[test]
fn ranked_summary_refuses_truncated_bins_by_name_while_keeping_complete_bins() {
    let fixture = TruncatedFixture::new();
    let query = fixture.query();

    let reply = answer(
        &query,
        "series-summary",
        &serde_json::json!({ "branchId": "1", "family": { "kind": "country-factories", "measure": "built1" }, "top": 2, "from": BASE, "to": LAST_HOUR, "width": 5 }),
    )
    .unwrap()
    .unwrap();
    let expected_missing = serde_json::json!([
        { "firstHour": T1, "lastHour": T2, "reason": FAC_TRUNC },
        { "firstHour": T3, "lastHour": T3, "reason": FAC_TRUNC },
        { "firstHour": UNAVAIL_FIRST, "lastHour": UNAVAIL_LAST, "reason": FAC_UNAVAIL },
        { "firstHour": NOTREC_FIRST, "lastHour": NOTREC_LAST, "reason": FAC_NOTREC },
    ]);
    assert_eq!(reply["missing"], expected_missing);
    let lines = reply["lines"].as_array().unwrap();
    assert_eq!(lines.len(), 2, "GER and ITA remain drawn on complete bins 0 and 4");
    for line in lines {
        let columns: crate::chart::ChartColumns = serde_json::from_value(line["summary"]["buckets"].clone()).unwrap();
        let buckets = columns.buckets().unwrap();
        assert_eq!(buckets.len(), 5);
        assert!(buckets[0].is_some(), "complete bin 0 stays drawn");
        assert!(buckets[1].is_none(), "truncated bin 1 is refused");
        assert!(buckets[2].is_none(), "truncated bin 2 is refused even though GER and ITA were witnessed");
        assert!(buckets[3].is_none(), "unavailable/not-recorded bin 3 is refused");
        assert!(buckets[4].is_some(), "complete bin 4 stays drawn");
    }

    let narrow = answer(
        &query,
        "series-summary",
        &serde_json::json!({ "branchId": "1", "family": { "kind": "country-factories", "measure": "built1" }, "top": 2, "from": T1, "to": T2, "width": 2 }),
    )
    .unwrap()
    .unwrap();
    assert_eq!(narrow["lines"], serde_json::json!([]));
    assert_eq!(narrow["missing"], serde_json::json!([{ "firstHour": T1, "lastHour": T2, "reason": FAC_TRUNC }]));
}

#[test]
fn scoreboard_applies_witness_rule_for_countries_and_top_ranking() {
    let fixture = TruncatedFixture::new();
    let query = fixture.query();
    let exp_series = dashboard_declare::series("country-experience").unwrap();
    let columns = [
        board::Column { line: Line::new(&query, exp_series, exp_series.measure("army").unwrap()).unwrap(), partial: None },
        board::Column { line: Line::new(&query, &RATIO_EXP, &RATIO_EXP.measures[0]).unwrap(), partial: None },
        board::Column { line: Line::new(&query, &ABSENT_ZERO_EXP, &ABSENT_ZERO_EXP.measures[0]).unwrap(), partial: None },
    ];
    let tags = [Country::from("GER"), Country::from("ITA"), Country::from("FRA")];

    let complete = board::at(&columns, board::Rows::Countries(&tags), BASE).unwrap();
    assert_eq!(complete.rows[0].cells[0].value_raw.as_deref(), Some("100"));
    assert_eq!(complete.rows[0].cells[1].missing.as_deref(), Some("No navy experience"));
    assert_eq!(complete.rows[2].cells[2].value_raw.as_deref(), Some("0"), "absent_none is 0 on complete hour");

    let at_t1 = board::at(&columns, board::Rows::Countries(&tags), T1).unwrap();
    assert_eq!(at_t1.missing, None);
    assert_eq!(at_t1.rows[0].cells[0].value_raw.as_deref(), Some("100"), "GER witnessed at T1");
    assert_eq!(at_t1.rows[0].cells[1].missing.as_deref(), Some("No navy experience"), "GER witnessed computed NotApplicable at T1");
    assert_eq!(at_t1.rows[1].cells[0].value_raw, None);
    assert_eq!(at_t1.rows[1].cells[0].missing.as_deref(), Some(EXP_TRUNC));
    assert_eq!(at_t1.rows[2].cells[2].value_raw, None, "unwitnessed absent_none row is never 0 on truncated hour");
    assert_eq!(at_t1.rows[2].cells[2].missing.as_deref(), Some(EXP_TRUNC));

    let at_t2 = board::at(&columns, board::Rows::Countries(&tags), T2).unwrap();
    assert_eq!(at_t2.rows[0].cells[0].missing.as_deref(), Some(EXP_TRUNC));
    assert_eq!(at_t2.rows[1].cells[0].value_raw.as_deref(), Some("200"), "ITA witnessed at T2");
    assert_eq!(at_t2.rows[1].cells[1].value_raw.as_deref(), Some("20"));

    let top_t1 = board::at(&columns, board::Rows::Top { by: 0, n: 2, least: false }, T1).unwrap();
    assert!(top_t1.rows.is_empty());
    assert_eq!(top_t1.missing.as_deref(), Some(EXP_TRUNC));

    let top_t3 = board::at(&columns, board::Rows::Top { by: 0, n: 2, least: false }, T3).unwrap();
    assert!(top_t3.rows.is_empty());
    assert_eq!(top_t3.missing.as_deref(), Some(EXP_TRUNC));
}
