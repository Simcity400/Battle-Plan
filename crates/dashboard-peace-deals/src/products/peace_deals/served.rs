//! Peace deals' served replies: each branch's deals and transfers, as the Diplomacy page's
//! widgets ask for them.

use dashboard_base::prepared::served::{Ask, Plan};
use serde_json::json;
use std::io;

pub static PLAN: Plan = Plan {
    queries,
    unordered: &[],
};

fn queries(ask: &mut dyn Ask) -> io::Result<()> {
    for branch in ask.branches().to_vec() {
        ask.ask(
            "events-page",
            json!({ "branchId": branch.name(), "family": { "kind": "peace-deals" } }),
        )?;
    }
    Ok(())
}
