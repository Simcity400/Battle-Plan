//! Wars' served replies: the recorded viewpoint's and some countries' wars, their casualties at
//! every served hour, and the groups and contributions each sees at each of those hours.

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
        let reads: Vec<Value> = hours.iter().map(|&hour| json!({ "hour": hour })).collect();
        let views = std::iter::once(None).chain(served::some(&countries).into_iter().map(Some));
        for view in views {
            ask.ask("wars", json!({ "branchId": id, "viewpointTag": view }))?;
            ask.ask(
                "wars",
                json!({ "branchId": id, "viewpointTag": view, "reads": reads }),
            )?;
            for &hour in &hours {
                ask.ask(
                    "wars",
                    json!({ "branchId": id, "viewpointTag": view, "current": [{ "hour": hour }] }),
                )?;
            }
        }
    }
    Ok(())
}
