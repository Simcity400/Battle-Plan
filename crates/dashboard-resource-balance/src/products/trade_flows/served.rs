//! Trade flows' served replies, every page of each: every flow's daily amounts, totals and
//! averages charted over each span, in one bin and at a small and a wide chart's width; some
//! countries' flows and the flows between two of them; every flow at each hour; and keyboard
//! navigation from each hour.

use dashboard_base::prepared::served::{self, Ask, Plan};
use serde_json::{Value, json};
use std::io;

pub static PLAN: Plan = Plan {
    queries,
    unordered: &[],
};

/// Pixels a dense summary draws over.
const WIDTH: u32 = 64;
const MEASURES: [&str; 3] = ["current", "total", "average"];

fn queries(ask: &mut dyn Ask) -> io::Result<()> {
    for branch in ask.branches().to_vec() {
        let id = branch.name();
        let (first, last) = (branch.first_hour, branch.last_hour);
        let countries = ask.countries(branch.id)?;
        let some = served::some(&countries);
        let chart =
            |tags: Value, (from, to): (i32, i32), width: u32, measure: &str, between: bool| {
                json!({
                    "branchId": id, "tags": tags, "from": from, "to": to, "width": width,
                    "basis": from, "measure": measure, "between": between, "hours": [to],
                })
            };
        let mut pages = |params: Value| served::pages(ask, "trade-flows", params, "cursor");
        for span in branch.spans() {
            for measure in MEASURES {
                for width in [1, WIDTH, served::WIDE] {
                    pages(chart(Value::Null, span, width, measure, false))?;
                }
            }
            for tag in &some {
                for width in [WIDTH, served::WIDE] {
                    pages(chart(json!([tag]), span, width, "current", false))?;
                }
            }
            if let [seller, buyer, ..] = some.as_slice() {
                let pair = json!([seller, buyer]);
                pages(chart(pair.clone(), span, WIDTH, "total", true))?;
                pages(chart(pair, span, served::WIDE, "current", true))?;
            }
        }
        let navigate = |key: &str, from: Option<i32>| {
            json!({
                "branchId": id, "tags": null, "between": false, "op": "navigate", "key": key,
                "from": from, "firstHour": first, "lastHour": last,
            })
        };
        for hour in branch.hours() {
            let point = json!({
                "branchId": id, "tags": null, "from": hour, "to": hour, "width": 1,
                "basis": first, "measure": "current", "between": false, "hours": [hour, last],
            });
            served::pages(ask, "trade-flows", point, "cursor")?;
            for key in ["ArrowLeft", "ArrowRight"] {
                ask.ask("trade-flows", navigate(key, Some(hour)))?;
            }
        }
        for key in ["Home", "End"] {
            ask.ask("trade-flows", navigate(key, None))?;
        }
    }
    Ok(())
}
