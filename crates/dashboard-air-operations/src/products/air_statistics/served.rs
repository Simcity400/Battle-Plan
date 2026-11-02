//! Air statistics' served replies: some countries' whole branch by every channel pair, and each
//! span by the first.

use dashboard_base::prepared::served::{self, Ask, Plan};
use serde_json::json;
use std::io;

pub static PLAN: Plan = Plan {
    queries,
    unordered: &[],
};

/// The channel pairs a reply selects: ours, and the enemy's twelve channels after.
const PAIRS: u8 = 12;

fn queries(ask: &mut dyn Ask) -> io::Result<()> {
    for branch in ask.branches().to_vec() {
        let id = branch.name();
        let countries = ask.countries(branch.id)?;
        for tag in served::some(&countries) {
            for channel in 0..PAIRS {
                ask.ask(
                    "air-statistics",
                    json!({ "branchId": id, "tag": tag, "channel": channel }),
                )?;
            }
            for (from, to) in branch.spans() {
                ask.ask(
                    "air-statistics",
                    json!({
                        "branchId": id, "tag": tag, "channel": 0, "firstHour": from,
                        "lastHour": to,
                    }),
                )?;
            }
        }
    }
    Ok(())
}
