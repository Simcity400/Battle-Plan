//! Country nukes' served replies: the ranked and every country's charts over each span, dense
//! and hour by hour, some countries' stockpiles at each hour and their drills, and navigation
//! from each hour.

use dashboard_base::prepared::served::{self, Ask, Plan};
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

fn family(top: Option<u32>, origin: Option<i32>) -> Value {
    json!({ "kind": "country-nukes", "top": top, "originHour": origin })
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
        let ranked = family(Some(TOP), None);
        let listed = family(None, None);
        let none: &[String] = &[];
        let hourly = |(from, to): (i32, i32)| i64::from(to) - i64::from(from) + 1;
        for span in branch.spans() {
            for width in [i64::from(WIDTH), hourly(span).max(1)] {
                ask.ask("series-summary", summary(&ranked, none, span, width))?;
                for batch in &batches {
                    ask.ask("series-summary", summary(&listed, batch.as_slice(), span, width))?;
                }
            }
        }
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
            for origin in [first, branch.middle(), last] {
                let drill = family(None, Some(origin));
                let tags = std::slice::from_ref(&tag);
                ask.ask(
                    "series-summary",
                    summary(&drill, tags, (first, last), i64::from(WIDTH)),
                )?;
            }
        }
        let navigate = |family: &Value, tags: &[String], key: &str, from: Option<i32>| {
            json!({
                "branchId": id, "family": family, "tags": tags, "key": key, "from": from,
                "firstHour": first, "lastHour": last,
            })
        };
        for hour in branch.hours() {
            for key in ["ArrowLeft", "ArrowRight"] {
                ask.ask("series-next-change", navigate(&ranked, none, key, Some(hour)))?;
                for batch in &batches {
                    ask.ask("series-next-change", navigate(&listed, batch.as_slice(), key, Some(hour)))?;
                }
            }
        }
        for key in ["Home", "End"] {
            ask.ask("series-next-change", navigate(&ranked, none, key, None))?;
        }
    }
    Ok(())
}
