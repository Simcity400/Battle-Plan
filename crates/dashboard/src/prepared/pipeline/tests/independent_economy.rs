//! The economy families' queries reuse their whole reply through the
//! observation cache while their family's publication is unchanged, and
//! compute again once the family publishes.
use super::*;
use replay_data::rows::WarLendLeaseHistoryRow;

/// The balance's table on the shared scale frame: country 0 sends to country 1.
fn balance_frame(hour: i32) -> DecodedFrame {
    let mut observed = frame(1, hour, 4);
    *observed.tables[TableId::WarLendLeaseHistories.index()] =
        TableView::Complete(TableRows::WarLendLeaseHistories(vec![WarLendLeaseHistoryRow {
            country: 100_000,
            target: 100_001,
            industry_sent: i64::from(hour),
            fuel_sent: 0,
            industry_received: 0,
            fuel_received: 0,
        }]));
    observed
}

fn ask(source: &Source) -> serde_json::Value {
    let request = crate::transport::ProductRequest {
        source_id: source.key().source_id.clone(),
        revision: source.key().revision.clone(),
        product: "lend-lease-balance".into(),
        params: serde_json::json!({"branchId": "1"}),
    };
    let reply =
        super::super::super::query::query(source, &request, None, &AtomicBool::new(false)).unwrap();
    let mut reply: serde_json::Value = serde_json::from_str(&reply).unwrap();
    reply["data"].take()
}

fn computed(source: &Source) -> usize {
    dashboard_base::prepared::query::observations::usage(source, "queries:lend-lease-balance").2
}

#[test]
fn economy_queries_reuse_their_reply_until_their_family_publishes() {
    let replay = Replay::from_records(
        CAMPAIGN.into(),
        &[records(1, (100..=124).map(balance_frame).collect())],
    )
    .unwrap();
    let mut fixture = Fixture::new(&replay);
    let source = fixture.source("economy-reply-cache");
    // Before preparation: an empty prefix, computed once and reused.
    let cold = ask(&source);
    assert_eq!(cold["histories"], serde_json::json!([]));
    assert_eq!(ask(&source), cold);
    assert_eq!(computed(&source), 1);
    // The family publishes: the binding moves and the reply is computed again.
    run(&source, None, &AtomicBool::new(false), |_| {}).unwrap();
    let prepared = ask(&source);
    assert_eq!(computed(&source), 2);
    assert_eq!(prepared["histories"][0]["holder"], "0");
    assert_eq!(prepared["histories"][0]["values"][0][0], serde_json::json!([100, 100, "100"]));
    // An unchanged publication is reused.
    assert_eq!(ask(&source), prepared);
    assert_eq!(computed(&source), 2);
}
