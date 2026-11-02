//! World tension's served replies: the world tension and contribution charts over each span,
//! dense and hour by hour, as contributions and as shares, with and without some contributors'
//! lines; the reads at each hour; and navigation from each hour.

use dashboard_base::prepared::served::{self, Ask, Plan};
use serde_json::{Map, Value, json};
use std::io;

pub static PLAN: Plan = Plan {
    queries,
    unordered: &[],
};

/// Pixels a dense summary draws over.
const WIDTH: u32 = 64;

fn queries(ask: &mut dyn Ask) -> io::Result<()> {
    for branch in ask.branches().to_vec() {
        let id = branch.name();
        let (first, last) = (branch.first_hour, branch.last_hour);
        let family = json!({ "kind": "world-tension" });
        let reads: Vec<Value> = branch
            .hours()
            .into_iter()
            .map(|hour| json!({ "hour": hour }))
            .collect();
        let mut keys = Vec::new();
        for share in [false, true] {
            let state = json!({
                "branchId": id, "share": share, "family": family, "reads": reads,
            });
            if let Some(reply) = ask.ask("series-state", state)? {
                for item in reply["catalogue"].as_array().into_iter().flatten() {
                    if let (Some(key), Some(tag), Some(slot)) = (
                        item["key"].as_str(),
                        item["tag"].as_str(),
                        item["slot"].as_u64(),
                    ) && !keys.iter().any(|(held, _)| held == key)
                    {
                        keys.push((key.to_owned(), serde_json::to_string(&(tag, slot))?));
                    }
                }
            }
        }
        // Some contributors, by catalogue key and by their recorded tag and slot.
        let mut within = Map::new();
        for (at, (key, native)) in served::spread(&keys, served::KEYS).into_iter().enumerate() {
            let named = if at % 2 == 0 { key } else { native };
            within.insert(format!("line {at}"), json!([[named, first, last, false]]));
        }
        let selections = [Map::new(), within];
        let hourly = |(from, to): (i32, i32)| i64::from(to) - i64::from(from) + 1;
        for share in [false, true] {
            for span in branch.spans() {
                for width in [i64::from(WIDTH), hourly(span).max(1)] {
                    for selected in &selections {
                        ask.ask(
                            "series-summary",
                            json!({
                                "branchId": id, "share": share, "family": family,
                                "from": span.0, "to": span.1, "width": width, "within": selected,
                            }),
                        )?;
                    }
                }
            }
            for selected in &selections {
                ask.ask(
                    "series-state",
                    json!({
                        "branchId": id, "share": share, "family": family, "reads": reads,
                        "within": selected,
                    }),
                )?;
            }
        }
        let navigate = |key: &str, from: Option<i32>, within: &Map<String, Value>| {
            json!({
                "branchId": id, "share": false, "family": family, "within": within,
                "key": key, "from": from, "firstHour": first, "lastHour": last,
            })
        };
        for hour in branch.hours() {
            for key in ["ArrowLeft", "ArrowRight"] {
                for selected in &selections {
                    ask.ask("series-next-change", navigate(key, Some(hour), selected))?;
                }
            }
        }
        for key in ["Home", "End"] {
            ask.ask("series-next-change", navigate(key, None, &selections[0]))?;
        }
    }
    Ok(())
}
