//! Research's served replies: some countries' slot states and grants at each hour, one read per
//! request as the active research view asks them, and their whole history pages.

use crate::prepared::served::{self, Ask, Plan};
use serde_json::{Value, json};
use std::io;

pub static PLAN: Plan = Plan {
    queries,
    unordered: &[],
};

fn family() -> Value {
    json!({ "kind": "research" })
}

fn queries(ask: &mut dyn Ask) -> io::Result<()> {
    for branch in ask.branches().to_vec() {
        let id = branch.name();
        let countries = ask.countries(branch.id)?;
        for tag in served::some(&countries) {
            for hour in branch.hours() {
                ask.ask(
                    "series-state",
                    json!({
                        "branchId": id, "family": family(),
                        "reads": [{ "tag": tag, "hour": hour }],
                    }),
                )?;
            }
            ask.ask(
                "events-page",
                json!({ "branchId": id, "tag": tag, "family": family() }),
            )?;
        }
    }
    Ok(())
}
