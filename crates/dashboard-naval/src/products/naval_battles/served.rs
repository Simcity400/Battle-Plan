//! Naval battles' served replies: the manifest, its windows, the details of
//! their battles, and the rankings over each span by every measure, of every
//! country and of each batch of countries.

use dashboard_base::prepared::served::{self, Ask, Plan};
use serde_json::{Value, json};
use std::io;

pub static PLAN: Plan = Plan {
    queries,
    unordered: &[],
};

const MEASURES: [&str; 3] = ["Ships sunk", "Peak ships", "Length"];
/// The battles a ranking names.
const TOP: u32 = 10;
/// The battles of a window whose details are asked together.
const DETAILS: usize = 32;

/// A filter to `tags`, or to every country.
fn country_filter(tags: Option<&[String]>) -> Value {
    json!({ "tags": tags, "membership": null, "unobserved": [] })
}

fn queries(ask: &mut dyn Ask) -> io::Result<()> {
    for branch in ask.branches().to_vec() {
        let id = branch.name();
        let (first, last) = (branch.first_hour, branch.last_hour);
        let manifest = ask
            .ask("naval-battles-manifest", json!({ "branchId": id }))?
            .unwrap_or(Value::Null);
        let windows = served::values_at(&manifest, &["windows", "*"]).len();
        for window in served::indices(windows) {
            let Some(reply) =
                ask.ask("naval-battles", json!({ "branchId": id, "window": window }))?
            else {
                continue;
            };
            let battles: Vec<Value> = served::values_at(&reply, &["rows", "*"])
                .into_iter()
                .filter_map(|row| {
                    let battle = row.get("id")?.as_str()?;
                    let hour = row.get("runs")?.get(0)?.get("firstHour")?.as_i64()?;
                    Some(json!({ "id": battle, "hour": hour }))
                })
                .take(DETAILS)
                .collect();
            if !battles.is_empty() {
                ask.ask(
                    "naval-battle-details",
                    json!({ "branchId": id, "battles": battles }),
                )?;
            }
        }
        let ranking = |(from, to): (i32, i32), measure: &str, tags: Option<&[String]>| {
            json!({
                "branchId": id, "firstHour": from, "lastHour": to, "topN": TOP,
                "measure": measure, "countries": country_filter(tags),
            })
        };
        for span in branch.spans() {
            for measure in MEASURES {
                ask.ask("naval-battles-ranking", ranking(span, measure, None))?;
            }
        }
        for batch in served::batches(&ask.countries(branch.id)?) {
            ask.ask(
                "naval-battles-ranking",
                ranking((first, last), MEASURES[0], Some(&batch)),
            )?;
        }
    }
    Ok(())
}
