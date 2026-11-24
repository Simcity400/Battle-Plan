use super::*;
use dashboard_follow::live::{CellValue, GroupState};
use replay_frame::frame::{Metrics, TableView, not_selected};

fn row(country: u32, region: u32) -> AirStatisticsRow {
    let mut channels = [0i64; 24];
    for (at, value) in channels.iter_mut().enumerate() {
        *value = (at as i64 + 1) * 100 * i64::from(region);
    }
    AirStatisticsRow { country, region, year_slot: 3, year_window_wrapped: 0, channels }
}

fn names() -> Names {
    Names {
        catalogue: Err("no content in this test".to_owned()),
        regions: Ok([(7, Ok("Western Europe".to_owned())), (8, Err("Region 8 name unsupported".to_owned()))].into()),
        channels: Vec::new(),
        warnings: Vec::new(),
    }
}

#[test]
fn a_pair_is_its_two_recorded_words_and_only_the_holders_regions_are_shown() {
    let rows = [row(9, 7), row(9, 8), row(9, 11), row(10, 7)];
    let cells = Cells::default();
    let reply = reply(Ok(&rows), &[9], Some(40), 3, &names(), &cells, &Shown::default()).unwrap();
    assert_eq!(reply.read_hour, Some(40));
    assert_eq!(reply.missing, None);
    let shown: Vec<_> = reply.regions.iter().map(|region| region.region.as_str()).collect();
    assert_eq!(shown, ["11", "8", "7"], "most active first: ours plus the enemy's, exact");
    let live = cells.finish();
    let mut frame = replay_frame::frame::DecodedFrame { branch: 1, hour: 41, tables: std::array::from_fn(|_| not_selected()), metrics: Metrics::default() };
    *frame.tables[TableId::AirStatistics.index()] = TableView::Complete(replay_data::rows::TableRows::AirStatistics(rows.to_vec()));
    let json = live.json(&live.values(&frame).into_iter().map(GroupState::Values).collect::<Vec<_>>());
    assert_eq!(json["rows"], serde_json::json!(["11", "8", "7"]));
    assert_eq!(json["kinds"], serde_json::json!([{ "id": "ours", "op": "value", "scale": 1.0 }, { "id": "enemy", "op": "value", "scale": 1.0 }]));
    let group = &json["groups"][0];
    assert_eq!(group["keys"], serde_json::json!([(9u128 << 32 | 7).to_string(), (9u128 << 32 | 8).to_string(), (9u128 << 32 | 11).to_string()]), "the holder's rows alone");
    assert_eq!(group["values"], serde_json::json!([2800, 11200, 3200, 12800, 4400, 17600]), "ours and the enemy's word of pair 3, exact");
    assert_eq!(live.values(&frame)[0][0], CellValue::Number(2800));
    assert_eq!((reply.regions[2].name.as_deref(), reply.regions[2].missing.as_deref()), (Some("Western Europe"), None));
    assert_eq!(reply.regions[1].missing.as_deref(), Some("Region 8 name unsupported"));
    assert_eq!(reply.regions[0].missing.as_deref(), Some("Region 11 absent from the recorded stack"));
    let top = reply_of(&rows, Shown { regions: None, top: Some(2) });
    assert_eq!(top, ["11", "8"], "the product cuts the most active regions");
    let chosen = reply_of(&rows, Shown { regions: Some([7, 11, 40].into()), top: None });
    assert_eq!(chosen, ["11", "7"], "chosen regions in rank order; one without statistics is not invented");
}

fn reply_of(rows: &[AirStatisticsRow], shown: Shown) -> Vec<String> {
    reply(Ok(rows), &[9], Some(40), 3, &names(), &Cells::default(), &shown).unwrap().regions.into_iter().map(|region| region.region).collect()
}

#[test]
fn statistics_not_recorded_at_the_hour_are_missing_not_empty() {
    let cells = Cells::default();
    let reply = reply(Err(io::Error::other("air statistics was not read")), &[9], None, 0, &names(), &cells, &Shown::default()).unwrap();
    assert!(reply.regions.is_empty());
    assert!(cells.finish().is_empty(), "no region, no cell");
    assert!(reply.missing.unwrap().contains("air statistics was not read"));
    assert_eq!(divisor(0), 1_000, "buildings bombed show in thousandths");
    assert_eq!(divisor(PAIRS), 1_000);
    assert_eq!(divisor(3), 100_000);
}

#[test]
fn a_holders_other_link_ids_add_their_regions_and_a_region_two_ids_hold_is_the_lowest_ids_row() {
    let rows = [row(9, 7), row(10, 8), row(12, 7), row(12, 11), row(13, 7)];
    let cells = Cells::default();
    let reply = reply(Ok(&rows), &[9, 12], Some(40), 3, &names(), &cells, &Shown::default()).unwrap();
    let shown: Vec<_> = reply.regions.iter().map(|region| region.region.as_str()).collect();
    assert_eq!(shown, ["11", "7"], "id 10 and 13 are not the holder's");
    assert_eq!(cells.finish().groups[0].keys, [9u128 << 32 | 7, 12u128 << 32 | 11], "region 7 is id 9's row, never 12's");
    assert!(reply.warnings.iter().any(|warning| warning.starts_with("1 air statistics rows")), "{:?}", reply.warnings);
    let ids: BTreeSet<u32> = [9, 12].into();
    let keys = rows.iter().map(|row| u128::from(row.country) << 32 | u128::from(row.region));
    assert_eq!(regions_of(keys, &ids), [7, 11].into(), "the regions a holder's ids held over the timeline");
}

