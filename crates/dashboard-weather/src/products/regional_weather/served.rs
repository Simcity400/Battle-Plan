//! Regional weather's served replies: every region's daily columns of each measure over the
//! whole branch and each span, as the widget asks them; some regions' history, hourly and
//! daily, over each span and of each modifier; and their provinces at each sampled hour.

use dashboard_base::prepared::served::{self, Ask, Plan};
use serde_json::json;
use std::io;

pub static PLAN: Plan = Plan {
    queries,
    unordered: &[],
};

/// Columns one daily request folds into at most.
const COLUMNS: i64 = 1000;

fn queries(ask: &mut dyn Ask) -> io::Result<()> {
    const PRODUCT: &str = "regional-weather";
    for branch in ask.branches().to_vec() {
        let id = branch.name();
        let spans = branch.spans();
        let mut regions = Vec::new();
        for &(from, to) in &spans {
            let day = i64::from(dashboard_keyed::daily::DAY_HOURS);
            let days = (i64::from(to).div_euclid(day) - i64::from(from).div_euclid(day) + 1)
                .clamp(1, COLUMNS);
            for show in ["conditions", "temperature", "modifier"] {
                let reply = ask.ask(
                    PRODUCT,
                    json!({
                        "branchId": id, "firstHour": from, "lastHour": to,
                        "daily": true, "show": show, "width": days,
                    }),
                )?;
                if regions.is_empty()
                    && let Some(reply) = reply
                {
                    regions = served::texts_at(&reply, &["regions", "*", "id"]);
                }
            }
        }
        let some = served::some(&regions);
        if some.is_empty() {
            continue;
        }
        for &(from, to) in &spans {
            for daily in [false, true] {
                ask.ask(
                    PRODUCT,
                    json!({
                        "branchId": id, "firstHour": from, "lastHour": to,
                        "regions": some, "daily": daily,
                    }),
                )?;
            }
        }
        for (modifier, _, _) in super::content::MODIFIERS {
            ask.ask(
                PRODUCT,
                json!({
                    "branchId": id, "regions": some, "show": "modifier", "modifier": modifier,
                }),
            )?;
        }
        for region in &some {
            for hour in branch.hours() {
                ask.ask(
                    PRODUCT,
                    json!({ "branchId": id, "view": "provinces", "region": region, "hour": hour }),
                )?;
            }
        }
    }
    Ok(())
}
