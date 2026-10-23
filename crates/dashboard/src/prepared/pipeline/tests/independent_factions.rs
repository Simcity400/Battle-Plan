//! Faction goals prepared by the pipeline over a damaged block: the accepted prefix ends before it.
use super::*;
use crate::products::{LoadContext, Product, faction_details};
use replay_data::rows::{FactionGoalEntryRow, FactionGoalStatusRow, FactionRow, LexicalTokenRow};

fn goals_frame(hour: i32) -> DecodedFrame {
    let mut observed = frame(1, hour, 2);
    match &mut *observed.tables[TableId::Factions.index()] {
        TableView::Complete(TableRows::Factions(rows)) if rows.iter().any(|row| row.id == 9) => {}
        TableView::Complete(TableRows::Factions(rows)) => {
            rows.push(FactionRow {
                id: 9,
                ..Default::default()
            });
            rows.sort_by_key(|row| row.id);
        }
        table => {
            *table = TableView::Complete(TableRows::Factions(vec![FactionRow {
                id: 9,
                ..Default::default()
            }]))
        }
    }
    let mut tokens = match &*observed.tables[TableId::LexicalTokens.index()] {
        TableView::Complete(TableRows::LexicalTokens(rows)) => rows.clone(),
        _ => Vec::new(),
    };
    let token = tokens.iter().map(|row| row.token).max().unwrap_or(0) + 1;
    tokens.push(LexicalTokenRow {
        token,
        literal: b"mod_goal".to_vec(),
    });
    *observed.tables[TableId::LexicalTokens.index()] =
        TableView::Complete(TableRows::LexicalTokens(tokens));
    *observed.tables[TableId::FactionGoalStatus.index()] =
        TableView::Complete(TableRows::FactionGoalStatus(vec![FactionGoalStatusRow {
            faction: 9,
            manifest: None,
        }]));
    *observed.tables[TableId::FactionGoalEntries.index()] =
        TableView::Complete(TableRows::FactionGoalEntries(vec![FactionGoalEntryRow {
            faction: 9,
            kind: 0,
            ordinal: 0,
            token: Some(token),
            progress_raw: i64::from(hour) * 1000,
            ..Default::default()
        }]));
    observed
}
#[test]
fn a_damaged_block_ends_the_prepared_goal_history_and_leaves_later_hours_pending() {
    let first = Replay::from_records(
        CAMPAIGN.into(),
        &[records(1, vec![goals_frame(10), goals_frame(11)])],
    )
    .unwrap();
    let second = first
        .append(&[continuation_records(
            &first,
            1,
            vec![goals_frame(12), goals_frame(13)],
        )])
        .unwrap();
    let replay = second
        .append(&[continuation_records(
            &second,
            1,
            vec![goals_frame(14), goals_frame(15)],
        )])
        .unwrap();
    let mut fixture = Fixture::new(&replay);
    let source = fixture.source("faction-goals-damaged");
    fixture.corrupt_block_preserving_stamp(&replay.index[1]);
    assert!(
        source
            .replay_ref()
            .read_block(&source.replay_ref().index[1])
            .is_err()
    );
    let flag = AtomicBool::new(false);
    // The failed read propagates; nothing after the damaged block is accepted.
    let walked = run(&source, None, &flag, |_| {});
    if let Ok(accepted) = &walked {
        assert!(accepted.next_block <= 1, "accepted {}", accepted.next_block);
    }
    assert!(
        unprepared_blocks(&source, None)
            .unwrap()
            .is_none_or(|left| left >= 2)
    );
    let request = crate::transport::ProductRequest {
        source_id: source.key().source_id.clone(),
        revision: source.key().revision.clone(),
        product: "faction-state".into(),
        params: serde_json::json!({"view":"goals","branchId":"1","faction":"9","firstHour":null,"lastHour":null}),
    };
    let reply: serde_json::Value = serde_json::from_str(
        &crate::prepared::query::query(&source, &request, None, &flag).unwrap(),
    )
    .unwrap();
    let data = &reply["data"];
    assert_eq!(data["coverage"]["complete"], false, "{data}");
    let last = data["coverage"]["lastHour"].as_i64().unwrap();
    assert!(
        last < 12,
        "the accepted prefix ends before the damaged block: {data}"
    );
    for run in data["runs"].as_array().unwrap() {
        assert!(run["lastHour"].as_i64().unwrap() <= last, "{run}");
    }
    // Every later hour is pending, so the widget asks that hour's own details.
    for hour in last + 1..=15 {
        assert!(
            data["pending"].as_array().unwrap().iter().any(|span| {
                span["firstHour"].as_i64().unwrap() <= hour
                    && hour <= span["lastHour"].as_i64().unwrap()
            }),
            "hour {hour} is pending: {data}"
        );
    }
    // Within the accepted prefix, each hour's lists are that hour's faction details.
    for hour in 10..=last {
        let details = faction_details::FactionDetails::load(
            &source,
            &faction_details::FactionDetailParams {
                branch_id: "1".into(),
                hour: hour as i32,
                faction: "9".into(),
            },
            &LoadContext { content: None },
            &flag,
        )
        .unwrap();
        let run = data["runs"]
            .as_array()
            .unwrap()
            .iter()
            .find(|run| {
                run["firstHour"].as_i64().unwrap() <= hour
                    && hour <= run["lastHour"].as_i64().unwrap()
            })
            .unwrap_or_else(|| panic!("hour {hour} is answered: {data}"));
        let list = &data["lists"][run["list"].as_u64().unwrap() as usize];
        let held: Vec<_> = list["goals"]
            .as_array()
            .unwrap()
            .iter()
            .map(|goal| {
                let definition =
                    &data["definitions"][goal["definition"].as_u64().unwrap() as usize];
                (
                    goal["ordinal"].as_u64().unwrap() as u32,
                    definition["token"].clone(),
                    definition["reason"].clone(),
                    goal["status"].as_i64().unwrap() as i32,
                )
            })
            .collect();
        let expected: Vec<_> = details
            .goals
            .iter()
            .map(|goal| {
                (
                    goal.ordinal,
                    serde_json::json!(goal.definition.token),
                    serde_json::json!(goal.definition.reason),
                    goal.status,
                )
            })
            .collect();
        assert_eq!(held, expected, "hour {hour}");
        let progress = data["progress"][0]["runs"]
            .as_array()
            .unwrap()
            .iter()
            .find(|run| {
                run["firstHour"].as_i64().unwrap() <= hour
                    && hour <= run["lastHour"].as_i64().unwrap()
            })
            .map(|run| run["progressRaw"].as_str().unwrap().to_owned());
        assert_eq!(
            progress,
            Some(details.goals[0].progress_raw.clone()),
            "hour {hour}"
        );
    }
}
