//! The political state history's served replies: each branch's elections and balances, some
//! countries' balances, and its exiles over the plan's spans.

use dashboard_base::prepared::served::{self, Ask, Plan};
use serde_json::json;
use std::io;

pub static PLAN: Plan = Plan {
    queries,
    unordered: &[],
};

fn queries(ask: &mut dyn Ask) -> io::Result<()> {
    for branch in ask.branches().to_vec() {
        let id = branch.name();
        for mode in ["elections", "balances"] {
            ask.ask(super::QUERY, json!({ "branchId": id, "mode": mode }))?;
        }
        let countries = ask.countries(branch.id)?;
        for tag in served::some(&countries) {
            ask.ask(
                super::QUERY,
                json!({ "branchId": id, "mode": "country", "tag": tag }),
            )?;
        }
        for (first, last) in branch.spans() {
            ask.ask(
                super::QUERY,
                json!({ "branchId": id, "mode": "exiles", "firstHour": first, "lastHour": last }),
            )?;
        }
    }
    Ok(())
}
