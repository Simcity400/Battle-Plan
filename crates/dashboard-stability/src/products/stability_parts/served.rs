//! Stability parts' served replies: every country's parts at each hour, and
//! summaries over each span, for stability and war support.

use dashboard_base::prepared::served::{self, Ask, Plan};
use serde_json::{Value, json};
use std::io;

pub static PLAN: Plan = Plan {
    queries,
    unordered: &[],
};

const MEASURES: [&str; 2] = ["stability", "warSupport"];
/// Pixels a summary draws over.
const WIDTH: u32 = 64;

fn queries(ask: &mut dyn Ask) -> io::Result<()> {
    for branch in ask.branches().to_vec() {
        let id = branch.name();
        let countries = ask.countries(branch.id)?;
        let batches = served::batches(&countries);
        for measure in MEASURES {
            let family = json!({ "kind": "stability-parts", "measure": measure });
            let hours = if measure == "stability" {
                branch.hours()
            } else {
                vec![branch.first_hour, branch.middle(), branch.last_hour]
            };
            for batch in &batches {
                for &hour in &hours {
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
            let Some(batch) = batches.first() else {
                continue;
            };
            for (from, to) in branch.spans() {
                ask.ask(
                    "series-summary",
                    json!({ "branchId": id, "family": family, "tags": batch, "from": from, "to": to, "width": WIDTH }),
                )?;
            }
        }
    }
    Ok(())
}
