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
    /// As the data service answers: on the prepared lane without block decodes,
    /// and again on the sampled-read loader where that lane refused.
    fn served(&self, product: &str, params: Value) -> Value {
        let (answer, refused) =
            dashboard_base::prepared::query::progressive::without_block_decodes(|| {
                self.ask(product, params.clone())
            });
        if refused {
            self.query(product, params)
        } else {
            answer.unwrap()
        }
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
        let reply: Value = serde_json::from_str(&crate::prepared::pipeline::with_test_hourly(
            || {
                crate::prepared::query::query(
                    &self.source,
                    &request,
                    self.context.content.as_ref(),
                    &AtomicBool::new(false),
                )
            },
        )?)?;
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
        let params = json!({"branchId":"1", "family":{"kind":"army-size","measure":"strength","countFor":"owner"},
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

mod army {
    use super::HubQuery;
    use crate::products::army_size::prepared::Prepared;
    use crate::products::parts_race_prepared::test_fixtures::{Fixture, frame, tag};
    use serde_json::json;
    use std::sync::atomic::AtomicBool;

    #[test]
    fn army_progressive_queries_join_prepared_prefix_and_tail_then_zoom_exactly_without_writing() {
        let fixture = Fixture::new();
        let family = json!({"kind":"army-size", "measure":"strength", "countFor":"owner"});
        let params = |from| json!({"branchId":"1", "family":family, "tags":[tag(0),tag(1)], "from":from, "to":31, "width":32});
        let cold = fixture.query("series-summary", params(0));
        assert!(
            !cold["series"][0]["summary"]["approximate"]
                .as_array()
                .unwrap()
                .is_empty()
        );
        assert!(
            cold["series"]
                .as_array()
                .unwrap()
                .iter()
                .any(|row| row["tag"] == tag(1))
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
                    && !row["summary"]["runs"].as_array().unwrap().is_empty()),
            "central branch-start identity evidence keeps a cold zoom attached to its original holder"
        );
        assert!(fixture.store.saved_key().unwrap().is_none());
        let mut prepared = Prepared::default();
        for hour in 0..2 {
            prepared
                .observe(
                    &frame(hour, 1),
                    fixture.source.replay_ref(),
                    &crate::prepared::frame_changes::FrameChangeTracker::new()
                        .observe(&frame(hour, 1), fixture.source.replay_ref().format),
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
            prepared = serde_json::from_slice(&serde_json::to_vec(&prepared).unwrap()).unwrap();
        }
        let ranges = fixture
            .store
            .range_count(&fixture.key, "army-values-1-1")
            .unwrap();
        let rough = fixture.query("series-summary", params(0));
        assert_eq!(rough["coverage"]["lastHour"], 1);
        let exact = fixture.query("series-summary", params(6));
        assert!(
            exact["series"]
                .as_array()
                .unwrap()
                .iter()
                .all(|row| row["summary"]["approximate"] == json!([]))
        );
        let primary = exact["series"]
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
                .any(|run| run["firstHour"] == 9 && run["value"].as_f64() == Some(1.9))
        );
        let aliases = fixture.query(
            "series-summary",
            json!({"branchId":"1", "family":family, "tags":[tag(0)], "from":0,"to":31,"width":32}),
        );
        assert!(
            aliases["series"]
                .as_array()
                .unwrap()
                .iter()
                .any(|row| row["tag"] == "renamed / first")
        );
        assert!(
            !aliases["series"]
                .as_array()
                .unwrap()
                .iter()
                .any(|row| row["tag"] == tag(1))
        );
        let top = fixture.query("series-summary", json!({"branchId":"1", "family":{"top":1,"kind":"army-size","measure":"strength","countFor":"owner"}, "tags":[],"from":0,"to":31,"width":32}));
        assert!(!top["series"].as_array().unwrap().is_empty());
        let navigation = fixture.query(
            "series-next-change",
            json!({"branchId":"1", "family":family,
            "tags":[tag(0)],"key":"ArrowRight","from":8,"firstHour":6,"lastHour":31}),
        );
        assert_eq!(navigation["hour"].as_f64(), Some(9.0));
        let prefix_navigation = fixture.query(
            "series-next-change",
            json!({"branchId":"1", "family":family,
            "tags":[tag(0)],"key":"ArrowRight","from":0,"firstHour":0,"lastHour":1}),
        );
        assert_eq!(prefix_navigation["hour"].as_f64(), Some(1.0));
        let block = fixture.query("army-size", json!({"branchId":"1","tags":[tag(0)],"measure":"battalions","countFor":"owner","byType":true,"window":4}));
        assert!(
            block["series"][0]["mix"]
                .as_array()
                .unwrap()
                .iter()
                .any(|run| run["firstHour"] == 8)
        );
        assert_eq!(
            fixture
                .store
                .range_count(&fixture.key, "army-values-1-1")
                .unwrap(),
            ranges
        );
        for hour in 2..4 {
            prepared
                .observe(
                    &frame(hour, 2),
                    fixture.source.replay_ref(),
                    &crate::prepared::frame_changes::FrameChangeTracker::new()
                        .observe(&frame(hour, 2), fixture.source.replay_ref().format),
                )
                .unwrap();
        }
        let mut writer = fixture.store.extend(&fixture.key).unwrap();
        prepared
            .flush(&mut writer, 1, &AtomicBool::new(false))
            .unwrap();
        fixture.identities(&mut writer, 3);
        writer.commit().unwrap();
        assert_eq!(
            fixture.query("series-summary", params(0))["coverage"]["lastHour"],
            3
        );
    }
}

mod supply {
    use super::HubQuery;
    use crate::products::parts_race_prepared::test_fixtures::{Fixture, frame, tag};
    use crate::products::supply_health::prepared::Prepared;
    use std::sync::atomic::AtomicBool;

    #[test]
    fn cached_supply_endpoint_refuses_after_unverified_content_is_disabled() {
        crate::prepared::pipeline::with_test_hourly(|| {
            let mut fixture = Fixture::new();
            assert!(matches!(
                fixture.source.replay_ref().identity.verification,
                replay_data::identity::Verification::Unchecked
            ));
            let params =
                serde_json::json!({"branchId":"1", "tags":[tag(0)], "from":31, "to":31, "width":1});
            let first = fixture.query("supply-health", params.clone());
            assert!(first["available"].as_bool().unwrap());
            assert!(
                first["warnings"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|warning| warning.as_str().unwrap().contains("UNVERIFIED CONTENT"))
            );
            let usage = || {
                dashboard_base::prepared::query::observations::usage(
                    &fixture.source,
                    "supply-health:point",
                )
                .2
            };
            assert_eq!(usage(), 1);
            let flag = AtomicBool::new(false);
            // Check the lower cache directly: a fresh whole reply must not hide
            // an endpoint which still accepts the previous verification choice.
            let again = crate::products::supply_health::prepared::query(
                &fixture.source,
                &params,
                &fixture.context,
                &flag,
            )
            .unwrap()
            .unwrap();
            assert!(again.available);
            assert_eq!(usage(), 1);
            assert!(
                dashboard_base::prepared::coverage::coverage(
                    &fixture.source,
                    fixture.context.content.as_ref(),
                    1,
                    &flag,
                )
                .unwrap()
                .is_none()
            );
            fixture.context.content.as_mut().unwrap().allow_unverified = false;
            let (_, current) =
                dashboard_base::prepared::files(&fixture.source, fixture.context.content.as_ref())
                    .unwrap();
            assert_eq!(current, fixture.key, "file stamps are unchanged");
            let progress = fixture.source.state::<crate::prepared::Progress>();
            crate::prepared::invalidate(&mut progress.lock().unwrap());
            let error = crate::products::supply_health::prepared::query(
                &fixture.source,
                &params,
                &fixture.context,
                &flag,
            )
            .unwrap_err();
            assert!(
                error
                    .to_string()
                    .contains("recording has unverified content"),
                "{error}"
            );
            let request = crate::transport::ProductRequest {
                source_id: fixture.source.key().source_id.clone(),
                revision: fixture.source.key().revision.clone(),
                product: "supply-health".into(),
                params,
            };
            let error = crate::prepared::query::query(
                &fixture.source,
                &request,
                fixture.context.content.as_ref(),
                &flag,
            )
            .unwrap_err();
            assert!(
                error
                    .to_string()
                    .contains("recording has unverified content"),
                "{error}"
            );
            assert_eq!(usage(), 1, "refused endpoints are not cached");
        });
    }

    #[test]
    fn supply_progressive_queries_join_prepared_prefix_and_tail_then_zoom_exactly_without_writing()
    {
        use serde_json::json;
        let fixture = Fixture::new();
        let point = json!({"branchId":"1", "tags":[tag(0),tag(1)], "from":31, "to":31, "width":1});
        let cold_point = fixture.query("supply-health", point.clone());
        assert_eq!(
            dashboard_base::prepared::query::frames::reads(&fixture.source),
            (0, 1),
            "a reached supply view does not fold preceding block hours"
        );
        let mut history_point = point.clone();
        history_point.as_object_mut().unwrap().remove("width");
        // The first tag names no country at 31: its holder is seeded from the
        // branch's first keyframe, which only the sampled-read loader decodes.
        let history_point = fixture.served("supply-health", history_point);
        assert_eq!(
            dashboard_base::prepared::query::frames::reads(&fixture.source),
            (0, 1),
            "the two mounted point views share the same extraction"
        );
        assert_eq!(history_point["series"], cold_point["series"]);
        assert_eq!(
            dashboard_base::prepared::query::observations::usage(
                &fixture.source,
                "supply-health:point",
            )
            .2,
            1
        );
        let params =
            |from| json!({"branchId":"1","tags":[tag(0),tag(1)],"from":from,"to":31,"width":32});
        let cold = fixture.query("supply-health", params(0));
        assert!(
            !cold["summaries"][0]["values"][0]["approximate"]
                .as_array()
                .unwrap()
                .is_empty()
        );
        assert!(
            cold["series"]
                .as_array()
                .unwrap()
                .iter()
                .any(|row| row["tag"] == tag(1))
        );
        let after_rename = fixture.query(
            "supply-health",
            json!({"branchId":"1", "tags":[tag(0)], "from":26,"to":31,"width":6}),
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
        let mut prepared = Prepared::default();
        prepared
            .configure(&fixture.source, fixture.context.content.as_ref())
            .unwrap();
        let mut changes = crate::prepared::frame_changes::FrameChangeTracker::new();
        for hour in 0..2 {
            let frame = frame(hour, 1);
            prepared
                .observe(
                    &frame,
                    &changes.observe(&frame, fixture.source.replay_ref().format),
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
            prepared = serde_json::from_slice(&serde_json::to_vec(&prepared).unwrap()).unwrap();
        }
        let ranges = fixture
            .store
            .range_count(&fixture.key, "supply-runs-1")
            .unwrap();
        let rough = fixture.query("supply-health", params(0));
        let (advanced_point, refused) =
            dashboard_base::prepared::query::progressive::without_block_decodes(|| {
                fixture.query("supply-health", point)
            });
        assert!(
            !refused,
            "a moved prefix does not resample the unchanged endpoint"
        );
        assert_eq!(advanced_point["coverage"]["lastHour"], 1);
        assert_eq!(
            dashboard_base::prepared::query::observations::usage(
                &fixture.source,
                "supply-health:point",
            )
            .2,
            1
        );
        assert_eq!(rough["coverage"]["lastHour"], 1);
        let exact = fixture.query("supply-health", params(6));
        for row in exact["summaries"].as_array().unwrap() {
            for summary in row["values"].as_array().unwrap() {
                assert_eq!(summary["approximate"], json!([]));
            }
        }
        let primary = exact["series"]
            .as_array()
            .unwrap()
            .iter()
            .find(|row| row["tag"] == tag(0))
            .unwrap();
        assert!(
            primary["runs"]
                .as_array()
                .unwrap()
                .iter()
                .any(|run| run["firstHour"] == 9 && run["inSupply"] == 1)
        );
        assert!(
            !primary["runs"]
                .as_array()
                .unwrap()
                .iter()
                .any(|run| run["firstHour"].as_i64().unwrap() <= 7
                    && run["lastHour"].as_i64().unwrap() >= 7),
            "the missing companion is a local unknown gap"
        );
        let aliases = fixture.query(
            "supply-health",
            json!({"branchId":"1","tags":[tag(0)],"from":0,"to":31,"width":32}),
        );
        assert!(
            aliases["series"]
                .as_array()
                .unwrap()
                .iter()
                .any(|row| row["tag"] == "renamed / first")
        );
        assert!(
            !aliases["series"]
                .as_array()
                .unwrap()
                .iter()
                .any(|row| row["tag"] == tag(1))
        );
        let navigation = fixture.query("series-next-change", json!({"branchId":"1","family":{"kind":"supply-health"},"tags":[tag(0)],"key":"ArrowRight","from":8,"firstHour":6,"lastHour":31}));
        assert_eq!(navigation["hour"].as_f64(), Some(9.0));
        assert_eq!(
            fixture
                .store
                .range_count(&fixture.key, "supply-runs-1")
                .unwrap(),
            ranges
        );
        for hour in 2..4 {
            let frame = frame(hour, 2);
            prepared
                .observe(
                    &frame,
                    &changes.observe(&frame, fixture.source.replay_ref().format),
                )
                .unwrap();
        }
        let mut writer = fixture.store.extend(&fixture.key).unwrap();
        prepared
            .flush(&mut writer, 1, &AtomicBool::new(false))
            .unwrap();
        fixture.identities(&mut writer, 3);
        writer.commit().unwrap();
        assert_eq!(
            fixture.query("supply-health", params(0))["coverage"]["lastHour"],
            3
        );
    }
}

#[test]
fn cold_force_replies_have_empty_coverage_and_pending_hours() {
    use crate::products::parts_race_prepared::test_fixtures::tag;
    use serde_json::json;
    let fixture = Fixture::new();
    for (product, params) in [
        (
            "volunteers-expeditionaries",
            json!({"branchId":"1","kind":"land","tags":null}),
        ),
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
        assert_eq!(cold["coverage"]["firstHour"], 0, "{product}: {cold}");
        assert_eq!(cold["coverage"]["lastHour"], -1, "{product}");
        assert_eq!(cold["coverage"]["complete"], false, "{product}");
        assert!(!cold["pending"].as_array().unwrap().is_empty(), "{product}");
    }
}

#[test]
fn military_aid_prepared_route_cold_prefix_cache_and_independent_coverage() {
    // Aid rows read their fields through the installation's table metadata.
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
    use crate::prepared::frame_changes::FrameChangeTracker;
    use crate::products::{
        army_size::prepared::Prepared,
        parts_race_prepared::test_fixtures::{frame, tag},
    };
    use replay_data::rows::{SentVolunteerRow, TableId, TableRows, VolunteerHostRow};
    use replay_frame::frame::TableView;
    use serde_json::json;
    let fixture = Fixture::recording_only();
    let flag = AtomicBool::new(false);
    let params = json!({"branchId":"1","kind":"land","tags":[tag(0)]});
    let (cold, decoded) = crate::prepared::query::progressive::without_block_decodes(|| {
        fixture.query("volunteers-expeditionaries", params.clone())
    });
    assert!(!decoded);
    assert_eq!(cold["links"], json!([]));
    assert_eq!(cold["coverage"]["lastHour"], -1);
    let mut state = Prepared::default();
    let mut changes = FrameChangeTracker::new();
    for hour in 0..4 {
        let mut frame = frame(hour, 2);
        *frame.tables[TableId::SentVolunteers.index()] =
            TableView::Complete(TableRows::SentVolunteers(vec![SentVolunteerRow {
                country: 100000,
                ordinal: 0,
                division: 2,
            }]));
        *frame.tables[TableId::VolunteerHosts.index()] = if hour == 1 {
            TableView::Unavailable {
                reason: "tally absent".into(),
            }
        } else {
            TableView::Complete(TableRows::VolunteerHosts(vec![VolunteerHostRow {
                country: 100000,
                position: 0,
                host: 100001,
                deployed_divisions: 7,
                ..Default::default()
            }]))
        };
        state
            .observe(
                &frame,
                fixture.source.replay_ref(),
                &changes.observe(&frame, fixture.source.replay_ref().format),
            )
            .unwrap();
    }
    let mut writer = fixture.store.replace(fixture.key.clone()).unwrap();
    state.flush(&mut writer, 1, &flag).unwrap();
    fixture.identities(&mut writer, 3);
    writer.commit().unwrap();
    let (reply, decoded) = crate::prepared::query::progressive::without_block_decodes(|| {
        fixture.query("volunteers-expeditionaries", params.clone())
    });
    assert!(!decoded);
    assert_eq!(reply["coverage"]["lastHour"], 3);
    assert!(!reply["pending"].as_array().unwrap().is_empty());
    assert_eq!(reply["links"][0]["runs"][0]["divisions"], 1.0);
    assert!(
        reply["tallies"][0]["runs"]
            .as_array()
            .unwrap()
            .iter()
            .any(|run| run["firstHour"] == 1 && run["divisions"].is_null())
    );
    let usage = || {
        dashboard_base::prepared::query::observations::usage(
            &fixture.source,
            "queries:volunteers-expeditionaries",
        )
    };
    let count = usage().2;
    assert_eq!(fixture.query("volunteers-expeditionaries", params), reply);
    assert_eq!(usage().2, count);
    assert!(usage().0 <= usage().1);
    let selected = fixture.query(
        "volunteers-expeditionaries",
        json!({"branchId":"1","kind":"land","tags":["unrelated/mod-country"]}),
    );
    assert_eq!(selected["links"], json!([]));
}
