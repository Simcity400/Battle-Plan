//! The catalogue's served replies, as the widgets ask them: every country's licences, and some
//! countries' organisations and designs, one at a time.

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
        ask.ask(
            super::QUERY,
            json!({ "branchId": id, "view": "licences", "tags": [] }),
        )?;
        for view in ["organisations", "designs"] {
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
