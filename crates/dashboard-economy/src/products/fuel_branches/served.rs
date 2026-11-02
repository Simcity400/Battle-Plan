//! Fuel by branch's served-check plan.

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
            ask.ask("fuel-branches", json!({ "branchId": id, "tags": [tag] }))?;
        }
        let batches = served::batches(&countries);
        match batches.as_slice() {
            [] => {}
            [every] => {
                ask.ask("fuel-branches", json!({ "branchId": id, "tags": [] }))?;
                ask.ask("fuel-branches", json!({ "branchId": id, "tags": every }))?;
            }
            [first, ..] => {
                ask.ask("fuel-branches", json!({ "branchId": id, "tags": first }))?;
            }
        }
    }
    Ok(())
}
