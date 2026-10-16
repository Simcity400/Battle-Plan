//! The casualty routes as the registry serves them, against the scoreboard's run cache.

use super::LoadContext;
use super::war_casualties::{
    scanned_branch,
    test_fixtures::{casualty_frame, chosen, opened, research_replay},
};
use replay::frame::TableView;
use replay::rows::{TableId, TableRows};
use std::sync::{Arc, atomic::AtomicBool};

#[test]
fn casualty_routes_share_exact_lifetimes_with_the_current_scoreboard_run_cache() {
    let mut first = casualty_frame(100, Some((0, 0)), 0);
    if let TableView::Complete(TableRows::WarRelations(rows)) =
        &mut *first.tables[TableId::WarRelations.index()]
    {
        rows[0].first_casualties = i64::MAX;
        rows[0].second_casualties = -3;
    }
    let mut next = first.clone();
    next.hour = 101;
    if let TableView::Complete(TableRows::WarRelations(rows)) =
        &mut *next.tables[TableId::WarRelations.index()]
    {
        rows[0].first_country = 2;
        rows[0].second_country = 1;
        rows[0].first_casualties = -2;
        rows[0].second_casualties = i64::MAX - 1;
        rows.push(replay::rows::WarRelationRow {
            first_country: 1,
            second_country: 2,
            start_hour: 91,
            first_casualties: i64::MAX,
            second_casualties: 7,
            ..Default::default()
        });
        rows.sort_by_key(|row| (row.first_country, row.second_country));
    }
    let source = opened(research_replay(vec![
        first,
        next,
        casualty_frame(102, None, 0),
    ]));
    let cancelled = AtomicBool::new(false);
    let cached = scanned_branch(&source, 1, &cancelled).unwrap();
    let context = LoadContext { content: None };
    let route = |key, params| {
        let json = (super::oracle_route(key)
            .expect("casualty oracle route")
            .load)(&source, "current lease", params, &context, &cancelled)
        .unwrap();
        let envelope: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(envelope["sourceId"], "current lease");
        assert_eq!(envelope["product"], key);
        envelope["data"].clone()
    };
    let pairs = route(
        "casualty-pairs",
        serde_json::json!({
            "branchId": "1", "tags": [], "between": false, "window": 0,
        }),
    );
    let last = pairs["pairs"][0]["runs"]
        .as_array()
        .unwrap()
        .last()
        .unwrap();
    let wide = i128::from(i64::MAX) * 2 - 1;
    assert_eq!(last["firstTaken"], wide.to_string());
    assert_eq!(last["secondTaken"], "5");
    assert_eq!(last["lastHour"], 102);
    let breakdown = route(
        "casualty-breakdown",
        serde_json::json!({
            "branchId": "1", "tag": chosen().tags[0], "window": 0,
        }),
    );
    let total = breakdown["series"]
        .as_array()
        .unwrap()
        .iter()
        .map(|series| {
            series["runs"].as_array().unwrap().last().unwrap()["value"]
                .as_str()
                .unwrap()
                .parse::<i128>()
                .unwrap()
        })
        .sum::<i128>();
    assert_eq!(total, wide);
    assert!(
        breakdown["warnings"]
            .as_array()
            .unwrap()
            .iter()
            .any(|warning| warning.as_str().unwrap().contains("no domain split"))
    );
    let mut at = None;
    cached
        .for_each_country_run(&chosen().tags[0], &cancelled, |run| {
            if run.first_hour <= 102 && 102 <= run.last_hour {
                at = Some(run.value);
            }
            Ok(())
        })
        .unwrap();
    assert_eq!(at.as_ref().unwrap().taken, total);
    assert_eq!(at.as_ref().unwrap().dealt, 5);
    assert!(Arc::ptr_eq(
        &cached,
        &scanned_branch(&source, 1, &cancelled).unwrap()
    ));
}
