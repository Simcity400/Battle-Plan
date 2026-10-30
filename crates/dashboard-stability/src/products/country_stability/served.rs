//! Stability's and war support's served replies: every country's summaries
//! over each span and values at each hour, by every measure, with marks, and
//! navigation to their changes from each hour.

use dashboard_base::prepared::served::{self, Ask, Plan};
use serde_json::{Value, json};
use std::io;

pub static PLAN: Plan = Plan {
    queries,
    unordered: &[],
};

const MEASURES: [&str; 3] = ["stability", "warSupport", "maxCommandPower"];
/// Pixels a summary draws over.
const WIDTH: u32 = 64;

fn queries(ask: &mut dyn Ask) -> io::Result<()> {
    for branch in ask.branches().to_vec() {
        let id = branch.name();
        let (first, last) = (branch.first_hour, branch.last_hour);
        let countries = ask.countries(branch.id)?;
        let batches = served::batches(&countries);
        for measure in MEASURES {
            let family = json!({ "kind": "country-stability", "measure": measure });
            let (spans, hours) = if measure == "stability" {
                (branch.spans(), branch.hours())
            } else {
                (vec![(first, last)], vec![first, branch.middle(), last])
            };
            for batch in &batches {
                for &(from, to) in &spans {
                    ask.ask(
                        "series-summary",
                        json!({
                            "branchId": id, "family": family, "tags": batch,
                            "from": from, "to": to, "width": WIDTH, "withMarks": false,
                        }),
                    )?;
                }
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
        }
        let Some(batch) = batches.first() else {
            continue;
        };
        let family = json!({ "kind": "country-stability", "measure": "stability" });
        ask.ask(
            "series-summary",
            json!({
                "branchId": id, "family": family, "tags": batch,
                "from": first, "to": last, "width": WIDTH, "withMarks": true,
            }),
        )?;
        for hour in branch.hours() {
            for key in ["ArrowLeft", "ArrowRight"] {
                ask.ask(
                    "series-next-change",
                    json!({
                        "branchId": id, "family": family, "tags": batch,
                        "measures": ["stability", "warSupport"], "key": key, "from": hour,
                        "firstHour": first, "lastHour": last,
                    }),
                )?;
            }
        }
    }
    Ok(())
}
