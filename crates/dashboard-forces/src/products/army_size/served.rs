//! Army size's served replies: the manifest, the windows' histories by every
//! measure and attribution, every country's histories, the summaries over each
//! span, navigation from each hour, and some countries' template rosters.

use dashboard_base::prepared::served::{self, Ask, Plan};
use serde_json::{Value, json};
use std::io;

pub static PLAN: Plan = Plan {
    queries,
    unordered: &[],
};

const MEASURES: [&str; 4] = ["divisions", "manpower", "strength", "battalions"];
const ATTRIBUTIONS: [&str; 2] = ["owner", "host"];
/// The countries a ranked selection names.
const TOP: u32 = 10;
/// Pixels a summary draws over.
const WIDTH: u32 = 64;

fn selection(
    branch: &str,
    tags: &[String],
    top: Option<u32>,
    (measure, count_for): (&str, &str),
    by_type: bool,
    window: Option<usize>,
) -> Value {
    json!({
        "branchId": branch, "tags": tags, "top": top, "measure": measure,
        "countFor": count_for, "byType": by_type, "window": window,
    })
}

fn queries(ask: &mut dyn Ask) -> io::Result<()> {
    for branch in ask.branches().to_vec() {
        let id = branch.name();
        let (first, last) = (branch.first_hour, branch.last_hour);
        let countries = ask.countries(branch.id)?;
        let batches = served::batches(&countries);
        let windows = branch.window_indices();
        ask.ask(
            "army-size",
            selection(&id, &[], None, ("divisions", "owner"), false, None),
        )?;
        for measure in MEASURES {
            for count_for in ATTRIBUTIONS {
                for &window in &windows {
                    ask.ask(
                        "army-size",
                        selection(
                            &id,
                            &[],
                            Some(TOP),
                            (measure, count_for),
                            false,
                            Some(window),
                        ),
                    )?;
                }
                let family = json!({ "kind": "army-size", "measure": measure, "countFor": count_for, "top": TOP });
                ask.ask(
                    "series-summary",
                    json!({ "branchId": id, "family": family, "tags": [], "from": first, "to": last, "width": WIDTH }),
                )?;
            }
        }
        for batch in &batches {
            for &window in &windows {
                ask.ask(
                    "army-size",
                    selection(
                        &id,
                        batch,
                        None,
                        ("divisions", "owner"),
                        false,
                        Some(window),
                    ),
                )?;
                ask.ask(
                    "army-size",
                    selection(
                        &id,
                        batch,
                        None,
                        ("battalions", "owner"),
                        true,
                        Some(window),
                    ),
                )?;
            }
        }
        let ranked =
            json!({ "kind": "army-size", "measure": "divisions", "countFor": "owner", "top": TOP });
        let listed = json!({ "kind": "army-size", "measure": "divisions", "countFor": "owner", "top": null });
        for (from, to) in branch.spans() {
            ask.ask(
                "series-summary",
                json!({ "branchId": id, "family": ranked, "tags": [], "from": from, "to": to, "width": WIDTH }),
            )?;
            for batch in &batches {
                ask.ask(
                    "series-summary",
                    json!({ "branchId": id, "family": listed, "tags": batch, "from": from, "to": to, "width": WIDTH }),
                )?;
            }
        }
        let navigate = |key: &str, from: Option<i32>| {
            json!({
                "branchId": id, "family": ranked, "tags": [], "key": key, "from": from,
                "firstHour": first, "lastHour": last,
            })
        };
        for hour in branch.hours() {
            for key in ["ArrowLeft", "ArrowRight"] {
                ask.ask("series-next-change", navigate(key, Some(hour)))?;
            }
        }
        for key in ["Home", "End"] {
            ask.ask("series-next-change", navigate(key, None))?;
        }
        for tag in served::some(&countries) {
            for hour in [first, branch.middle(), last] {
                let mut roster = selection(
                    &id,
                    std::slice::from_ref(&tag),
                    None,
                    ("divisions", "owner"),
                    false,
                    None,
                );
                roster["templateAt"] = json!([tag, hour]);
                ask.ask("army-size", roster)?;
            }
        }
    }
    Ok(())
}
