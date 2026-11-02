//! Navy size's served replies: under each measure, ownership and grouping, the ranked and every
//! country's charts over each span, dense and hour by hour, and some countries' runs at each
//! hour; and under each measure and ownership, navigation from each hour.

use dashboard_base::prepared::served::{self, Ask, Plan};
use serde_json::{Value, json};
use std::io;

pub static PLAN: Plan = Plan {
    queries,
    unordered: &[],
};

/// The countries a ranked selection names.
const TOP: u32 = 10;
/// Pixels a dense summary draws over.
const WIDTH: u32 = 64;

fn family(top: Option<u32>, measure: &str, ownership: &str, grouping: &str) -> Value {
    json!({
        "kind": "navy-size", "top": top, "measure": measure, "ownership": ownership,
        "grouping": grouping,
    })
}

fn queries(ask: &mut dyn Ask) -> io::Result<()> {
    for branch in ask.branches().to_vec() {
        let id = branch.name();
        let (first, last) = (branch.first_hour, branch.last_hour);
        let countries = ask.countries(branch.id)?;
        let batches = served::batches(&countries);
        let none: &[String] = &[];
        let hourly = |(from, to): (i32, i32)| i64::from(to) - i64::from(from) + 1;
        for measure in ["ships", "strength"] {
            for ownership in ["roster", "logical"] {
                for grouping in ["class", "type"] {
                    let ranked = family(Some(TOP), measure, ownership, grouping);
                    let listed = family(None, measure, ownership, grouping);
                    let summary =
                        |family: &Value, tags: &[String], (from, to): (i32, i32), width: i64| {
                            json!({
                                "branchId": id, "family": family, "tags": tags, "from": from,
                                "to": to, "width": width,
                            })
                        };
                    for span in branch.spans() {
                        for width in [i64::from(WIDTH), hourly(span).max(1)] {
                            ask.ask("series-summary", summary(&ranked, none, span, width))?;
                            for batch in &batches {
                                ask.ask(
                                    "series-summary",
                                    summary(&listed, batch.as_slice(), span, width),
                                )?;
                            }
                        }
                    }
                    for tag in served::some(&countries) {
                        let reads: Vec<Value> = branch
                            .hours()
                            .into_iter()
                            .map(|hour| json!({ "tag": tag, "hour": hour }))
                            .collect();
                        ask.ask(
                            "series-state",
                            json!({ "branchId": id, "family": listed, "reads": reads }),
                        )?;
                    }
                }
                // Navigation follows totals, which no grouping changes.
                let ranked = family(Some(TOP), measure, ownership, "class");
                let listed = family(None, measure, ownership, "class");
                let navigate = |family: &Value, tags: &[String], key: &str, from: Option<i32>| {
                    json!({
                        "branchId": id, "family": family, "tags": tags, "key": key, "from": from,
                        "firstHour": first, "lastHour": last,
                    })
                };
                for hour in branch.hours() {
                    for key in ["ArrowLeft", "ArrowRight"] {
                        ask.ask(
                            "series-next-change",
                            navigate(&ranked, none, key, Some(hour)),
                        )?;
                        for batch in &batches {
                            ask.ask(
                                "series-next-change",
                                navigate(&listed, batch.as_slice(), key, Some(hour)),
                            )?;
                        }
                    }
                }
                for key in ["Home", "End"] {
                    ask.ask("series-next-change", navigate(&ranked, none, key, None))?;
                }
            }
        }
    }
    Ok(())
}
