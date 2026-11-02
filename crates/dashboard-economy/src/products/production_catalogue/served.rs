//! The catalogue's served replies: every view of every branch, for every country and for some
//! countries one at a time.

use dashboard_base::prepared::served::{self, Ask, Plan};
use serde_json::json;
use std::io;

pub static PLAN: Plan = Plan {
    queries,
    unordered: &[],
};

fn queries(ask: &mut dyn Ask) -> io::Result<()> {
    for branch in ask.branches().to_vec() {
        let id = branch.name();
        let countries = ask.countries(branch.id)?;
        for view in ["licences", "organisations", "designs"] {
            ask.ask(
                super::QUERY,
                json!({ "branchId": id, "view": view, "tags": [] }),
            )?;
            for tag in served::some(&countries) {
                ask.ask(
                    super::QUERY,
                    json!({ "branchId": id, "view": view, "tags": [tag] }),
                )?;
            }
        }
    }
    Ok(())
}
