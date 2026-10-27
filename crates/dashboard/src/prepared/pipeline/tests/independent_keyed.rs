//! Keyed families prepared on their own (`dashboard_keyed`): registered outside
//! the hourly walk, they answer their product queries cold with an empty
//! window and every recorded hour pending, and once prepared with the whole
//! branch's runs, as the air missions family shows.
use super::*;
use crate::test_fixtures::world_frame;
use replay_data::rows::{
    AirRegionalMissionRow, AirWingRow, CountryOccupationReserveManpowerRow, OccupationSettingsRow,
};

fn missions(hour: i32, planes: i32) -> DecodedFrame {
    let mut frame = world_frame(hour, false);
    let wing = AirWingRow {
        id: 40,
        owner: 9001,
        planes,
        mission_mask: 0b101,
        mission_region: Some(3),
        ..Default::default()
    };
    *frame.tables[TableId::AirWings.index()] = TableView::Complete(TableRows::AirWings(vec![wing]));
    *frame.tables[TableId::AirRegionalMissions.index()] =
        TableView::Complete(TableRows::AirRegionalMissions(vec![
            AirRegionalMissionRow {
                wing: 40,
                ordinal: 0,
                active_order: None,
            },
        ]));
    frame.metrics.table_ns[TableId::AirRegionalMissions.index()] = 1;
    frame
}

fn query(source: &Source, product: &str, params: serde_json::Value) -> serde_json::Value {
    let request = crate::transport::ProductRequest {
        source_id: source.key().source_id.clone(),
        revision: source.key().revision.clone(),
        product: product.into(),
        params,
    };
    let reply =
        super::super::super::query::query(source, &request, None, &AtomicBool::new(false)).unwrap();
    let mut reply: serde_json::Value = serde_json::from_str(&reply).unwrap();
    reply["data"].take()
}

#[test]
fn keyed_families_are_prepared_on_their_own_and_answer_cold_and_prepared() {
    for name in [
        "air-missions",
        "air-statistics",
        "regional-weather",
        "minefields",
        "naval-supremacy",
        "doctrine-mastery",
        "occupation-reserve",
        "intel-network-coverage",
        "naval-repairs",
        "naval-refits",
    ] {
        assert!(independent::registered(name), "{name} is registered");
        assert!(
            !independent::hourly(name),
            "{name} stays out of the hourly walk"
        );
    }
    let frames: Vec<_> = (10..=20)
        .map(|hour| missions(hour, if hour < 15 { 24 } else { 12 }))
        .collect();
    let replay = Replay::from_records(CAMPAIGN.into(), &[records(1, frames)]).unwrap();
    let mut fixture = Fixture::new(&replay);
    let source = fixture.source("keyed-air-missions");
    let params = serde_json::json!({"branchId":"1","tag":"GER","view":"history"});
    let cold = query(&source, "air-missions", params.clone());
    assert_eq!(cold["window"]["series"], serde_json::json!([]));
    assert_eq!(cold["coverage"]["complete"], false);
    assert_eq!(
        cold["pending"],
        serde_json::json!([{"firstHour":10,"lastHour":20}])
    );
    run(&source, None, &AtomicBool::new(false), |_| {}).unwrap();
    let prepared = query(&source, "air-missions", params);
    assert_eq!(prepared["coverage"]["complete"], true, "{prepared}");
    assert_eq!(prepared["pending"], serde_json::json!([]));
    let series = prepared["window"]["series"].as_array().unwrap();
    let runs = |name: &str| {
        series
            .iter()
            .find(|item| item["name"] == name)
            .map(|item| item["runs"].clone())
            .unwrap_or_default()
    };
    // Two missions on one wing: its planes in each, once in wings.
    assert_eq!(
        runs("GER@7|3|m0"),
        serde_json::json!([[10, 14, "24", true], [15, 20, "12", false]])
    );
    assert_eq!(runs("GER@7|3|m2"), runs("GER@7|3|m0"));
    assert_eq!(
        runs("GER@7|3|wings"),
        serde_json::json!([[10, 20, "1", true]])
    );
    assert_eq!(
        prepared["window"]["known"],
        serde_json::json!([{"firstHour":10,"lastHour":20}])
    );
    super::super::super::files(&source, None)
        .unwrap()
        .0
        .clear()
        .unwrap();
}

/// GER holds ITA's 500 in reserve; ITA becomes RSI at hour 130, between the
/// occupation walk's daily reads at 123 and 147. As recorded, that walk's read
/// is timed on OccupationSettings alone.
fn reserve(hour: i32) -> DecodedFrame {
    let mut frame = world_frame(hour, false);
    if hour >= 130
        && let TableView::Complete(TableRows::Countries(rows)) =
            &mut *frame.tables[TableId::Countries.index()]
    {
        rows[0].tag = b"RSI".to_vec();
    }
    *frame.tables[TableId::OccupationSettings.index()] =
        TableView::Complete(TableRows::OccupationSettings(
            [21, 9001]
                .map(|country| OccupationSettingsRow {
                    country,
                    ..Default::default()
                })
                .into(),
        ));
    *frame.tables[TableId::CountryOccupationReserveManpower.index()] =
        TableView::Complete(TableRows::CountryOccupationReserveManpower(vec![
            CountryOccupationReserveManpowerRow {
                country: 9001,
                ordinal: 0,
                source_country: 21,
                amount: 500,
            },
        ]));
    frame.metrics.table_ns[TableId::OccupationSettings.index()] = u64::from(hour % 24 == 3);
    frame
}

#[test]
fn occupation_reserve_waits_for_the_next_walk_read_after_an_identity_change_in_blocks_as_hourly() {
    let frames: Vec<_> = (123..=160).map(reserve).collect();
    let replay = Replay::from_records(CAMPAIGN.into(), &[records(1, frames)]).unwrap();
    let mut fixture = Fixture::new(&replay);
    for hourly in [false, true] {
        use crate::prepared::prepass;
        let source = fixture.source(&format!("keyed-occupation-reserve-{hourly}"));
        let prepare = || run(&source, None, &AtomicBool::new(false), |_| {});
        // A refused pre-pass worker leaves every block to hourly observation.
        prepass::with_test_prepass(1, None, || {
            prepass::with_test_refused_workers(usize::from(hourly), prepare)
        })
        .unwrap();
        let reply = query(
            &source,
            "occupation-reserve-manpower",
            serde_json::json!({"branchId":"1"}),
        );
        assert_eq!(
            reply["window"]["known"],
            serde_json::json!([{"firstHour":123,"lastHour":129},{"firstHour":147,"lastHour":160}]),
            "hourly {hourly}: {reply}"
        );
        assert_eq!(
            reply["window"]["reads"],
            serde_json::json!([[123, 129, 123], [147, 160, 147]])
        );
        let runs = |name: &str| {
            reply["window"]["series"]
                .as_array()
                .unwrap()
                .iter()
                .find(|item| item["name"] == name)
                .map(|item| item["runs"].clone())
                .unwrap_or_default()
        };
        assert_eq!(
            runs("GER@7|0|amount"),
            serde_json::json!([[123, 129, "500", true], [147, 160, "500", true]])
        );
        assert_eq!(
            runs("ITA@8|holder"),
            serde_json::json!([[123, 129, "1", true]])
        );
        assert_eq!(
            runs("RSI@8|holder"),
            serde_json::json!([[147, 160, "1", true]])
        );
        super::super::super::files(&source, None)
            .unwrap()
            .0
            .clear()
            .unwrap();
    }
}
