//! Land battles' served replies: the manifest, its windows, the ranking over each span by each
//! measure, of every country and of each batch of countries, and some battles' courses.

use dashboard_base::prepared::served::{self, Ask, Plan};
use serde_json::{Value, json};
use std::io;

pub static PLAN: Plan = Plan {
    queries,
    unordered: &[],
};

/// The battles a ranking names.
const TOP: f64 = 10.0;
const MEASURES: [&str; 3] = ["Casualties", "Peak size", "Length"];

/// A filter to `tags`, or to every country.
fn country_filter(tags: Option<&[String]>) -> Value {
    json!({ "tags": tags, "membership": null, "unobserved": [] })
}

fn queries(ask: &mut dyn Ask) -> io::Result<()> {
    for branch in ask.branches().to_vec() {
        let id = branch.name();
        let (first, last) = (branch.first_hour, branch.last_hour);
        let manifest = ask
            .ask("land-battles-manifest", json!({ "branchId": id }))?
            .unwrap_or(Value::Null);
        let windows = served::values_at(&manifest, &["windows", "*"]).len();
        let mut battles = Vec::new();
        for window in served::indices(windows) {
            let reply = ask.ask("land-battles", json!({ "branchId": id, "window": window }))?;
            if let Some(reply) = reply {
                battles.extend(served::texts_at(&reply, &["rows", "*", "id"]));
            }
        }
        let ranking = |(from, to): (i32, i32), measure: &str, tags: Option<&[String]>| {
            json!({
                "branchId": id, "firstHour": from, "lastHour": to, "topN": TOP,
                "measure": measure, "countries": country_filter(tags),
            })
        };
        for span in branch.spans().into_iter().filter(|(from, to)| from <= to) {
            for measure in MEASURES {
                ask.ask("land-battles-ranking", ranking(span, measure, None))?;
            }
        }
        for batch in served::batches(&ask.countries(branch.id)?) {
            ask.ask(
                "land-battles-ranking",
                ranking((first, last), MEASURES[0], Some(batch.as_slice())),
            )?;
        }
        battles.sort_unstable();
        battles.dedup();
        for battle in served::some(&battles) {
            ask.ask(
                "land-battle-course",
                json!({ "branchId": id, "id": battle }),
            )?;
        }
    }
    Ok(())
}
