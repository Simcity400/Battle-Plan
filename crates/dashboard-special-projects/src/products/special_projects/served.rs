//! Special projects' served replies: every country's projects over the whole branch, and some
//! countries' through their own selection root.

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
        for tag in &countries {
            ask.ask(
                "special-projects",
                json!({ "branchId": id, "tag": tag, "selectionTags": [] }),
            )?;
        }
        for tag in served::some(&countries) {
            ask.ask(
                "special-projects",
                json!({ "branchId": id, "tag": tag, "selectionTags": [tag] }),
            )?;
        }
    }
    Ok(())
}
