use dashboard_query::{Query, log::{self, FacetFilter}, series::{MissingSpan, PAGE_LIMIT, hour_of}};
use replay_columnar::manifest::Status;
use serde::{Deserialize, Serialize};
use std::io;
#[cfg(any(test, feature = "export-types"))]
use ts_rs::TS;

fn invalid(what: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, what.into())
}

fn declared(name: &str) -> io::Result<&'static dashboard_declare::Log> {
    dashboard_declare::log(name).ok_or_else(|| invalid(format!("No declared log {name}")))
}

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct LogPageParams {
    pub branch_id: String,
    pub log: String,
    #[serde(default)]
    pub facets: Vec<FacetFilter>,
    #[doc = " The timeline's first hour when none."]
    pub from: Option<f64>,
    pub to: f64,
    #[doc = " The ordinal the page starts at, counted from the timeline's first hour."]
    pub cursor: Option<String>,
    pub page_size: u32,
    #[doc = " The columns a row carries, by name; every column when none. A followed page pushes these."]
    pub columns: Option<Vec<String>>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum LogCell {
    Number { value: String },
    Text { value: String },
    List { bytes: String },
    Missing,
    NotRecorded,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct LogPageRow {
    pub hour: i32,
    #[doc = " The row's key, as a decimal string."]
    pub key: String,
    #[doc = " Its ordinal among the asked rows from the timeline's first hour."]
    pub ordinal: String,
    #[doc = " The country whose filter it matched, when a facet is filtered by countries."]
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(any(test, feature = "export-types"), ts(optional))]
    pub country: Option<dashboard_query::identity::Country>,
    #[doc = " Its ordinal among its country's rows from the timeline's first hour, with a country."]
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(any(test, feature = "export-types"), ts(optional))]
    pub own_ordinal: Option<String>,
    #[doc = " By the reply's `columns`."]
    pub cells: Vec<LogCell>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct LogPageReply {
    pub branch_id: String,
    pub log: String,
    pub columns: Vec<String>,
    pub first_ordinal: String,
    pub total: String,
    pub cursor: Option<String>,
    pub rows: Vec<LogPageRow>,
    #[doc = " Hours of the range the timeline does not hold, with why."]
    pub missing: Vec<MissingSpan>,
    pub warnings: Vec<String>,
}

crate::reply_warnings!(LogPageReply);

fn unheld(query: &Query<'_>, from: i32, to: i32) -> Vec<MissingSpan> {
    query
        .timeline
        .unheld
        .iter()
        .filter(|gap| gap.last >= from && gap.first <= to)
        .map(|gap| MissingSpan { first_hour: gap.first.max(from), last_hour: gap.last.min(to), reason: gap.reason.clone() })
        .collect()
}

fn unread(query: &Query<'_>, log: &dashboard_declare::Log, from: i32, to: i32) -> io::Result<Vec<MissingSpan>> {
    let recording = query.replay.recording();
    let mut out: Vec<MissingSpan> = Vec::new();
    for stretch in query.timeline.within(from, to) {
        for run in dashboard_query::read::statuses(recording, stretch.run, log.table, stretch.first, stretch.last)? {
            let reason = match run.status {
                Status::Complete => continue,
                Status::NotRecorded => format!("The {} log was not recorded here", log.name),
                Status::Truncated { missing } => format!("The {} log was read in part here ({missing} rows not read)", log.name),
                Status::Unavailable { reason } => {
                    let held = dashboard_query::cost::storage(|| replay_lanes::lanes::at_keys(recording, stretch.run, log.table, run.first, None, replay_lanes::lanes::Keys::These(&[])))?;
                    held.strings.get(reason).map_or_else(|| format!("The {} log was not available here", log.name), |text| String::from_utf8_lossy(text).into_owned())
                }
            };
            match out.last_mut() {
                Some(held) if held.reason == reason && held.last_hour + 1 == run.first => held.last_hour = run.last,
                _ => out.push(MissingSpan { first_hour: run.first, last_hour: run.last, reason }),
            }
        }
    }
    Ok(out)
}

pub fn missing(query: &Query<'_>, log: &dashboard_declare::Log, from: i32, to: i32) -> io::Result<Vec<MissingSpan>> {
    let mut out = unheld(query, from, to);
    out.extend(unread(query, log, from, to)?);
    out.sort_by_key(|span| (span.first_hour, span.last_hour));
    Ok(out)
}

pub fn page<R>(query: &Query<'_>, params: &LogPageParams, mut map: impl FnMut(&log::Paged, Option<&dashboard_query::identity::Country>) -> R) -> io::Result<(log::Page, Vec<R>, Vec<MissingSpan>)> {
    let log = declared(&params.log)?;
    let reader = log::Reader::new(query, log)?;
    let to = hour_of(Some(params.to), "to")?;
    let from = params.from.map(|from| hour_of(Some(from), "from")).transpose()?.unwrap_or(query.timeline.first);
    if !(1..=PAGE_LIMIT).contains(&params.page_size) {
        return Err(invalid("Invalid page size"));
    }
    let cursor = params.cursor.as_deref().unwrap_or("0").parse::<u64>().map_err(|_| invalid("Invalid cursor"))?;
    let selection = reader.select(&params.facets, query.timeline.first, to)?;
    let page = reader.page(&selection.patterns, selection.groups.len(), from, to, cursor, params.page_size)?;
    let rows = page.rows.iter().map(|paged| map(paged, selection.groups[paged.group].as_ref())).collect();
    Ok((page, rows, missing(query, log, from, to)?))
}
