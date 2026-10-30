//! The political and command power ledger's served replies: every country's
//! ledger, and some countries' by every view, over the branch and through
//! some hours.

use dashboard_base::prepared::served::{self, Ask, Plan};
use serde_json::json;
use std::io;

pub static PLAN: Plan = Plan {
    queries,
    unordered: &[],
};

const VIEWS: [&str; 4] = ["raw", "spending", "balance", "governments"];

fn queries(ask: &mut dyn Ask) -> io::Result<()> {
    for branch in ask.branches().to_vec() {
        let id = branch.name();
        let countries = ask.countries(branch.id)?;
        for tag in &countries {
            ask.ask("politics-ledger", json!({ "branchId": id, "tag": tag }))?;
        }
        for tag in served::some(&countries) {
            for view in VIEWS {
                ask.ask(
                    "politics-ledger",
                    json!({ "branchId": id, "tag": tag, "view": view }),
                )?;
                for hour in [branch.first_hour, branch.middle(), branch.last_hour] {
                    ask.ask(
                        "politics-ledger",
                        json!({ "branchId": id, "tag": tag, "view": view, "hour": hour }),
                    )?;
                }
            }
        }
    }
    Ok(())
}
