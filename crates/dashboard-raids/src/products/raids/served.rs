//! Raids' served replies: the catalogue, the log, every country's tallies, the top ranked by
//! each metric, and some countries' tallies by name.

use dashboard_base::prepared::served::{self, Ask, Plan};
use serde_json::json;
use std::io;

pub static PLAN: Plan = Plan {
    queries,
    unordered: &[],
};

/// The countries a ranked selection names.
const TOP: u32 = 10;

fn queries(ask: &mut dyn Ask) -> io::Result<()> {
    for branch in ask.branches().to_vec() {
        let id = branch.name();
        let countries = ask.countries(branch.id)?;
        let none: &[String] = &[];
        for view in ["catalogue", "log", "tally"] {
            ask.ask(
                "raids",
                json!({ "branchId": id, "view": view, "tags": none, "top": null }),
            )?;
        }
        for metric in ["launched", "succeeded", "both"] {
            ask.ask(
                "raids",
                json!({
                    "branchId": id, "view": "tally", "tags": none,
                    "top": { "count": TOP, "metric": metric },
                }),
            )?;
        }
        ask.ask(
            "raids",
            json!({ "branchId": id, "view": "tally", "tags": served::some(&countries), "top": null }),
        )?;
    }
    Ok(())
}
