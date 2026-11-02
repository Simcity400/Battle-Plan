//! Minefields' served replies: every country's history over the branch and over each span, some
//! countries' alone, and the standing reports of some mined regions at each of its hours.

use dashboard_base::prepared::served::{self, Ask, Plan};
use serde_json::{Value, json};
use std::{collections::BTreeSet, io};

pub static PLAN: Plan = Plan {
    queries,
    unordered: &[],
};

/// Regions sampled from each source of report choices.
const REGIONS: usize = 4;

fn queries(ask: &mut dyn Ask) -> io::Result<()> {
    for branch in ask.branches().to_vec() {
        let id = branch.name();
        let whole = ask
            .ask("minefields", json!({ "branchId": id }))?
            .unwrap_or(Value::Null);
        for (from, to) in branch.spans() {
            ask.ask(
                "minefields",
                json!({ "branchId": id, "firstHour": from, "lastHour": to }),
            )?;
        }
        for tag in served::some(&ask.countries(branch.id)?) {
            ask.ask("minefields", json!({ "branchId": id, "tags": [tag] }))?;
        }
        // A series named `OWNER|<region>` holds that owner's mines in the region.
        let mined: BTreeSet<u32> = served::values_at(&whole, &["window", "series", "*", "name"])
            .into_iter()
            .filter_map(|name| name.as_str()?.split_once('|')?.1.parse().ok())
            .collect();
        let mined: Vec<u32> = mined.into_iter().collect();
        let choices: Vec<u32> = served::values_at(&whole, &["regions", "*", "id"])
            .into_iter()
            .filter_map(|id| id.as_str()?.parse().ok())
            .collect();
        let regions: BTreeSet<u32> = served::spread(&mined, REGIONS)
            .into_iter()
            .chain(served::spread(&choices, REGIONS))
            .collect();
        for region in regions {
            for hour in branch.hours() {
                ask.ask(
                    "minefields",
                    json!({ "branchId": id, "view": "reports", "region": region, "hour": hour }),
                )?;
            }
        }
    }
    Ok(())
}
