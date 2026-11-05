//! Air superiority's served replies, asked as its widgets ask them: the catalogue and the moment
//! windows; the campaign timeline by sides, whose busiest and quietest days pick the hours read
//! beside the branch's spread hours; timelines by sides and by viewpoint, of the top regions and
//! of chosen ones, as following, pinned and ranged widgets ask them; each picked hour's moment
//! window and following day; and the choices a name-ordered moment widget asks of its window.

use dashboard_base::prepared::served::{self, Ask, Branch, Plan};
use serde_json::{Map, Value, json};
use std::io;

pub static PLAN: Plan = Plan {
    queries,
    unordered: &[],
};

const PRODUCT: &str = "air-superiority";
/// The widgets' default region count.
const TOP: u64 = 10;
/// Busiest and quietest days the campaign timeline picks, each.
const DAYS: usize = 3;

fn queries(ask: &mut dyn Ask) -> io::Result<()> {
    for branch in ask.branches().to_vec() {
        queries_of(ask, &branch)?;
    }
    Ok(())
}

/// The members a request names besides its branch and view.
type Params = Map<String, Value>;

fn params(pairs: &[(&str, Value)]) -> Params {
    pairs
        .iter()
        .map(|(key, value)| ((*key).to_owned(), value.clone()))
        .collect()
}

fn request(branch: &Branch, view: &str, mut rest: Params) -> Value {
    rest.insert("branchId".into(), branch.name().into());
    rest.insert("view".into(), view.into());
    Value::Object(rest)
}

fn texts(reply: Option<&Value>, path: &[&str]) -> Vec<String> {
    reply.map_or_else(Vec::new, |reply| served::texts_at(reply, path))
}

fn numbers(reply: Option<&Value>, path: &[&str]) -> Vec<u64> {
    let mut found: Vec<u64> = reply
        .map(|reply| served::values_at(reply, path))
        .unwrap_or_default()
        .into_iter()
        .filter_map(Value::as_u64)
        .collect();
    found.sort_unstable();
    found.dedup();
    found
}

fn hour(value: &Value, key: &str) -> Option<i32> {
    value.get(key)?.as_i64()?.try_into().ok()
}

/// The reply's spans at `path`, as hours.
fn spans(reply: Option<&Value>, path: &[&str]) -> Vec<(i32, i32)> {
    reply
        .map(|reply| served::values_at(reply, path))
        .unwrap_or_default()
        .into_iter()
        .filter_map(|span| Some((hour(span, "firstHour")?, hour(span, "lastHour")?)))
        .collect()
}

/// The first hour of the day holding `at`, as the widgets align a following request.
fn day_start(at: i32, day: i32) -> i32 {
    at.div_euclid(day) * day
}

/// The campaign timeline's busiest days and its quietest, by the cells they hold, as hours: each
/// day's first hour, [`DAYS`] of each, the busiest first.
fn picked_days(timeline: Option<&Value>) -> Vec<i32> {
    let Some(timeline) = timeline else {
        return Vec::new();
    };
    let (Some(first), Some(day)) = (hour(timeline, "firstHour"), hour(timeline, "dayHours"))
    else {
        return Vec::new();
    };
    let days = timeline
        .get("days")
        .and_then(Value::as_array)
        .map_or(0, Vec::len);
    if day <= 0 || days == 0 {
        return Vec::new();
    }
    // Cells run-length code each row's days as `(repeat, value)`: walk them with a day cursor.
    let mut cells = vec![0_usize; days];
    for row in served::values_at(timeline, &["rows", "*", "cells"]) {
        let mut day_at = 0_usize;
        for cell in row.as_array().into_iter().flatten() {
            let repeat = cell.get(0).and_then(Value::as_u64).unwrap_or(1) as usize;
            let held = cell.get(1).is_some_and(|value| !value.is_null());
            let end = (day_at + repeat).min(days);
            if held {
                for count in &mut cells[day_at.min(days)..end] {
                    *count += 1;
                }
            }
            day_at += repeat;
        }
    }
    // Only days the timeline read count as quiet: an unread day is a gap, not a quiet one.
    let read: Vec<usize> = timeline
        .get("days")
        .and_then(Value::as_array)
        .map(|days| {
            days.iter()
                .enumerate()
                .filter(|(_, read)| !read.is_null())
                .map(|(at, _)| at)
                .collect()
        })
        .unwrap_or_default();
    let mut busy: Vec<usize> = read.iter().copied().filter(|&at| cells[at] > 0).collect();
    busy.sort_by_key(|&at| (std::cmp::Reverse(cells[at]), at));
    busy.truncate(DAYS);
    let quiet: Vec<usize> = read.iter().copied().filter(|&at| cells[at] == 0).collect();
    let start = day_start(first, day);
    busy.into_iter()
        .chain(served::spread(&quiet, DAYS))
        .filter_map(|at| i32::try_from(at).ok()?.checked_mul(day)?.checked_add(start))
        .collect()
}

fn queries_of(ask: &mut dyn Ask, branch: &Branch) -> io::Result<()> {
    let (first, last) = (branch.first_hour, branch.last_hour);
    let catalogue = ask.ask(PRODUCT, request(branch, "choices", Params::new()))?;
    let windows_reply = ask.ask(
        PRODUCT,
        request(branch, "choices", params(&[("windowsOnly", true.into())])),
    )?;
    let windows = spans(windows_reply.as_ref(), &["windows", "*"]);
    let regions = numbers(catalogue.as_ref(), &["regions", "*", "id"]);
    let sides = texts(catalogue.as_ref(), &["sides", "*", "key"]);
    let theatres = texts(catalogue.as_ref(), &["theatres", "*", "id"]);
    let countries = texts(catalogue.as_ref(), &["countries", "*"]);

    let top = || ("top", Value::from(TOP));
    let sides_mode = || ("mode", Value::from("sides"));
    let timeline = |from: i32, to: i32, mut rest: Params| {
        rest.insert("firstHour".into(), from.into());
        rest.insert("lastHour".into(), to.into());
        request(branch, "timeline", rest)
    };

    // The campaign widget: every day of the branch, by sides, of the top regions.
    let campaign = ask.ask(
        PRODUCT,
        timeline(first, last, params(&[sides_mode(), top()])),
    )?;
    let day = campaign
        .as_ref()
        .and_then(|reply| hour(reply, "dayHours"))
        .filter(|day| *day > 0)
        .unwrap_or(24);
    let picked = picked_days(campaign.as_ref());
    let unique = |hours: Vec<i32>| {
        let mut kept: Vec<i32> = Vec::with_capacity(hours.len());
        for at in hours {
            if (first..=last).contains(&at) && !kept.contains(&at) {
                kept.push(at);
            }
        }
        kept
    };
    // Moments at the busiest and quietest days and at every spread hour; following timelines,
    // each the whole campaign so far, at those days and a few spread hours.
    let hours = unique(picked.iter().copied().chain(branch.hours()).collect());
    let followed = unique(
        picked
            .iter()
            .copied()
            .chain(served::spread(&branch.hours(), 4))
            .collect(),
    );

    // Viewpoints a widget may take: countries spread over the catalogue, and sides, which read
    // their leader's view.
    let mut viewpoints: Vec<Params> = served::some(&countries)
        .into_iter()
        .map(|tag| {
            params(&[
                ("mode", "viewpoint".into()),
                ("viewpoint", tag.into()),
                ("viewpointCountry", true.into()),
            ])
        })
        .collect();
    viewpoints.extend(
        served::spread(&sides, served::KEYS / 2)
            .into_iter()
            .map(|key| params(&[("mode", "viewpoint".into()), ("viewpoint", key.into())])),
    );
    // Filters a widget may set: two sides compared, chosen regions, one theatre.
    let mut filters: Vec<Params> = Vec::new();
    if sides.len() >= 2 {
        let pair = served::spread(&sides, 2);
        filters.push(params(&[sides_mode(), top(), ("sides", json!(pair))]));
    }
    let chosen = served::spread(&regions, served::KEYS);
    if !chosen.is_empty() {
        filters.push(params(&[sides_mode(), ("regions", json!(chosen))]));
    }
    for theatre in served::spread(&theatres, 2) {
        filters.push(params(&[sides_mode(), top(), ("theatres", json!([theatre]))]));
    }

    // Timelines: following widgets at the picked hours, each request ending on its day's first
    // hour; the viewpoints and filters over the campaign; a ranged widget over the branch's spans.
    let mut follows = Vec::new();
    for &at in &followed {
        let end = day_start(at, day).max(first);
        let reply = ask.ask(
            PRODUCT,
            timeline(first, end, params(&[sides_mode(), top()])),
        )?;
        follows.push((at, reply));
    }
    for viewpoint in &viewpoints {
        let mut rest = viewpoint.clone();
        rest.insert("top".into(), TOP.into());
        ask.ask(PRODUCT, timeline(first, last, rest))?;
    }
    for filter in &filters {
        ask.ask(PRODUCT, timeline(first, last, filter.clone()))?;
    }
    for (from, to) in branch.spans() {
        if first <= from && from <= to && to <= last {
            ask.ask(PRODUCT, timeline(from, to, params(&[sides_mode(), top()])))?;
        }
    }

    // A following timeline's day: the moment of its playhead's day over the regions it drew.
    for (at, reply) in &follows {
        let start = day_start(*at, day).max(first);
        let end = day_start(*at, day)
            .saturating_add(day - 1)
            .min(last)
            .max(start);
        // In the timeline's row order, as the widget names them.
        let drawn: Vec<u64> = reply
            .as_ref()
            .map(|reply| served::values_at(reply, &["rows", "*", "region"]))
            .unwrap_or_default()
            .into_iter()
            .filter_map(Value::as_u64)
            .collect();
        ask.ask(
            PRODUCT,
            request(
                branch,
                "moment",
                params(&[
                    ("firstHour", start.into()),
                    ("lastHour", end.into()),
                    sides_mode(),
                    ("regions", json!(drawn)),
                ]),
            ),
        )?;
    }

    // Moment widgets read whole recorded windows: each picked hour's, by sides; then the windows
    // of the middle and last hours again by each viewpoint and filter, and their choices as a
    // name-ordered widget asks them.
    let window_of = |at: i32| {
        let after = windows.partition_point(|&(from, _)| from <= at);
        after.checked_sub(1).map(|index| windows[index])
    };
    let mut read: Vec<(i32, i32)> = Vec::new();
    for &at in &hours {
        if let Some(window) = window_of(at)
            && !read.contains(&window)
        {
            read.push(window);
        }
    }
    let moment = |(from, to): (i32, i32), mut rest: Params| {
        rest.insert("firstHour".into(), from.into());
        rest.insert("lastHour".into(), to.into());
        request(branch, "moment", rest)
    };
    for &window in &read {
        ask.ask(PRODUCT, moment(window, params(&[sides_mode()])))?;
    }
    let mut held = Vec::new();
    for at in [branch.middle(), last] {
        if let Some(window) = window_of(at)
            && !held.contains(&window)
        {
            held.push(window);
        }
    }
    for &window in &held {
        for rest in viewpoints.iter().chain(&filters) {
            let mut rest = rest.clone();
            rest.remove("top");
            ask.ask(PRODUCT, moment(window, rest))?;
        }
        let (from, to) = window;
        ask.ask(
            PRODUCT,
            request(
                branch,
                "choices",
                params(&[("firstHour", from.into()), ("lastHour", to.into())]),
            ),
        )?;
        if !chosen.is_empty() {
            ask.ask(
                PRODUCT,
                request(
                    branch,
                    "choices",
                    params(&[
                        ("firstHour", from.into()),
                        ("lastHour", to.into()),
                        ("regions", json!(chosen)),
                    ]),
                ),
            )?;
        }
        for theatre in served::spread(&theatres, 2) {
            ask.ask(
                PRODUCT,
                request(
                    branch,
                    "choices",
                    params(&[
                        ("firstHour", from.into()),
                        ("lastHour", to.into()),
                        ("theatres", json!([theatre])),
                    ]),
                ),
            )?;
        }
    }
    Ok(())
}
