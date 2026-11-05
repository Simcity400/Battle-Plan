//! Generals' and admirals' served replies, as the widgets ask them: the
//! choices over the branch, the leaderboards at each hour, every country's
//! leaders at the last hour, and some leaders' careers.

use dashboard_base::prepared::served::{self, Ask, Plan};
use serde_json::{Value, json};
use std::io;

pub static PLAN: Plan = Plan {
    queries,
    unordered: &[],
};

/// The leaders a leaderboard ranks: the generals widget's every kind, and the career widget's
/// first general.
const KINDS: [&str; 2] = ["all", "land"];

fn leaders(branch: &str, selection: Value, view: &str, kind: &str, hour: Option<i32>) -> Value {
    let mut params =
        json!({ "branchId": branch, "selection": selection, "view": view, "leaders": kind });
    if let Some(hour) = hour {
        params["hour"] = json!(hour);
    }
    params
}

fn queries(ask: &mut dyn Ask) -> io::Result<()> {
    for branch in ask.branches().to_vec() {
        let id = branch.name();
        let all = json!({ "kind": "all" });
        let choices = ask.ask(
            "unit-leaders",
            leaders(&id, all.clone(), "choices", "all", None),
        )?;
        let references = choices
            .map(|reply| served::texts_at(&reply, &["leaders", "*", "reference"]))
            .unwrap_or_default();
        for hour in branch.hours() {
            for kind in KINDS {
                ask.ask(
                    "unit-leaders",
                    leaders(&id, all.clone(), "leaderboard", kind, Some(hour)),
                )?;
            }
        }
        let countries = ask.countries(branch.id)?;
        for batch in served::batches(&countries) {
            let selection = json!({ "kind": "countries", "tags": batch });
            ask.ask(
                "unit-leaders",
                leaders(&id, selection, "leaderboard", "all", Some(branch.last_hour)),
            )?;
        }
        for reference in served::some(&references) {
            let selection = json!({ "kind": "leader", "reference": reference });
            ask.ask(
                "unit-leaders",
                leaders(&id, selection.clone(), "career", "all", None),
            )?;
            ask.ask(
                "unit-leaders",
                leaders(&id, selection, "career", "all", Some(branch.middle())),
            )?;
        }
    }
    Ok(())
}
