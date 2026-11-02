//! Construction's served replies: each branch's manifest, some of its windows of the log, and
//! every country's spending at the branch's first, middle and last hours, some countries' at
//! each hour the check picks.

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
        ask.ask("construction-log-manifest", json!({ "branchId": id }))?;
        for window in served::indices(branch.windows.len()) {
            ask.ask(
                "construction-log",
                json!({ "branchId": id, "window": window }),
            )?;
        }
        let countries = ask.countries(branch.id)?;
        let ends = [branch.first_hour, branch.middle(), branch.last_hour];
        for tag in &countries {
            ask.ask(
                "construction-spending",
                json!({ "branchId": id, "tag": tag, "hours": ends }),
            )?;
        }
        let hours = branch.hours();
        for tag in served::some(&countries) {
            ask.ask(
                "construction-spending",
                json!({ "branchId": id, "tag": tag, "hours": hours }),
            )?;
        }
    }
    Ok(())
}
