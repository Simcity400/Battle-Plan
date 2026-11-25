
pub use dashboard_base::products::log_page::{
    LogCaches, LogCell, LogFacetCountAsk, LogPageRow, LogTailParams, LogTailReply, dated_tail_panel, missing, quiet_tail_missing, tail,
    tail_with,
};

use dashboard_base::products::{LoadContext, Product};
use dashboard_base::Source;
use dashboard_follow::panel::Panel;
#[cfg(any(test, feature = "export-types"))]
use dashboard_query::log::{CountrySpan, FacetFilter, UnobservedSpan};
#[cfg(test)]
use dashboard_query::series::MissingSpan;
use dashboard_query::{Query, log};
use std::{io, sync::{Arc, atomic::AtomicBool}};
#[cfg(any(test, feature = "export-types"))]
use ts_rs::TS;

fn invalid(what: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, what.into())
}

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
        vec![CountrySpan::decl(), UnobservedSpan::decl(), FacetFilter::decl(), LogCell::decl(), LogPageRow::decl(), LogFacetCountAsk::decl(), dashboard_base::products::occurrences::OccurrenceCount::decl(), LogTailParams::decl(), LogTailReply::decl()]
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

#[cfg(test)]
mod tests;
