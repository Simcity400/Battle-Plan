//! The occupation trend's served replies: each measure's top occupiers and
//! every occupier at each hour and over each span, navigation from each
//! hour, and the collaboration context.

use dashboard_base::prepared::served::{Ask, Plan};
use serde_json::{Value, json};
use std::io;

pub static PLAN: Plan = Plan {
    queries,
    unordered: &[],
};

const MEASURES: [&str; 2] = ["resistance", "compliance"];
/// The occupiers a ranked selection names.
const TOP: u32 = 10;
/// Pixels a summary draws over.
const WIDTH: u32 = 64;

fn family(measure: &str, top: Option<u32>, collaboration: bool) -> Value {
    json!({
        "kind": "occupation-trend", "measure": measure, "rankMeasure": measure, "top": top,
        "collaboration": collaboration, "both": false,
    })
}

fn queries(ask: &mut dyn Ask) -> io::Result<()> {
    for branch in ask.branches().to_vec() {
        let id = branch.name();
        let (first, last) = (branch.first_hour, branch.last_hour);
        for measure in MEASURES {
            let ranked = family(measure, Some(TOP), false);
            let every = family(measure, None, false);
            for hour in branch.hours() {
                ask.ask(
                    "series-state",
                    json!({ "branchId": id, "family": ranked, "tags": [], "reads": [{ "hour": hour }] }),
                )?;
                for key in ["ArrowLeft", "ArrowRight"] {
                    ask.ask(
                        "series-next-change",
                        json!({
                            "branchId": id, "family": ranked, "tags": [], "key": key, "from": hour,
                            "firstHour": first, "lastHour": last,
                        }),
                    )?;
                }
            }
            for (from, to) in branch.spans() {
                ask.ask(
                    "series-summary",
                    json!({ "branchId": id, "family": ranked, "tags": [], "from": from, "to": to, "width": WIDTH }),
                )?;
            }
            ask.ask(
                "series-summary",
                json!({ "branchId": id, "family": every, "tags": [], "from": first, "to": last, "width": WIDTH }),
            )?;
            for hour in [first, branch.middle(), last] {
                ask.ask(
                    "series-state",
                    json!({ "branchId": id, "family": every, "tags": [], "reads": [{ "hour": hour }] }),
                )?;
            }
        }
        ask.ask(
            "series-summary",
            json!({
                "branchId": id, "family": family("resistance", None, true), "tags": [],
                "from": first, "to": last, "width": WIDTH,
            }),
        )?;
    }
    Ok(())
}
