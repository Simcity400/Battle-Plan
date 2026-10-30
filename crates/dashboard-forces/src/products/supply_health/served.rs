//! Supply health's served replies: every country's runs over each span and
//! at each hour, and navigation to their changes from each hour.

use dashboard_base::prepared::served::{self, Ask, Plan};
use serde_json::json;
use std::io;

pub static PLAN: Plan = Plan {
    queries,
    unordered: &[],
};

/// Pixels a summary draws over.
const WIDTH: u32 = 64;

fn queries(ask: &mut dyn Ask) -> io::Result<()> {
    for branch in ask.branches().to_vec() {
        let id = branch.name();
        let (first, last) = (branch.first_hour, branch.last_hour);
        let countries = ask.countries(branch.id)?;
        let batches = served::batches(&countries);
        let hours = branch.hours();
        for batch in &batches {
            for (from, to) in branch.spans() {
                ask.ask(
                    "supply-health",
                    json!({ "branchId": id, "tags": batch, "from": from, "to": to, "width": WIDTH }),
                )?;
            }
            for &hour in &hours {
                ask.ask(
                    "supply-health",
                    json!({ "branchId": id, "tags": batch, "from": hour, "to": hour }),
                )?;
            }
        }
        let Some(batch) = batches.first() else {
            continue;
        };
        let navigate = |key: &str, from: Option<i32>| {
            json!({
                "branchId": id, "family": { "kind": "supply-health" }, "tags": batch,
                "key": key, "from": from, "firstHour": first, "lastHour": last,
            })
        };
        for &hour in &hours {
            for key in ["ArrowLeft", "ArrowRight"] {
                ask.ask("series-next-change", navigate(key, Some(hour)))?;
            }
        }
        for key in ["Home", "End"] {
            ask.ask("series-next-change", navigate(key, None))?;
        }
    }
    Ok(())
}
