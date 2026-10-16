//! When a widget's default view can draw: the requests each registered widget
//! issues on first render, and whether a reply answers them over the whole
//! timeline. The acceptance probe polls these while background preparation runs.
//!
//! A reply is ready when nothing in its requested span waits for preparation:
//! either exact, or answered beyond the accepted prefix by the product's own
//! stateless sampling (its approximate spans are flagged in the reply). A null
//! reply is pending coverage; an event list cut at the accepted prefix is partial.

use serde::Deserialize;
use serde_json::Value;
use std::collections::BTreeMap;

/// The checked-in table of every registered widget's default requests.
pub const WIDGETS: &str = include_str!("readiness/widgets.json");

/// Products the desktop client routes through prepared queries rather than
/// product loads (the frontend's prepared routing list).
pub const UI_PREPARED_ROUTES: &[&str] = &[
    "land-battles",
    "land-battles-manifest",
    "land-battles-ranking",
    "land-battle-course",
    "naval-battles",
    "naval-battles-manifest",
    "naval-battles-ranking",
    "naval-battle-details",
    "division-log",
    "division-log-manifest",
    "division-log-totals",
    "ships-sunk",
    "ships-sunk-manifest",
    "ships-sunk-battle-titles",
    "naval-losses",
    "special-projects",
    "event-feed",
    "politics-ledger",
    "army-size",
    "supply-health",
    "series-summary",
    "military-air-ledger",
    "air-aces",
    "air-ace-portraits",
    "raids",
    "air-superiority",
    "casualty-pairs",
    "equipment-losses",
    "subjects",
    "wars",
    "country-groups",
    "party-history",
    "scoreboard",
    "series-state",
    "unit-leaders",
    "construction-log",
    "construction-log-manifest",
    "construction-spending",
    "resource-balance",
    "trade-flows",
    "production-spending",
    "casualty-breakdown",
    "equipment-stockpile",
    "series-next-change",
    "events-page",
    "events-clusters",
    "occupied-states",
    "occupied-states-manifest",
    "land-owners",
    "espionage-log",
    "operative-placements",
    "operatives-roster",
    "agency-comparison",
    "decryption",
];

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum Route {
    /// A prepared query, answered from accepted pages or the product's sampling.
    Prepared,
    /// A product load: a whole-branch reply computed by the data service.
    Product,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Template {
    pub product: String,
    pub route: Route,
    pub params: Value,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Widget {
    pub kind: String,
    /// What the default view draws from. The widget is ready when all are;
    /// a widget whose default view reads nothing is ready when the recording opens.
    #[serde(default)]
    pub requests: Vec<Template>,
    /// Issued on first render but not needed to draw the default view
    /// (drill countries, group membership for other selections).
    #[serde(default)]
    pub auxiliary: Vec<Template>,
    /// The native map: its first frame at the opening hour.
    #[serde(default)]
    pub map: bool,
    #[serde(default)]
    pub note: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Table {
    widgets: Vec<Widget>,
}

pub fn widgets(text: &str) -> Result<Vec<Widget>, String> {
    let table: Table = serde_json::from_str(text).map_err(|error| error.to_string())?;
    let mut seen = std::collections::BTreeSet::new();
    for widget in &table.widgets {
        if !seen.insert(widget.kind.clone()) {
            return Err(format!("Widget {} is listed twice", widget.kind));
        }
        if widget.map && !widget.requests.is_empty() {
            return Err(format!(
                "Map widget {} reads its first frame, not products",
                widget.kind
            ));
        }
        for (index, template) in widget.requests.iter().enumerate() {
            for dependency in dependencies(&template.params) {
                if dependency >= index {
                    return Err(format!(
                        "Widget {} request {index} depends on a later request",
                        widget.kind
                    ));
                }
            }
        }
        for template in &widget.auxiliary {
            if !dependencies(&template.params).is_empty() {
                return Err(format!(
                    "Widget {} auxiliary requests cannot depend on replies",
                    widget.kind
                ));
            }
        }
    }
    Ok(table.widgets)
}

/// The recording facts a template names.
#[derive(Clone, Debug)]
pub struct Bindings {
    pub branch: String,
    pub tag: String,
    pub first: i32,
    pub last: i32,
    pub width: u32,
}

impl Bindings {
    fn day_start(&self, hour: i32) -> i64 {
        let offset = i64::from(hour) - i64::from(self.first);
        i64::from(self.first) + offset.div_euclid(24) * 24
    }

    fn value(&self, name: &str) -> Option<Value> {
        Some(match name {
            "$branch" => Value::from(self.branch.clone()),
            "$tag" => Value::from(self.tag.clone()),
            "$tags" => Value::from(vec![self.tag.clone()]),
            "$first" => Value::from(self.first),
            "$last" => Value::from(self.last),
            "$width" => Value::from(self.width),
            "$firstDayEnd" => Value::from((i64::from(self.first) + 23).min(i64::from(self.last))),
            "$lastDayStart" => Value::from(self.day_start(self.last)),
            "$lastDayEnd" => {
                Value::from((self.day_start(self.last) + 23).min(i64::from(self.last)))
            }
            _ => return None,
        })
    }
}

/// `$window:N:first|last` and `$windowLast:N:first|last` read request N's
/// `windows` reply: the index (or last hour) of the window holding that hour.
fn dependency(text: &str) -> Option<(usize, bool, bool)> {
    let (kind, rest) = text.split_once(':')?;
    let last_hour = match kind {
        "$window" => false,
        "$windowLast" => true,
        _ => return None,
    };
    let (index, at) = rest.split_once(':')?;
    let at_last = match at {
        "first" => false,
        "last" => true,
        _ => return None,
    };
    Some((index.parse().ok()?, at_last, last_hour))
}

fn dependencies(value: &Value) -> Vec<usize> {
    match value {
        Value::String(text) => dependency(text)
            .map(|(index, ..)| index)
            .into_iter()
            .collect(),
        Value::Array(items) => items.iter().flat_map(dependencies).collect(),
        Value::Object(fields) => fields.values().flat_map(dependencies).collect(),
        _ => Vec::new(),
    }
}

/// Request ordinals this template waits for.
pub fn waits_for(template: &Template) -> Vec<usize> {
    dependencies(&template.params)
}

/// The window index (or that window's last hour) holding `hour` in a reply's
/// ordered `windows`; a window list without that hour has no such request.
fn window_at(reply: &Value, hour: i32, last_hour: bool) -> Option<Value> {
    let windows = reply.get("windows")?.as_array()?;
    let found = windows.iter().position(|window| {
        let first = window.get("firstHour").and_then(Value::as_i64);
        let last = window.get("lastHour").and_then(Value::as_i64);
        matches!((first, last), (Some(first), Some(last)) if first <= i64::from(hour) && i64::from(hour) <= last)
    })?;
    Some(if last_hour {
        windows[found].get("lastHour")?.clone()
    } else {
        Value::from(found)
    })
}

#[derive(Debug, PartialEq, Eq)]
pub enum Instantiated {
    Ready(Value),
    /// A request it depends on has not answered yet.
    Waiting,
    /// The reply it depends on names no window at that hour.
    Absent,
}

/// Substitute bindings and answered dependencies into a template's parameters.
pub fn instantiate(
    params: &Value,
    bindings: &Bindings,
    replies: &BTreeMap<usize, Value>,
) -> Result<Instantiated, String> {
    fn walk(
        value: &Value,
        bindings: &Bindings,
        replies: &BTreeMap<usize, Value>,
    ) -> Result<Instantiated, String> {
        Ok(Instantiated::Ready(match value {
            Value::String(text) if text.starts_with('$') => {
                if let Some((index, at_last, last_hour)) = dependency(text) {
                    let Some(reply) = replies.get(&index) else {
                        return Ok(Instantiated::Waiting);
                    };
                    let hour = if at_last {
                        bindings.last
                    } else {
                        bindings.first
                    };
                    match window_at(reply, hour, last_hour) {
                        Some(value) => value,
                        None => return Ok(Instantiated::Absent),
                    }
                } else {
                    bindings
                        .value(text)
                        .ok_or_else(|| format!("Unknown template binding {text}"))?
                }
            }
            Value::Array(items) => {
                let mut out = Vec::with_capacity(items.len());
                for item in items {
                    match walk(item, bindings, replies)? {
                        Instantiated::Ready(value) => out.push(value),
                        other => return Ok(other),
                    }
                }
                Value::Array(out)
            }
            Value::Object(fields) => {
                let mut out = serde_json::Map::with_capacity(fields.len());
                for (key, item) in fields {
                    match walk(item, bindings, replies)? {
                        Instantiated::Ready(value) => {
                            out.insert(key.clone(), value);
                        }
                        other => return Ok(other),
                    }
                }
                Value::Object(out)
            }
            other => other.clone(),
        }))
    }
    walk(params, bindings, replies)
}

/// How a reply answers its request.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Outcome {
    /// Null: its coverage has not been accepted yet.
    Pending,
    /// An event list cut at the accepted prefix; later events are still to come.
    Partial,
    /// Every requested hour comes from accepted pages or a complete reply.
    Exact,
    /// Accepted pages joined with the product's own transient tail; approximate
    /// when the reply flags keyframe-estimated spans.
    Sampled { approximate: bool },
    /// Still null after preparation finished: the product has nothing to show.
    Unavailable,
}

impl Outcome {
    pub fn ready(self) -> bool {
        matches!(self, Self::Exact | Self::Sampled { .. })
    }

    /// No later preparation can change this reply.
    pub fn settled(self) -> bool {
        self.ready() || self == Self::Unavailable
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Partial => "partial",
            Self::Exact => "exact",
            Self::Sampled { approximate: false } => "sampled-exact",
            Self::Sampled { approximate: true } => "sampled-approximate",
            Self::Unavailable => "unavailable",
        }
    }
}

/// Prepared coverage as replies carry it: `{firstHour, lastHour, complete}` or null.
fn prepared_coverage(value: &Value) -> Option<Option<(i64, bool)>> {
    match value {
        Value::Null => Some(None),
        Value::Object(fields) => {
            let complete = fields.get("complete")?.as_bool()?;
            let last = fields.get("lastHour")?.as_i64()?;
            fields.get("firstHour")?.as_i64()?;
            Some(Some((last, complete)))
        }
        _ => None,
    }
}

fn flags_approximate(value: &Value) -> bool {
    match value {
        Value::Object(fields) => fields.iter().any(|(key, value)| {
            (key == "approximate"
                && match value {
                    Value::Bool(flag) => *flag,
                    Value::Array(spans) => !spans.is_empty(),
                    _ => false,
                })
                || flags_approximate(value)
        }),
        Value::Array(items) => items.iter().any(flags_approximate),
        _ => false,
    }
}

/// The last hour a request's reply must answer, when its parameters name one.
fn terminal_hour(params: &Value) -> Option<i64> {
    let mut last: Option<i64> = None;
    let mut take = |value: Option<&Value>| {
        if let Some(hour) = value
            .and_then(Value::as_f64)
            .filter(|hour| hour.is_finite())
        {
            let hour = hour.ceil() as i64;
            last = Some(last.map_or(hour, |previous| previous.max(hour)));
        }
    };
    for field in ["to", "lastHour", "hour"] {
        take(params.get(field));
    }
    for read in params
        .get("reads")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        take(read.get("hour"));
    }
    last
}

/// Classify a reply's `data` for the request that produced it.
/// `finished` is true once preparation has accepted every block.
pub fn classify(product: &str, params: &Value, data: &Value, finished: bool) -> Outcome {
    if data.is_null() {
        return if finished {
            Outcome::Unavailable
        } else {
            Outcome::Pending
        };
    }
    let approximate = flags_approximate(data);
    let Some(coverage) = data.get("coverage").and_then(prepared_coverage) else {
        // No prepared extent: a complete reply, or a product gated on its own
        // completeness. It can still carry sampled spans of its own.
        return if approximate {
            Outcome::Sampled { approximate }
        } else {
            Outcome::Exact
        };
    };
    if coverage.is_some_and(|(_, complete)| complete) {
        return Outcome::Exact;
    }
    let accepted = coverage.map(|(last, _)| last);
    let within = terminal_hour(params)
        .zip(accepted)
        .is_some_and(|(terminal, accepted)| terminal <= accepted);
    if within && !approximate {
        return Outcome::Exact;
    }
    if matches!(product, "events-page" | "events-clusters") && !within {
        // Event pages and clusters list only accepted events; the tail's
        // events are not sampled, so the list is still growing.
        return Outcome::Partial;
    }
    Outcome::Sampled { approximate }
}

#[cfg(test)]
mod tests;
