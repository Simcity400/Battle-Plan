//! Naval supremacy's served replies: some countries' daily control over the branch and over
//! each span.

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
        for tag in served::some(&ask.countries(branch.id)?) {
            ask.ask("naval-supremacy", json!({ "branchId": id, "tag": tag }))?;
            for (from, to) in branch.spans() {
                ask.ask(
                    "naval-supremacy",
                    json!({ "branchId": id, "tag": tag, "firstHour": from, "lastHour": to }),
                )?;
            }
        }
    }
    Ok(())
}
