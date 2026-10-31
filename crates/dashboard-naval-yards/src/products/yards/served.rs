//! The yard views' served replies: every country at some of its branch's hours, and each
//! country holding a port or line there at every one of them.

use dashboard_base::prepared::served::{self, Ask, Plan};
use serde_json::{Value, json};
use std::io;

pub static REPAIRS: Plan = Plan {
    queries: repairs,
    unordered: &[],
};

pub static REFITS: Plan = Plan {
    queries: refits,
    unordered: &[],
};

/// Requests asking every country of a branch, at most: half a plan's share of the branch.
const EVERY: usize = 2048;

fn repairs(ask: &mut dyn Ask) -> io::Result<()> {
    queries(ask, "naval-repairs")
}

fn refits(ask: &mut dyn Ask) -> io::Result<()> {
    queries(ask, "naval-refits")
}

fn queries(ask: &mut dyn Ask, product: &str) -> io::Result<()> {
    for branch in ask.branches().to_vec() {
        let id = branch.name();
        let hours = branch.hours();
        let tags = ask.countries(branch.id)?;
        let first = served::spread(&hours, if tags.len() * 3 > EVERY { 1 } else { 3 });
        for tag in &tags {
            let mut holds = false;
            for &hour in &first {
                let data = ask.ask(product, json!({ "branchId": id, "tag": tag, "hour": hour }))?;
                holds |= data
                    .as_ref()
                    .and_then(|data| data.get("holders"))
                    .and_then(Value::as_array)
                    .is_some_and(|holders| !holders.is_empty());
            }
            if !holds {
                continue;
            }
            for &hour in hours.iter().filter(|hour| !first.contains(hour)) {
                ask.ask(product, json!({ "branchId": id, "tag": tag, "hour": hour }))?;
            }
        }
    }
    Ok(())
}
