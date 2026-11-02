//! The intelligence products' served replies, asked as their widgets ask them: each product's
//! day at some of every branch's hours and over the whole branch, the agencies of all, the top
//! and some countries, some countries' rosters with and without those who left, and the
//! espionage log's manifest and every day those hours fall in.

use dashboard_base::prepared::served::{self, Ask, Plan};
use serde_json::{Value, json};
use std::io;

pub static PLAN: Plan = Plan {
    queries,
    unordered: &[],
};

/// The countries a ranked agency selection names.
const TOP: u32 = 10;

fn queries(ask: &mut dyn Ask) -> io::Result<()> {
    for branch in ask.branches().to_vec() {
        let id = branch.name();
        let (first, last) = (branch.first_hour, branch.last_hour);
        let some = served::some(&ask.countries(branch.id)?);
        // The widgets ask the day an hour falls in, from the branch's first hour.
        let mut days: Vec<i32> = branch
            .hours()
            .into_iter()
            .map(|hour| first + (hour - first).div_euclid(24) * 24)
            .collect();
        days.dedup();
        let spans = days
            .iter()
            .map(|&day| Some((day, (day + 23).min(last))))
            .chain([None]);
        for span in spans {
            let hours = |mut params: Value| {
                if let Some((from, to)) = span {
                    params["firstHour"] = json!(from);
                    params["lastHour"] = json!(to);
                }
                params
            };
            ask.ask("decryption", hours(json!({ "branchId": id })))?;
            ask.ask("operative-placements", hours(json!({ "branchId": id })))?;
            let selections = [
                json!({ "branchId": id, "tags": [], "includeUncreated": false, "top": null }),
                json!({ "branchId": id, "tags": [], "includeUncreated": true, "top": null }),
                json!({
                    "branchId": id, "tags": [], "includeUncreated": false,
                    "top": { "count": TOP, "field": "defense" },
                }),
                json!({ "branchId": id, "tags": some, "includeUncreated": false, "top": null }),
            ];
            for selection in selections {
                ask.ask(
                    "agency-comparison",
                    hours(json!({ "selection": selection })),
                )?;
            }
            if some.is_empty() {
                continue;
            }
            for departed in [false, true] {
                ask.ask(
                    "operatives-roster",
                    hours(json!({ "branchId": id, "tags": some, "includeDeparted": departed })),
                )?;
            }
        }
        ask.ask("espionage-log", json!({ "branchId": id, "window": null }))?;
        for day in days {
            let window = (day - first) / 24;
            ask.ask("espionage-log", json!({ "branchId": id, "window": window }))?;
        }
    }
    Ok(())
}
