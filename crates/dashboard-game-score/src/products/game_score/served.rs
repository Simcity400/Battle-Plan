//! Game score's served replies: every country's score at each hour and some countries' alone,
//! each part's charts over each span, dense and hour by hour, and navigation from each hour.

use dashboard_base::prepared::served::{self, Ask, Plan};
use serde_json::{Value, json};
use std::io;

pub static PLAN: Plan = Plan {
    queries,
    unordered: &[],
};

/// Pixels a dense summary draws over.
const WIDTH: u32 = 64;

fn family(measure: Option<&str>) -> Value {
    json!({ "kind": "game-score", "measure": measure })
}

fn queries(ask: &mut dyn Ask) -> io::Result<()> {
    for branch in ask.branches().to_vec() {
        let id = branch.name();
        let (first, last) = (branch.first_hour, branch.last_hour);
        let countries = ask.countries(branch.id)?;
        let batches = served::batches(&countries);
        let some = served::some(&countries);
        let state = |tags: &[String], hour: i32| {
            json!({
                "branchId": id, "family": family(None), "tags": tags, "reads": [{ "hour": hour }],
            })
        };
        for hour in branch.hours() {
            ask.ask("series-state", state(&[], hour))?;
            ask.ask("series-state", state(&some, hour))?;
        }
        let hourly = |(from, to): (i32, i32)| i64::from(to) - i64::from(from) + 1;
        for measure in super::PARTS {
            let measured = family(Some(measure));
            for span in branch.spans() {
                for width in [i64::from(WIDTH), hourly(span).max(1)] {
                    for batch in &batches {
                        ask.ask(
                            "series-summary",
                            json!({
                                "branchId": id, "family": measured, "tags": batch,
                                "from": span.0, "to": span.1, "width": width,
                            }),
                        )?;
                    }
                }
            }
            let navigate = |tags: &[String], key: &str, from: Option<i32>| {
                json!({
                    "branchId": id, "family": measured, "tags": tags, "key": key, "from": from,
                    "firstHour": first, "lastHour": last,
                })
            };
            for hour in branch.hours() {
                for key in ["ArrowLeft", "ArrowRight"] {
                    for batch in &batches {
                        ask.ask("series-next-change", navigate(batch, key, Some(hour)))?;
                    }
                }
            }
            for key in ["Home", "End"] {
                ask.ask("series-next-change", navigate(&some, key, None))?;
            }
        }
    }
    Ok(())
}
