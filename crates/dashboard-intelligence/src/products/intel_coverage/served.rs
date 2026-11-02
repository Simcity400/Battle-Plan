//! Intel network coverage's served replies: the matrix of the most active countries over the
//! whole branch and each span, with every network touching them, of each batch of countries,
//! and the pairs of some countries.

use dashboard_base::prepared::served::{self, Ask, Plan};
use serde_json::json;
use std::io;

pub static PLAN: Plan = Plan {
    queries,
    unordered: &[],
};

fn queries(ask: &mut dyn Ask) -> io::Result<()> {
    const PRODUCT: &str = "intel-network-coverage";
    for branch in ask.branches().to_vec() {
        let id = branch.name();
        let countries = ask.countries(branch.id)?;
        let mut spans = vec![(branch.first_hour, branch.last_hour)];
        spans.extend(branch.spans());
        for (from, to) in spans {
            for touching in [false, true] {
                ask.ask(
                    PRODUCT,
                    json!({
                        "branchId": id, "view": "matrix", "touching": touching,
                        "firstHour": from, "lastHour": to,
                    }),
                )?;
            }
        }
        for batch in served::batches(&countries) {
            ask.ask(
                PRODUCT,
                json!({ "branchId": id, "view": "matrix", "tags": batch }),
            )?;
        }
        let some = served::some(&countries);
        for owner in &some {
            for target in some.iter().filter(|target| *target != owner) {
                ask.ask(
                    PRODUCT,
                    json!({ "branchId": id, "view": "pair", "owner": owner, "target": target }),
                )?;
            }
        }
    }
    Ok(())
}
