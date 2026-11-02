//! The market's served replies: each view's history of every country and of some countries
//! alone and together, and ledger pages of every country, some countries and a group's members,
//! newest first through each hour asked and followed by their cursors.

use super::{HISTORY, LEDGER, LEDGER_KINDS};
use dashboard_base::prepared::served::{self, Ask, PAGES, Plan};
use serde_json::{Value, json};
use std::io;

pub static PLAN: Plan = Plan {
    queries,
    unordered: &[],
};

/// Rows a ledger page asks.
const PAGE: u32 = 64;
/// Pages followed from a page asked at each of the branch's hours.
const FOLLOWED: usize = 4;

/// `params`'s ledger pages, each next one from the reply's `next`, `pages` at most.
fn pages(ask: &mut dyn Ask, mut params: Value, pages: usize) -> io::Result<()> {
    for _ in 0..pages {
        let Some(reply) = ask.ask(LEDGER, params.clone())? else {
            return Ok(());
        };
        match reply.get("next").and_then(Value::as_str) {
            Some(cursor) => params["before"] = Value::from(cursor),
            None => return Ok(()),
        }
    }
    Ok(())
}

fn queries(ask: &mut dyn Ask) -> io::Result<()> {
    for branch in ask.branches().to_vec() {
        let id = branch.name();
        let (first, last) = (branch.first_hour, branch.last_hour);
        let countries = ask.countries(branch.id)?;
        let some = served::some(&countries);
        let mut selections: Vec<Vec<String>> = vec![Vec::new(), some.clone()];
        selections.extend(some.iter().map(|tag| vec![tag.clone()]));
        for view in ["contracts", "requests", "inventory", "agreements"] {
            for tags in &selections {
                ask.ask(
                    HISTORY,
                    json!({ "branchId": id, "view": view, "tags": tags }),
                )?;
            }
        }
        let ledger = |tags: &[String], kinds: &[u8], from: Option<i32>, to: i32| {
            json!({
                "branchId": id, "tags": tags, "kinds": kinds, "firstHour": from,
                "lastHour": to, "pageSize": PAGE,
            })
        };
        let none: &[String] = &[];
        for hour in branch.hours() {
            pages(ask, ledger(none, &[], None, hour), FOLLOWED)?;
        }
        pages(ask, ledger(none, &[], None, last), PAGES)?;
        pages(ask, ledger(none, &[], Some(branch.middle()), last), PAGES)?;
        for kind in LEDGER_KINDS {
            pages(ask, ledger(none, &[kind], None, last), PAGES)?;
        }
        for tags in &selections[1..] {
            pages(ask, ledger(tags, &[], None, last), PAGES)?;
            pages(ask, ledger(tags, &[], None, branch.middle()), FOLLOWED)?;
        }
        let members: Vec<Value> = some
            .iter()
            .enumerate()
            .map(|(at, tag)| {
                let (from, to) = if at % 2 == 0 {
                    (first, branch.middle())
                } else {
                    (branch.middle(), last)
                };
                json!({ "tag": tag, "firstHour": from, "lastHour": to })
            })
            .collect();
        if !members.is_empty() {
            let mut params = ledger(none, &[], None, last);
            params["members"] = Value::from(members);
            pages(ask, params, PAGES)?;
        }
    }
    Ok(())
}
