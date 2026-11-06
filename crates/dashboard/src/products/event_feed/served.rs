//! The event feed's served replies: every page of the branch's feed through
//! its end, and in small pages, each category's
//! first page, first pages from each hour and over each span, and the rows a
//! following widget reads from one hour to the next; and the event choices'
//! and decisions' pages likewise.

use dashboard_base::prepared::served::{self, Ask, Plan};
use serde_json::{Value, json};
use std::io;

pub static FEED: Plan = Plan {
    queries: feed,
    unordered: &[],
};

pub static SCRIPT: Plan = Plan {
    queries: script,
    unordered: &[],
};

const CATEGORIES: [&str; 4] = ["warsPeace", "diplomacy", "politics", "milestones"];
/// The widgets' page size.
const PAGE: u32 = 256;
/// The page size of the page a widget asks first at an hour.
const FIRST: u32 = 64;
/// A page size whose pages' edges fall all through the feed.
const SMALL: u32 = 7;
const NEXT: &str = "nextCursor";

fn feed_page(
    branch: &str,
    (first, last): (Option<i32>, i32),
    categories: &[&str],
    size: u32,
) -> Value {
    json!({
        "branchId": branch, "firstHour": first, "lastHour": last, "categories": categories,
        "cursor": null, "pageSize": size,
    })
}

fn feed(ask: &mut dyn Ask) -> io::Result<()> {
    for branch in ask.branches().to_vec() {
        let id = branch.name();
        let whole = (None, branch.last_hour);
        served::pages(
            ask,
            "event-feed",
            feed_page(&id, whole, &CATEGORIES, PAGE),
            NEXT,
        )?;
        served::pages(
            ask,
            "event-feed",
            feed_page(&id, whole, &CATEGORIES, SMALL),
            NEXT,
        )?;
        for category in CATEGORIES {
            ask.ask(
                "event-feed",
                feed_page(&id, whole, &[category], PAGE),
            )?;
        }
        for hour in branch.hours() {
            ask.ask(
                "event-feed",
                feed_page(&id, (None, hour), &CATEGORIES, PAGE),
            )?;
        }
        for (from, to) in branch.spans() {
            ask.ask(
                "event-feed",
                feed_page(&id, (Some(from), to), &CATEGORIES, PAGE),
            )?;
        }
        // A following widget's first page at an hour, then only the rows read after it.
        let hours = branch.hours();
        for (index, &hour) in hours.iter().enumerate() {
            ask.ask(
                "event-feed",
                feed_page(&id, (None, hour), &CATEGORIES, FIRST),
            )?;
            if let Some(&before) = index.checked_sub(1).and_then(|at| hours.get(at)) {
                let mut params = feed_page(&id, (None, hour), &CATEGORIES, FIRST);
                params["after"] = json!(before);
                ask.ask("event-feed", params)?;
            }
        }
    }
    Ok(())
}

fn script_page(branch: &str, (first, last): (Option<i32>, i32), size: u32) -> Value {
    json!({
        "branchId": branch, "firstHour": first, "lastHour": last, "cursor": null,
        "pageSize": size, "kinds": [],
    })
}

fn script(ask: &mut dyn Ask) -> io::Result<()> {
    for branch in ask.branches().to_vec() {
        let id = branch.name();
        let whole = (None, branch.last_hour);
        for product in ["event-choices", "decisions"] {
            served::pages(ask, product, script_page(&id, whole, PAGE), NEXT)?;
            served::pages(ask, product, script_page(&id, whole, SMALL), NEXT)?;
            for hour in branch.hours() {
                ask.ask(product, script_page(&id, (None, hour), PAGE))?;
            }
            for (from, to) in branch.spans() {
                ask.ask(product, script_page(&id, (Some(from), to), PAGE))?;
            }
        }
    }
    Ok(())
}
