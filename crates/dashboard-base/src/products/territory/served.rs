//! Territory's served replies: the metadata, every measure's ranked and listed charts over each
//! span, dense and hour by hour, a group's sums, the states at each hour with the regional
//! breakdown, and navigation from each hour.

use crate::prepared::served::{self, Ask, Plan};
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
const MEASURES: [&str; 4] = ["states", "provinces", "victoryPoints", "area"];
const RULES: [&str; 3] = ["controlled", "owned", "occupied"];

fn family(measure: &str, rule: &str, top: Option<u32>, aggregate: bool) -> Value {
    json!({ "kind": "territory", "measure": measure, "rule": rule, "top": top, "aggregate": aggregate })
}

fn queries(ask: &mut dyn Ask) -> io::Result<()> {
    for branch in ask.branches().to_vec() {
        let id = branch.name();
        let (first, last) = (branch.first_hour, branch.last_hour);
        let metadata = json!({ "kind": "territory", "metadata": true });
        ask.ask(
            "series-state",
            json!({ "branchId": id, "family": metadata, "reads": [{ "hour": first }] }),
        )?;
        let countries = ask.countries(branch.id)?;
        let batches = served::batches(&countries);
        let some = served::some(&countries);
        let none: &[String] = &[];
        let summary = |family: &Value, tags: &[String], (from, to): (i32, i32), width: i64| {
            json!({
                "branchId": id, "family": family, "tags": tags, "from": from, "to": to,
                "width": width,
            })
        };
        let state = |family: &Value, tags: &[String], hour: i32| json!({ "branchId": id, "family": family, "tags": tags, "reads": [{ "hour": hour }] });
        let hourly = |(from, to): (i32, i32)| i64::from(to) - i64::from(from) + 1;
        for measure in MEASURES {
            for rule in RULES {
                let ranked = family(measure, rule, Some(TOP), false);
                let listed = family(measure, rule, None, false);
                for span in branch.spans() {
                    for width in [i64::from(WIDTH), hourly(span).max(1)] {
                        ask.ask("series-summary", summary(&ranked, none, span, width))?;
                        ask.ask("series-summary", summary(&listed, &some, span, width))?;
                    }
                }
                for hour in [first, branch.middle(), last] {
                    ask.ask("series-state", state(&ranked, none, hour))?;
                    ask.ask("series-state", state(&listed, &some, hour))?;
                }
            }
        }
        // Every country's states and the group sums of each batch, for one measure of each
        // table and the area.
        for (measure, rule) in [
            ("states", "owned"),
            ("provinces", "controlled"),
            ("area", "owned"),
        ] {
            let listed = family(measure, rule, None, false);
            let grouped = family(measure, rule, None, true);
            let ranked_group = family(measure, rule, Some(TOP), true);
            for batch in &batches {
                for hour in branch.hours() {
                    ask.ask("series-state", state(&listed, batch, hour))?;
                }
                for span in branch.spans() {
                    for width in [i64::from(WIDTH), hourly(span).max(1)] {
                        ask.ask("series-summary", summary(&listed, batch, span, width))?;
                        ask.ask("series-summary", summary(&grouped, batch, span, width))?;
                    }
                }
                ask.ask("series-state", state(&grouped, batch, branch.middle()))?;
            }
            for span in branch.spans() {
                ask.ask(
                    "series-summary",
                    summary(&ranked_group, none, span, i64::from(WIDTH)),
                )?;
            }
        }
        // The regional breakdown at each hour, of every measure of both rules it holds.
        for measure in MEASURES {
            for rule in ["controlled", "owned"] {
                let regional = json!({
                    "kind": "territory", "measure": measure, "rule": rule, "breakdown": true,
                });
                for hour in branch.hours() {
                    ask.ask("series-state", state(&regional, none, hour))?;
                }
            }
        }
        let navigate = |family: &Value, tags: &[String], key: &str, from: Option<i32>| {
            json!({
                "branchId": id, "family": family, "tags": tags, "key": key, "from": from,
                "firstHour": first, "lastHour": last,
            })
        };
        let ranked = family("states", "controlled", Some(TOP), false);
        for hour in branch.hours() {
            for key in ["ArrowLeft", "ArrowRight"] {
                ask.ask(
                    "series-next-change",
                    navigate(&ranked, none, key, Some(hour)),
                )?;
                for batch in &batches {
                    let listed = family("provinces", "owned", None, false);
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
    Ok(())
}
