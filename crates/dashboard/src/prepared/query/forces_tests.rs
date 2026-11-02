//! The force families' queries through the hub, as the host reads them.

use crate::products::force_fixtures::Fixture;
use serde_json::Value;
use std::sync::atomic::AtomicBool;

/// The fixture read through the query hub, as the host reads it. The fixture
/// publishes the force families' pages and their coverage pages.
trait HubQuery {
    fn ask(&self, product: &str, params: Value) -> std::io::Result<Value>;
    fn query(&self, product: &str, params: Value) -> Value {
        self.ask(product, params).unwrap()
    }
}
impl HubQuery for Fixture {
    fn ask(&self, product: &str, params: Value) -> std::io::Result<Value> {
        let request = crate::transport::ProductRequest {
            source_id: self.source.key().source_id.clone(),
            revision: self.source.key().revision.clone(),
            product: product.into(),
            params,
        };
        let reply: Value = serde_json::from_str(&crate::prepared::query::query(
            &self.source,
            &request,
            self.context.content.as_ref(),
            &AtomicBool::new(false),
        )?)?;
        Ok(reply["data"].clone())
    }
}

#[test]
fn cold_force_replies_have_empty_coverage_and_pending_hours() {
    use crate::products::force_fixtures::tag;
    use serde_json::json;
    let fixture = Fixture::new();
    let cold = fixture.query(
        "volunteers-expeditionaries",
        json!({"branchId":"1","kind":"land","tags":null}),
    );
    assert_eq!(cold["coverage"]["firstHour"], 0, "{cold}");
    assert_eq!(cold["coverage"]["lastHour"], -1);
    assert_eq!(cold["coverage"]["complete"], false);
    assert!(!cold["pending"].as_array().unwrap().is_empty());
    assert_eq!(cold["links"], json!([]));
    // Army size, supply health and navy size are the segment driver's: before their first
    // publication there is no reply.
    for (product, params) in [
        (
            "army-size",
            json!({"branchId":"1", "tags":[tag(0)], "measure":"battalions", "countFor":"owner", "byType":true, "window":0}),
        ),
        (
            "supply-health",
            json!({"branchId":"1", "tags":[tag(0)], "from":0, "to":1, "width":2}),
        ),
        (
            "series-summary",
            json!({"branchId":"1", "tags":[tag(0)], "from":0, "to":1, "width":2,
                "family":{"kind":"navy-size", "top":null, "measure":"ships",
                    "ownership":"roster", "grouping":"class"}}),
        ),
    ] {
        let cold = fixture.query(product, params);
        assert!(cold.is_null(), "{product}: {cold}");
    }
}
