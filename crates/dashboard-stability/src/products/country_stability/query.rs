//! Stability's measure replies (series family `country-stability`), read from the family's
//! series publication ([`super::emit`]): values with their holder and supplying read at
//! hours, charts from each country's first hour with the holder changes as marks, and
//! keyboard navigation over the measures.

use super::emit::{DECIMALS, READ_HOUR, measure};
use dashboard_base::{
    prepared::{
        engine,
        series::{self, MeasureKey, Navigation},
    },
    prepared_chart::{ChartSummary, MarkSummary},
    prepared_measure::{
        MeasureRun,
        query::{
            MeasureKeyReply, MeasureMarkReply, MeasureRead, MeasureState, MeasureStateReply,
            MeasureSummaryLine, MeasureSummaryReply,
        },
    },
    products::country_groups::HourSpan,
};
use dashboard_series::{PageRead, Snapshot, family::Directory};
use serde::Deserialize;
use serde_json::Value;
use std::{collections::BTreeMap, io, sync::atomic::AtomicBool};

#[derive(Deserialize)]
#[serde(tag = "kind", deny_unknown_fields)]
enum Family {
    #[serde(rename = "country-stability")]
    Stability { measure: String },
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct SeriesParams {
    branch_id: String,
    family: Family,
    #[serde(default)]
    measures: Vec<String>,
    #[serde(default)]
    within: BTreeMap<String, Vec<HourSpan>>,
    #[serde(default)]
    tags: Vec<String>,
    #[serde(default)]
    reads: Vec<MeasureRead>,
    tag: Option<String>,
    from: Option<f64>,
    to: Option<f64>,
    width: Option<u32>,
    key: Option<MeasureKey>,
    first_hour: Option<f64>,
    last_hour: Option<f64>,
    #[serde(default)]
    with_marks: bool,
    #[serde(default)]
    #[allow(dead_code, reason = "The family's event pages are not served")]
    cursor: Option<String>,
    #[serde(default)]
    #[allow(dead_code, reason = "The family's event pages are not served")]
    page_size: Option<u32>,
}

fn invalid(message: &'static str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}

/// A country's key of `measure`, and its first hour.
fn key(directory: &Directory, tag: &str, measure: u64) -> Option<(u64, i32)> {
    let actor = directory.actor(tag.as_bytes())?;
    Some((
        directory.key(actor, measure, None)?,
        directory.actors[actor as usize].first_hour,
    ))
}

/// The run of `key` at `hour` with the holder its evidence names.
pub fn measure_run<P: PageRead>(
    snapshot: &mut Snapshot<'_, P>,
    key: u64,
    hour: i32,
) -> io::Result<Option<MeasureRun>> {
    let Some((run, evidence)) = engine::run_at(snapshot, key, hour)? else {
        return Ok(None);
    };
    let holder = evidence
        .holder
        .ok_or_else(|| invalid("Missing measure holder segment"))?;
    Ok(Some(MeasureRun {
        first_hour: run.span.first,
        last_hour: run.span.last,
        value_raw: run.value.to_string(),
        slot: (holder >> 32) as u32,
        country_id: holder as u32,
        starts_segment: run.break_before,
        segment_first_hour: evidence.identity_first.unwrap_or(evidence.span.first),
        read_hour: evidence.supplying_read,
    }))
}

/// The amount `key` holds at `hour`.
pub fn value_at<P: PageRead>(
    snapshot: &mut Snapshot<'_, P>,
    key: u64,
    hour: i32,
) -> io::Result<Option<i128>> {
    Ok(engine::run_at(snapshot, key, hour)?.map(|(run, _)| run.value))
}

/// Each tag's `measure` and supplying read hour at `hour`, for the scoreboard; None where
/// the tag holds no value.
pub fn cells<P: PageRead>(
    snapshot: &mut Snapshot<'_, P>,
    directory: &Directory,
    tags: &[&str],
    measure_name: &str,
    hour: i32,
) -> io::Result<Vec<(Option<i128>, Option<i128>)>> {
    let measure = measure(measure_name).ok_or_else(|| invalid("Unknown stability measure"))?;
    tags.iter()
        .map(|tag| {
            let amount = match key(directory, tag, measure) {
                Some((key, _)) => value_at(snapshot, key, hour)?,
                None => None,
            };
            let read = match key(directory, tag, READ_HOUR) {
                Some((key, _)) => value_at(snapshot, key, hour)?,
                None => None,
            };
            Ok((amount, read))
        })
        .collect()
}

/// Answers `product` for the `country-stability` series family.
pub fn query<P: PageRead>(
    product: &str,
    params: Value,
    snapshot: &mut Snapshot<'_, P>,
    directory: &Directory,
    cancelled: &AtomicBool,
) -> io::Result<Value> {
    let params: SeriesParams = serde_json::from_value(params)?;
    let Family::Stability { measure: name } = &params.family;
    let measure = match name.as_str() {
        "stability" | "warSupport" | "maxCommandPower" => {
            super::emit::measure(name).expect("a displayed measure")
        }
        _ => return Err(invalid("Unknown series measure")),
    };
    let name = name.clone();
    params
        .branch_id
        .parse::<u64>()
        .map_err(|_| invalid("Invalid series branch"))?;
    let last = snapshot.publication().accepted.last;
    let mut warnings = Vec::new();
    let mut warn = |tag: &str| {
        if directory.actor(tag.as_bytes()).is_none() {
            let warning = format!("{tag}: no recorded stability inputs in this branch");
            if !warnings.contains(&warning) {
                warnings.push(warning);
            }
        }
    };
    Ok(match product {
        "series-state" => {
            let mut states = Vec::with_capacity(params.reads.len());
            for read in params.reads {
                dashboard_text::metadata::check_cancel(cancelled)?;
                dashboard_core::protocol::validate_tag(&read.tag).map_err(invalid)?;
                let run = match key(directory, &read.tag, measure) {
                    Some((key, _)) => measure_run(snapshot, key, read.hour)?,
                    None => None,
                };
                warn(&read.tag);
                let read_hour = match key(directory, &read.tag, READ_HOUR) {
                    Some((key, _)) => value_at(snapshot, key, read.hour)?.map(|hour| hour as i32),
                    None => None,
                };
                states.push(MeasureState {
                    approximate: false,
                    sample_hour: None,
                    experience: None,
                    segment_first_hour: None,
                    available: None,
                    read_hour,
                    recorded: Some(directory.actor(read.tag.as_bytes()).is_some()),
                    tag: read.tag,
                    hour: read.hour,
                    run,
                    drop: None,
                });
            }
            serde_json::to_value(MeasureStateReply {
                coverage: None,
                branch_id: params.branch_id,
                measure: name,
                decimals: DECIMALS,
                scale: None,
                day_hours: None,
                states,
                warnings,
            })?
        }
        "series-summary" => {
            let (from, to, width) = series::viewport(params.from, params.to, params.width)?;
            series::summary_budget(from, to, width, params.tags.len())?;
            let mut lines = Vec::with_capacity(params.tags.len());
            for tag in params.tags {
                dashboard_text::metadata::check_cancel(cancelled)?;
                dashboard_core::protocol::validate_tag(&tag).map_err(invalid)?;
                let found = key(directory, &tag, measure);
                let summary = match found {
                    Some((key, first)) => engine::summary(
                        snapshot,
                        key,
                        (first, last),
                        (from, to, width),
                        DECIMALS,
                        params.within.get(&tag).map(Vec::as_slice),
                    )?,
                    None => ChartSummary {
                        from,
                        to,
                        width,
                        buckets: Vec::new(),
                        runs: Some(Vec::new()),
                        approximate: Vec::new(),
                    },
                };
                let marks = if params.with_marks {
                    Some(match found {
                        Some((key, _)) => engine::clusters(snapshot, key, (from, to, width))?,
                        None => MarkSummary {
                            from,
                            to,
                            width,
                            clusters: Vec::new(),
                        },
                    })
                } else {
                    None
                };
                warn(&tag);
                lines.push(MeasureSummaryLine {
                    tag,
                    summary,
                    marks,
                });
            }
            serde_json::to_value(MeasureSummaryReply {
                coverage: None,
                branch_id: params.branch_id,
                measure: name,
                decimals: DECIMALS,
                series: lines,
                warnings,
            })?
        }
        "series-next-change" => {
            let (first_hour, last_hour) = series::range(params.first_hour, params.last_hour)?;
            let navigation = Navigation {
                from: params.from,
                first_hour,
                last_hour,
                key: params
                    .key
                    .ok_or_else(|| invalid("Missing navigation direction"))?,
            };
            let selected = if params.measures.is_empty() {
                vec![name.clone()]
            } else {
                params.measures.clone()
            };
            let mut keys = Vec::new();
            for tag in &params.tags {
                for measure in &selected {
                    if let Some((key, _)) = super::emit::measure(measure)
                        .and_then(|measure| key(directory, tag, measure))
                    {
                        keys.push((key, params.within.get(tag).map(Vec::as_slice)));
                    }
                }
            }
            serde_json::to_value(MeasureKeyReply {
                hour: engine::navigate(snapshot, &keys, &navigation)?,
                approximate: false,
                coverage: None,
            })?
        }
        "events-clusters" => {
            let (from, to, width) = series::viewport(params.from, params.to, params.width)?;
            let tag = params.tag.ok_or_else(|| invalid("Missing event key"))?;
            dashboard_core::protocol::validate_tag(&tag).map_err(invalid)?;
            let summary = match key(directory, &tag, measure) {
                Some((key, _)) => engine::clusters(snapshot, key, (from, to, width))?,
                None => MarkSummary {
                    from,
                    to,
                    width,
                    clusters: Vec::new(),
                },
            };
            warn(&tag);
            serde_json::to_value(MeasureMarkReply {
                coverage: None,
                branch_id: params.branch_id,
                measure: name,
                tag,
                summary,
                warnings,
            })?
        }
        _ => return Err(invalid("Unknown series query")),
    })
}
