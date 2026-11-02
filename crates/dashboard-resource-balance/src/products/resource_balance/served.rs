//! Resource balance's served replies: some countries' balances at each branch hour as the
//! widget's point view asks them, their charts over each span, dense and hour by hour, with and
//! without a membership, and navigation from each hour.

use dashboard_base::prepared::served::{self, Ask, Plan};
use serde_json::json;
use std::io;

pub static PLAN: Plan = Plan {
    queries,
    unordered: &[],
};

/// Pixels a dense summary draws over.
const WIDTH: i64 = 64;

fn queries(ask: &mut dyn Ask) -> io::Result<()> {
    for branch in ask.branches().to_vec() {
        let id = branch.name();
        let (first, last) = (branch.first_hour, branch.last_hour);
        let countries = ask.countries(branch.id)?;
        let within = json!([{ "firstHour": first, "lastHour": branch.middle() }]);
        for tag in served::some(&countries) {
            for hour in branch.hours() {
                ask.ask(
                    "resource-balance",
                    json!({ "branchId": id, "tag": tag, "from": hour, "to": hour, "hours": [hour] }),
                )?;
            }
            for (from, to) in branch.spans() {
                let hourly = (i64::from(to) - i64::from(from) + 1).max(1);
                for width in [WIDTH, hourly] {
                    ask.ask(
                        "resource-balance",
                        json!({
                            "branchId": id, "tag": tag, "from": from, "to": to, "width": width,
                            "hours": [to],
                        }),
                    )?;
                }
            }
            ask.ask(
                "resource-balance",
                json!({
                    "branchId": id, "tag": tag, "from": first, "to": last, "width": WIDTH,
                    "hours": [last], "within": within,
                }),
            )?;
            for hour in branch.hours() {
                for key in ["ArrowLeft", "ArrowRight"] {
                    ask.ask(
                        "resource-balance",
                        json!({
                            "branchId": id, "tag": tag, "op": "navigate", "resources": [],
                            "key": key, "from": hour, "firstHour": first, "lastHour": last,
                        }),
                    )?;
                }
            }
            for key in ["Home", "End"] {
                ask.ask(
                    "resource-balance",
                    json!({
                        "branchId": id, "tag": tag, "op": "navigate", "resources": [],
                        "within": within, "key": key, "from": null, "firstHour": first,
                        "lastHour": last,
                    }),
                )?;
            }
        }
    }
    Ok(())
}
