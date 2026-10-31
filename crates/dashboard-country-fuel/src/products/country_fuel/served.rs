//! Fuel's served replies: every country's summaries of every measure over
//! each span, and its values at each hour.

use dashboard_base::prepared::served::{self, Ask, Plan};
use serde_json::{Value, json};
use std::io;

pub static PLAN: Plan = Plan {
    queries,
    unordered: &[&["series"]],
};

const MEASURES: [&str; 4] = ["fuel", "capacity", "stripGain", "stripBurn"];
/// Pixels a summary draws over.
const WIDTH: u32 = 64;

fn queries(ask: &mut dyn Ask) -> io::Result<()> {
    for branch in ask.branches().to_vec() {
        let id = branch.name();
        let family = json!({ "kind": "country-fuel" });
        let countries = ask.countries(branch.id)?;
        for batch in served::batches(&countries) {
            for (from, to) in branch.spans() {
                ask.ask(
                    "series-summary",
                    json!({
                        "branchId": id, "family": family, "tags": batch, "measures": MEASURES,
                        "from": from, "to": to, "width": WIDTH,
                    }),
                )?;
            }
            for hour in branch.hours() {
                let reads: Vec<Value> = batch
                    .iter()
                    .map(|tag| json!({ "tag": tag, "hour": hour }))
                    .collect();
                ask.ask(
                    "series-state",
                    json!({ "branchId": id, "family": family, "reads": reads }),
                )?;
            }
        }
    }
    Ok(())
}
