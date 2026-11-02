//! Party history's served replies: some countries' charts over each span, dense and hour by
//! hour, their states at each hour, their mark pages, navigation from each hour and their whole
//! histories.

use dashboard_base::prepared::served::{self, Ask, Plan};
use serde_json::{Value, json};
use std::io;

pub static PLAN: Plan = Plan {
    queries,
    unordered: &[],
};

/// Pixels a dense summary draws over.
const WIDTH: u32 = 64;
/// Marks one page holds, as the widget asks them.
const PAGE: u32 = 128;

fn queries(ask: &mut dyn Ask) -> io::Result<()> {
    for branch in ask.branches().to_vec() {
        let id = branch.name();
        let (first, last) = (branch.first_hour, branch.last_hour);
        let countries = ask.countries(branch.id)?;
        let hourly = |(from, to): (i32, i32)| i64::from(to) - i64::from(from) + 1;
        let params = |tag: &str, mode: &str, extra: Value| {
            let mut params = json!({ "branchId": id, "tag": tag, "mode": mode });
            if let (Some(params), Value::Object(extra)) = (params.as_object_mut(), extra) {
                params.extend(extra);
            }
            params
        };
        for tag in served::some(&countries) {
            for span in branch.spans() {
                for width in [i64::from(WIDTH), hourly(span).max(1)] {
                    let extra = json!({ "from": span.0, "to": span.1, "width": width });
                    ask.ask("party-history", params(&tag, "summary", extra))?;
                }
            }
            let hours = branch.hours();
            ask.ask(
                "party-history",
                params(&tag, "state", json!({ "hours": hours })),
            )?;
            for field in ["all", "ruling", "leader", "election"] {
                let extra = json!({ "field": field, "from": first, "to": last, "pageSize": PAGE });
                served::pages(ask, "party-history", params(&tag, "events", extra), "cursor")?;
            }
            for &hour in &hours {
                for key in ["ArrowLeft", "ArrowRight"] {
                    let navigation = json!({
                        "key": key, "from": hour, "firstHour": first, "lastHour": last,
                    });
                    let extra = json!({ "navigation": navigation });
                    ask.ask("party-history", params(&tag, "next", extra))?;
                }
            }
            for key in ["Home", "End"] {
                let navigation = json!({
                    "key": key, "from": null, "firstHour": first, "lastHour": last,
                });
                ask.ask(
                    "party-history",
                    params(&tag, "next", json!({ "navigation": navigation })),
                )?;
            }
            ask.ask("party-history", params(&tag, "history", json!({})))?;
        }
    }
    Ok(())
}
