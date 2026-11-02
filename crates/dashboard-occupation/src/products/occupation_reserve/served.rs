//! Occupation reserve's served replies: the race of every holder and of the top ones over the
//! whole branch and each span, some holders' own views, and the race of each batch of holders.

use dashboard_base::prepared::served::{self, Ask, Plan};
use serde_json::json;
use std::io;

pub static PLAN: Plan = Plan {
    queries,
    unordered: &[],
};

/// The holders a ranked race names.
const TOP: u32 = 10;

fn queries(ask: &mut dyn Ask) -> io::Result<()> {
    const PRODUCT: &str = "occupation-reserve-manpower";
    for branch in ask.branches().to_vec() {
        let id = branch.name();
        let countries = ask.countries(branch.id)?;
        let mut spans = vec![(branch.first_hour, branch.last_hour)];
        spans.extend(branch.spans());
        for (from, to) in spans {
            let window = |top: Option<u32>| {
                json!({ "branchId": id, "view": "race", "top": top, "firstHour": from, "lastHour": to })
            };
            ask.ask(PRODUCT, window(None))?;
            ask.ask(PRODUCT, window(Some(TOP)))?;
        }
        for batch in served::batches(&countries) {
            ask.ask(
                PRODUCT,
                json!({ "branchId": id, "view": "race", "tags": batch, "top": TOP }),
            )?;
        }
        for tag in served::some(&countries) {
            ask.ask(
                PRODUCT,
                json!({ "branchId": id, "view": "holder", "tag": tag }),
            )?;
        }
    }
    Ok(())
}
