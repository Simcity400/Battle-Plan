//! Trade flows' served replies: every flow's daily amounts, totals and averages charted over
//! each span, in one bin, dense and hour by hour; some countries' flows and the flows between
//! two of them; every flow at each hour; and keyboard navigation from each hour.

use dashboard_base::prepared::served::{self, Ask, Plan};
use serde_json::{Value, json};
use std::io;

pub static PLAN: Plan = Plan {
    queries,
    unordered: &[],
};

/// Pixels a dense summary draws over.
const WIDTH: i64 = 64;
const MEASURES: [&str; 3] = ["current", "total", "average"];

fn queries(ask: &mut dyn Ask) -> io::Result<()> {
    for branch in ask.branches().to_vec() {
        let id = branch.name();
        let (first, last) = (branch.first_hour, branch.last_hour);
        let countries = ask.countries(branch.id)?;
        let some = served::some(&countries);
        let page = |tags: Value, (from, to): (i32, i32), width: i64, measure: &str, between: bool| {
            json!({
                "branchId": id, "tags": tags, "from": from, "to": to, "width": width,
                "basis": from, "measure": measure, "between": between, "hours": [to],
            })
        };
        for span in branch.spans() {
            let hourly = (i64::from(span.1) - i64::from(span.0) + 1).max(1);
            for measure in MEASURES {
                for width in [1, WIDTH, hourly] {
                    ask.ask("trade-flows", page(Value::Null, span, width, measure, false))?;
                }
            }
            for tag in &some {
                for width in [WIDTH, hourly] {
                    ask.ask("trade-flows", page(json!([tag]), span, width, "current", false))?;
                }
            }
            if let [seller, buyer, ..] = some.as_slice() {
                let pair = json!([seller, buyer]);
                ask.ask("trade-flows", page(pair.clone(), span, WIDTH, "total", true))?;
                ask.ask("trade-flows", page(pair, span, hourly, "current", true))?;
            }
        }
        let navigate = |key: &str, from: Option<i32>| {
            json!({
                "branchId": id, "tags": null, "between": false, "op": "navigate", "key": key,
                "from": from, "firstHour": first, "lastHour": last,
            })
        };
        for hour in branch.hours() {
            ask.ask(
                "trade-flows",
                json!({
                    "branchId": id, "tags": null, "from": hour, "to": hour, "width": 1,
                    "basis": first, "measure": "current", "between": false, "hours": [hour, last],
                }),
            )?;
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
