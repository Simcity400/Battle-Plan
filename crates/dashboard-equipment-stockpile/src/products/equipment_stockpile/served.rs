//! The equipment stockpile's served replies: every country's catalogue, and some countries'
//! stockpiles at each hour and their arms' lines over each span.

use dashboard_base::prepared::served::{self, Ask, Plan};
use serde_json::json;
use std::io;

pub static AT: Plan = Plan {
    queries: at,
    unordered: &[],
};

pub static CATALOGUE: Plan = Plan {
    queries: catalogue,
    unordered: &[],
};

pub static CHART: Plan = Plan {
    queries: chart,
    unordered: &[],
};

/// Pixels a summary draws over.
const WIDTH: u32 = 64;

fn at(ask: &mut dyn Ask) -> io::Result<()> {
    for branch in ask.branches().to_vec() {
        let id = branch.name();
        let countries = ask.countries(branch.id)?;
        for tag in served::some(&countries) {
            for hour in branch.hours() {
                ask.ask(
                    "equipment-stockpile-at",
                    json!({ "branchId": id, "tag": tag, "hour": hour }),
                )?;
            }
        }
    }
    Ok(())
}

fn catalogue(ask: &mut dyn Ask) -> io::Result<()> {
    for branch in ask.branches().to_vec() {
        let id = branch.name();
        for tag in ask.countries(branch.id)? {
            ask.ask(
                "equipment-stockpile-catalogue",
                json!({ "branchId": id, "holders": [{ "tag": tag }] }),
            )?;
        }
    }
    Ok(())
}

fn chart(ask: &mut dyn Ask) -> io::Result<()> {
    for branch in ask.branches().to_vec() {
        let id = branch.name();
        let countries = ask.countries(branch.id)?;
        let lines: Vec<_> = ["land", "air", "naval"]
            .into_iter()
            .flat_map(|arm| {
                let group = json!(["domain", arm]).to_string();
                ["stock", "share", "need"]
                    .map(|measure| json!({ "group": group, "measure": measure }))
            })
            .collect();
        for tag in served::some(&countries) {
            for (from, to) in branch.spans() {
                ask.ask(
                    "equipment-stockpile-chart",
                    json!({
                        "branchId": id, "holders": [{ "tag": tag }], "lines": lines,
                        "from": from, "to": to, "width": WIDTH,
                    }),
                )?;
            }
        }
    }
    Ok(())
}
