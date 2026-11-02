use super::*;
use serde_json::json;
use std::path::{Path, PathBuf};

fn ui(path: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../ui/src")
        .join(path)
}

/// Every widget kind the frontend registers, read from its registration list.
fn registered_kinds() -> Vec<String> {
    let text = std::fs::read_to_string(ui("widgets/definitions.ts")).unwrap();
    let mut folders = BTreeMap::new();
    for line in text.lines() {
        let Some(rest) = line.trim().strip_prefix("import { definition as ") else {
            continue;
        };
        let (name, rest) = rest.split_once(" }").unwrap();
        let folder = rest
            .split_once("'./")
            .and_then(|(_, rest)| rest.split_once("/definition.ts'"))
            .map(|(folder, _)| folder)
            .unwrap();
        folders.insert(name.trim().to_owned(), folder.to_owned());
    }
    let list = text
        .split_once("export const widgetMetas = [")
        .and_then(|(_, rest)| rest.split_once(']'))
        .map(|(list, _)| list)
        .unwrap();
    let mut kinds = Vec::new();
    for name in list
        .split(',')
        .map(str::trim)
        .filter(|name| !name.is_empty())
    {
        let folder = &folders[name];
        let definition =
            std::fs::read_to_string(ui(&format!("widgets/{folder}/definition.ts"))).unwrap();
        let kind = definition
            .split_once("export const definition = {")
            .and_then(|(_, rest)| rest.split_once("kind: '"))
            .and_then(|(_, rest)| rest.split_once('\''))
            .map(|(kind, _)| kind.to_owned())
            .unwrap();
        kinds.push(kind);
    }
    assert!(!kinds.is_empty());
    kinds
}

#[test]
fn every_registered_widget_has_exactly_one_table_entry() {
    let table = widgets(WIDGETS).unwrap();
    let listed: std::collections::BTreeSet<_> =
        table.iter().map(|widget| widget.kind.clone()).collect();
    let registered: std::collections::BTreeSet<_> = registered_kinds().into_iter().collect();
    let missing: Vec<_> = registered.difference(&listed).collect();
    let unknown: Vec<_> = listed.difference(&registered).collect();
    assert!(
        missing.is_empty() && unknown.is_empty(),
        "widgets without a readiness entry: {missing:?}; entries for no registered widget: {unknown:?}"
    );
    assert_eq!(table.iter().filter(|widget| widget.map).count(), 1);
}

#[test]
fn table_routes_match_the_frontend_and_registry() {
    let products = std::fs::read_to_string(ui("dashboard/products.ts")).unwrap();
    let series = std::fs::read_to_string(ui("dashboard/preparedSeries.ts")).unwrap();
    let mut frontend = std::collections::BTreeSet::new();
    for piece in products.split("request.product === '").skip(1) {
        frontend.insert(piece.split_once('\'').unwrap().0.to_owned());
    }
    let list = series
        .split_once("const PREPARED_PRODUCTS = [")
        .and_then(|(_, rest)| rest.split_once("] as const"))
        .unwrap()
        .0;
    for piece in list.split('\'').skip(1).step_by(2) {
        frontend.insert(piece.to_owned());
    }
    let ours: std::collections::BTreeSet<_> = UI_PREPARED_ROUTES
        .iter()
        .map(|key| key.to_string())
        .collect();
    assert_eq!(ours, frontend, "prepared routing differs from the frontend");
    for widget in widgets(WIDGETS).unwrap() {
        for template in widget.requests.iter().chain(&widget.auxiliary) {
            if template.route == Route::Product {
                let registered = crate::products::find(&template.product);
                assert!(
                    registered.is_some_and(|product| !product.prepared_only)
                        && !UI_PREPARED_ROUTES.contains(&template.product.as_str()),
                    "{} requests {} as a product load, which the client sends as a prepared query or the service refuses",
                    widget.kind,
                    template.product
                );
            }
        }
    }
}

#[test]
fn templates_bind_recording_facts_and_wait_for_their_windows() {
    let bindings = Bindings {
        branch: "3".into(),
        tag: "opaque tag".into(),
        first: 10,
        last: 100,
        width: 800,
    };
    let params = json!({"branchId":"$branch","tags":"$tags","from":"$first","to":"$last",
        "day":["$lastDayStart","$lastDayEnd","$firstDayEnd"],"window":"$window:0:last","at":"$windowLast:0:first"});
    let mut replies = BTreeMap::new();
    assert_eq!(
        instantiate(&params, &bindings, &replies).unwrap(),
        Instantiated::Waiting
    );
    replies.insert(
        0,
        json!({"windows":[{"firstHour":10,"lastHour":40},{"firstHour":41,"lastHour":100}]}),
    );
    assert_eq!(
        instantiate(&params, &bindings, &replies).unwrap(),
        Instantiated::Ready(
            json!({"branchId":"3","tags":["opaque tag"],"from":10,"to":100,"day":[82,100,33],"window":1,"at":40})
        )
    );
    replies.insert(0, json!({"windows":[]}));
    assert_eq!(
        instantiate(&params, &bindings, &replies).unwrap(),
        Instantiated::Absent
    );
    assert!(instantiate(&json!({"x":"$unknown"}), &bindings, &replies).is_err());
    let hours = json!(["$afterLast", "$firstWeekEnd", "$firstBlockEnd"]);
    assert_eq!(
        instantiate(&hours, &bindings, &replies).unwrap(),
        Instantiated::Ready(json!([101, 177, 100]))
    );
    let long = Bindings {
        first: 2150,
        last: 9000,
        ..bindings
    };
    assert_eq!(
        instantiate(&hours, &long, &replies).unwrap(),
        Instantiated::Ready(json!([9001, 2317, 2159]))
    );
}

#[test]
fn replies_classify_by_their_own_coverage_and_sampling_fields() {
    let summary = json!({"to":29});
    assert_eq!(
        classify("series-summary", &summary, &Value::Null, false),
        Outcome::Pending
    );
    assert_eq!(
        classify("series-summary", &summary, &Value::Null, true),
        Outcome::Unavailable
    );
    let rough = json!({"coverage":{"schema":1,"firstHour":0,"lastHour":1,"complete":false},
        "series":[{"summary":{"approximate":[{"firstHour":2,"lastHour":29}]}}]});
    assert_eq!(
        classify("series-summary", &summary, &rough, false),
        Outcome::Sampled { approximate: true }
    );
    let page = json!({"coverage":{"schema":1,"firstHour":0,"lastHour":1,"complete":false},"total":"0","rows":[]});
    assert_eq!(
        classify("events-page", &json!({"to":29}), &page, false),
        Outcome::Prefix
    );
    assert_eq!(
        classify("events-page", &json!({"to":1}), &page, false),
        Outcome::Exact
    );
    assert_eq!(
        classify("events-page", &json!({}), &page, false),
        Outcome::Prefix
    );
    assert!(Outcome::Prefix.ready());
    // A reply answered from its gate's prefix names the hours still pending;
    // within the prefix, nothing is pending and the reply is exact.
    let prefix = json!({"coverage":{"schema":1,"firstHour":0,"lastHour":1,"complete":false},
        "pending":[{"firstHour":2,"lastHour":29}],"spans":[]});
    assert_eq!(
        classify("idea-history", &json!({}), &prefix, false),
        Outcome::Prefix
    );
    let within = json!({"coverage":{"schema":1,"firstHour":0,"lastHour":1,"complete":false},
        "pending":[],"rows":[]});
    assert_eq!(
        classify("politics-ledger", &json!({"hour":1}), &within, false),
        Outcome::Exact
    );
    // A value still being prepared inside an answered row, with or without
    // prepared coverage at the root.
    for row in [
        json!({"preparing":["completedByHour"]}),
        json!({"reason":"preparing"}),
        json!({"unknown":"preparing"}),
    ] {
        assert_eq!(
            classify(
                "focus-hour",
                &json!({"hour":1}),
                &json!({"rows":[row]}),
                false
            ),
            Outcome::Prefix
        );
    }
    assert_eq!(
        classify(
            "focus-hour",
            &json!({"hour":1}),
            &json!({"rows":[{"preparing":[]}],"token":"preparing"}),
            false
        ),
        Outcome::Exact
    );
    let complete = json!({"coverage":{"schema":1,"firstHour":0,"lastHour":29,"complete":true}});
    assert_eq!(
        classify("events-page", &json!({}), &complete, false),
        Outcome::Exact
    );
    // A product-specific coverage list is not prepared coverage.
    assert_eq!(
        classify("decryption", &json!({}), &json!({"coverage":[]}), false),
        Outcome::Exact
    );
}

/// Requests name their first view's class, which the probe reports beside
/// their first-view and exact times; a class outside the four is refused.
#[test]
fn table_requests_name_their_first_view_class() {
    let table = widgets(WIDGETS).unwrap();
    let class = |kind: &str| -> Vec<Option<FirstView>> {
        table
            .iter()
            .find(|widget| widget.kind == kind)
            .unwrap()
            .requests
            .iter()
            .map(|template| template.first_view)
            .collect()
    };
    assert_eq!(class("casualties"), [Some(FirstView::Prefix)]);
    assert_eq!(class("event-feed"), [Some(FirstView::Prefix)]);
    assert_eq!(
        class("experience"),
        [
            Some(FirstView::Sampled),
            Some(FirstView::Prefix),
            Some(FirstView::Prefix),
            Some(FirstView::Prefix),
            Some(FirstView::Sampled),
            Some(FirstView::Sampled),
            Some(FirstView::Sampled),
        ]
    );
    let named = |class: &str| {
        widgets(&format!(
            r#"{{"widgets": [{{"kind": "x", "requests": [{{"product": "wars", "route": "prepared", "firstView": "{class}", "params": {{}}}}]}}]}}"#
        ))
        .map(|table| table[0].requests[0].first_view.map(FirstView::name))
    };
    for class in ["a", "b", "c", "d"] {
        assert_eq!(named(class), Ok(Some(class)));
    }
    assert!(named("e").is_err());
    assert!(Outcome::Exact.complete() && Outcome::Unavailable.complete());
    assert!(!Outcome::Prefix.complete() && Outcome::Prefix.settled());
    assert!(!Outcome::Sampled { approximate: false }.complete());
}

#[test]
fn a_proven_newest_detail_is_exact_without_whole_branch_preparation() {
    let data = serde_json::json!({"coverage":{"firstHour":100,"lastHour":99,"complete":false},"pending":[],"exact":100,"warningsComplete":false,
        "interval":{"firstHour":100,"lastHour":123},"rows":[]});
    assert_eq!(
        classify(
            "division-log",
            &serde_json::json!({"branchId":"1","window":3}),
            &data,
            false
        ),
        Outcome::Exact
    );
}

#[test]
fn experience_templates_send_native_holder_keys_from_the_groups_reply() {
    let widget = widgets(WIDGETS)
        .unwrap()
        .into_iter()
        .find(|widget| widget.kind == "experience")
        .unwrap();
    let bindings = Bindings {
        branch: "1".into(),
        tag: "opaque tag".into(),
        first: 10,
        last: 30,
        width: 20,
    };
    let key = serde_json::to_string(&("opaque tag", 7, 9001)).unwrap();
    let replies = BTreeMap::from([(
        0,
        json!({"countryIdentities":[
            {"tag":"opaque tag", "identities":[{"firstHour":10,"lastHour":30,"slot":7,"countryId":9001}]},
            {"tag":"other", "identities":[{"firstHour":10,"lastHour":30,"slot":8,"countryId":9002}]}
        ]}),
    )]);
    for template in &widget.requests[1..] {
        assert_eq!(waits_for(template), vec![0]);
        let Instantiated::Ready(params) =
            instantiate(&template.params, &bindings, &replies).unwrap()
        else {
            panic!("groups are ready")
        };
        if template.product == "series-state" {
            assert_eq!(params["reads"], json!([{"tag": key, "hour": 10}]));
        } else {
            assert_eq!(params["tags"], json!([key]));
        }
    }
    // Each experience channel reads its reached state at the opening hour, as the widget does.
    let measures = |product: &str| -> Vec<Value> {
        widget
            .requests
            .iter()
            .filter(|template| template.product == product)
            .map(|template| template.params["family"]["measure"].clone())
            .collect()
    };
    let channels = json!(["army-banked", "navy-banked", "air-banked"]);
    assert_eq!(Value::Array(measures("series-state")), channels);
    assert_eq!(Value::Array(measures("series-summary")), channels);
    let at_end = json!({"reads": "$experienceReads:0:last"});
    assert_eq!(
        instantiate(&at_end, &bindings, &replies).unwrap(),
        Instantiated::Ready(json!({"reads": [{"tag": key, "hour": 30}]}))
    );
    assert!(instantiate(&json!({"reads": "$experienceReads:0"}), &bindings, &replies).is_err());
}

#[test]
fn the_air_timeline_reads_the_day_its_first_reply_names() {
    let widget = widgets(WIDGETS)
        .unwrap()
        .into_iter()
        .find(|widget| widget.kind == "air-superiority-timeline")
        .unwrap();
    let bindings = Bindings {
        branch: "1".into(),
        tag: "opaque tag".into(),
        first: 30,
        last: 100,
        width: 20,
    };
    let views: Vec<_> = widget
        .requests
        .iter()
        .map(|template| template.params["view"].clone())
        .collect();
    assert_eq!(
        views,
        [json!("choices"), json!("timeline"), json!("moment")]
    );
    let day = &widget.requests[2];
    assert_eq!(waits_for(day), vec![1, 1]);
    let mut replies = BTreeMap::new();
    assert_eq!(
        instantiate(&day.params, &bindings, &replies).unwrap(),
        Instantiated::Waiting
    );
    replies.insert(
        1,
        json!({"dayHours": 24, "rows": [{"region": 9}, {"region": 4}]}),
    );
    assert_eq!(
        instantiate(&day.params, &bindings, &replies).unwrap(),
        Instantiated::Ready(json!({"branchId": "1", "view": "moment", "firstHour": 30,
            "lastHour": 47, "mode": "sides", "regions": [9, 4]}))
    );
    let short = Bindings {
        last: 40,
        ..bindings.clone()
    };
    assert_eq!(
        instantiate(&day.params, &short, &replies).unwrap(),
        Instantiated::Ready(json!({"branchId": "1", "view": "moment", "firstHour": 30,
            "lastHour": 40, "mode": "sides", "regions": [9, 4]}))
    );
    replies.insert(1, json!({"rows": []}));
    assert_eq!(
        instantiate(&day.params, &bindings, &replies).unwrap(),
        Instantiated::Absent
    );
}

#[test]
fn battle_first_views_use_forward_prefixes_and_keep_the_tail_pending() {
    let widgets = widgets(WIDGETS).unwrap();
    for kind in ["land-battles", "naval-battles"] {
        let widget = widgets.iter().find(|widget| widget.kind == kind).unwrap();
        assert!(
            widget
                .requests
                .iter()
                .all(|request| request.first_view == Some(FirstView::Prefix))
        );
        let params = serde_json::json!({"branchId":"1", "window":3});
        let data = serde_json::json!({"coverage":{"firstHour":100,"lastHour":123,"complete":false},
            "pending":[{"firstHour":124,"lastHour":199}],"rows":[],"observed":[]});
        assert_eq!(classify(kind, &params, &data, false), Outcome::Prefix);
        assert_eq!(
            classify(kind, &params, &Value::Null, false),
            Outcome::Pending
        );
    }
}
