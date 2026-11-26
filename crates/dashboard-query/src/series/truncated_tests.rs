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
const EXP_TRUNC_TABLE: &str = "Country experience read only in part at this hour (1 rows not read), and which rows were read is not held at the head";
const EXP_TRUNC_STATUS: &str = "country experience: read only in part at this hour (1 rows not read)";
const EXP_UNAVAIL: &str = "Country experience not available at this hour: reader fault";
const EXP_UNAVAIL_STATUS: &str = "country experience: not available at this hour";
const EXP_NOTREC: &str = "Country experience not recorded at this hour";
const EXP_NOTREC_STATUS: &str = "country experience: not recorded at this hour";
const RES_TRUNC: &str = "Country resources read only in part at this hour (1 rows not read)";
const RES_TRUNC_STATUS: &str = "country resources: read only in part at this hour (1 rows not read)";
const FAC_TRUNC_STATUS: &str = "country factories: read only in part at this hour (1 rows not read)";
const FAC_TRUNC: &str = "Country factories read only in part at this hour (1 rows not read)";
const FAC_TRUNC_TABLE: &str = "Country factories read only in part at this hour (1 rows not read), and which rows were read is not held at the head";
const REL_TRUNC: &str = "Relation opinions read only in part at this hour (1 rows not read)";
const REL_TRUNC_STATUS: &str = "relation opinions: read only in part at this hour (1 rows not read)";

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
        rank_zero: dashboard_declare::RankZero::Keep,
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
        rank_zero: dashboard_declare::RankZero::Keep,
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
            CountryRow { id: 9001, canonical_slot: 7, tag: b"GER".to_vec(), political_power_raw: Some(300_000), ..CountryRow::default() },
            CountryRow { id: 9002, canonical_slot: 8, tag: b"ITA".to_vec(), political_power_raw: Some(200_000), ..CountryRow::default() },
            CountryRow { id: 9003, canonical_slot: 9, tag: b"FRA".to_vec(), political_power_raw: Some(100_000), ..CountryRow::default() },
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
fn exact_states_apply_witness_rule_for_scalar_computed_and_parts() {
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
    assert_eq!(part_states[2].missing.as_deref(), Some(RES_TRUNC_STATUS));

    let ita_parts = res.part_lives(&["ITA"], BASE, LAST_HOUR).unwrap();
    assert_eq!(ita_parts.len(), 1);
    assert!(
        ita_parts[0]
            .missing
            .iter()
            .any(|span| span.first_hour <= T2 && T2 <= span.last_hour && span.reason == RES_TRUNC_STATUS),
        "owner part_lives uses Table coverage even when ITA's only known part is witnessed at T2: {:?}",
        ita_parts[0].missing
    );

}

fn exp_gap(cells: &dyn Cells, slot: u32) -> Computed {
    match cells.number(ARMY_COL, slot) {
        Some(100) | None => Computed::Unrecorded,
        Some(army) => Computed::Value(army),
    }
}

static GAP_EXP: Series = Series {
    kind: "test-exp-gap",
    table: TableId::CountryExperience,
    key: Key::Country,
    parts: None,
    measures: &[Measure { name: "army", column: Column::Computed { id: 911, inputs: &["army_raw"], value: exp_gap }, decimals: 0, aggregates: 0, carry: Carry::None, rank: 0, rank_zero: dashboard_declare::RankZero::Keep }],
    drops: false,
    absent_none: false,
};

#[test]
fn a_rank_reply_keeps_every_asked_country_reads_every_measure_and_holds_only_where_proven() {
    let fixture = TruncatedFixture::new();
    let query = fixture.query();
    let rank = |params: serde_json::Value| -> ranked::SeriesRankReply {
        let mut request = serde_json::json!({ "branchId": "1", "family": { "kind": "country-factories", "measure": "built1" }, "measures": ["free1"] });
        request.as_object_mut().unwrap().extend(params.as_object().unwrap().clone());
        serde_json::from_value(answer(&query, ranked::PRODUCT, &request).unwrap().unwrap()).unwrap()
    };
    let tags = |reply: &ranked::SeriesRankReply| reply.rows.iter().map(|row| row.country.to_string()).collect::<Vec<_>>();
    let missing = |row: &ranked::SeriesRankRow| row.readings.iter().map(|reading| reading.missing.clone()).collect::<Vec<_>>();

    let refused = rank(serde_json::json!({ "top": 2, "hour": T1 }));
    assert_eq!(refused.rows, Vec::new(), "an unknown population ranks nobody");
    assert_eq!(refused.refused.as_deref(), Some(FAC_TRUNC_TABLE), "and says why, never an empty rank");
    assert_eq!((refused.from, refused.until), (T1, T1));

    let chosen = rank(serde_json::json!({ "countries": ["ITA", "GER"], "hour": T1 }));
    assert_eq!(tags(&chosen), ["GER", "ITA"], "the witnessed country first, the unread one kept after it");
    assert_eq!(chosen.measures.iter().map(|measure| measure.name.as_str()).collect::<Vec<_>>(), ["built1", "free1"]);
    assert_eq!(missing(&chosen.rows[0]), [None, None], "a witnessed row gives every measure from a truncated table");
    assert_eq!(missing(&chosen.rows[1]), [Some(FAC_TRUNC.to_owned()), Some(FAC_TRUNC.to_owned())], "the unread row keeps its own reason");
    assert_eq!(chosen.refused.as_deref(), Some(FAC_TRUNC), "the order of the chosen is not a proven rank");
    assert_eq!((chosen.from, chosen.until), (T1, T1));

    let top = rank(serde_json::json!({ "top": 1, "hour": T2 + 1 }));
    assert_eq!(tags(&top), ["ITA"]);
    assert_eq!(top.refused, None);
    assert!(top.rows[0].readings.iter().all(|reading| reading.run.is_some()), "the detail measure is read for a ranked country");
    assert_eq!((top.from, top.until), (T2 + 1, T3 - 1), "an hour whose population is unknown is not proven, though ITA's values run on");

    let group = serde_json::json!({ "branchId": "1", "family": { "kind": "country-factories", "measure": "built1" }, "hour": T2 + 1, "group": { "type": "faction", "id": "42" } });
    let members = |refused: Option<&'static str>| {
        move |_: &ranked::GroupSelection, _: i32| Ok(ranked::Members { tags: vec!["GER".into(), "ITA".into()], refused: refused.map(str::to_owned), from: T2 + 1, until: T2 + 2 })
    };
    let grouped = |resolve: ranked::Resolve<'_>| -> ranked::SeriesRankReply { serde_json::from_value(answer_grouped(&query, ranked::PRODUCT, &group, resolve).unwrap().unwrap()).unwrap() };
    let known = grouped(&members(None));
    assert_eq!(tags(&known), ["ITA", "GER"], "a group's members at the hour are ranked by the product");
    assert_eq!(known.rows.iter().map(|row| row.slot).collect::<Vec<_>>(), [Some(8), Some(7)], "each row names the recorded slot holding it at the hour");
    let once = rank(serde_json::json!({ "countries": ["GER", 7, "ITA"], "hour": T2 + 1 }));
    assert_eq!((tags(&once), once.rows.iter().map(|row| row.slot).collect::<Vec<_>>()), (vec!["ITA".to_owned(), "GER".to_owned()], vec![Some(8), Some(7)]), "a holder asked by tag and by slot is one row");
    assert_eq!((known.from, known.until), (T2 + 1, T2 + 2), "the reply holds only while the membership does");
    let unknown = grouped(&members(Some("Membership of this group is not recorded at this hour")));
    assert_eq!((unknown.rows.len(), unknown.refused.as_deref()), (0, Some("Membership of this group is not recorded at this hour")), "unknown membership is refused, never a union");
    assert!(answer(&query, ranked::PRODUCT, &group).is_err(), "a group is only answered where its membership is read");

    for (first, last) in [(UNAVAIL_FIRST, UNAVAIL_LAST), (NOTREC_FIRST, NOTREC_LAST)] {
        let params = serde_json::json!({ "branchId": "1", "family": { "kind": "country-experience", "measure": "army" }, "countries": ["GER", "ITA"], "hour": last });
        let reply: ranked::SeriesRankReply = serde_json::from_value(answer(&query, ranked::PRODUCT, &params).unwrap().unwrap()).unwrap();
        assert_eq!((reply.from, reply.until), (first, last));
        assert!(reply.rows.iter().all(|row| row.readings[0].missing.is_some()));
    }

    let gap = Line::new(&query, &GAP_EXP, &GAP_EXP.measures[0]).unwrap();
    assert_eq!(gap.rank_at(BASE, 2, false).unwrap(), Err("GER's army was not recorded at this hour".to_owned()), "an unrecorded contender refuses the rank");

    let zero = Line::new(&query, &ABSENT_ZERO_EXP, &ABSENT_ZERO_EXP.measures[0]).unwrap();
    assert_eq!(zero.rank_at(BASE, 3, false).unwrap(), Ok(vec![Country::from("ITA"), Country::from("GER"), Country::from("FRA")]));
    let states = zero.states(&[read("FRA", BASE, None)], None).unwrap();
    assert_eq!(states[0].run.as_ref().map(|run| run.value_raw.as_str()), Some("0"), "a country absent from a complete absent-is-none table reads zero, as its rank does");
}

#[test]
fn summaries_and_tiles_apply_witness_rule_and_preserve_complete_mid_bin_buckets() {
    let fixture = TruncatedFixture::new();
    let query = fixture.query();
    let exp_series = dashboard_declare::series("country-experience").unwrap();
    let exp = Line::new(&query, exp_series, exp_series.measure("army").unwrap()).unwrap();

    let table_missing = vec![
        MissingSpan { first_hour: T1, last_hour: T2, reason: EXP_TRUNC_STATUS.into() },
        MissingSpan { first_hour: T3, last_hour: T3, reason: EXP_TRUNC_STATUS.into() },
        MissingSpan { first_hour: UNAVAIL_FIRST, last_hour: UNAVAIL_LAST, reason: EXP_UNAVAIL_STATUS.into() },
        MissingSpan { first_hour: NOTREC_FIRST, last_hour: NOTREC_LAST, reason: EXP_NOTREC_STATUS.into() },
    ];
    let hourly = exp.summaries(&["GER", "ITA", "FRA"], BASE, LAST_HOUR, 30).unwrap();
    assert_eq!(hourly[0].1, table_missing, "Line::summaries defaults to Table coverage matching main");
    assert_eq!(hourly[1].1, table_missing, "Line::summaries defaults to Table coverage matching main");

    let binned = exp.summaries(&["GER", "ITA", "FRA"], BASE, LAST_HOUR, 5).unwrap();
    assert_eq!(binned[0].1, table_missing);
    assert_eq!(binned[1].1, table_missing);
    assert_eq!(binned[0].0.buckets.len(), 5);
    let fra_bin4 = binned[2].0.buckets[4].expect("FRA enters and leaves mid-bin 4 on complete hours");
    assert!(!fra_bin4.has_break, "mid-bin start and end on complete hours must match main with has_break == false");
    assert_eq!((fra_bin4.first_value, fra_bin4.last_value, fra_bin4.min_value, fra_bin4.max_value), (50.0, 50.0, 50.0, 50.0));

    let tile_reply = tile::tile(&exp, "1", &[Ask::whole("GER"), Ask::whole("ITA"), Ask::whole("FRA")], None, 6, 0, false).unwrap();
    let bytes = crate::fixture::unbase64(&tile_reply.bins);
    let bin0_slot = ((BASE - tile_reply.first_hour) / 6) as usize;
    let bin1_slot = bin0_slot + 1;
    let bin2_slot = bin1_slot + 1;
    let bin4_slot = bin1_slot + 3;
    assert_eq!(bytes[bin0_slot] & (PRESENT | HAS_BREAK), PRESENT, "GER complete bin 0 has no HAS_BREAK");
    assert_eq!(bytes[bin1_slot] & (PRESENT | HAS_BREAK), PRESENT | HAS_BREAK, "GER unwitnessed at T2 sets HAS_BREAK on bin 1");
    assert_eq!(bytes[bin2_slot] & (PRESENT | HAS_BREAK), PRESENT, "GER witnessed at T3 keeps bin 2 unbroken");
    assert_eq!(bytes[BINS + bin1_slot] & (PRESENT | HAS_BREAK), PRESENT | HAS_BREAK, "ITA unwitnessed at T1 sets HAS_BREAK on bin 1");
    assert_eq!(bytes[BINS + bin2_slot] & (PRESENT | HAS_BREAK), PRESENT, "ITA witnessed at T3 keeps bin 2 unbroken");
    assert_eq!(bytes[2 * BINS + bin4_slot] & (PRESENT | HAS_BREAK), PRESENT, "FRA complete mid-bin tile has no HAS_BREAK");
    assert!(
        tile_reply.lines[0].missing.iter().any(|span| span.first_hour == T2 && span.last_hour == T2 && span.reason == EXP_TRUNC)
            && !tile_reply.lines[0].missing.iter().any(|span| span.first_hour <= T1 && T1 <= span.last_hour)
            && !tile_reply.lines[0].missing.iter().any(|span| span.first_hour <= T3 && T3 <= span.last_hour),
        "GER tile missing omits witnessed T1 and T3 and reports unwitnessed T2: {:?}",
        tile_reply.lines[0].missing
    );
    assert!(
        tile_reply.lines[1].missing.iter().any(|span| span.first_hour == T1 && span.last_hour == T1 && span.reason == EXP_TRUNC)
            && !tile_reply.lines[1].missing.iter().any(|span| span.first_hour <= T2 && T2 <= span.last_hour)
            && !tile_reply.lines[1].missing.iter().any(|span| span.first_hour <= T3 && T3 <= span.last_hour),
        "ITA tile missing omits witnessed T2 and T3 and reports unwitnessed T1: {:?}",
        tile_reply.lines[1].missing
    );
}

#[test]
fn six_hour_ranked_request_and_complete_absent_none_summary_pin_main_behaviour() {
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
    let branch = BranchStart { id: 1, session: 1, loaded_hour: None, first_hour: 120, parent: Parentage::Unknown, reason: BranchReason::SessionStart };
    let campaign = CampaignRow { id: b"ranked-main-pin".to_vec() };
    let countries = [
        CountryRow { id: 9001, canonical_slot: 7, tag: b"GER".to_vec(), ..CountryRow::default() },
        CountryRow { id: 9002, canonical_slot: 8, tag: b"FRA".to_vec(), ..CountryRow::default() },
    ];
    let links = [CountryLinkRow { id: 9001, canonical_slot: 7 }, CountryLinkRow { id: 9002, canonical_slot: 8 }];
    let fac_ger = CountryFactoriesRow { country: 9001, military_total_raw: 2_000_000, ..CountryFactoriesRow::default() };
    let exp_ger = CountryExperienceRow { country: 9001, army_raw: 100, navy_raw: 10, ..CountryExperienceRow::default() };
    let replay = Replay::from_synthetic(recorder_derive::writers(), campaign.id.clone(), session, branch, 125, |frame, hour| {
        frame.table(TableId::Clock, Input::Snapshot { rows: &[ClockRow { hour, ..ClockRow::default() }], partial: None, read: true, sample: Sample::FromReadBit })?;
        frame.table(TableId::Campaign, Input::Snapshot { rows: std::slice::from_ref(&campaign), partial: None, read: true, sample: Sample::FromReadBit })?;
        frame.table(TableId::Countries, Input::Snapshot { rows: &countries, partial: None, read: true, sample: Sample::FromReadBit })?;
        frame.table(TableId::CountryLinks, Input::Snapshot { rows: &links, partial: None, read: true, sample: Sample::FromReadBit })?;
        frame.table(
            TableId::CountryFactories,
            Input::Snapshot { rows: std::slice::from_ref(&fac_ger), partial: (hour == 122).then_some(1), read: true, sample: Sample::FromReadBit },
        )?;
        frame.table(TableId::CountryExperience, Input::Snapshot { rows: std::slice::from_ref(&exp_ger), partial: None, read: true, sample: Sample::FromReadBit })?;
        Ok(())
    })
    .unwrap();
    let timeline = Arc::new(Timeline { id: 1, first: 120, last: 125, stretches: vec![Stretch { run: 0, first: 120, last: 125, join: Join::Start }], unheld: Vec::new() });
    let holders = Arc::new(crate::identity::holders(replay.recording(), &timeline).unwrap());
    let query = Query { replay: &replay, timeline, holders };

    let fac_series = dashboard_declare::series("country-factories").unwrap();
    let fac = Line::new(&query, fac_series, fac_series.measure("built1").unwrap()).unwrap();
    let expected_missing = vec![MissingSpan { first_hour: 122, last_hour: 122, reason: FAC_TRUNC_STATUS.into() }];
    let six_hour = fac.ranked(120, 125, 6, 1).unwrap();
    assert_eq!(six_hour.len(), 1, "six-hour ranked request keeps GER's complete-hour points as on main");
    assert_eq!(six_hour[0].tag, "GER");
    assert_eq!(six_hour[0].membership, vec![(120, 125)]);
    assert_eq!(six_hour[0].missing, expected_missing, "ranked summary keeps Table-wide truncation missing even when GER was witnessed");
    assert_eq!(six_hour[0].summary.buckets.len(), 6);
    assert!(six_hour[0].summary.buckets.iter().all(|b| b.is_some()));

    let one_bin = fac.ranked(120, 125, 1, 1).unwrap();
    assert_eq!(one_bin.len(), 1);
    assert_eq!(one_bin[0].tag, "GER");
    assert_eq!(one_bin[0].membership, vec![(120, 125)]);
    assert_eq!(one_bin[0].missing, expected_missing, "wider-bin ranked summary keeps Table-wide truncation missing");
    assert_eq!(one_bin[0].summary.buckets.len(), 1);
    assert!(one_bin[0].summary.buckets[0].is_some());

    let zero_line = Line::new(&query, &ABSENT_ZERO_EXP, &ABSENT_ZERO_EXP.measures[0]).unwrap();
    let one_hour = zero_line.ranked(120, 120, 1, 2).unwrap();
    let tags: Vec<&str> = one_hour.iter().map(|line| line.tag.as_str()).collect();
    assert_eq!(tags, ["GER", "FRA"], "complete-hour absent_none ranked summary keeps both GER and zero-row FRA");
}

#[test]
fn initially_truncated_ranked_leaves_first_hour_empty_while_keyed_state_keeps_witness() {
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
    let branch = BranchStart { id: 1, session: 1, loaded_hour: None, first_hour: 120, parent: Parentage::Unknown, reason: BranchReason::SessionStart };
    let campaign = CampaignRow { id: b"init-trunc-ranked".to_vec() };
    let countries = [
        CountryRow { id: 9001, canonical_slot: 7, tag: b"GER".to_vec(), ..CountryRow::default() },
        CountryRow { id: 9002, canonical_slot: 8, tag: b"ITA".to_vec(), ..CountryRow::default() },
    ];
    let links = [CountryLinkRow { id: 9001, canonical_slot: 7 }, CountryLinkRow { id: 9002, canonical_slot: 8 }];
    let exp_ger = CountryExperienceRow { country: 9001, army_raw: 100, navy_raw: 10, ..CountryExperienceRow::default() };
    let exp_ita = CountryExperienceRow { country: 9002, army_raw: 50, navy_raw: 5, ..CountryExperienceRow::default() };
    let replay = Replay::from_synthetic(recorder_derive::writers(), campaign.id.clone(), session, branch, 121, |frame, hour| {
        frame.table(TableId::Clock, Input::Snapshot { rows: &[ClockRow { hour, ..ClockRow::default() }], partial: None, read: true, sample: Sample::FromReadBit })?;
        frame.table(TableId::Campaign, Input::Snapshot { rows: std::slice::from_ref(&campaign), partial: None, read: true, sample: Sample::FromReadBit })?;
        frame.table(TableId::Countries, Input::Snapshot { rows: &countries, partial: None, read: true, sample: Sample::FromReadBit })?;
        frame.table(TableId::CountryLinks, Input::Snapshot { rows: &links, partial: None, read: true, sample: Sample::FromReadBit })?;
        if hour == 120 {
            frame.table(TableId::CountryExperience, Input::Snapshot { rows: std::slice::from_ref(&exp_ger), partial: Some(1), read: true, sample: Sample::FromReadBit })?;
        } else {
            frame.table(TableId::CountryExperience, Input::Snapshot { rows: &[exp_ger.clone(), exp_ita.clone()], partial: None, read: true, sample: Sample::FromReadBit })?;
        }
        Ok(())
    })
    .unwrap();
    let timeline = Arc::new(Timeline { id: 1, first: 120, last: 121, stretches: vec![Stretch { run: 0, first: 120, last: 121, join: Join::Start }], unheld: Vec::new() });
    let holders = Arc::new(crate::identity::holders(replay.recording(), &timeline).unwrap());
    let query = Query { replay: &replay, timeline, holders };
    let exp_series = dashboard_declare::series("country-experience").unwrap();
    let exp = Line::new(&query, exp_series, exp_series.measure("army").unwrap()).unwrap();

    let ranked = exp.ranked(120, 121, 2, 1).unwrap();
    assert_eq!(ranked.len(), 1);
    assert_eq!(ranked[0].tag, "GER");
    assert_eq!(ranked[0].membership, vec![(120, 121)]);
    assert_eq!(ranked[0].missing, vec![MissingSpan { first_hour: 120, last_hour: 120, reason: EXP_TRUNC_STATUS.into() }]);
    assert_eq!(ranked[0].summary.buckets.len(), 2);
    assert_eq!(ranked[0].summary.buckets[0], None, "initially truncated ordinal 0 stays empty in default ranked as on main");
    let b1 = ranked[0].summary.buckets[1].expect("complete hour 121 bucket");
    assert_eq!((b1.first_hour, b1.last_hour, b1.first_value, b1.last_value, b1.first_break, b1.has_break), (121, 121, 100.0, 100.0, true, false));

    let states = exp.states(&[read("GER", 120, None)], None).unwrap();
    assert_eq!(states[0].run.as_ref().map(|r| r.value_raw.as_str()), Some("100"), "keyed Row state preserves first-hour witness");
    assert_eq!(states[0].missing, None);
}

#[test]
fn complete_tile_and_unvalued_scalar_state_match_main_storage_read_counts() {
    use crate::fixture::{Data, FIRST, Fixture, UNIT_HOURS, power};

    let fixture = Fixture::new(Data::Power, FIRST..=FIRST + 6 * UNIT_HOURS);
    let query = fixture.query();
    let series = dashboard_declare::series("country-measure").unwrap();
    let line = Line::new(&query, series, series.measure("politicalPower").unwrap()).unwrap();
    let ask = [Ask::whole("GER")];
    let first_index = i64::from(FIRST) / (6 * tile::BINS as i64) + 1;

    let _ = crate::cost::take_calls();
    let first_tile = tile::tile(&line, "1", &ask, None, 6, first_index, false).unwrap();
    assert!(first_tile.lines[0].missing.is_empty());
    let first_calls = crate::cost::take_calls();
    assert_eq!(first_calls, 3, "non-initial complete tile performs exactly chart + statuses + lifetimes (same as main)");

    let second_tile = tile::tile(&line, "1", &ask, None, 6, first_index + 1, false).unwrap();
    assert!(second_tile.lines[0].missing.is_empty());
    let second_calls = crate::cost::take_calls();
    assert_eq!(second_calls, 2, "subsequent complete tile reuses cached lifetimes and performs exactly chart + statuses");

    let filtered_line = Line::new(&query, series, series.measure("politicalPower").unwrap()).unwrap();
    let tile1_first = (first_index * 6 * tile::BINS as i64) as i32;
    let within1 = [(Country::from("GER"), vec![Span { first_hour: tile1_first, last_hour: tile1_first + 5 }])];
    let _ = crate::cost::take_calls();
    let first_filtered = tile::tile(&filtered_line, "1", &ask, Some(&within1), 6, first_index, false).unwrap();
    assert!(first_filtered.lines[0].missing.is_empty());
    assert_eq!(
        crate::cost::take_calls(),
        3,
        "within-filtered complete tile reads statuses once for the tile span and shares the cache with missing_scoped"
    );

    let tile2_first = ((first_index + 1) * 6 * tile::BINS as i64) as i32;
    let within2 = [(Country::from("GER"), vec![Span { first_hour: tile2_first, last_hour: tile2_first + 5 }])];
    let second_filtered = tile::tile(&filtered_line, "1", &ask, Some(&within2), 6, first_index + 1, false).unwrap();
    assert!(second_filtered.lines[0].missing.is_empty());
    assert_eq!(crate::cost::take_calls(), 2, "subsequent within-filtered complete tile performs exactly chart + statuses");

    let unvalued_hour = (FIRST..FIRST + 200).find(|&hour| power(9001, hour).is_none()).unwrap();
    let _ = crate::cost::take_calls();
    let states = line.states(&[read("GER", unvalued_hour, None)], None).unwrap();
    let state_calls = crate::cost::take_calls();
    assert_eq!(states[0].run, None);
    assert_eq!(states[0].missing.as_deref(), Some("GER's politicalPower was not recorded at this hour"));
    assert_eq!(state_calls, 1, "unvalued scalar on a complete hour uses key_runs_covered alone with zero at_each_with_status re-read");

    let trunc_fixture = TruncatedFixture::new();
    let trunc_query = trunc_fixture.query();
    let exp_series = dashboard_declare::series("country-experience").unwrap();
    let exp = Line::new(&trunc_query, exp_series, exp_series.measure("army").unwrap()).unwrap();
    let _ = crate::cost::take_calls();
    let absent = exp.states(&[read("FRA", BASE, None)], None).unwrap();
    let absent_calls = crate::cost::take_calls();
    assert_eq!(absent[0].run, None);
    assert_eq!(absent[0].missing.as_deref(), Some("FRA's army was not recorded at this hour"));
    assert_eq!(absent_calls, 1, "absent scalar holder on a complete hour uses key_runs_covered alone with zero at_each_with_status re-read");
}

#[test]
fn aliased_relation_opinion_witnessed_at_truncated_hour_agrees_between_state_and_tile() {
    use replay_data::rows::{RelationOpinionRow, SliceReadRow};

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
    let branch = BranchStart { id: 1, session: 1, loaded_hour: None, first_hour: 120, parent: Parentage::Unknown, reason: BranchReason::SessionStart };
    let campaign = CampaignRow { id: b"alias-trunc".to_vec() };
    let countries = [
        CountryRow { id: 1, canonical_slot: 10, tag: b"GER".to_vec(), ..CountryRow::default() },
        CountryRow { id: 2, canonical_slot: 20, tag: b"ITA".to_vec(), ..CountryRow::default() },
    ];
    let links = [
        CountryLinkRow { id: 1, canonical_slot: 10 },
        CountryLinkRow { id: 7, canonical_slot: 10 },
        CountryLinkRow { id: 2, canonical_slot: 20 },
    ];
    let opinion = RelationOpinionRow { owner_country: 7, target_country: 2, opinion: 42 };
    let replay = Replay::from_synthetic(recorder_derive::writers(), campaign.id.clone(), session, branch, 131, |frame, hour| {
        frame.table(TableId::Clock, Input::Snapshot { rows: &[ClockRow { hour, ..ClockRow::default() }], partial: None, read: true, sample: Sample::FromReadBit })?;
        frame.table(TableId::Campaign, Input::Snapshot { rows: std::slice::from_ref(&campaign), partial: None, read: true, sample: Sample::FromReadBit })?;
        frame.table(TableId::Countries, Input::Snapshot { rows: &countries, partial: None, read: true, sample: Sample::FromReadBit })?;
        frame.table(TableId::CountryLinks, Input::Snapshot { rows: &links, partial: None, read: true, sample: Sample::FromReadBit })?;
        frame.table(
            TableId::SliceReads,
            Input::Snapshot {
                rows: &[SliceReadRow { table: TableId::RelationOpinions.index() as u32, owner: 7, sample_hour: hour }],
                partial: None,
                read: true,
                sample: Sample::FromReadBit,
            },
        )?;
        let rows: &[RelationOpinionRow] = if hour == 125 { &[] } else { std::slice::from_ref(&opinion) };
        frame.table(
            TableId::RelationOpinions,
            Input::Snapshot { rows, partial: (hour == 125 || hour == 128).then_some(1), read: true, sample: Sample::FromReadBit },
        )?;
        Ok(())
    })
    .unwrap();
    let timeline = Arc::new(Timeline { id: 1, first: 120, last: 131, stretches: vec![Stretch { run: 0, first: 120, last: 131, join: Join::Start }], unheld: Vec::new() });
    let holders = Arc::new(crate::identity::holders(replay.recording(), &timeline).unwrap());
    let query = Query { replay: &replay, timeline, holders };

    let rel_series = dashboard_declare::series("relation-opinions").unwrap();
    let rel = Line::new(&query, rel_series, rel_series.measure("opinion").unwrap()).unwrap();
    let states = rel.states(&[read("GER", 128, Some("2")), read("GER", 125, Some("2"))], None).unwrap();
    assert_eq!(states[0].run.as_ref().map(|r| r.value_raw.as_str()), Some("42"));
    assert_eq!(states[0].missing, None);
    assert_eq!(states[1].run, None);
    assert_eq!(states[1].missing.as_deref(), Some(REL_TRUNC));

    let ask = Ask { country: Country::from("GER"), part: 2 };
    let parts = rel.held(&ask, 120, 131);
    let table_missing = rel.missing("GER", 120, 131, &parts).unwrap();
    assert_eq!(
        table_missing,
        vec![
            MissingSpan { first_hour: 125, last_hour: 125, reason: REL_TRUNC_STATUS.into() },
            MissingSpan { first_hour: 128, last_hour: 128, reason: REL_TRUNC_STATUS.into() },
        ],
        "default Table-scoped Line::missing reports both truncated hours"
    );
    let row_missing = rel.missing_for_tile("GER", 120, 131, &parts).unwrap();
    assert_eq!(
        row_missing,
        vec![MissingSpan { first_hour: 125, last_hour: 125, reason: REL_TRUNC.into() }],
        "Row-scoped missing omits aliased witness at 128 and keeps unwitnessed trailing hour 125"
    );

    let tile_reply = tile::tile(&rel, "1", std::slice::from_ref(&ask), None, 6, 0, false).unwrap();
    let in_range: Vec<MissingSpan> = tile_reply
        .lines[0]
        .missing
        .iter()
        .filter(|span| span.first_hour <= 131 && 120 <= span.last_hour)
        .cloned()
        .collect();
    assert_eq!(in_range, row_missing);
    let bytes = crate::fixture::unbase64(&tile_reply.bins);
    let slot0 = ((120 - tile_reply.first_hour) / 6) as usize;
    assert_eq!(
        bytes[slot0] & (PRESENT | HAS_BREAK),
        PRESENT | HAS_BREAK,
        "binned_from_runs_for_tile marks HAS_BREAK for trailing unwitnessed truncated hour 125"
    );
    assert_eq!(
        bytes[slot0 + 1] & (PRESENT | HAS_BREAK),
        PRESENT,
        "aliased witness at truncated hour 128 keeps bin 126..=131 unbroken"
    );

    let within = [(Country::from("GER"), vec![Span { first_hour: 120, last_hour: 121 }])];
    let filtered = tile::tile(&rel, "1", &[ask], Some(&within), 6, 0, false).unwrap();
    let filtered_bytes = crate::fixture::unbase64(&filtered.bins);
    assert_eq!(
        filtered_bytes[slot0] & (PRESENT | HAS_BREAK),
        PRESENT,
        "within-filtered tile does not mark HAS_BREAK for unwitnessed truncated hour 125 outside within"
    );
}

#[test]
fn a_step_over_a_measure_the_summaries_do_not_hold_reads_the_hours_themselves() {
    let fixture = TruncatedFixture::new();
    let query = fixture.query();
    let line = Line::new(&query, &RATIO_EXP, &RATIO_EXP.measures[0]).unwrap();
    let ask = Ask { country: Country::from("ITA"), part: 0 };
    let step = |origin: i32, key| line.change_lines(std::slice::from_ref(&ask), &[&RATIO_EXP.measures[0]], Some(origin), key, (BASE, LAST_HOUR), &Within::new());
    let forward = step(BASE, NavigationKey::ArrowRight).expect("a summary piece without the measure's column is read hour by hour, never refused");
    let back = step(LAST_HOUR, NavigationKey::ArrowLeft).expect("both directions");
    assert_eq!((forward, back), (Some(T1), Some(NOTREC_LAST + 1)), "ITA's ratio is first unread at T1 and last read again after the unrecorded hours");
}

#[test]
fn a_row_read_at_a_truncated_hour_is_served_only_where_that_hour_witnessed_it() {
    use replay_data::rows::tables::CountryExperience;
    let fixture = TruncatedFixture::new();
    let recording = fixture.replay.recording();
    let format = recording.format();
    let army = dashboard_declare::field(TableId::CountryExperience, "army_raw").unwrap();
    let mut walked = 0;
    crate::read::walk(recording, 0, &[(TableId::CountryExperience, None)], T1, T1, |_, readers| {
        let reader = &readers[0];
        assert_eq!(crate::read::row::<CountryExperience>(reader, 9001, format).map(|row| row.map(|row| row.army_raw)), Ok(Some(100)), "GER is witnessed at T1");
        assert_eq!(crate::read::row::<CountryExperience>(reader, 9002, format).map(|row| row.is_some()), Err(EXP_TRUNC.to_owned()), "ITA is held from an earlier hour, not read at T1");
        assert_eq!(crate::read::number(reader, 9001, army), Some(100));
        assert_eq!(crate::read::number(reader, 9002, army), None, "an unwitnessed row's held cell is never served as the hour's");
        walked += 1;
        Ok(())
    })
    .unwrap();
    assert_eq!(walked, 1);
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
    assert_eq!(top_t1.missing.as_deref(), Some(EXP_TRUNC_TABLE));

    let top_t3 = board::at(&columns, board::Rows::Top { by: 0, n: 2, least: false }, T3).unwrap();
    assert!(top_t3.rows.is_empty());
    assert_eq!(top_t3.missing.as_deref(), Some(EXP_TRUNC_TABLE));

    let power_series = dashboard_declare::series("country-measure").unwrap();
    let two_table = [
        board::Column { line: Line::new(&query, power_series, power_series.measure("politicalPower").unwrap()).unwrap(), partial: None },
        board::Column { line: Line::new(&query, exp_series, exp_series.measure("army").unwrap()).unwrap(), partial: None },
    ];
    let top_two = board::at(&two_table, board::Rows::Top { by: 0, n: 1, least: false }, T1).unwrap();
    assert_eq!(top_two.missing, None);
    assert_eq!(top_two.rows.len(), 1);
    assert_eq!(top_two.rows[0].tag, "GER");
    assert_eq!(top_two.rows[0].cells[0].value_raw.as_deref(), Some("300000"));
    assert_eq!(top_two.rows[0].cells[1].value_raw, None, "Rows::Top keeps Table refusal on truncated secondary columns");
    assert_eq!(top_two.rows[0].cells[1].missing.as_deref(), Some(EXP_TRUNC_TABLE));

    let keyed_two = board::at(&two_table, board::Rows::Countries(&[Country::from("GER")]), T1).unwrap();
    assert_eq!(keyed_two.rows[0].cells[1].value_raw.as_deref(), Some("100"), "Rows::Countries shows witnessed cell on truncated secondary column");
    assert_eq!(keyed_two.rows[0].cells[1].missing, None);
}

#[test]
fn cold_truncated_tile_batches_witness_walk_once_across_many_countries() {
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
    let branch = BranchStart { id: 1, session: 1, loaded_hour: None, first_hour: 120, parent: Parentage::Unknown, reason: BranchReason::SessionStart };
    let campaign = CampaignRow { id: b"batch-witness".to_vec() };
    let countries: Vec<CountryRow> = (0u32..20)
        .map(|i| CountryRow { id: 9001 + i, canonical_slot: 10 + i, tag: format!("C{i:02}").into_bytes(), ..CountryRow::default() })
        .collect();
    let links: Vec<CountryLinkRow> = (0u32..20).map(|i| CountryLinkRow { id: 9001 + i, canonical_slot: 10 + i }).collect();
    let all_exp: Vec<CountryExperienceRow> = (0u32..20)
        .map(|i| CountryExperienceRow { country: 9001 + i, army_raw: 100 + i64::from(i), ..CountryExperienceRow::default() })
        .collect();
    let even_exp: Vec<CountryExperienceRow> = all_exp.iter().enumerate().filter(|(i, _)| i % 2 == 0).map(|(_, r)| r.clone()).collect();
    let odd_exp: Vec<CountryExperienceRow> = all_exp.iter().enumerate().filter(|(i, _)| i % 2 == 1).map(|(_, r)| r.clone()).collect();

    let replay = Replay::from_synthetic(recorder_derive::writers(), campaign.id.clone(), session, branch, 131, |frame, hour| {
        frame.table(TableId::Clock, Input::Snapshot { rows: &[ClockRow { hour, ..ClockRow::default() }], partial: None, read: true, sample: Sample::FromReadBit })?;
        frame.table(TableId::Campaign, Input::Snapshot { rows: std::slice::from_ref(&campaign), partial: None, read: true, sample: Sample::FromReadBit })?;
        frame.table(TableId::Countries, Input::Snapshot { rows: &countries, partial: None, read: true, sample: Sample::FromReadBit })?;
        frame.table(TableId::CountryLinks, Input::Snapshot { rows: &links, partial: None, read: true, sample: Sample::FromReadBit })?;
        match hour {
            128 => frame.table(TableId::CountryExperience, Input::Snapshot { rows: &even_exp, partial: Some(1), read: true, sample: Sample::FromReadBit })?,
            129 => frame.table(TableId::CountryExperience, Input::Snapshot { rows: &odd_exp, partial: Some(1), read: true, sample: Sample::FromReadBit })?,
            _ => frame.table(TableId::CountryExperience, Input::Snapshot { rows: &all_exp, partial: None, read: true, sample: Sample::FromReadBit })?,
        }
        Ok(())
    })
    .unwrap();
    let timeline = Arc::new(Timeline { id: 1, first: 120, last: 131, stretches: vec![Stretch { run: 0, first: 120, last: 131, join: Join::Start }], unheld: Vec::new() });
    let holders = Arc::new(crate::identity::holders(replay.recording(), &timeline).unwrap());
    let query = Query { replay: &replay, timeline, holders };
    let exp_series = dashboard_declare::series("country-experience").unwrap();
    let exp = Line::new(&query, exp_series, exp_series.measure("army").unwrap()).unwrap();
    let asks: Vec<Ask> = (0u32..20).map(|i| Ask::whole(format!("C{i:02}"))).collect();

    let _ = crate::cost::take_calls();
    let reply = tile::tile(&exp, "1", &asks, None, 6, 0, false).unwrap();
    let calls = crate::cost::take_calls();
    assert_eq!(calls, 4, "cold truncated tile batches witness walk once across all 20 countries (chart + statuses + batched witness + lifetimes)");
    for (i, line) in reply.lines.iter().enumerate() {
        let in_range: Vec<MissingSpan> = line.missing.iter().filter(|span| span.first_hour <= 131 && 120 <= span.last_hour).cloned().collect();
        let missing_hour = if i % 2 == 0 { 129 } else { 128 };
        assert_eq!(
            in_range,
            vec![MissingSpan { first_hour: missing_hour, last_hour: missing_hour, reason: EXP_TRUNC.into() }],
            "country C{i:02} missing spans"
        );
    }
}

#[test]
fn a_ranked_top_drops_zeros_for_counts_and_keeps_zeros_for_standings() {
    use replay_data::rows::{AutonomyRow, NukeRow};

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
    let branch = BranchStart { id: 1, session: 1, loaded_hour: None, first_hour: 120, parent: Parentage::Unknown, reason: BranchReason::SessionStart };
    let campaign = CampaignRow { id: b"rank-zero-rule".to_vec() };
    let countries = [
        CountryRow { id: 9001, canonical_slot: 7, tag: b"ABK".to_vec(), political_power_raw: Some(0), ..CountryRow::default() },
        CountryRow { id: 9002, canonical_slot: 8, tag: b"GER".to_vec(), political_power_raw: Some(50_000), ..CountryRow::default() },
        CountryRow { id: 9003, canonical_slot: 9, tag: b"USA".to_vec(), political_power_raw: Some(-10_000), ..CountryRow::default() },
    ];
    let links = [
        CountryLinkRow { id: 9001, canonical_slot: 7 },
        CountryLinkRow { id: 9002, canonical_slot: 8 },
        CountryLinkRow { id: 9003, canonical_slot: 9 },
    ];
    let autonomy = [
        AutonomyRow { subject: 9001, score: 0, lower_score: 0, upper_score: 1_000, ..AutonomyRow::default() },
        AutonomyRow { subject: 9002, score: 500, lower_score: 0, upper_score: 1_000, ..AutonomyRow::default() },
        AutonomyRow { subject: 9003, score: 0, lower_score: 0, upper_score: 1_000, ..AutonomyRow::default() },
    ];
    let fac_all = [
        CountryFactoriesRow { country: 9001, military_total_raw: 0, ..CountryFactoriesRow::default() },
        CountryFactoriesRow { country: 9002, military_total_raw: 500_000, ..CountryFactoriesRow::default() },
        CountryFactoriesRow { country: 9003, military_total_raw: 0, ..CountryFactoriesRow::default() },
    ];

    let replay = Replay::from_synthetic(recorder_derive::writers(), campaign.id.clone(), session, branch, 125, |frame, hour| {
        frame.table(TableId::Clock, Input::Snapshot { rows: &[ClockRow { hour, ..ClockRow::default() }], partial: None, read: true, sample: Sample::FromReadBit })?;
        frame.table(TableId::Campaign, Input::Snapshot { rows: std::slice::from_ref(&campaign), partial: None, read: true, sample: Sample::FromReadBit })?;
        frame.table(TableId::Countries, Input::Snapshot { rows: &countries, partial: None, read: true, sample: Sample::FromReadBit })?;
        frame.table(TableId::CountryLinks, Input::Snapshot { rows: &links, partial: None, read: true, sample: Sample::FromReadBit })?;
        frame.table(TableId::Autonomy, Input::Snapshot { rows: &autonomy, partial: None, read: true, sample: Sample::FromReadBit })?;
        let nukes = if hour < 123 {
            vec![
                NukeRow { country: 9001, nuke_type: 0, amount_raw: 0, ready: 0 },
                NukeRow { country: 9002, nuke_type: 0, amount_raw: 0, ready: 0 },
                NukeRow { country: 9003, nuke_type: 0, amount_raw: 0, ready: 0 },
            ]
        } else {
            vec![
                NukeRow { country: 9001, nuke_type: 0, amount_raw: 0, ready: 0 },
                NukeRow { country: 9002, nuke_type: 0, amount_raw: 0, ready: 0 },
                NukeRow { country: 9003, nuke_type: 0, amount_raw: if hour < 125 { 200_000_000 } else { -100_000_000 }, ready: 0 },
            ]
        };
        frame.table(TableId::Nukes, Input::Snapshot { rows: &nukes, partial: None, read: true, sample: Sample::FromReadBit })?;
        if hour == 124 {
            frame.table(TableId::CountryFactories, Input::Snapshot { rows: &fac_all[..2], partial: Some(1), read: true, sample: Sample::FromReadBit })?;
        } else {
            frame.table(TableId::CountryFactories, Input::Snapshot { rows: &fac_all, partial: None, read: true, sample: Sample::FromReadBit })?;
        }
        Ok(())
    })
    .unwrap();
    let timeline = Arc::new(Timeline { id: 1, first: 120, last: 125, stretches: vec![Stretch { run: 0, first: 120, last: 125, join: Join::Start }], unheld: Vec::new() });
    let holders = Arc::new(crate::identity::holders(replay.recording(), &timeline).unwrap());
    let query = Query { replay: &replay, timeline, holders };

    let nukes_series = dashboard_declare::series("country-nuke-totals").unwrap();
    let nukes = Line::new(&query, nukes_series, nukes_series.measure("total").unwrap()).unwrap();
    assert_eq!(nukes.rank_at(121, 5, false).unwrap(), Ok(Vec::new()), "all zeros drops every country from a count top");
    assert_eq!(nukes.rank_at(124, 5, false).unwrap(), Ok(vec![Country::from("USA")]), "count top shows only positive rows and fewer than N");

    let rank_top: ranked::SeriesRankReply = serde_json::from_value(
        answer(&query, ranked::PRODUCT, &serde_json::json!({ "branchId": "1", "family": { "kind": "country-nuke-totals", "measure": "total" }, "top": 5, "hour": 124 }))
            .unwrap()
            .unwrap(),
    )
    .unwrap();
    assert_eq!(rank_top.rows.iter().map(|r| r.country.to_string()).collect::<Vec<_>>(), vec!["USA"]);
    assert_eq!(rank_top.refused, None);
    let tiles_top = tile::reply(&nukes, "1", &serde_json::json!({ "binHours": 1, "index": 124 / BINS as i64, "top": 5, "hour": 124 })).unwrap();
    assert_eq!(tiles_top.lines.iter().map(|line| line.country.to_string()).collect::<Vec<_>>(), vec!["USA"], "lines and race agree on positive-only top");
    let tiles_empty = tile::reply(&nukes, "1", &serde_json::json!({ "binHours": 1, "index": 121 / BINS as i64, "top": 5, "hour": 121 })).unwrap();
    assert!(tiles_empty.lines.is_empty() && tiles_empty.ranking.unwrap().refused.is_none(), "all-zero count top yields an empty unproven-free tile reply");
    assert_eq!(nukes.rank_at(125, 1, false).unwrap(), Ok(vec![Country::from("USA")]), "only zero is dropped: a negative count is ranked");
    let summary_top: Vec<String> = nukes.ranked(120, 125, 1, 1).unwrap().into_iter().map(|line| line.tag).collect();
    assert_eq!(summary_top, vec!["USA".to_owned()], "a bin whose stored top holds only dropped zeros is ranked exactly, never drawn short");

    let chosen: ranked::SeriesRankReply = serde_json::from_value(
        answer(&query, ranked::PRODUCT, &serde_json::json!({ "branchId": "1", "family": { "kind": "country-nuke-totals", "measure": "total" }, "countries": ["USA", "ABK"], "hour": 124 }))
            .unwrap()
            .unwrap(),
    )
    .unwrap();
    assert_eq!(chosen.rows.iter().map(|r| r.country.to_string()).collect::<Vec<_>>(), vec!["USA", "ABK"], "explicit countries still keep their recorded zero");

    let pp_series = dashboard_declare::series("country-measure").unwrap();
    let pp = Line::new(&query, pp_series, pp_series.measure("politicalPower").unwrap()).unwrap();
    assert_eq!(pp.rank_at(124, 5, false).unwrap(), Ok(vec![Country::from("GER"), Country::from("ABK"), Country::from("USA")]), "signed balance keeps zeros");

    let auto_series = dashboard_declare::series("autonomy").unwrap();
    let auto = Line::new(&query, auto_series, auto_series.measure("progress").unwrap()).unwrap();
    assert_eq!(auto.rank_at(124, 5, false).unwrap(), Ok(vec![Country::from("GER"), Country::from("ABK"), Country::from("USA")]), "percentage standing keeps zeros");

    let fac_series = dashboard_declare::series("country-factories").unwrap();
    let fac = Line::new(&query, fac_series, fac_series.measure("built1").unwrap()).unwrap();
    assert_eq!(fac.rank_at(121, 5, false).unwrap(), Ok(vec![Country::from("GER")]));
    assert!(fac.rank_at(124, 5, false).unwrap().is_err(), "an unrecorded contender still refuses a positive-only count top");
}
