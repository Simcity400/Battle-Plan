//! Lend-lease balance's served replies: every country's totals, and some holders' records of
//! each partner.

use dashboard_base::prepared::served::{self, Ask, Plan};
use serde_json::{Value, json};
use std::io;

pub static PLAN: Plan = Plan {
    queries,
    unordered: &[],
};

fn queries(ask: &mut dyn Ask) -> io::Result<()> {
    for branch in ask.branches().to_vec() {
        let id = branch.name();
        let Some(every) = ask.ask(super::QUERY, json!({ "branchId": id }))? else {
            continue;
        };
        let holders: Vec<String> = served::values_at(&every, &["histories", "*", "holder"])
            .into_iter()
            .filter_map(Value::as_str)
            .map(str::to_owned)
            .collect();
        for holder in served::some(&holders) {
            ask.ask(super::QUERY, json!({ "branchId": id, "holder": holder }))?;
        }
    }
    Ok(())
}
