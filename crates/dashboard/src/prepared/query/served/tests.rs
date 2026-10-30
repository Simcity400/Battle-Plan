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
/// The text `served` writes, as long as it says.
fn text(served: &Served) -> String {
    let mut text = String::with_capacity(served.len());
    served.write(&mut text);
    assert_eq!(text.len(), served.len());
    text
}
/// What `answer` serves with `lines`, the same from the answer as from it kept and read back.
fn serve(answer: &Answer, lines: Vec<String>, refresh: fn(&mut Vec<String>)) -> String {
    let fresh = text(&Served::decorate(answer, lines.clone(), refresh));
    let (kept, ()) = Answer::kept(answer.keep(())().unwrap()).unwrap();
    assert_eq!(text(&Served::decorate(&kept, lines, refresh)), fresh);
    fresh
}
fn check(data: Value, product: &str, name: &str) {
    let lines = vec!["presented".into(), "presented".into(), "kept".into()];
    let old = expected(&data, product, name, lines.clone());
    let answer = Answer::new(
        &data,
        content_warning_paths(product, name),
        false,
        &AtomicBool::new(false),
    )
    .unwrap();
    assert_eq!(*answer.text, serde_json::to_string(&data).unwrap());
    assert_eq!(
        serve(&answer, lines, refresh),
        serde_json::to_string(&old).unwrap()
    );
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
fn a_reply_rewrites_only_the_warning_lists_and_copies_the_rest() {
    let data = json!({"warnings":["old"],"rows":(0..100_000).collect::<Vec<_>>()});
    let answer = Answer::new(
        &data,
        content_warning_paths("army-size", "army-size"),
        false,
        &AtomicBool::new(false),
    )
    .unwrap();
    let warnings = answer.text.find("[\"old\"]").unwrap();
    assert_eq!(answer.marks.lists, [(warnings, warnings + 7)]);
    assert_eq!(
        serve(&answer, vec![], |_| {}),
        serde_json::to_string(&data).unwrap()
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
        json!({"a":1,"warnings":["kept"]}),
        json!({"warnings":["kept"],"z":[]}),
        json!({}),
    ] {
        let mut old = data.clone();
        old.as_object_mut()
            .unwrap()
            .insert("served".into(), json!(["fresh"]));
        let answer = Answer::new(&data, &[&["warnings"]], true, &AtomicBool::new(false)).unwrap();
        let (kept, ()) = Answer::kept(answer.keep(())().unwrap()).unwrap();
        for answer in [&answer, &kept] {
            let mut view = Served::decorate(answer, vec![], |_| {});
            view.served = Some(json!(["fresh"]).to_string());
            assert_eq!(text(&view), serde_json::to_string(&old).unwrap());
        }
    }
    // Only an object takes the field.
    let answer = Answer::new(&json!([{"served":1}]), &[], true, &AtomicBool::new(false)).unwrap();
    assert!(answer.marks.served.is_none());
}
/// [`expected`] over `paths` rather than a product's.
fn expected_at(data: &Value, paths: &[&[&str]], lines: Vec<String>) -> Value {
    let mut copied = data.clone();
    for path in paths {
        each_at(&mut copied, path, &mut |value| {
            if let Value::Array(array) = value
                && let Some(mut strings) = array
                    .iter()
                    .map(|line| line.as_str().map(str::to_owned))
                    .collect::<Option<Vec<String>>>()
            {
                refresh(&mut strings);
                *array = strings.into_iter().map(Value::String).collect();
            }
        });
    }
    let mut lines = Some(lines);
    for path in paths {
        each_at(&mut copied, path, &mut |warnings| {
            if let (Value::Array(warnings), Some(lines)) = (warnings, lines.take()) {
                for line in lines {
                    if !warnings.iter().any(|kept| kept.as_str() == Some(&line)) {
                        warnings.push(Value::String(line));
                    }
                }
            }
        });
    }
    copied
}
#[test]
fn a_list_below_a_reached_value_that_is_not_one_is_still_served() {
    let paths: &[&[&str]] = &[&["outer"], &["outer", "warnings"]];
    for data in [
        json!({"outer":{"warnings":["stale: old","kept"]}}),
        json!({"outer":null}),
        json!({"outer":{"warnings":null}}),
    ] {
        let answer = Answer::new(&data, paths, false, &AtomicBool::new(false)).unwrap();
        assert_eq!(
            serve(&answer, vec!["added".into()], refresh),
            serde_json::to_string(&expected_at(&data, paths, vec!["added".into()])).unwrap()
        );
    }
}
#[test]
fn a_key_named_like_the_wildcard_is_only_a_key() {
    let data = json!({"*":{"warnings":["stale: old"]},"items":[{"warnings":["stale: old"]}]});
    let paths: &[&[&str]] = &[&["*", "warnings"], &["items", "*", "warnings"]];
    let answer = Answer::new(&data, paths, false, &AtomicBool::new(false)).unwrap();
    assert_eq!(
        serve(&answer, vec![], refresh),
        serde_json::to_string(
            &json!({"*":{"warnings":["stale: old"]},"items":[{"warnings":[refresh_line()]}]})
        )
        .unwrap()
    );
}
fn refresh_line() -> String {
    let mut lines = Vec::new();
    refresh(&mut lines);
    lines.remove(0)
}
#[test]
fn writing_an_answer_stops_once_its_request_is_cancelled_or_its_text_passes_the_budget() {
    let data = json!({"warnings":[],"rows":[1,2,3]});
    let error = Answer::new(&data, &[&["warnings"]], false, &AtomicBool::new(true))
        .err()
        .unwrap();
    assert_eq!(
        error.to_string(),
        check_cancel(&AtomicBool::new(true)).unwrap_err().to_string()
    );
    let long = json!({"warnings":[],"text":"x".repeat(MAX_REPLY_BYTES)});
    let error = Answer::new(&long, &[&["warnings"]], false, &AtomicBool::new(false))
        .err()
        .unwrap();
    assert!(error.to_string().contains("budget"), "{error}");
    assert_eq!(error.kind(), io::ErrorKind::InvalidData);
}
#[test]
fn kept_answers_whose_marks_do_not_fit_their_text_are_refused() {
    let answer = Answer::new(
        &json!({"warnings":["a"]}),
        &[&["warnings"]],
        false,
        &AtomicBool::new(false),
    )
    .unwrap();
    let json = answer.keep(())().unwrap();
    assert!(Answer::kept::<()>(json.clone()).is_some());
    // Cut short, the text no longer holds the lists its marks name; without its line, no marks.
    let line = json.rfind('\n').unwrap();
    assert!(Answer::kept::<()>(json[line - 3..].to_owned()).is_none());
    assert!(Answer::kept::<()>(json[..line].to_owned()).is_none());
}
