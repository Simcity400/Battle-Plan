//! Air force size's served replies: under each measure and ownership, the ranked and every
//! country's charts over each span at a small and a wide chart's width, every page of each, some
//! countries' runs at each hour, and navigation from each hour.

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

fn family(top: Option<u32>, measure: &str, ownership: &str) -> Value {
    json!({ "kind": "air-force-size", "top": top, "measure": measure, "ownership": ownership })
}

fn queries(ask: &mut dyn Ask) -> io::Result<()> {
    for branch in ask.branches().to_vec() {
        let id = branch.name();
        let (first, last) = (branch.first_hour, branch.last_hour);
        let countries = ask.countries(branch.id)?;
        let batches = served::batches(&countries);
        let none: &[String] = &[];
        for measure in ["planes", "wings"] {
            for ownership in ["roster", "effective"] {
                let ranked = family(Some(TOP), measure, ownership);
                let listed = family(None, measure, ownership);
                let summary =
                    |family: &Value, tags: &[String], (from, to): (i32, i32), width: u32| {
                        json!({
                            "branchId": id, "family": family, "tags": tags, "from": from, "to": to,
                            "width": width,
                        })
                    };
                for span in branch.spans() {
                    for width in [WIDTH, served::WIDE] {
                        served::pages(
                            ask,
                            "series-summary",
                            summary(&ranked, none, span, width),
                            "cursor",
                        )?;
                        served::pages(
                            ask,
                            "series-summary",
                            summary(&listed, &countries, span, width),
                            "cursor",
                        )?;
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
