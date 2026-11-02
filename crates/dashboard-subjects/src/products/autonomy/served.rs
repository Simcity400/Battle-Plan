//! Subject autonomy's served replies, as the Autonomy widget asks them: the whole branch's
//! subjects, some subjects' charts over each span, dense and hour by hour, two subjects joined as
//! one series, every subject's values at each hour, and navigation from each hour.

use dashboard_base::prepared::served::{self, Ask, Plan};
use serde_json::{Map, Value, json};
use std::io;

pub static PLAN: Plan = Plan {
    queries,
    unordered: &[],
};

/// Pixels a dense summary draws over.
const WIDTH: u32 = 64;

/// `request` with the widget's selection of every subject on `branch`.
fn asked(branch: &str, request: Value) -> Value {
    let mut asked = json!({
        "branchId": branch, "family": { "kind": "autonomy" }, "overlords": [], "subjects": [],
    });
    if let (Some(asked), Value::Object(request)) = (asked.as_object_mut(), request) {
        asked.extend(request);
    }
    asked
}

/// Each of `tags` as a series of its own over `(first, last)`.
fn series(tags: &[String], (first, last): (i32, i32)) -> Value {
    let mut within = Map::new();
    for tag in tags {
        within.insert(tag.clone(), json!([[tag, first, last, true]]));
    }
    Value::Object(within)
}

fn queries(ask: &mut dyn Ask) -> io::Result<()> {
    for branch in ask.branches().to_vec() {
        let id = branch.name();
        let (first, last) = (branch.first_hour, branch.last_hour);
        let whole = ask.ask(
            "series-summary",
            asked(
                &id,
                json!({ "from": first, "to": last, "width": WIDTH, "within": {} }),
            ),
        )?;
        let tags = whole
            .map(|reply| served::texts_at(&reply, &["bands", "*", "tag"]))
            .unwrap_or_default();
        let some = served::some(&tags);
        let hourly = |(from, to): (i32, i32)| i64::from(to) - i64::from(from) + 1;
        for span in branch.spans() {
            for width in [i64::from(WIDTH), hourly(span).max(1)] {
                ask.ask(
                    "series-summary",
                    asked(
                        &id,
                        json!({
                            "from": span.0, "to": span.1, "width": width,
                            "within": series(&some, span),
                        }),
                    ),
                )?;
            }
        }
        if let [early, late, ..] = some.as_slice() {
            let middle = branch.middle();
            let joined = json!({
                "lineage": [[early, first, middle, true], [late, middle.saturating_add(1), last, false]],
            });
            for width in [i64::from(WIDTH), hourly((first, last)).max(1)] {
                ask.ask(
                    "series-summary",
                    asked(
                        &id,
                        json!({ "from": first, "to": last, "width": width, "within": joined }),
                    ),
                )?;
            }
        }
        let reads: Vec<Value> = branch
            .hours()
            .into_iter()
            .map(|hour| json!({ "hour": hour }))
            .collect();
        ask.ask("series-state", asked(&id, json!({ "reads": reads })))?;
        for hour in branch.hours() {
            ask.ask(
                "series-state",
                asked(&id, json!({ "reads": [{ "hour": hour }] })),
            )?;
            for key in ["ArrowLeft", "ArrowRight"] {
                for tag in &some {
                    ask.ask(
                        "series-next-change",
                        asked(
                            &id,
                            json!({
                                "tag": tag, "within": series(std::slice::from_ref(tag), (first, last)),
                                "key": key, "from": hour, "firstHour": first, "lastHour": last,
                            }),
                        ),
                    )?;
                }
            }
        }
    }
    Ok(())
}
