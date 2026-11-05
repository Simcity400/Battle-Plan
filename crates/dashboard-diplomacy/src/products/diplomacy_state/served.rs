//! Diplomacy's served replies, as the widgets ask them: some countries' pacts,
//! each alone, the opinions between every country at each hour, and some pairs'
//! histories.

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
        let batches = served::batches(&countries);
        let some = served::some(&countries);
        for tag in &some {
            ask.ask(
                "diplomacy-state",
                json!({ "view": "pacts", "branchId": id, "tags": [tag] }),
            )?;
        }
        for hour in branch.hours() {
            for batch in batches.iter().filter(|batch| batch.len() > 1) {
                ask.ask(
                    "diplomacy-state",
                    json!({ "view": "opinions", "branchId": id, "hour": hour, "tags": batch }),
                )?;
            }
        }
        let pairs = some
            .windows(2)
            .flat_map(|pair| [(&pair[0], &pair[1]), (&pair[1], &pair[0])]);
        for (first, second) in pairs {
            for hour in [branch.first_hour, branch.middle(), branch.last_hour] {
                ask.ask(
                    "diplomacy-state",
                    json!({ "view": "pair", "branchId": id, "first": first, "second": second, "hour": hour }),
                )?;
            }
        }
    }
    Ok(())
}
