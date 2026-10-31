//! The manpower view's served replies: some countries' views at each branch's hours.

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
            for hour in branch.hours() {
                ask.ask(
                    "country-people",
                    json!({ "branchId": id, "tag": tag, "hour": hour }),
                )?;
            }
        }
    }
    Ok(())
}
