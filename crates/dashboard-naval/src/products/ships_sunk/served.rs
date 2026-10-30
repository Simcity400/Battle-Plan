//! Ships sunk's served replies: the manifest, the window holding each hour
//! and each window's first hour, and the battle titles of their rows.

use dashboard_base::prepared::served::{self, Ask, Plan, WINDOWS};
use serde_json::{Value, json};
use std::io;

pub static PLAN: Plan = Plan {
    queries,
    unordered: &[],
};

/// The rows of a window whose battle titles are asked together.
const TITLES: usize = 64;

fn queries(ask: &mut dyn Ask) -> io::Result<()> {
    for branch in ask.branches().to_vec() {
        let id = branch.name();
        let manifest = ask
            .ask("ships-sunk-manifest", json!({ "branchId": id }))?
            .unwrap_or(Value::Null);
        let starts: Vec<i32> = served::values_at(&manifest, &["windows", "*", "firstHour"])
            .into_iter()
            .filter_map(|hour| i32::try_from(hour.as_i64()?).ok())
            .collect();
        let mut hours = branch.hours();
        hours.extend(served::spread(&starts, WINDOWS));
        hours.sort_unstable();
        hours.dedup();
        for hour in hours {
            let Some(reply) = ask.ask("ships-sunk", json!({ "branchId": id, "hour": hour }))?
            else {
                continue;
            };
            let rows: Vec<Value> = served::values_at(&reply, &["indices", "*"])
                .into_iter()
                .take(TITLES)
                .cloned()
                .collect();
            if !rows.is_empty() {
                ask.ask(
                    "ships-sunk-battle-titles",
                    json!({ "branchId": id, "rows": rows }),
                )?;
            }
        }
    }
    Ok(())
}
