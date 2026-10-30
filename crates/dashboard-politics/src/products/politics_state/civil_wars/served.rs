//! Civil wars' served replies: each branch's whole list.

use dashboard_base::prepared::served::{Ask, Plan};
use serde_json::json;
use std::io;

pub static PLAN: Plan = Plan {
    queries,
    unordered: &[],
};

fn queries(ask: &mut dyn Ask) -> io::Result<()> {
    for branch in ask.branches().to_vec() {
        ask.ask(super::QUERY, json!({ "branchId": branch.name() }))?;
    }
    Ok(())
}
