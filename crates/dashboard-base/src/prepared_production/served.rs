//! Production's served replies: every measure and grouping of the ranked top's equipment
//! produced, and some countries' own, at the branch's hours and over the branch; some
//! countries' production spending over the branch and its spans, and their military factory
//! spending.

use crate::prepared::served::{self, Ask, Plan};
use serde_json::{Value, json};
use std::io;

pub static PLAN: Plan = Plan {
    queries,
    unordered: &[],
};

/// The countries a ranked selection names.
const TOP: u32 = 10;
/// Countries a selection page holds.
const LIMIT: usize = 25;
/// A chart's plot width the history is gathered for.
const VIEW_WIDTH: u32 = 640;

fn selection(
    branch: &str,
    tags: &[String],
    top: Option<u32>,
    (measure, count, grouping): (&str, &str, &str),
    refits: bool,
) -> Value {
    json!({
        "branchId": branch, "tags": tags, "top": top, "measure": measure, "count": count,
        "grouping": grouping, "included": [], "refits": refits,
    })
}

fn queries(ask: &mut dyn Ask) -> io::Result<()> {
    for branch in ask.branches().to_vec() {
        let id = branch.name();
        let countries = ask.countries(branch.id)?;
        let some = served::some(&countries);
        let hours = branch.hours();
        let select = |selection: Value, hour: i32, ranked: bool| {
            json!({
                "hour": hour, "selection": selection, "offset": "0", "limit": LIMIT,
                "ranked": ranked,
            })
        };
        for &hour in &hours {
            for measure in ["total", "day", "week", "month"] {
                for (count, grouping, refits) in [
                    ("units", "type", false),
                    ("ic", "domain", true),
                    ("units", "variant", true),
                ] {
                    let ranked = selection(&id, &[], Some(TOP), (measure, count, grouping), refits);
                    ask.ask("production-select", select(ranked, hour, true))?;
                }
            }
            for tag in &some {
                let tags = std::slice::from_ref(tag);
                for measure in ["total", "week"] {
                    let own = selection(&id, tags, None, (measure, "units", "variant"), false);
                    ask.ask("production-select", select(own, hour, false))?;
                }
            }
        }
        for (measure, count, grouping, refits) in [
            ("total", "units", "type", true),
            ("day", "units", "variant", false),
            ("week", "ic", "type", false),
        ] {
            let ranked = selection(&id, &[], Some(TOP), (measure, count, grouping), refits);
            ask.ask("equipment-produced", ranked.clone())?;
            if let (Some(first), Some(last)) = (hours.first(), hours.last()) {
                let mut viewed = ranked;
                viewed["view"] = json!({ "from": first, "to": last, "width": VIEW_WIDTH });
                ask.ask("equipment-produced", viewed)?;
            }
        }
        for tag in &some {
            for (measure, count) in [("day", "units"), ("month", "ic")] {
                let own = selection(&id, std::slice::from_ref(tag), None, (measure, count, "variant"), false);
                ask.ask("equipment-produced", own.clone())?;
                if let Some(&hour) = hours.get(hours.len() / 2) {
                    let mut viewed = own;
                    viewed["view"] = json!({ "from": hour, "to": hour, "width": 1 });
                    ask.ask("equipment-produced", viewed)?;
                }
            }
            let mut windows = vec![(None, None, None)];
            windows.extend(
                branch
                    .spans()
                    .into_iter()
                    .map(|(from, to)| (Some(from), Some(to), Some(to))),
            );
            for (from, hour, end) in windows {
                for refits in [false, true] {
                    ask.ask(
                        "production-spending",
                        json!({
                            "branchId": id, "tag": tag, "refits": refits, "from": from,
                            "hour": hour, "end": end,
                        }),
                    )?;
                }
            }
            for equipment in ["land", "air"] {
                for view in ["circle", "months"] {
                    ask.ask(
                        "military-factory-spending",
                        json!({
                            "branchId": id, "tag": tag, "view": view, "equipment": equipment,
                            "from": null, "hour": null, "end": null,
                        }),
                    )?;
                }
            }
        }
    }
    Ok(())
}
