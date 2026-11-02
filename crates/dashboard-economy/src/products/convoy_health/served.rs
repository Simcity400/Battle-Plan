//! Convoy health's served replies: some countries' regional efficiency and their trade and
//! lend-lease convoys, on every branch.

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
        let countries = ask.countries(branch.id)?;
        for tag in served::some(&countries) {
            for view in ["regions", "trade"] {
                ask.ask(
                    "convoy-health",
                    json!({ "branchId": id, "view": view, "tag": tag }),
                )?;
            }
        }
    }
    Ok(())
}
