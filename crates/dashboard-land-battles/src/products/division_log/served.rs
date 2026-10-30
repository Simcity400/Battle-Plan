//! The division log's served replies: the manifest, its windows, and the
//! totals over each span, of every country, of each batch of countries and of
//! some templates.

use dashboard_base::prepared::served::{self, Ask, Plan};
use serde_json::{Value, json};
use std::io;

pub static PLAN: Plan = Plan {
    queries,
    unordered: &[],
};

/// A filter to `tags`, or to every country.
fn country_filter(tags: Option<&[String]>) -> Value {
    json!({ "tags": tags, "membership": null, "unobserved": [] })
}

fn queries(ask: &mut dyn Ask) -> io::Result<()> {
    for branch in ask.branches().to_vec() {
        let id = branch.name();
        let (first, last) = (branch.first_hour, branch.last_hour);
        let manifest = ask
            .ask("division-log-manifest", json!({ "branchId": id }))?
            .unwrap_or(Value::Null);
        let windows = served::values_at(&manifest, &["windows", "*"]).len();
        let templates = served::texts_at(&manifest, &["templates", "*", "id"]);
        for window in served::indices(windows) {
            ask.ask("division-log", json!({ "branchId": id, "window": window }))?;
        }
        let totals =
            |(from, to): (i32, i32), tags: Option<&[String]>, templates: Option<&[String]>| {
                json!({
                    "branchId": id, "firstHour": from, "lastHour": to,
                    "countries": country_filter(tags), "templates": templates,
                })
            };
        for span in branch.spans() {
            ask.ask("division-log-totals", totals(span, None, None))?;
        }
        for batch in served::batches(&ask.countries(branch.id)?) {
            ask.ask(
                "division-log-totals",
                totals((first, last), Some(&batch), None),
            )?;
        }
        for template in served::some(&templates) {
            ask.ask(
                "division-log-totals",
                totals((first, last), None, Some(std::slice::from_ref(&template))),
            )?;
        }
    }
    Ok(())
}
