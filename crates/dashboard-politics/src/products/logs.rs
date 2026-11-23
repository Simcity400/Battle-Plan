
pub use dashboard_base::products::log_page::{LogCell, LogPageParams, LogPageReply, LogPageRow, missing, page};

use dashboard_base::products::{LoadContext, Product};
use dashboard_base::Source;
use dashboard_query::{
    log::{self, FacetFilter},
    series::{MissingSpan, hour_of},
};
use serde::{Deserialize, Serialize};
use std::{io, sync::atomic::AtomicBool};
#[cfg(any(test, feature = "export-types"))]
use ts_rs::TS;

fn invalid(what: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, what.into())
}

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct LogCountsParams {
    pub branch_id: String,
    pub log: String,
    #[serde(default)]
    pub facets: Vec<FacetFilter>,
    pub from: f64,
    pub to: f64,
    #[doc = " 1 to 5, or whole six-hour buckets."]
    #[cfg_attr(any(test, feature = "export-types"), ts(type = "number"))]
    pub bin_hours: i64,
    #[doc = " The countries a row must name, each tag over the hours it counts, for a log whose rows a"]
    #[doc = " scope reads (the feed's); every row when none."]
    #[serde(default)]
    pub scope: Option<Vec<dashboard_world::products::groups::MembershipRun>>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct LogCountLine {
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(any(test, feature = "export-types"), ts(optional))]
    pub country: Option<dashboard_base::products::Country>,
    #[doc = " Its rows on the timeline before the range: a mark's ordinals count from it."]
    pub before: String,
    pub total: String,
    #[doc = " Per bin with rows: its index from `firstHour` and its count."]
    #[cfg_attr(any(test, feature = "export-types"), ts(type = "Array<[number, number]>"))]
    pub bins: Vec<(u32, u64)>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct LogCountsReply {
    pub branch_id: String,
    pub log: String,
    #[doc = " The first hour of bin 0."]
    pub first_hour: i32,
    #[cfg_attr(any(test, feature = "export-types"), ts(type = "number"))]
    pub bin_hours: i64,
    pub lines: Vec<LogCountLine>,
    pub missing: Vec<MissingSpan>,
    pub warnings: Vec<String>,
}

dashboard_base::reply_warnings!(LogCountsReply);

fn branch(text: &str) -> io::Result<u64> {
    text.parse().map_err(|_| invalid("Invalid branch"))
}

fn declared(name: &str) -> io::Result<&'static dashboard_declare::Log> {
    dashboard_declare::log(name).ok_or_else(|| invalid(format!("No declared log {name}")))
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

pub struct LogPage;

impl Product for LogPage {
    const KEY: &'static str = "log-page";
    const CONTENT_DEPENDENT: bool = false;
    type Params = LogPageParams;
    const FIELDS: dashboard_base::products::Fields = dashboard_base::fields![branchId, log, facets, from: Start, to: End, cursor, pageSize, columns, "facets[].countries[]", "facets[].values[]"];
    type Reply = LogPageReply;

    #[cfg(any(test, feature = "export-types"))]
    fn types() -> Vec<String> {
        vec![FacetFilter::decl(), LogPageParams::decl(), LogCell::decl(), LogPageRow::decl(), LogPageReply::decl()]
    }

    fn validate(params: &Self::Params) -> Result<(), &'static str> {
        params.branch_id.parse::<u64>().map(|_| ()).map_err(|_| "Invalid branch")
    }

    fn load(source: &Source, params: &Self::Params, _context: &LoadContext, cancelled: &AtomicBool) -> io::Result<Self::Reply> {
        dashboard_text::metadata::check_cancel(cancelled)?;
        let query = dashboard_base::queries::query(source, branch(&params.branch_id)?)?;
        let log = declared(&params.log)?;
        let all: Vec<&'static str> = (0..log.table.field_count()).filter_map(|column| log.table.field_name(column)).collect();
        let shown: Vec<usize> = match &params.columns {
            None => (0..all.len()).collect(),
            Some(names) => names.iter().map(|name| dashboard_declare::field(log.table, name)).collect::<io::Result<_>>()?,
        };
        let tagged = params.facets.iter().any(|filter| !filter.countries.is_empty());
        let (page, rows, missing) = page(&query, params, |paged, tag| LogPageRow {
            hour: paged.row.hour,
            key: paged.row.key.to_string(),
            ordinal: paged.ordinal.to_string(),
            country: tag.cloned(),
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
    const FIELDS: dashboard_base::products::Fields = dashboard_base::fields![branchId, log, facets, from: Start, to: End, binHours, "scope[].firstHour": Start, "scope[].lastHour": End, "facets[].countries[]", "facets[].values[]"];
    type Reply = LogCountsReply;

    #[cfg(any(test, feature = "export-types"))]
    fn types() -> Vec<String> {
        vec![FacetFilter::decl(), LogCountsParams::decl(), LogCountLine::decl(), LogCountsReply::decl()]
    }

    fn validate(params: &Self::Params) -> Result<(), &'static str> {
        params.branch_id.parse::<u64>().map(|_| ()).map_err(|_| "Invalid branch")
    }

    fn load(source: &Source, params: &Self::Params, _context: &LoadContext, cancelled: &AtomicBool) -> io::Result<Self::Reply> {
        dashboard_text::metadata::check_cancel(cancelled)?;
        let query = dashboard_base::queries::query(source, branch(&params.branch_id)?)?;
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
                .map(|(counts, country)| LogCountLine {
                    country,
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
