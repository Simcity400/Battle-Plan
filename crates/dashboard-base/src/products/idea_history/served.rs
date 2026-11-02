//! The idea history's served replies: every country's history over the branch, and some
//! countries' through some hours and at some hours.

use crate::prepared::served::{self, Ask, Plan};
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
        for tag in &countries {
            ask.ask("idea-history", json!({ "branchId": id, "tag": tag }))?;
        }
        for tag in served::some(&countries) {
            for hour in [branch.first_hour, branch.middle(), branch.last_hour] {
                ask.ask(
                    "idea-history",
                    json!({ "branchId": id, "tag": tag, "through": hour, "preview": true }),
                )?;
                ask.ask(
                    "idea-history",
                    json!({
                        "branchId": id, "tag": tag, "through": hour, "hour": hour,
                        "preview": false,
                    }),
                )?;
            }
        }
    }
    Ok(())
}
