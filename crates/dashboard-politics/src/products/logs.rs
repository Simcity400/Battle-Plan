
pub use dashboard_base::products::log_page::{
    LogCaches, LogCell, LogFacetCountAsk, LogPageParams, LogPageReply, LogPageRow, LogTailParams, LogTailReply, dated_tail_panel, missing, page, page_with, quiet_tail_missing, tail,
    tail_with,
};

use dashboard_base::products::{LoadContext, Product};
use dashboard_base::Source;
use dashboard_follow::panel::Panel;
#[cfg(any(test, feature = "export-types"))]
use dashboard_query::log::{CountrySpan, UnobservedSpan};
use dashboard_query::{
    Query,
    log::{self, FacetFilter},
    series::{MissingSpan, hour_of},
};
use serde::{Deserialize, Serialize};
use std::{io, sync::{Arc, atomic::AtomicBool}};
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
    const FIELDS: dashboard_base::products::Fields = dashboard_base::fields![
        branchId,
        log,
        facets,
        from: Start,
        to: End,
        cursor,
        pageSize,
        columns,
        "facets[].countries[]",
        "facets[].values[]",
        "facets[].membership[].country",
        "facets[].membership[].firstHour": Start,
        "facets[].membership[].lastHour": End,
        "facets[].unobserved[].firstHour": Start,
        "facets[].unobserved[].lastHour": End
    ];
    type Reply = LogPageReply;

    #[cfg(any(test, feature = "export-types"))]
    fn types() -> Vec<String> {
        vec![CountrySpan::decl(), UnobservedSpan::decl(), FacetFilter::decl(), LogPageParams::decl(), LogCell::decl(), LogPageRow::decl(), LogPageReply::decl()]
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
        let tagged = params.facets.iter().any(|filter| !filter.countries.is_empty() || !filter.membership.is_empty());
        let caches = dashboard_base::queries::log_caches(source);
        let (page, rows, missing) = page_with(&query, params, Some(&caches), |paged, tag| LogPageRow {
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

fn load_tail_at(query: &Query<'_>, caches: Option<&LogCaches>, params: &LogTailParams, hour: i32, cancelled: &AtomicBool) -> io::Result<LogTailReply> {
    dashboard_text::metadata::check_cancel(cancelled)?;
    let log = declared(&params.log)?;
    let all: Vec<&'static str> = (0..log.table.field_count()).filter_map(|column| log.table.field_name(column)).collect();
    let shown: Vec<usize> = match &params.columns {
        None => (0..all.len()).collect(),
        Some(names) => names.iter().map(|name| dashboard_declare::field(log.table, name)).collect::<io::Result<_>>()?,
    };
    let tagged = params.facets.iter().any(|filter| !filter.countries.is_empty() || !filter.membership.is_empty());
    let offset = params.offset.as_deref().unwrap_or("0").parse::<u64>().map_err(|_| invalid("Invalid offset"))?;
    let (page, rows, counts, missing) = tail_with(query, &params.log, &params.facets, params.from, hour, offset, params.limit, &params.count_facets, caches, |paged, tag| LogPageRow {
        hour: paged.row.hour,
        key: paged.row.key.to_string(),
        ordinal: paged.ordinal.to_string(),
        country: tag.cloned(),
        own_ordinal: tagged.then(|| paged.own.to_string()),
        cells: shown.iter().map(|&column| paged.row.cells.get(column).map_or(LogCell::NotRecorded, cell)).collect(),
    })?;
    let first_ordinal = rows.first().map_or_else(|| page.base.to_string(), |row| row.ordinal.clone());
    Ok(LogTailReply {
        branch_id: params.branch_id.clone(),
        log: params.log.clone(),
        columns: shown.iter().map(|&column| all[column].to_owned()).collect(),
        base_ordinal: page.base.to_string(),
        first_ordinal,
        offset: page.offset.to_string(),
        total: page.total.to_string(),
        rows,
        counts,
        missing,
        warnings: Vec::new(),
    })
}

pub struct LogTail;

impl Product for LogTail {
    const KEY: &'static str = "log-tail";
    const CONTENT_DEPENDENT: bool = false;
    const INTERACTIVE: bool = true;
    type Params = LogTailParams;
    const FIELDS: dashboard_base::products::Fields = dashboard_base::fields![
        branchId,
        log,
        facets,
        from: Start,
        hour: Read,
        offset,
        limit,
        columns,
        countFacets,
        "facets[].countries[]",
        "facets[].values[]",
        "facets[].membership[].country",
        "facets[].membership[].firstHour": Start,
        "facets[].membership[].lastHour": End,
        "facets[].unobserved[].firstHour": Start,
        "facets[].unobserved[].lastHour": End,
        "countFacets[].facets[].countries[]",
        "countFacets[].facets[].values[]",
        "countFacets[].facets[].membership[].country",
        "countFacets[].facets[].membership[].firstHour": Start,
        "countFacets[].facets[].membership[].lastHour": End,
        "countFacets[].facets[].unobserved[].firstHour": Start,
        "countFacets[].facets[].unobserved[].lastHour": End
    ];
    type Reply = LogTailReply;

    #[cfg(any(test, feature = "export-types"))]
    fn types() -> Vec<String> {
        vec![LogFacetCountAsk::decl(), dashboard_base::products::occurrences::OccurrenceCount::decl(), LogTailParams::decl(), LogTailReply::decl()]
    }

    fn validate(params: &Self::Params) -> Result<(), &'static str> {
        params.branch_id.parse::<u64>().map(|_| ()).map_err(|_| "Invalid branch")
    }

    fn load(source: &Source, params: &Self::Params, _context: &LoadContext, cancelled: &AtomicBool) -> io::Result<Self::Reply> {
        let query = dashboard_base::queries::query(source, branch(&params.branch_id)?)?;
        let hour = params.hour.unwrap_or(query.timeline.last);
        let caches = dashboard_base::queries::log_caches(source);
        load_tail_at(&query, Some(&caches), params, hour, cancelled)
    }
}

pub fn dated_panel(source: &Source, _load: &LoadContext, asked: &serde_json::Value) -> Result<Box<dyn Panel>, String> {
    let mut request = asked.clone();
    request.as_object_mut().ok_or("Invalid log-tail panel")?.remove("panel");
    let params: LogTailParams = dashboard_query::named::named(&request).map_err(|error| format!("Invalid log-tail panel: {error}"))?;
    LogTail::validate(&params)?;
    let query = dashboard_base::queries::query(source, branch(&params.branch_id).map_err(|error| error.to_string())?).map_err(|error| error.to_string())?;
    let log = declared(&params.log).map_err(|error| error.to_string())?;
    let caches = dashboard_base::queries::log_caches(source);
    let pre_reader = log::Reader::with_cache(&query, log, Arc::clone(&caches.buckets)).map_err(|error| error.to_string())?;
    let selection = Arc::new(pre_reader.select(&params.facets, query.timeline.first, query.timeline.last).map_err(|error| error.to_string())?);
    let count_selections: Arc<Vec<log::Selection>> = Arc::new(
        params
            .count_facets
            .iter()
            .map(|ask| pre_reader.select(&ask.facets, query.timeline.first, query.timeline.last))
            .collect::<io::Result<Vec<_>>>()
            .map_err(|error| error.to_string())?,
    );
    let (replay, timeline, holders, executable) = (source.replay(), query.timeline, query.holders, source.tables());
    let params = Arc::new(params);
    let q_replay = Arc::clone(&replay);
    let q_timeline = timeline.clone();
    let q_holders = Arc::clone(&holders);
    let q_params = Arc::clone(&params);
    let q_caches = caches.clone();
    Ok(dated_tail_panel(
        executable,
        move |prev_hour, hour, prev_reply, cancelled| {
            dashboard_text::metadata::check_cancel(cancelled)?;
            let query = Query { replay: &q_replay, timeline: q_timeline.clone(), holders: q_holders.clone() };
            quiet_tail_missing(&query, &q_params.log, &q_caches, &selection, &count_selections, &q_params.facets, q_params.from, prev_hour, hour, prev_reply)
        },
        move |hour, cancelled| {
            let query = Query { replay: &replay, timeline: timeline.clone(), holders: holders.clone() };
            load_tail_at(&query, Some(&caches), &params, hour, cancelled)
        },
    ))
}

pub struct LogCounts;

impl Product for LogCounts {
    const KEY: &'static str = "log-counts";
    const CONTENT_DEPENDENT: bool = false;
    type Params = LogCountsParams;
    const FIELDS: dashboard_base::products::Fields = dashboard_base::fields![
        branchId,
        log,
        facets,
        from: Start,
        to: End,
        binHours,
        "facets[].countries[]",
        "facets[].values[]",
        "facets[].membership[].country",
        "facets[].membership[].firstHour": Start,
        "facets[].membership[].lastHour": End,
        "facets[].unobserved[].firstHour": Start,
        "facets[].unobserved[].lastHour": End
    ];
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
        let selection = reader.select(&params.facets, query.timeline.first, to)?;
        let (counts, groups) = (reader.counts_with(&selection.patterns, selection.groups.len(), from, to, params.bin_hours, selection.overlap)?, selection.groups);
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
