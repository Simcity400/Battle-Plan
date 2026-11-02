//! Country factories' served replies: the ranked and every country's charts over each span,
//! dense and hour by hour, under two counts, some countries' values at each hour, and
//! navigation from each hour.

use crate::prepared::served::{self, Ask, Plan};
use serde_json::{Value, json};
use std::io;

pub static PLAN: Plan = Plan {
    queries,
    unordered: &[],
};

/// The countries a ranked selection names.
const TOP: u32 = 10;
/// Pixels a dense summary draws over.
const WIDTH: u32 = 64;

/// A family under `mode` over `groups`, ranked when `top`.
fn family(top: bool, mode: &str, groups: &[&str]) -> Value {
    json!({
        "kind": "country-factories",
        "top": top.then(|| json!({ "count": TOP, "mode": mode, "groups": groups })),
        "mode": mode,
        "groups": groups,
    })
}

fn queries(ask: &mut dyn Ask) -> io::Result<()> {
    for branch in ask.branches().to_vec() {
        let id = branch.name();
        let (first, last) = (branch.first_hour, branch.last_hour);
        let countries = ask.countries(branch.id)?;
        let batches = served::batches(&countries);
        let summary = |family: &Value, tags: &[String], (from, to): (i32, i32), width: i64| {
            json!({
                "branchId": id, "family": family, "tags": tags, "from": from, "to": to,
                "width": width,
            })
        };
        let counts = [("free", &[][..]), ("built", &["military", "naval"][..])];
        let none: &[String] = &[];
        let hourly = |(from, to): (i32, i32)| i64::from(to) - i64::from(from) + 1;
        for span in branch.spans() {
            for width in [i64::from(WIDTH), hourly(span).max(1)] {
                for (mode, groups) in counts {
                    let ranked = family(true, mode, groups);
                    ask.ask("series-summary", summary(&ranked, none, span, width))?;
                    let listed = family(false, mode, groups);
                    for batch in &batches {
                        ask.ask("series-summary", summary(&listed, batch.as_slice(), span, width))?;
                    }
                }
            }
        }
        let listed = family(false, "topBar", &["civilian"]);
        for tag in served::some(&countries) {
            let reads: Vec<Value> = branch
                .hours()
                .into_iter()
                .map(|hour| json!({ "tag": tag, "hour": hour }))
                .collect();
            ask.ask(
                "series-state",
                json!({ "branchId": id, "family": listed, "reads": reads }),
            )?;
        }
        let navigate = |family: &Value, tags: &[String], key: &str, from: Option<i32>| {
            json!({
                "branchId": id, "family": family, "tags": tags, "key": key, "from": from,
                "firstHour": first, "lastHour": last,
            })
        };
        let ranked = family(true, "free", &[]);
        let listed = family(false, "free", &[]);
        for hour in branch.hours() {
            for key in ["ArrowLeft", "ArrowRight"] {
                ask.ask("series-next-change", navigate(&ranked, none, key, Some(hour)))?;
                for batch in &batches {
                    ask.ask(
                        "series-next-change",
                        navigate(&listed, batch.as_slice(), key, Some(hour)),
                    )?;
                }
            }
        }
        for key in ["Home", "End"] {
            ask.ask("series-next-change", navigate(&ranked, none, key, None))?;
        }
    }
    Ok(())
}
