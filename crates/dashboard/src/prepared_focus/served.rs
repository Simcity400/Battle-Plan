//! National focus' served replies: the four tables' refusals at each hour, some countries' focus
//! at each hour with every page of the completed focuses it names, and every country's dated
//! completions.

use dashboard_base::prepared::served::{self, Ask, Plan};
use serde_json::{Value, json};
use std::io;

pub static PLAN: Plan = Plan {
    queries,
    unordered: &[],
};

fn queries(ask: &mut dyn Ask) -> io::Result<()> {
    for branch in ask.branches().to_vec() {
        let id = branch.name();
        let countries = ask.countries(branch.id)?;
        let hours = branch.hours();
        for &hour in &hours {
            ask.ask("focus-unavailable", json!({ "branchId": id, "hour": hour }))?;
        }
        for tag in served::some(&countries) {
            for &hour in &hours {
                let reply = ask.ask(
                    "focus-hour",
                    json!({ "branchId": id, "tag": tag, "hour": hour }),
                )?;
                let Some(pages) = reply
                    .as_ref()
                    .and_then(|reply| reply.get("focus")?.get("completed"))
                else {
                    continue;
                };
                let count = pages
                    .get("count")
                    .and_then(Value::as_str)
                    .and_then(|count| count.parse::<u64>().ok())
                    .unwrap_or(0);
                let pages = pages.clone();
                for page in 0..count.min(served::PAGES as u64) {
                    ask.ask(
                        "focus-order-page",
                        json!({
                            "branchId": id, "tag": tag, "hour": hour, "pages": pages,
                            "page": page.to_string(),
                        }),
                    )?;
                }
            }
        }
        for tag in &countries {
            ask.ask("focus-completions", json!({ "branchId": id, "tag": tag }))?;
        }
    }
    Ok(())
}
