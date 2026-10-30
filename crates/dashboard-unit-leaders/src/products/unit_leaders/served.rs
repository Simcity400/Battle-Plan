//! Generals' and admirals' served replies: each kind's leaderboard and
//! choices over the branch, the leaderboards at each hour, every country's
//! leaders, and some leaders' careers.

use dashboard_base::prepared::served::{self, Ask, Plan};
use serde_json::{Value, json};
use std::io;

pub static PLAN: Plan = Plan {
    queries,
    unordered: &[],
};

const KINDS: [&str; 3] = ["all", "land", "navy"];

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
        let mut references = Vec::new();
        for kind in KINDS {
            for view in ["leaderboard", "choices"] {
                let reply = ask.ask("unit-leaders", leaders(&id, all.clone(), view, kind, None))?;
                if let (Some(reply), "all", "leaderboard") = (reply, kind, view) {
                    references = served::texts_at(&reply, &["leaders", "*", "reference"]);
                }
            }
        }
        for hour in branch.hours() {
            for kind in ["land", "navy"] {
                ask.ask(
                    "unit-leaders",
                    leaders(&id, all.clone(), "leaderboard", kind, Some(hour)),
                )?;
            }
        }
        for hour in [branch.first_hour, branch.middle(), branch.last_hour] {
            ask.ask(
                "unit-leaders",
                leaders(&id, all.clone(), "choices", "all", Some(hour)),
            )?;
        }
        let countries = ask.countries(branch.id)?;
        for batch in served::batches(&countries) {
            let selection = json!({ "kind": "countries", "tags": batch });
            ask.ask(
                "unit-leaders",
                leaders(&id, selection, "leaderboard", "all", None),
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
