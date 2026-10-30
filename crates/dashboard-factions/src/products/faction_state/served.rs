//! Faction history's served replies: every faction's influence, rules,
//! manpower and goals, and some factions' goals over each span. The factions
//! are those the country groups name.

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
        let groups = ask
            .ask("country-groups", json!({ "branchId": id }))?
            .unwrap_or(Value::Null);
        let factions = served::texts_at(&groups, &["factions", "*", "id"]);
        for faction in &factions {
            for view in ["influence", "rules", "manpower"] {
                ask.ask(
                    "faction-state",
                    json!({ "view": view, "branchId": id, "faction": faction }),
                )?;
            }
            ask.ask(
                "faction-state",
                json!({ "view": "goals", "branchId": id, "faction": faction, "firstHour": null, "lastHour": null }),
            )?;
        }
        for faction in served::some(&factions) {
            for (from, to) in branch.spans() {
                ask.ask(
                    "faction-state",
                    json!({ "view": "goals", "branchId": id, "faction": faction, "firstHour": from, "lastHour": to }),
                )?;
            }
        }
    }
    Ok(())
}
