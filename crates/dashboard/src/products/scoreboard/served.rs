//! The scoreboard's served replies: its column registry, each hour's ranked
//! index whole and read against the hour before, the index ranked by every
//! column and both ways, and every country's cells.

use dashboard_base::prepared::served::{self, Ask, Plan};
use serde_json::{Value, json};
use std::io;

pub static PLAN: Plan = Plan {
    queries,
    unordered: &[],
};

/// The countries a ranked index names.
const TOP: u32 = 10;

/// A prepared page: ranked by `sort` when `top`, else of `tags`; the index
/// with its bars when `index`, else the cells.
fn page(
    branch: &str,
    columns: &[String],
    (tags, top): (&[String], bool),
    hour: i32,
    (sort, descending): (&str, bool),
    index: bool,
) -> Value {
    let top = top.then(|| json!({ "count": TOP, "sort": sort, "descending": descending }));
    json!({
        "selection": { "branchId": branch, "columns": columns, "tags": tags, "top": top },
        "hour": hour, "sort": sort, "descending": descending, "index": index, "bars": index,
    })
}

fn queries(ask: &mut dyn Ask) -> io::Result<()> {
    for branch in ask.branches().to_vec() {
        let id = branch.name();
        let registry = json!({
            "selection": { "branchId": id, "columns": [], "tags": [], "top": null },
            "hour": 0, "sort": "factories", "descending": true, "index": true, "bars": false,
        });
        let Some(registry) = ask.ask("scoreboard", registry)? else {
            continue;
        };
        let columns: Vec<String> = served::values_at(&registry, &["data", "columns", "*"])
            .into_iter()
            .filter(|column| column["available"] == true)
            .filter_map(|column| column["key"].as_str().map(str::to_owned))
            .collect();
        let Some(sort) = registry["data"]["defaultSort"]
            .as_str()
            .map(str::to_owned)
            .or_else(|| columns.first().cloned())
        else {
            continue;
        };
        let countries = ask.countries(branch.id)?;
        let ends = [branch.first_hour, branch.middle(), branch.last_hour];
        for hour in branch.hours() {
            ask.ask(
                "scoreboard",
                page(
                    &id,
                    &columns,
                    (&[], true),
                    hour,
                    (sort.as_str(), true),
                    true,
                ),
            )?;
        }
        // Playback's index, each hour read against the hour before.
        for pair in branch.hours().windows(2) {
            let mut params = page(
                &id,
                &columns,
                (&[], true),
                pair[1],
                (sort.as_str(), true),
                true,
            );
            params["since"] = json!(pair[0]);
            ask.ask("scoreboard", params)?;
        }
        for column in &columns {
            for hour in ends {
                ask.ask(
                    "scoreboard",
                    page(
                        &id,
                        &columns,
                        (&[], true),
                        hour,
                        (column.as_str(), true),
                        true,
                    ),
                )?;
            }
        }
        for hour in ends {
            ask.ask(
                "scoreboard",
                page(
                    &id,
                    &columns,
                    (&[], true),
                    hour,
                    (sort.as_str(), false),
                    true,
                ),
            )?;
            for batch in served::batches(&countries) {
                for index in [true, false] {
                    ask.ask(
                        "scoreboard",
                        page(
                            &id,
                            &columns,
                            (&batch, false),
                            hour,
                            (sort.as_str(), true),
                            index,
                        ),
                    )?;
                }
            }
        }
    }
    Ok(())
}
