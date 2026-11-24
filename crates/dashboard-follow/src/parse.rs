
use crate::{
    invalid,
    query::{Identity, Query},
};
use dashboard_declare::Series;
use dashboard_query::{coverage::CoverageScope, identity::Country};
use serde_json::Value;
use std::io;

pub fn parse(request: &Value, identity: &Identity) -> io::Result<Query> {
    let product = request.get("product").and_then(Value::as_str).ok_or_else(|| invalid("A followed query names no product"))?;
    let params = request.get("params").ok_or_else(|| invalid("A followed query has no params"))?;
    let scope = match request.get("coverage") {
        None | Some(Value::Null) => CoverageScope::Table,
        Some(Value::String(name)) if name == "table" => CoverageScope::Table,
        Some(Value::String(name)) if name == "row" => CoverageScope::Row,
        _ => return Err(invalid("A followed coverage scope is table or row")),
    };
    match product {
        "series-state" => {
            let family = params.get("family").ok_or_else(|| invalid("A series-state request names no family"))?;
            let kind = text(family, "kind")?;
            let series = dashboard_declare::series(kind).ok_or_else(|| invalid(format!("No declared series {kind}")))?;
            let measure = measure(series, text(family, "measure")?)?;
            let mut tags: Vec<Country> = Vec::new();
            match params.get("reads") {
                Some(Value::String(word)) if word == "every" => {
                    tags = identity.holders.tags.keys().map(|tag| Country::Tag(tag.clone())).collect();
                }
                Some(Value::String(word)) => return Err(invalid(format!("A followed series reads every country or a list of reads, not {word}"))),
                _ => {}
            }
            for read in params.get("reads").and_then(Value::as_array).into_iter().flatten() {
                if read.get("part").is_some_and(|part| !part.is_null()) {
                    return Err(invalid("A followed series follows an owner's parts, all or the top N, not one part"));
                }
                let country = country(read.get("country"))?;
                if !tags.contains(&country) {
                    tags.push(country);
                }
            }
            let top = match params.get("top").filter(|top| !top.is_null()) {
                None => None,
                Some(top) => Some(top.as_u64().and_then(|top| usize::try_from(top).ok()).filter(|top| *top >= 1).ok_or_else(|| invalid("A followed top names one row or more"))?),
            };
            let parts = series.parts.is_some();
            if let (Some(n), false, true) = (top, parts, tags.is_empty()) {
                if scope == CoverageScope::Row {
                    return Err(invalid("Row coverage is only for keyed non-ranked series reads"));
                }
                if series.key.unowned() {
                    return Err(invalid(format!("{} is rows no country owns: it is not ranked", series.kind)));
                }
                let top = Query::series_top(series, measure, n, identity.clone());
                return Ok(if params.get("least").and_then(Value::as_bool) == Some(true) { top.least() } else { top });
            }
            if tags.is_empty() {
                return Err(invalid("A followed series names a country or more"));
            }
            if top.is_some() && !parts {
                return Err(invalid(format!("{} ranks its countries by a top naming no tag", series.kind)));
            }
            if scope == CoverageScope::Row && (top.is_some() || parts) {
                return Err(invalid("Row coverage is only for keyed non-ranked series reads"));
            }
            Ok(Query::Series { series, measure, countries: tags, top, least: false, parts, scope, identity: identity.clone() })
        }
        "log-page" => {
            if scope == CoverageScope::Row {
                return Err(invalid("Row coverage is only for keyed non-ranked series reads"));
            }
            let name = text(params, "log")?;
            let log = dashboard_declare::log(name).ok_or_else(|| invalid(format!("No declared log {name}")))?;
            let mut columns = Vec::new();
            for column in params.get("columns").and_then(Value::as_array).ok_or_else(|| invalid("A followed log page names the columns it pushes"))? {
                let column = column.as_str().ok_or_else(|| invalid("A log's column is named by text"))?;
                columns.push(dashboard_declare::field(log.table, column)?);
            }
            Ok(Query::Cells { table: log.table.index(), columns, rows: crate::Rows::Appended })
        }
        "army-size" => {
            if params.get("byType").and_then(Value::as_bool) == Some(true) {
                return Err(invalid("Army sizes by unit type are not followed at the head"));
            }
            let series = dashboard_declare::series("army-totals").ok_or_else(|| invalid("Army totals are not declared"))?;
            let name = match (text(params, "measure")?, text(params, "countFor")?) {
                ("divisions", "owner") => "ownerDivisions",
                ("strength", "owner") => "ownerStrength",
                ("battalions", "owner") => "ownerBattalions",
                ("manpower", "owner") => "ownerManpower",
                ("divisions", "host") => "hostDivisions",
                ("strength", "host") => "hostStrength",
                ("battalions", "host") => "hostBattalions",
                ("manpower", "host") => "hostManpower",
                (measure, count) => return Err(invalid(format!("No army measure {measure} counted for {count}"))),
            };
            let measure = measure(series, name)?;
            let tags = countries(params.get("countries"))?;
            let top = params.get("top").and_then(Value::as_u64).and_then(|top| usize::try_from(top).ok());
            match (tags.is_empty(), top) {
                (true, Some(n)) if n >= 1 => {
                    if scope == CoverageScope::Row {
                        return Err(invalid("Row coverage is only for keyed non-ranked series reads"));
                    }
                    Ok(Query::series_top(series, measure, n, identity.clone()))
                }
                (false, _) => Ok(Query::Series { series, measure, countries: tags, top: None, least: false, parts: false, scope, identity: identity.clone() }),
                _ => Err(invalid("A followed army size names a country or more, or a top of one row or more")),
            }
        }
        "air-force-size" | "navy-size" => {
            let (kind, name) = match (product, text(params, "measure")?, text(params, "ownership")?) {
                ("air-force-size", "planes", "roster") => ("air-force-totals", "rosterPlanes"),
                ("air-force-size", "wings", "roster") => ("air-force-totals", "rosterWings"),
                ("air-force-size", "planes", "effective") => ("air-force-totals", "effectivePlanes"),
                ("air-force-size", "wings", "effective") => ("air-force-totals", "effectiveWings"),
                ("navy-size", "ships", "roster") => ("navy-totals", "rosterShips"),
                ("navy-size", "strength", "roster") => ("navy-totals", "rosterStrength"),
                ("navy-size", "ships", "logical") => ("navy-totals", "logicalShips"),
                ("navy-size", "strength", "logical") => ("navy-totals", "logicalStrength"),
                (_, measure, ownership) => return Err(invalid(format!("No {product} measure {measure} counted for {ownership}"))),
            };
            let series = dashboard_declare::series(kind).ok_or_else(|| invalid(format!("{kind} is not declared")))?;
            let measure = measure(series, name)?;
            let tags = countries(params.get("countries"))?;
            let top = params.get("top").and_then(Value::as_u64).and_then(|top| usize::try_from(top).ok());
            match (tags.is_empty(), top) {
                (true, Some(n)) if n >= 1 => {
                    if scope == CoverageScope::Row {
                        return Err(invalid("Row coverage is only for keyed non-ranked series reads"));
                    }
                    Ok(Query::series_top(series, measure, n, identity.clone()))
                }
                (false, _) => Ok(Query::Series { series, measure, countries: tags, top: None, least: false, parts: false, scope, identity: identity.clone() }),
                _ => Err(invalid(format!("A followed {product} names a country or more, or a top of one row or more"))),
            }
        }
        other => Err(invalid(format!("{other} is not followed at the head"))),
    }
}

fn text<'a>(value: &'a Value, name: &str) -> io::Result<&'a str> {
    value.get(name).and_then(Value::as_str).ok_or_else(|| invalid(format!("A followed query names no {name}")))
}

fn measure(series: &'static Series, name: &str) -> io::Result<&'static dashboard_declare::Measure> {
    series.measure(name).ok_or_else(|| invalid(format!("{} has no measure {name}", series.kind)))
}

fn country(value: Option<&Value>) -> io::Result<Country> {
    value.and_then(|value| serde_json::from_value(value.clone()).ok()).ok_or_else(|| invalid("A followed read names no country"))
}

fn countries(value: Option<&Value>) -> io::Result<Vec<Country>> {
    match value {
        None | Some(Value::Null) => Ok(Vec::new()),
        Some(value) => serde_json::from_value(value.clone()).map_err(|_| invalid("A followed request's countries are slots or tags")),
    }
}
