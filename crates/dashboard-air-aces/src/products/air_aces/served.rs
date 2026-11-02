//! Air aces' served replies: each branch's manifest and catalogue, every window, the roster at
//! the branch's middle and last hour, and the portraits of the first aces the manifest names.

use dashboard_base::prepared::served::{self, Ask, Plan};
use serde_json::{Value, json};
use std::io;

pub static PLAN: Plan = Plan {
    queries,
    unordered: &[],
};

fn queries(ask: &mut dyn Ask) -> io::Result<()> {
    for branch in ask.branches().to_vec() {
        let id = branch.name();
        let manifest = ask.ask("air-aces", json!({ "branchId": id, "view": "manifest" }))?;
        ask.ask("air-aces", json!({ "branchId": id, "view": "catalogue" }))?;
        let windows = manifest
            .as_ref()
            .and_then(|reply| reply.get("windows"))
            .and_then(Value::as_array)
            .map_or(branch.windows.len(), Vec::len);
        for window in 0..windows {
            ask.ask(
                "air-aces",
                json!({ "branchId": id, "view": "window", "window": window }),
            )?;
        }
        let middle = branch.middle();
        let mut hours = vec![middle, branch.last_hour];
        hours.dedup();
        for hour in hours {
            ask.ask(
                "air-aces",
                json!({ "branchId": id, "view": "roster", "hour": hour }),
            )?;
        }
        let ids: Vec<String> = manifest
            .as_ref()
            .map(|reply| served::values_at(reply, &["aces", "*", "id"]))
            .unwrap_or_default()
            .into_iter()
            .filter_map(|id| id.as_str().map(str::to_owned))
            .take(served::KEYS)
            .collect();
        if !ids.is_empty() {
            ask.ask(
                "air-ace-portraits",
                json!({ "branchId": id, "hour": middle, "ids": ids }),
            )?;
        }
    }
    Ok(())
}
