//! Volunteers' and expeditionaries' served replies: each kind's every link,
//! and every country's links.

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
        for kind in ["land", "expeditionary"] {
            ask.ask(
                "volunteers-expeditionaries",
                json!({ "branchId": id, "kind": kind, "tags": null }),
            )?;
            for batch in served::batches(&countries) {
                ask.ask(
                    "volunteers-expeditionaries",
                    json!({ "branchId": id, "kind": kind, "tags": batch }),
                )?;
            }
        }
    }
    Ok(())
}
