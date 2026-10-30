//! The force families' progressive queries through the hub: prepared prefixes joined with
//! recorded tails, zooms, navigation and coverage as the host reads them.

use crate::products::parts_race_prepared::test_fixtures::Fixture;
use serde_json::Value;
use std::sync::atomic::AtomicBool;

/// The fixture read through the query hub, as the host reads it. The fixture
/// publishes the hourly walk's pages and branch coverage, so its products are
/// read as when every family is prepared hour by hour.
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
        let reply: Value =
            serde_json::from_str(&crate::prepared::pipeline::with_test_hourly(|| {
                crate::prepared::query::query(
                    &self.source,
                    &request,
                    self.context.content.as_ref(),
                    &AtomicBool::new(false),
                )
            })?)?;
        Ok(reply["data"].clone())
    }
}

mod families {
    use super::HubQuery;
    use crate::products::parts_race_prepared::test_fixtures::{Fixture, frame, tag};
    use crate::products::parts_race_prepared::{Family, Prepared};
    use serde_json::Value;
    use serde_json::json;
    use std::sync::atomic::AtomicBool;

    #[test]
    fn unchanged_suppliers_reuse_one_reply_across_store_epochs() {
        let fixture = Fixture::new();
        let params = json!({"branchId":"1", "family":{"kind":"air-force-size","measure":"planes","ownership":"roster"},
            "tags":[tag(0)], "from":0, "to":31, "width":32});
        let first = fixture.query("series-summary", params.clone());
        let retained = || {
            dashboard_base::prepared::query::observations::usage(
                &fixture.source,
                "queries:series-summary",
            )
        };
        let bytes = retained().0;
        for _ in 0..27 {
            let progress = fixture.source.state::<crate::prepared::Progress>();
            crate::prepared::invalidate(&mut progress.lock().unwrap());
            assert_eq!(fixture.query("series-summary", params.clone()), first);
            let (current, bound, _) = retained();
            assert!(
                current < 2 * bytes,
                "superseded replies accumulate: {current} > {bytes}"
            );
            assert!(current <= bound);
        }
        assert_eq!(retained().2, 1);
        let mut writer = fixture.store.replace(fixture.key.clone()).unwrap();
        writer
            .write_at("unrelated-family", 0, b"unrelated publication".as_slice())
            .unwrap();
        writer.commit().unwrap();
        {
            let progress = fixture.source.state::<crate::prepared::Progress>();
            crate::prepared::invalidate(&mut progress.lock().unwrap());
        }
        assert_eq!(fixture.query("series-summary", params.clone()), first);
        assert_eq!(retained().2, 1, "an unrelated page invalidated the reply");
        // No epoch change accompanies these fixture publications. Page identity
        // alone must invalidate an accepted prefix, including completion.
        for complete in [false, true] {
            let mut writer = fixture.store.extend(&fixture.key).unwrap();
            dashboard_base::prepared::coverage::publish_coverage(&mut writer, 1, 0, 1, complete)
                .unwrap();
            writer.commit().unwrap();
            let _ = fixture.query("series-summary", params.clone());
        }
        assert_eq!(retained().2, 3, "changed gate extents reused a stale reply");
        let mut writer = fixture.store.extend(&fixture.key).unwrap();
        let identities = crate::products::country_history::HistoryScan::new(false, &[]);
        writer
            .write_at(
                "country-identities-1",
                0,
                serde_json::to_vec(&identities).unwrap().as_slice(),
            )
            .unwrap();
        writer.commit().unwrap();
        let _ = fixture.query("series-summary", params);
        assert_eq!(
            retained().2,
            4,
            "an independently replaced identity page reused the reply"
        );
    }

    fn tail_and_zoom<F: Family>(family: Value, expected: f64) {
        let fixture = Fixture::new();
        let params = |from| json!({"branchId":"1", "family":family, "tags":[tag(0), tag(1)], "from":from, "to":31, "width":32});
        let cold = fixture.query("series-summary", params(0));
        let computed = dashboard_base::prepared::query::observations::usage(
            &fixture.source,
            "queries:series-summary",
        )
        .2;
        assert_eq!(computed, 1);
        let (again, refused) =
            dashboard_base::prepared::query::progressive::without_block_decodes(|| {
                fixture.query("series-summary", params(0))
            });
        assert!(
            !refused,
            "an unchanged gate reuses the reply and its sampled metadata"
        );
        assert_eq!(again, cold);
        assert_eq!(
            dashboard_base::prepared::query::observations::usage(
                &fixture.source,
                "queries:series-summary",
            )
            .2,
            computed
        );
        assert!(
            !cold["summaries"][0]["summary"]["approximate"]
                .as_array()
                .unwrap()
                .is_empty()
        );
        assert!(
            cold["series"]
                .as_array()
                .unwrap()
                .iter()
                .any(|row| row["tag"] == tag(1)),
            "tail-only countries need no catalogue key"
        );
        let after_rename = fixture.query(
            "series-summary",
            json!({"branchId":"1", "family":family,
            "tags":[tag(0)], "from":26,"to":31,"width":6}),
        );
        assert!(
            after_rename["series"]
                .as_array()
                .unwrap()
                .iter()
                .any(|row| row["tag"] == "renamed / first"
                    && !row["runs"].as_array().unwrap().is_empty())
        );
        assert!(fixture.store.saved_key().unwrap().is_none());
        let mut prepared = Prepared::<F>::default();
        for hour in 0..2 {
            prepared
                .observe(
                    &frame(hour, 1),
                    fixture.source.replay_ref(),
                    &fixture.source,
                    &fixture.context,
                    &AtomicBool::new(false),
                    &crate::prepared::frame_changes::FrameChangeTracker::new().observe(
                        &frame(hour, if hour < 2 { 1 } else { 2 }),
                        fixture.source.replay_ref().format,
                    ),
                )
                .unwrap();
            let mut writer = if hour == 0 {
                fixture.store.replace(fixture.key.clone()).unwrap()
            } else {
                fixture.store.extend(&fixture.key).unwrap()
            };
            prepared
                .flush(&mut writer, 1, &AtomicBool::new(false))
                .unwrap();
            fixture.identities(&mut writer, hour);
            writer.commit().unwrap();
            let bytes = serde_json::to_vec(&prepared).unwrap();
            prepared = serde_json::from_slice(&bytes).unwrap();
            prepared
                .resume(
                    &frame(hour, 1),
                    fixture.source.replay_ref(),
                    &AtomicBool::new(false),
                )
                .unwrap();
        }
        let channel = family["measure"]
            .as_str()
            .map_or(0, |measure| usize::from(measure == "strength"));
        let ranges = fixture
            .store
            .range_count(&fixture.key, &format!("{}-values-1-{channel}", F::NAME))
            .unwrap();
        let rough = fixture.query("series-summary", params(0));
        assert!(
            dashboard_base::prepared::query::observations::usage(
                &fixture.source,
                "queries:series-summary",
            )
            .2 > computed,
            "a publication invalidates the reply"
        );
        assert_eq!(rough["coverage"]["lastHour"], 1);
        let exact = fixture.query("series-summary", params(6));
        for row in exact["summaries"].as_array().unwrap() {
            assert_eq!(row["summary"]["approximate"], json!([]));
        }
        let primary = exact["summaries"]
            .as_array()
            .unwrap()
            .iter()
            .find(|row| row["tag"] == tag(0))
            .unwrap();
        assert!(
            primary["summary"]["runs"]
                .as_array()
                .unwrap()
                .iter()
                .any(|run| run["firstHour"] == 9 && run["value"] == expected)
        );
        let alias = fixture.query(
            "series-summary",
            json!({"branchId":"1","family":family,"tags":[tag(0)],"from":0,"to":31,"width":32}),
        );
        assert!(
            alias["series"]
                .as_array()
                .unwrap()
                .iter()
                .any(|row| row["tag"] == "renamed / first")
        );
        assert!(
            !alias["series"]
                .as_array()
                .unwrap()
                .iter()
                .any(|row| row["tag"] == tag(1)),
            "sampling all holders must not expand manual selection"
        );
        let mut top = params(0);
        top["tags"] = json!([]);
        top["family"]["top"] = json!(1);
        assert!(
            !fixture.query("series-summary", top)["series"]
                .as_array()
                .unwrap()
                .is_empty()
        );
        let navigation = fixture.query("series-next-change", json!({"branchId":"1", "family":family,"tags":[tag(0)],"key":"ArrowRight","from":8,"firstHour":6,"lastHour":31}));
        assert_eq!(navigation["hour"].as_f64(), Some(9.0));
        assert_eq!(
            fixture
                .store
                .range_count(&fixture.key, &format!("{}-values-1-{channel}", F::NAME))
                .unwrap(),
            ranges,
            "transient queries never publish"
        );
        // Resume preparation into recording blocks appended after the original prefix.
        for hour in 2..4 {
            prepared
                .observe(
                    &frame(hour, 2),
                    fixture.source.replay_ref(),
                    &fixture.source,
                    &fixture.context,
                    &AtomicBool::new(false),
                    &crate::prepared::frame_changes::FrameChangeTracker::new().observe(
                        &frame(hour, if hour < 2 { 1 } else { 2 }),
                        fixture.source.replay_ref().format,
                    ),
                )
                .unwrap();
        }
        let mut writer = fixture.store.extend(&fixture.key).unwrap();
        prepared
            .flush(&mut writer, 1, &AtomicBool::new(false))
            .unwrap();
        fixture.identities(&mut writer, 3);
        writer.commit().unwrap();
        let appended = fixture.query("series-summary", params(0));
        assert_eq!(appended["coverage"]["lastHour"], 3);
        assert!(
            appended["series"]
                .as_array()
                .unwrap()
                .iter()
                .any(|row| row["tag"] == tag(1))
        );
    }
    #[test]
    fn air_force_progressive_queries_join_prepared_prefix_and_tail_then_zoom_exactly_without_writing()
     {
        tail_and_zoom::<crate::products::air_force::prepared::Air>(
            json!({"kind":"air-force-size","measure":"planes","ownership":"roster"}),
            9.0,
        );
    }
    #[test]
    fn navy_progressive_queries_join_prepared_prefix_and_tail_then_zoom_exactly_without_writing() {
        tail_and_zoom::<crate::products::navy::prepared::Navy>(
            json!({"kind":"navy-size","measure":"strength","ownership":"roster","grouping":"class"}),
            100_009.0,
        );
    }

    #[test]
    fn navy_progressive_parts_and_maximum_use_native_point_projection_and_exact_state() {
        let fixture = Fixture::new();
        let family = json!({"kind":"navy-size", "measure":"strength", "ownership":"roster", "grouping":"class"});
        let rough = fixture.query(
            "series-summary",
            json!({"branchId":"1", "family":family,
            "tags":[tag(0)], "from":0, "to":31, "width":32, "requireParts":true}),
        );
        assert!(!rough["parts"].as_array().unwrap().is_empty());
        assert!(
            !rough["summaries"][0]["parts"][0]["summary"]["approximate"]
                .as_array()
                .unwrap()
                .is_empty()
        );
        let state = fixture.query(
            "series-state",
            json!({"branchId":"1", "family":family,
            "reads":[{"tag":tag(0), "hour":9}], "requireParts":true}),
        );
        let run = &state["series"][0]["runs"][0];
        assert_eq!(run["total"], 100_009);
        assert_eq!(run["referenceTotal"], 200_009);
        assert_eq!(
            run["values"]
                .as_array()
                .unwrap()
                .iter()
                .filter_map(Value::as_i64)
                .sum::<i64>(),
            100_009
        );
        assert!(fixture.store.saved_key().unwrap().is_none());
    }

    #[test]
    fn force_progressive_state_returns_only_each_requested_country_moment() {
        let fixture = Fixture::new();
        let state = fixture.query(
            "series-state",
            json!({"branchId":"1", "family":{
            "kind":"navy-size", "measure":"strength", "ownership":"roster", "grouping":"class"},
            "reads":[{"tag":tag(0),"hour":9},{"tag":tag(1),"hour":11}]}),
        );
        let rows = state["series"].as_array().unwrap();
        assert_eq!(rows.len(), 2);
        for row in rows {
            let hour = if row["tag"] == tag(0) { 9 } else { 11 };
            assert_eq!(row["runs"].as_array().unwrap().len(), 1);
            assert_eq!(row["runs"][0]["firstHour"], hour);
            assert_eq!(row["runs"][0]["lastHour"], hour);
            assert_eq!(row["runs"][0]["total"], 100_000 + hour);
        }
        assert!(fixture.store.saved_key().unwrap().is_none());
    }

    #[test]
    fn air_force_progressive_totals_draw_while_dated_variant_parts_keep_their_coverage_gate() {
        let fixture = Fixture::new();
        let params = json!({"branchId":"1", "family":{
            "kind":"air-force-size", "measure":"planes", "ownership":"roster"},
            "tags":[tag(0)], "from":0,"to":31,"width":32});
        assert!(!fixture.query("series-summary", params.clone()).is_null());
        let mut parts = params;
        parts["requireParts"] = json!(true);
        assert!(fixture.query("series-summary", parts).is_null());
        assert!(fixture.store.saved_key().unwrap().is_none());
    }
}

#[test]
fn cold_force_replies_have_empty_coverage_and_pending_hours() {
    use crate::products::parts_race_prepared::test_fixtures::tag;
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
    // Army size and supply health are the segment driver's: before their first publication
    // there is no reply.
    for (product, params) in [
        (
            "army-size",
            json!({"branchId":"1", "tags":[tag(0)], "measure":"battalions", "countFor":"owner", "byType":true, "window":0}),
        ),
        (
            "supply-health",
            json!({"branchId":"1", "tags":[tag(0)], "from":0, "to":1, "width":2}),
        ),
    ] {
        let cold = fixture.query(product, params);
        assert!(cold.is_null(), "{product}: {cold}");
    }
}
