//! Occupied states' served replies: the branch's manifest and its windows.

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
        let manifest = ask.ask("occupied-states-manifest", json!({ "branchId": id }))?;
        let count = manifest.as_ref().map_or(0, |manifest| {
            served::values_at(manifest, &["windows", "*"]).len()
        });
        let mut windows = branch.window_indices();
        windows.extend(served::indices(count));
        windows.sort_unstable();
        windows.dedup();
        for window in windows {
            ask.ask(
                "occupied-states",
                json!({ "branchId": id, "window": window }),
            )?;
        }
    }
    Ok(())
}
