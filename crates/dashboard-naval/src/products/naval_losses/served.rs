//! Naval losses' served replies: the top countries by every kind, attribution
//! and period, their windows and hours, every country's, and the summaries
//! over each span.

use dashboard_base::prepared::served::{self, Ask, Plan};
use serde_json::{Value, json};
use std::io;

pub static PLAN: Plan = Plan {
    queries,
    unordered: &[],
};

const KINDS: [&str; 3] = ["ships", "convoys", "both"];
const ATTRIBUTIONS: [&str; 2] = ["lost", "sunkBy"];
const PERIODS: [&str; 2] = ["running", "month"];
/// The countries a ranked selection names.
const TOP: u32 = 10;
/// Pixels a summary draws over.
const WIDTH: u32 = 64;

fn losses(
    branch: &str,
    tags: &[String],
    top: Option<u32>,
    (kind, attribution, period): (&str, &str, &str),
) -> Value {
    json!({
        "branchId": branch, "tags": tags, "top": top, "kind": kind,
        "attribution": attribution, "period": period,
    })
}

fn queries(ask: &mut dyn Ask) -> io::Result<()> {
    for branch in ask.branches().to_vec() {
        let id = branch.name();
        for kind in KINDS {
            for attribution in ATTRIBUTIONS {
                for period in PERIODS {
                    ask.ask(
                        "naval-losses",
                        losses(&id, &[], Some(TOP), (kind, attribution, period)),
                    )?;
                }
            }
        }
        let ranked = losses(&id, &[], Some(TOP), ("both", "lost", "running"));
        for window in branch.window_indices() {
            let mut params = ranked.clone();
            params["window"] = json!(window);
            ask.ask("naval-losses", params)?;
        }
        for hour in branch.hours() {
            let mut params = ranked.clone();
            params["hour"] = json!(hour);
            ask.ask("naval-losses", params)?;
        }
        for batch in served::batches(&ask.countries(branch.id)?) {
            ask.ask(
                "naval-losses",
                losses(&id, &batch, None, ("both", "lost", "running")),
            )?;
        }
        let family = json!({
            "kind": "naval-losses", "navalKind": "both", "attribution": "lost",
            "period": "running", "top": TOP,
        });
        for (from, to) in branch.spans() {
            ask.ask(
                "series-summary",
                json!({
                    "branchId": id, "tags": [], "parts": false, "family": family,
                    "from": from, "to": to, "width": WIDTH,
                }),
            )?;
        }
    }
    Ok(())
}
