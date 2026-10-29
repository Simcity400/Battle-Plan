use super::*;
use serde_json::json;

fn refresh(lines: &mut Vec<String>) {
    lines.retain(|line| !line.starts_with("stale:"));
    lines.push("live: \"quoted\" \u{96ea}\u{1f680}".into());
}
fn expected(data: &Value, product: &str, name: &str, lines: Vec<String>) -> Value {
    let mut copied = data.clone();
    for path in content_warning_paths(product, name) {
        each_at(&mut copied, path, &mut |value| {
            let Value::Array(array) = value else {
                return;
            };
            let mut strings: Vec<String> = array
                .iter()
                .filter_map(|line| line.as_str().map(str::to_owned))
                .collect();
            if strings.len() != array.len() {
                return;
            }
            refresh(&mut strings);
            *array = strings.into_iter().map(Value::String).collect();
        });
    }
    append_lines(product, name, lines, &mut copied);
    copied
}
fn check(data: Value, product: &str, name: &str) {
    let before = serde_json::to_vec(&data).unwrap();
    let lines = vec!["presented".into(), "presented".into(), "kept".into()];
    let old = expected(&data, product, name, lines.clone());
    let borrowed = Served::decorate(&data, content_warning_paths(product, name), lines, refresh);
    assert_eq!(
        serde_json::to_vec(&borrowed).unwrap(),
        serde_json::to_vec(&old).unwrap()
    );
    assert_eq!(serde_json::to_vec(&data).unwrap(), before);
}
#[test]
fn borrowed_warning_overlays_match_the_independent_mutating_pipeline_for_every_path_shape() {
    let warnings = json!(["kept", "kept", "stale: previous", "\"\\\n\u{96ea}\u{1f680}"]);
    for (product, name, data) in [
        (
            "land-battles",
            "land-battles",
            json!({"contentWarnings":warnings,"warnings":["battle"]}),
        ),
        (
            "air-aces",
            "air-aces",
            json!({"warnings":warnings,"manifest":{"warnings":warnings},"roster":{"warnings":warnings}}),
        ),
        (
            "ships-sunk-manifest",
            "ships-sunk-manifest",
            json!({"contentWarnings":warnings,"classes":{"warnings":warnings}}),
        ),
        (
            "party-history",
            "party-history",
            json!({"reply":{"warnings":warnings},"warnings":["unrelated"]}),
        ),
        (
            "series-state",
            "game-score",
            json!({"reply":{"warnings":warnings}}),
        ),
        (
            "scoreboard",
            "scoreboard",
            json!({"data":{"warnings":warnings}}),
        ),
        (
            "series-summary",
            "country-fuel",
            json!({"replies":[{"warnings":warnings},{"warnings":warnings}]}),
        ),
        (
            "series-summary",
            "naval-losses",
            json!({"warnings":warnings,"sampled":{"warnings":warnings}}),
        ),
        (
            "series-summary",
            "country-measure",
            json!({"warnings":warnings,"history":[{"missing":null,"number":-0.0,"min":i64::MIN,"max":u64::MAX}]}),
        ),
        (
            "series-state",
            "country-measure",
            json!([{"warnings":warnings},{"warnings":warnings}]),
        ),
    ] {
        check(data, product, name);
    }
}
#[test]
fn non_string_warnings_and_missing_arrays_keep_the_first_list_semantics() {
    for data in [
        Value::Null,
        json!({}),
        json!({"warnings":null}),
        json!({"warnings":"bad"}),
        json!({"warnings":[1,null,"kept"]}),
        json!({"warnings":null,"manifest":{"warnings":["kept"]}}),
        json!({"warnings":"bad","roster":{"warnings":[]}}),
        json!({"replies":[{"warnings":null},{"warnings":["kept"]}]}),
        json!({"manifest":{"warnings":["kept"]},"roster":{"warnings":["stale: old"]}}),
        json!([{}, {"warnings":[1]}, {"warnings":[]}]),
    ] {
        check(data.clone(), "air-aces", "air-aces");
        check(data.clone(), "series-state", "country-measure");
        check(data, "series-summary", "country-fuel");
    }
}
#[test]
fn unchanged_large_subtrees_are_borrowed_without_overlay_descendants() {
    let data = json!({"warnings":["old"],"rows":(0..100_000).collect::<Vec<_>>()});
    let view = Served::decorate(
        &data,
        content_warning_paths("army-size", "army-size"),
        vec![],
        |_| {},
    );
    assert_eq!(view.patches.len(), 2);
    assert!(!view.patches.contains_key(&(&data["rows"] as *const Value)));
    assert_eq!(
        serde_json::to_vec(&view).unwrap(),
        serde_json::to_vec(&data).unwrap()
    );
}
#[test]
fn fifteen_hundred_country_warning_lists_keep_their_order_and_only_first_served_lines() {
    let data = json!({"replies":(0..1500).map(|n|json!({"tag":format!("mod \u{96ea} / {n}"),"warnings":["kept","stale: previous"],"runs":[[n,null]]})).collect::<Vec<_>>()});
    check(data, "series-summary", "country-fuel");
}
#[test]
fn served_field_inserts_or_replaces_only_at_root_in_value_key_order() {
    for data in [
        json!({"a":1,"z":2}),
        json!({"a":1,"served":["old"],"z":2}),
        json!({"a":{"served":"nested"},"warnings":[],"servedAfter":1}),
        json!({}),
    ] {
        let mut view = Served::decorate(&data, &[], vec![], |_| {});
        view.served = Some(json!(["fresh"]));
        view.patches.insert(&data, None);
        let mut old = data.clone();
        old.as_object_mut()
            .unwrap()
            .insert("served".into(), json!(["fresh"]));
        assert_eq!(
            serde_json::to_vec(&view).unwrap(),
            serde_json::to_vec(&old).unwrap()
        );
    }
}
#[test]
fn cancellation_and_transport_failure_propagate_like_value_serialization() {
    let data = json!({"warnings":[],"rows":[null,1,2]});
    let view = Served::decorate(&data, &[&["warnings"]], vec![], |_| {});
    let flag = AtomicBool::new(true);
    assert!(
        bounded_json(&view, &flag)
            .unwrap_err()
            .contains("cancelled")
    );
    struct Broken;
    impl std::io::Write for Broken {
        fn write(&mut self, _: &[u8]) -> io::Result<usize> {
            Err(io::ErrorKind::BrokenPipe.into())
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }
    assert!(serde_json::to_writer(Broken, &view).unwrap_err().is_io());
}
