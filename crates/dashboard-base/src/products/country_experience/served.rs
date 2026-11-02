//! Experience's served replies: every recorded country's summaries of each banked, gain and
//! change measure over spans of the branch, its values and supplying reads at the branch's
//! hours, a chart with its reads as marks, and navigation over its reads from each hour. A
//! country is named as the widget names it: the JSON of its tag, canonical slot and id.

use crate::prepared::served::{self, Ask, Plan};
use serde_json::{Value, json};
use std::io;

pub static PLAN: Plan = Plan {
    queries,
    unordered: &[],
};

/// Pixels a summary draws over.
const WIDTH: u32 = 64;

fn queries(ask: &mut dyn Ask) -> io::Result<()> {
    for branch in ask.branches().to_vec() {
        let id = branch.name();
        let (first, last) = (branch.first_hour, branch.last_hour);
        let mut keys = ask
            .holders(branch.id)?
            .into_iter()
            .map(|(tag, slot, country)| serde_json::to_string(&(tag, slot, country)))
            .collect::<Result<Vec<_>, _>>()?;
        keys.sort_unstable();
        keys.dedup();
        let batches = served::batches(&keys);
        for mode in ["banked", "gain", "change"] {
            for unit in ["army", "navy", "air"] {
                let measure = format!("{unit}-{mode}");
                let family = json!({ "kind": "country-experience", "measure": measure });
                let (spans, hours) = if measure == "army-banked" {
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
        }
        let Some(batch) = batches.first() else {
            continue;
        };
        let family = json!({ "kind": "country-experience", "measure": "army-banked" });
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
                        "measures": ["experienceRead"], "key": key, "from": hour,
                        "firstHour": first, "lastHour": last,
                    }),
                )?;
            }
        }
    }
    Ok(())
}
