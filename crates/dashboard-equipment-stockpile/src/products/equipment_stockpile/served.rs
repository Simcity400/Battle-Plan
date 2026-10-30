//! The equipment stockpile's served replies: every country's whole history,
//! some countries' chart fields over each span and their stockpiles at each
//! hour, and each batch of countries' together.

use dashboard_base::prepared::served::{self, Ask, Plan};
use serde_json::json;
use std::io;

pub static PLAN: Plan = Plan {
    queries,
    unordered: &[],
};

const FIELDS: [&str; 4] = ["stock", "balance", "logistics-stock", "available"];
/// Pixels a summary draws over.
const WIDTH: u32 = 64;

fn queries(ask: &mut dyn Ask) -> io::Result<()> {
    for branch in ask.branches().to_vec() {
        let id = branch.name();
        let (first, last) = (branch.first_hour, branch.last_hour);
        let countries = ask.countries(branch.id)?;
        for tag in &countries {
            ask.ask(
                "equipment-stockpile",
                json!({ "branchId": id, "tag": tag, "from": first, "to": last, "width": WIDTH }),
            )?;
        }
        for tag in served::some(&countries) {
            for (from, to) in branch.spans() {
                ask.ask(
                    "equipment-stockpile",
                    json!({
                        "branchId": id, "tag": tag, "from": from, "to": to,
                        "width": WIDTH, "fields": FIELDS,
                    }),
                )?;
            }
            for hour in branch.hours() {
                ask.ask(
                    "equipment-stockpile",
                    json!({ "branchId": id, "tag": tag, "from": hour, "to": hour, "width": 1 }),
                )?;
            }
        }
        for batch in served::batches(&countries) {
            for hour in [first, branch.middle(), last] {
                ask.ask(
                    "equipment-stockpile",
                    json!({ "branchId": id, "tags": batch, "from": hour, "to": hour, "width": 1 }),
                )?;
            }
        }
    }
    Ok(())
}
