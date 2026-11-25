
use crate::{
    invalid,
    query::{Identity, Query, SeriesMode},
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
                    if scope == CoverageScope::Row {
                        return Err(invalid("Row coverage is only for an explicit list of countries, not every country"));
                    }
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
            if let (Some(n), false) = (top, parts) {
                if scope == CoverageScope::Row {
                    return Err(invalid("Row coverage is only for keyed non-ranked series reads"));
                }
                if series.key.unowned() {
                    return Err(invalid(format!("{} is rows no country owns: it is not ranked", series.kind)));
                }
                let least = params.get("least").and_then(Value::as_bool) == Some(true);
                return Ok(Query::Series { series, measure, countries: tags, mode: SeriesMode::Top { n, least, population: false }, identity: identity.clone() });
            }
            let mode = if parts {
                if scope == CoverageScope::Row {
                    return Err(invalid("Row coverage is only for keyed non-ranked series reads"));
                }
                SeriesMode::Parts { top }
            } else {
                SeriesMode::Keyed(scope)
            };
            Ok(Query::Series { series, measure, countries: tags, mode, identity: identity.clone() })
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
            let has_top = params.get("top").is_some_and(|top| !top.is_null());
            if scope == CoverageScope::Row && has_top {
                return Err(invalid("Row coverage is only for keyed non-ranked series reads"));
            }
            let top = params.get("top").and_then(Value::as_u64).and_then(|top| usize::try_from(top).ok());
            match (tags.is_empty(), top) {
                (true, Some(n)) if n >= 1 => Ok(Query::series_top(series, measure, n, identity.clone())),
                (false, _) => Ok(Query::Series { series, measure, countries: tags, mode: SeriesMode::Keyed(scope), identity: identity.clone() }),
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
            let has_top = params.get("top").is_some_and(|top| !top.is_null());
            if scope == CoverageScope::Row && has_top {
                return Err(invalid("Row coverage is only for keyed non-ranked series reads"));
            }
            let top = params.get("top").and_then(Value::as_u64).and_then(|top| usize::try_from(top).ok());
            match (tags.is_empty(), top) {
                (true, Some(n)) if n >= 1 => Ok(Query::series_top(series, measure, n, identity.clone())),
                (false, _) => Ok(Query::Series { series, measure, countries: tags, mode: SeriesMode::Keyed(scope), identity: identity.clone() }),
                _ => Err(invalid(format!("A followed {product} names a country or more, or a top of one row or more"))),
            }
        }
        "naval-losses" => {
            use replay_data::rows::NavalLossTotalRow as Row;
            match text(params, "period")? {
                "running" => {}
                "month" => return Err(invalid("Monthly naval losses are not followed at the head")),
                other => return Err(invalid(format!("No naval-losses period {other}"))),
            }
            let attribution = match text(params, "attribution")? {
                "lost" => Row::LOST,
                "sunkBy" => Row::SUNK_BY,
                other => return Err(invalid(format!("No naval-losses attribution {other}"))),
            };
            const SHIPS: &[u8] = &[Row::SHIPS];
            const CONVOYS: &[u8] = &[Row::CONVOYS];
            const BOTH: &[u8] = &[Row::SHIPS, Row::CONVOYS];
            let kinds = match text(params, "kind")? {
                "ships" => SHIPS,
                "convoys" => CONVOYS,
                "both" => BOTH,
                other => return Err(invalid(format!("No naval-losses kind {other}"))),
            };
            let series = dashboard_declare::series("naval-losses").ok_or_else(|| invalid("naval-losses is not declared"))?;
            let measure = measure(series, "total")?;
            let tags = countries(params.get("countries"))?;
            let has_top = params.get("top").is_some_and(|top| !top.is_null());
            if scope == CoverageScope::Row {
                return Err(invalid("Row coverage is only for keyed non-ranked series reads"));
            }
            let top = params.get("top").and_then(Value::as_u64).and_then(|top| usize::try_from(top).ok());
            match (tags.is_empty(), has_top, top) {
                (true, true, Some(n)) if n >= 1 => {
                    let all = identity.holders.tags.keys().map(Country::from).collect();
                    Ok(Query::Series {
                        series,
                        measure,
                        countries: all,
                        mode: SeriesMode::NavalLosses { attribution, kinds, top: Some(n) },
                        identity: identity.clone(),
                    })
                }
                (false, false, _) => Ok(Query::Series {
                    series,
                    measure,
                    countries: tags,
                    mode: SeriesMode::NavalLosses { attribution, kinds, top: None },
                    identity: identity.clone(),
                }),
                _ => Err(invalid("A followed naval-losses names a country or more, or a top of one row or more")),
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
