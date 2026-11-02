//! Political and command power's served replies: every country's summaries over each span, with
//! and without their drops as marks, values at each hour, navigation from each hour, and some
//! countries' drop pages and marks.

use crate::prepared::served::{self, Ask, Plan};
use serde_json::{Value, json};
use std::io;

pub static PLAN: Plan = Plan {
    queries,
    unordered: &[],
};

const MEASURES: [&str; 2] = ["politicalPower", "commandPower"];
/// Pixels a summary draws over.
const WIDTH: u32 = 64;
/// Drops a page asks for, as the drop list does.
const PAGE: u32 = 32;

fn queries(ask: &mut dyn Ask) -> io::Result<()> {
    for branch in ask.branches().to_vec() {
        let id = branch.name();
        let (first, last) = (branch.first_hour, branch.last_hour);
        let countries = ask.countries(branch.id)?;
        let batches = served::batches(&countries);
        for measure in MEASURES {
            let family = json!({ "kind": "country-measure", "measure": measure });
            for batch in &batches {
                for (from, to) in branch.spans() {
                    for marks in [false, true] {
                        ask.ask(
                            "series-summary",
                            json!({
                                "branchId": id, "family": family, "tags": batch,
                                "from": from, "to": to, "width": WIDTH, "withMarks": marks,
                            }),
                        )?;
                    }
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
                    for key in ["ArrowLeft", "ArrowRight"] {
                        ask.ask(
                            "series-next-change",
                            json!({
                                "branchId": id, "family": family, "tags": batch, "key": key,
                                "from": hour, "firstHour": first, "lastHour": last,
                            }),
                        )?;
                    }
                }
            }
            for tag in served::some(&countries) {
                served::pages(
                    ask,
                    "events-page",
                    json!({
                        "branchId": id, "family": family, "tags": [tag], "cursor": "0",
                        "pageSize": PAGE, "to": last,
                    }),
                    "cursor",
                )?;
                ask.ask(
                    "events-page",
                    json!({
                        "branchId": id, "family": family, "tags": [tag], "cursor": "0",
                        "pageSize": PAGE, "from": branch.middle(), "to": last,
                    }),
                )?;
                ask.ask(
                    "events-clusters",
                    json!({
                        "branchId": id, "family": family, "tag": tag,
                        "from": first, "to": last, "width": WIDTH,
                    }),
                )?;
            }
            if let Some(batch) = batches.first() {
                ask.ask(
                    "events-page",
                    json!({
                        "branchId": id, "family": family, "tags": batch, "cursor": "0",
                        "pageSize": PAGE, "to": last,
                    }),
                )?;
            }
        }
    }
    Ok(())
}
