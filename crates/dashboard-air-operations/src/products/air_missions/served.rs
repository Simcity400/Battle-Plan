//! Air missions' served replies: some countries' whole branch and each span.

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
            ask.ask("air-missions", json!({ "branchId": id, "tag": tag }))?;
            for (from, to) in branch.spans() {
                ask.ask(
                    "air-missions",
                    json!({ "branchId": id, "tag": tag, "firstHour": from, "lastHour": to }),
                )?;
            }
        }
    }
    Ok(())
}
