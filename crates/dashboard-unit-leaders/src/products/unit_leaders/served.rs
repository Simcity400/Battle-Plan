//! Generals' and admirals' served replies, as the widgets ask them: every page of the choices
//! over the branch and of every country's, the leaderboards at each hour whole and as playback
//! reads their rows, every country's
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
        // A choice row is [reference, name, owner, last hour].
        let mut references = Vec::new();
        served::each_page(
            ask,
            "unit-leaders",
            leaders(&id, all.clone(), "choices", "all", None),
            "cursor",
            |reply| {
                references.extend(
                    served::values_at(reply, &["choices", "*"])
                        .into_iter()
                        .filter_map(|row| row.get(0)?.as_str().map(str::to_owned)),
                );
            },
        )?;
        for hour in branch.hours() {
            for kind in KINDS {
                ask.ask(
                    "unit-leaders",
                    leaders(&id, all.clone(), "leaderboard", kind, Some(hour)),
                )?;
            }
        }
        // Playback's rows: each hour's whole, then each read against the hour before.
        for kind in KINDS {
            let mut before: Option<(i32, Value)> = None;
            for hour in branch.hours() {
                let mut params = leaders(&id, all.clone(), "leaderboard", kind, Some(hour));
                params["rows"] = json!(true);
                let whole = ask.ask("unit-leaders", params.clone())?;
                if let Some((since, index)) = before.take()
                    && index.is_string()
                {
                    params["since"] = json!(since);
                    params["held"] = index;
                    ask.ask("unit-leaders", params)?;
                }
                before = whole.map(|reply| (hour, reply["index"].clone()));
            }
        }
        let countries = ask.countries(branch.id)?;
        for batch in served::batches(&countries) {
            let selection = json!({ "kind": "countries", "tags": batch });
            ask.ask(
                "unit-leaders",
                leaders(
                    &id,
                    selection.clone(),
                    "leaderboard",
                    "all",
                    Some(branch.last_hour),
                ),
            )?;
            served::pages(
                ask,
                "unit-leaders",
                leaders(&id, selection, "choices", "all", None),
                "cursor",
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
