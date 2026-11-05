//! War casualties' served replies: each measure's ranked and some countries' navigation from each
//! hour; the ranked and some countries' states at each served hour; their observations' pages;
//! and the pair and war breakdown manifests and windows, for every country and some. The widget
//! draws no chart, so none is asked.

use dashboard_base::prepared::served::{self, Ask, Plan};
use serde_json::{Value, json};
use std::io;

pub static PLAN: Plan = Plan {
    queries,
    unordered: &[],
};

/// The countries a ranked selection names.
const TOP: u32 = 10;
/// Observations an events page holds.
const PAGE: u32 = 64;
const MEASURES: [&str; 3] = ["dealt", "taken", "unknown"];

fn family(measure: &str, top: Option<u32>) -> Value {
    json!({
        "kind": "war-casualties",
        "measure": measure,
        "top": top.map(|count| json!({ "count": count })),
    })
}

fn queries(ask: &mut dyn Ask) -> io::Result<()> {
    for branch in ask.branches().to_vec() {
        let id = branch.name();
        let (first, last) = (branch.first_hour, branch.last_hour);
        let countries = ask.countries(branch.id)?;
        let some = served::some(&countries);
        let none: &[String] = &[];
        for measure in MEASURES {
            let ranked = family(measure, Some(TOP));
            let listed = family(measure, None);
            let navigate = |family: &Value, tags: &[String], key: &str, from: Option<i32>| {
                json!({
                    "branchId": id, "family": family, "tags": tags, "key": key, "from": from,
                    "firstHour": first, "lastHour": last,
                })
            };
            for hour in branch.hours() {
                for key in ["ArrowLeft", "ArrowRight"] {
                    ask.ask(
                        "series-next-change",
                        navigate(&ranked, none, key, Some(hour)),
                    )?;
                    ask.ask(
                        "series-next-change",
                        navigate(&listed, &some, key, Some(hour)),
                    )?;
                }
            }
            for key in ["Home", "End"] {
                ask.ask("series-next-change", navigate(&ranked, none, key, None))?;
            }
        }
        let ranked = family("taken", Some(TOP));
        let listed = family("taken", None);
        for hour in branch.hours() {
            ask.ask(
                "series-state",
                json!({
                    "branchId": id, "tags": none, "family": ranked,
                    "reads": [{ "tag": "", "hour": hour }],
                }),
            )?;
            let reads: Vec<Value> = some
                .iter()
                .map(|tag| json!({ "tag": tag, "hour": hour }))
                .collect();
            ask.ask(
                "series-state",
                json!({ "branchId": id, "tags": some, "family": listed, "reads": reads }),
            )?;
        }
        served::pages(
            ask,
            "events-page",
            json!({
                "branchId": id, "tags": some, "family": listed, "from": first, "to": last,
                "pageSize": PAGE,
            }),
            "cursor",
        )?;
        let windows = std::iter::once(None).chain(branch.window_indices().into_iter().map(Some));
        for window in windows {
            for (tags, between) in [(none, false), (&some[..], false), (&some[..], true)] {
                ask.ask(
                    "casualty-pairs",
                    json!({ "branchId": id, "tags": tags, "between": between, "window": window }),
                )?;
            }
            for tag in &some {
                ask.ask(
                    "casualty-breakdown",
                    json!({ "branchId": id, "tag": tag, "window": window }),
                )?;
            }
        }
    }
    Ok(())
}
