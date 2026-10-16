use super::*;
use crate::test_fixtures::{CAMPAIGN, continuation_records, records, source as frame};
use replay::container::Replay;
use serde_json::json;
use std::{
    path::{Path, PathBuf},
    sync::{Arc, atomic::AtomicBool},
    time::Duration,
};

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
        Outcome::Partial
    );
    assert_eq!(
        classify("events-page", &json!({"to":1}), &page, false),
        Outcome::Exact
    );
    assert_eq!(
        classify("events-page", &json!({}), &page, false),
        Outcome::Partial
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

fn recording() -> Replay {
    let power = |hour: i32| Some(i64::from(hour % 5));
    let mut replay = Replay::from_records(
        CAMPAIGN.into(),
        &[records(
            1,
            vec![frame(1, 0, power(0)), frame(1, 1, power(1))],
        )],
    )
    .unwrap();
    for block in 1..15 {
        let hours = [block * 2, block * 2 + 1];
        replay = replay
            .append(&[continuation_records(
                &replay,
                1,
                hours
                    .iter()
                    .map(|&hour| frame(1, hour, power(hour)))
                    .collect(),
            )])
            .unwrap();
    }
    replay
}

#[test]
fn coverage_gated_replies_wait_for_preparation_and_sampled_ones_do_not() {
    let replay = recording();
    assert_eq!(replay.index.len(), 15);
    let root = std::env::temp_dir().join(format!("replay-readiness-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir(&root).unwrap();
    let path = root.join("campaign.battleplan");
    std::fs::write(&path, replay.bytes()).unwrap();
    let source = Arc::new(
        crate::Source::open(
            crate::SourceKey {
                source_id: "readiness".into(),
                revision: crate::protocol::replay_revision(&path).unwrap(),
            },
            path,
            &AtomicBool::new(false),
        )
        .unwrap(),
    );
    let flag = AtomicBool::new(false);
    let ask = |product: &str, params: Value| {
        let request = crate::transport::ProductRequest {
            source_id: source.key().source_id.clone(),
            revision: source.key().revision.clone(),
            product: product.into(),
            params: params.clone(),
        };
        let reply: Value = serde_json::from_str(
            &crate::prepared::query::query(&source, &request, None, &flag).unwrap(),
        )
        .unwrap();
        let finished = crate::prepared::pipeline::coverage(&source, None, 1, &flag)
            .unwrap()
            .is_some_and(|coverage| coverage.complete);
        classify(product, &params, &reply["data"], finished)
    };
    let family = json!({"kind":"country-measure","measure":"politicalPower"});
    let sampled = || {
        ask(
            "series-summary",
            json!({"branchId":"1","family":family,"tags":["GER"],"from":0,"to":29,"width":30}),
        )
    };
    let gated = || ask("country-groups", json!({"branchId":"1"}));
    let events = || {
        ask(
            "events-page",
            json!({"branchId":"1","family":family,"tags":["GER"],"to":29,"pageSize":20}),
        )
    };
    // Nothing prepared: the sampled summary already answers the whole branch,
    // while the event list holds no accepted events yet.
    assert_eq!(sampled(), Outcome::Sampled { approximate: true });
    assert_eq!(gated(), Outcome::Pending);
    assert_eq!(events(), Outcome::Partial);
    // One accepted block of fifteen.
    let (prefix, _) =
        crate::prepared::pipeline::run_measured(&source, None, &flag, true, Duration::ZERO, |_| {})
            .unwrap();
    assert_eq!(prefix.next_block, 1);
    assert!(sampled().ready());
    assert_eq!(gated(), Outcome::Pending);
    assert_eq!(events(), Outcome::Partial);
    assert!(!events().ready());
    crate::prepared::pipeline::run(&source, None, &flag, |_| {}).unwrap();
    assert_eq!(sampled(), Outcome::Exact);
    assert_eq!(gated(), Outcome::Exact);
    assert_eq!(events(), Outcome::Exact);
    drop(source);
    let _ = std::fs::remove_dir_all(root);
}
