//! Every declared log's pages and counts with no product of its own ([`dashboard_query::log`]):
//! `log-page` (a page of rows, typed cells by column, with ordinals and the total in range) and
//! `log-counts` (rows per bin, by tag where a facet is filtered by tags: marks and totals). A
//! product with its own row shape calls the same reads and maps each [`log::Row`] itself. At the
//! head a log page is followed as the rows appended since the last push (`dashboard_follow`).

use super::{LoadContext, Product};
use crate::Source;
use dashboard_query::{
    Query,
    log::{self, FacetFilter},
    series::{MissingSpan, PAGE_LIMIT, hour_of},
};
use replay_columnar::manifest::Status;
use serde::{Deserialize, Serialize};
use std::{io, sync::atomic::AtomicBool};
use ts_rs::TS;

fn invalid(what: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, what.into())
}

#[derive(Clone, Debug, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct LogPageParams {
    pub branch_id: String,
    pub log: String,
    #[serde(default)]
    pub facets: Vec<FacetFilter>,
    /// The timeline's first hour when none.
    pub from: Option<f64>,
    pub to: f64,
    /// The ordinal the page starts at, counted from the timeline's first hour.
    pub cursor: Option<String>,
    pub page_size: u32,
    /// The columns a row carries, by name; every column when none. A followed page pushes these.
    pub columns: Option<Vec<String>>,
}

/// A logged cell.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum LogCell {
    /// A number, as a decimal string.
    Number { value: String },
    Text { value: String },
    /// A list cell's bytes, base64.
    List { bytes: String },
    /// An optional number without a value.
    Missing,
    NotRecorded,
}

#[derive(Clone, Debug, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct LogPageRow {
    pub hour: i32,
    /// The row's key, as a decimal string.
    pub key: String,
    /// Its ordinal among the asked rows from the timeline's first hour.
    pub ordinal: String,
    /// The tag whose filter it matched, when a facet is filtered by tags.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub tag: Option<String>,
    /// Its ordinal among its tag's rows from the timeline's first hour, with a tag.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub own_ordinal: Option<String>,
    /// By the reply's `columns`.
    pub cells: Vec<LogCell>,
}

#[derive(Clone, Debug, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct LogPageReply {
    pub branch_id: String,
    pub log: String,
    pub columns: Vec<String>,
    pub first_ordinal: String,
    pub total: String,
    pub cursor: Option<String>,
    pub rows: Vec<LogPageRow>,
    /// Hours of the range the timeline does not hold, with why.
    pub missing: Vec<MissingSpan>,
    pub warnings: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct LogCountsParams {
    pub branch_id: String,
    pub log: String,
    #[serde(default)]
    pub facets: Vec<FacetFilter>,
    pub from: f64,
    pub to: f64,
    /// 1 to 5, or whole six-hour buckets.
    #[ts(type = "number")]
    pub bin_hours: i64,
    /// The countries a row must name, each tag over the hours it counts, for a log whose rows a
    /// scope reads (the feed's); every row when none.
    #[serde(default)]
    pub scope: Option<Vec<super::groups::MembershipRun>>,
}

#[derive(Clone, Debug, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct LogCountLine {
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub tag: Option<String>,
    /// Its rows on the timeline before the range: a mark's ordinals count from it.
    pub before: String,
    pub total: String,
    /// Per bin with rows: its index from `firstHour` and its count.
    #[ts(type = "Array<[number, number]>")]
    pub bins: Vec<(u32, u64)>,
}

#[derive(Clone, Debug, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct LogCountsReply {
    pub branch_id: String,
    pub log: String,
    /// The first hour of bin 0.
    pub first_hour: i32,
    #[ts(type = "number")]
    pub bin_hours: i64,
    pub lines: Vec<LogCountLine>,
    pub missing: Vec<MissingSpan>,
    pub warnings: Vec<String>,
}

dashboard_base::reply_warnings!(LogPageReply, LogCountsReply);

fn branch(text: &str) -> io::Result<u64> {
    text.parse().map_err(|_| invalid("Invalid branch"))
}

fn declared(name: &str) -> io::Result<&'static dashboard_declare::Log> {
    dashboard_declare::log(name).ok_or_else(|| invalid(format!("No declared log {name}")))
}

/// The hours of `from..=to` the timeline does not hold, with why.
fn unheld(query: &Query<'_>, from: i32, to: i32) -> Vec<MissingSpan> {
    query
        .timeline
        .unheld
        .iter()
        .filter(|gap| gap.last >= from && gap.first <= to)
        .map(|gap| MissingSpan { first_hour: gap.first.max(from), last_hour: gap.last.min(to), reason: gap.reason.clone() })
        .collect()
}

/// The hours of `from..=to` the timeline holds where `log`'s table was not recorded whole, each
/// with the recording's reason: a log not read or not derived there is missing, never no rows.
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
                    let held = dashboard_query::cost::storage(|| replay::lanes::at_keys(recording, stretch.run, log.table, run.first, None, replay::lanes::Keys::These(&[])))?;
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

/// What `from..=to` of `log` does not hold, in hour order: the hours no run holds and the hours its
/// table was not recorded whole.
pub(crate) fn missing(query: &Query<'_>, log: &dashboard_declare::Log, from: i32, to: i32) -> io::Result<Vec<MissingSpan>> {
    let mut out = unheld(query, from, to);
    out.extend(unread(query, log, from, to)?);
    out.sort_by_key(|span| (span.first_hour, span.last_hour));
    Ok(out)
}

fn cell(held: &log::Cell) -> LogCell {
    match held {
        log::Cell::Number(value) => LogCell::Number { value: value.to_string() },
        log::Cell::Text(value) => LogCell::Text { value: value.clone() },
        log::Cell::List(bytes) => LogCell::List { bytes: dashboard_query::tile::base64(bytes) },
        log::Cell::Missing => LogCell::Missing,
        log::Cell::Unrecorded => LogCell::NotRecorded,
    }
}

/// A page of `params.log`'s rows, each mapped by `map` (a product's own row shape).
pub fn page<R>(query: &Query<'_>, params: &LogPageParams, mut map: impl FnMut(&log::Paged, Option<&str>) -> R) -> io::Result<(log::Page, Vec<R>, Vec<MissingSpan>)> {
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
    let rows = page.rows.iter().map(|paged| map(paged, selection.groups[paged.group].as_deref())).collect();
    Ok((page, rows, missing(query, log, from, to)?))
}

pub struct LogPage;

impl Product for LogPage {
    const KEY: &'static str = "log-page";
    const CONTENT_DEPENDENT: bool = false;
    type Params = LogPageParams;
    type Reply = LogPageReply;

    fn types() -> Vec<String> {
        vec![FacetFilter::decl(), LogPageParams::decl(), LogCell::decl(), LogPageRow::decl(), LogPageReply::decl()]
    }

    fn validate(params: &Self::Params) -> Result<(), &'static str> {
        params.branch_id.parse::<u64>().map(|_| ()).map_err(|_| "Invalid branch")
    }

    fn load(source: &Source, params: &Self::Params, _context: &LoadContext, cancelled: &AtomicBool) -> io::Result<Self::Reply> {
        crate::metadata::check_cancel(cancelled)?;
        let query = crate::answer::query(source, branch(&params.branch_id)?)?;
        let log = declared(&params.log)?;
        let all: Vec<&'static str> = (0..log.table.field_count()).filter_map(|column| log.table.field_name(column)).collect();
        let shown: Vec<usize> = match &params.columns {
            None => (0..all.len()).collect(),
            Some(names) => names.iter().map(|name| dashboard_declare::field(log.table, name)).collect::<io::Result<_>>()?,
        };
        let tagged = params.facets.iter().any(|filter| !filter.tags.is_empty());
        let (page, rows, missing) = page(&query, params, |paged, tag| LogPageRow {
            hour: paged.row.hour,
            key: paged.row.key.to_string(),
            ordinal: paged.ordinal.to_string(),
            tag: tag.map(str::to_owned),
            own_ordinal: tagged.then(|| paged.own.to_string()),
            cells: shown.iter().map(|&column| paged.row.cells.get(column).map_or(LogCell::NotRecorded, cell)).collect(),
        })?;
        Ok(LogPageReply {
            branch_id: params.branch_id.clone(),
            log: params.log.clone(),
            columns: shown.iter().map(|&column| all[column].to_owned()).collect(),
            first_ordinal: page.first.to_string(),
            total: page.total().to_string(),
            cursor: page.cursor().map(|cursor| cursor.to_string()),
            rows,
            missing,
            warnings: Vec::new(),
        })
    }
}

pub struct LogCounts;

impl Product for LogCounts {
    const KEY: &'static str = "log-counts";
    const CONTENT_DEPENDENT: bool = false;
    type Params = LogCountsParams;
    type Reply = LogCountsReply;

    fn types() -> Vec<String> {
        vec![FacetFilter::decl(), LogCountsParams::decl(), LogCountLine::decl(), LogCountsReply::decl()]
    }

    fn validate(params: &Self::Params) -> Result<(), &'static str> {
        params.branch_id.parse::<u64>().map(|_| ()).map_err(|_| "Invalid branch")
    }

    fn load(source: &Source, params: &Self::Params, _context: &LoadContext, cancelled: &AtomicBool) -> io::Result<Self::Reply> {
        crate::metadata::check_cancel(cancelled)?;
        let query = crate::answer::query(source, branch(&params.branch_id)?)?;
        let log = declared(&params.log)?;
        let reader = log::Reader::new(&query, log)?;
        let (from, to) = (hour_of(Some(params.from), "from")?, hour_of(Some(params.to), "to")?);
        if from > to {
            return Err(invalid("Invalid span"));
        }
        let (counts, groups) = match &params.scope {
            None => {
                let selection = reader.select(&params.facets, query.timeline.first, to)?;
                (reader.counts(&selection.patterns, selection.groups.len(), from, to, params.bin_hours)?, selection.groups)
            }
            Some(_) if log.name != super::feed::LOG => return Err(invalid(format!("The {} log cannot be kept to a country scope", log.name))),
            Some(runs) => (vec![super::feed::scope::counts(source, &query, &params.facets, runs, from, to, params.bin_hours)?], vec![None]),
        };
        Ok(LogCountsReply {
            branch_id: params.branch_id.clone(),
            log: params.log.clone(),
            first_hour: (i64::from(from).div_euclid(params.bin_hours) * params.bin_hours) as i32,
            bin_hours: params.bin_hours,
            lines: counts
                .into_iter()
                .zip(groups)
                .map(|(counts, tag)| LogCountLine {
                    tag,
                    before: counts.before.to_string(),
                    total: counts.bins.iter().map(|(_, count)| count).sum::<u64>().to_string(),
                    bins: counts.bins,
                })
                .collect(),
            missing: missing(&query, log, from, to)?,
            warnings: Vec::new(),
        })
    }
}

#[cfg(test)]
mod tests;
