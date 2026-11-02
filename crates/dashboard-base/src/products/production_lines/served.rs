//! Production lines' served replies: some countries' lines over each branch, as they are
//! waited for and exactly, every detail window holding one of the branch's read hours, and some
//! of their lines' every factor, and their output from the branch's first, middle and last hour.

use super::{DETAIL_HOURS, breakdown::FACTORS};
use crate::prepared::served::{self, Ask, Plan};
use serde_json::json;
use std::io;

pub static PLAN: Plan = Plan {
    queries,
    unordered: &[],
};

fn queries(ask: &mut dyn Ask) -> io::Result<()> {
    for branch in ask.branches().to_vec() {
        let id = branch.name();
        let mut windows: Vec<i32> = branch
            .hours()
            .into_iter()
            .map(|hour| hour.div_euclid(DETAIL_HOURS))
            .collect();
        windows.dedup();
        let countries = ask.countries(branch.id)?;
        for tag in served::some(&countries) {
            let mut keys = Vec::new();
            for exact in [None, Some(true)] {
                let mut params = json!({ "branchId": id, "tag": tag });
                if let Some(exact) = exact {
                    params["exact"] = json!(exact);
                }
                if let Some(reply) = ask.ask("production-lines", params)? {
                    keys = served::texts_at(&reply, &["lines", "*", "key"]);
                }
            }
            for &window in &windows {
                ask.ask(
                    "production-line-detail",
                    json!({ "branchId": id, "tag": tag, "window": window }),
                )?;
            }
            for key in served::some(&keys) {
                let breakdown =
                    |group: &str| json!({ "branchId": id, "tag": tag, "key": key, "group": group });
                for &(group, _) in FACTORS {
                    ask.ask("production-line-breakdown", breakdown(group))?;
                }
                for origin in [branch.first_hour, branch.middle(), branch.last_hour] {
                    let mut params = breakdown("output");
                    params["originHour"] = json!(origin);
                    ask.ask("production-line-breakdown", params)?;
                }
            }
        }
    }
    Ok(())
}
