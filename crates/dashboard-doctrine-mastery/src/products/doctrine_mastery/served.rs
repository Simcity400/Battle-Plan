//! Doctrine mastery's served replies: some countries' whole branch, each span, and the whole
//! branch described at each of its hours.

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
            ask.ask("doctrine-mastery", json!({ "branchId": id, "tag": tag }))?;
            for hour in branch.hours() {
                ask.ask(
                    "doctrine-mastery",
                    json!({ "branchId": id, "tag": tag, "hour": hour }),
                )?;
            }
            for (from, to) in branch.spans() {
                ask.ask(
                    "doctrine-mastery",
                    json!({ "branchId": id, "tag": tag, "firstHour": from, "lastHour": to }),
                )?;
            }
        }
    }
    Ok(())
}
