//! The event feed's served replies: every page of the branch's feed through
//! its end, with and without observations and in small pages, each category's
//! first page, first pages from each hour and over each span; and the event
//! choices' and decisions' pages likewise.

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
/// A page size whose pages' edges fall all through the feed.
const SMALL: u32 = 7;
const NEXT: &str = "nextCursor";

fn feed_page(
    branch: &str,
    (first, last): (Option<i32>, i32),
    categories: &[&str],
    observations: bool,
    size: u32,
) -> Value {
    json!({
        "branchId": branch, "firstHour": first, "lastHour": last, "categories": categories,
        "observations": observations, "cursor": null, "pageSize": size,
    })
}

fn feed(ask: &mut dyn Ask) -> io::Result<()> {
    for branch in ask.branches().to_vec() {
        let id = branch.name();
        let whole = (None, branch.last_hour);
        served::pages(
            ask,
            "event-feed",
            feed_page(&id, whole, &CATEGORIES, false, PAGE),
            NEXT,
        )?;
        served::pages(
            ask,
            "event-feed",
            feed_page(&id, whole, &CATEGORIES, true, PAGE),
            NEXT,
        )?;
        served::pages(
            ask,
            "event-feed",
            feed_page(&id, whole, &CATEGORIES, false, SMALL),
            NEXT,
        )?;
        for category in CATEGORIES {
            ask.ask(
                "event-feed",
                feed_page(&id, whole, &[category], false, PAGE),
            )?;
        }
        for hour in branch.hours() {
            ask.ask(
                "event-feed",
                feed_page(&id, (None, hour), &CATEGORIES, false, PAGE),
            )?;
        }
        for (from, to) in branch.spans() {
            ask.ask(
                "event-feed",
                feed_page(&id, (Some(from), to), &CATEGORIES, false, PAGE),
            )?;
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
