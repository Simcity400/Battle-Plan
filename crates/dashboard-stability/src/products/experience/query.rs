//! Experience's replies (series family `country-experience`), read from the family's series
//! publication ([`super::emit`]): a country's measure at hours with the read that supplies it,
//! charts from the country's first hour with its reads as marks, and keyboard navigation over
//! its measures. A country is named by the JSON of its tag, canonical slot and id.

use super::emit::{DECIMALS, MEASURES, READ_HOUR, STARTS, held, measure};
use crate::products::country_stability::query::{measure_run, value_at};
use dashboard_base::{
    prepared::{
        engine,
        series::{self, MeasureKey, Navigation},
    },
    prepared_chart::{ChartSummary, MarkSummary},
    prepared_measure::query::{
        MeasureKeyReply, MeasureRead, MeasureState, MeasureStateReply, MeasureSummaryLine,
        MeasureSummaryReply,
    },
    products::{
        country_experience::{DAY_HOURS, ExperienceRead, UNITS},
        country_groups::HourSpan,
    },
};
use dashboard_series::{PageRead, Snapshot, family::Directory};
use serde::Deserialize;
use serde_json::Value;
use std::{collections::BTreeMap, io, sync::atomic::AtomicBool};

#[derive(Deserialize)]
#[serde(tag = "kind", deny_unknown_fields)]
enum Family {
    #[serde(rename = "country-experience")]
    Experience { measure: String },
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
    #[allow(dead_code, reason = "The family's event pages are not served")]
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

/// A country's actor, by the JSON name a reply asks by.
fn actor(directory: &Directory, tag: &str) -> Option<u64> {
    directory.actor(tag.as_bytes())
}

/// The read supplying `actor`'s measures at `hour`, with the first hour of the segment its
/// holder keeps.
fn supplying<P: PageRead>(
    snapshot: &mut Snapshot<'_, P>,
    directory: &Directory,
    actor: u64,
    hour: i32,
) -> io::Result<Option<(ExperienceRead, Option<i32>)>> {
    let key = |measure| {
        directory
            .key(actor, measure, None)
            .ok_or_else(|| invalid("An experience country lost a key"))
    };
    let Some((run, evidence)) = engine::run_at(snapshot, key(READ_HOUR)?, hour)? else {
        return Ok(None);
    };
    let mut values = [[None; 3]; 3];
    for (row, measures) in values.iter_mut().zip(MEASURES) {
        for (value, measure) in row.iter_mut().zip(measures) {
            *value = value_at(snapshot, key(measure)?, hour)?;
        }
    }
    let starts = value_at(snapshot, key(STARTS)?, hour)?
        .ok_or_else(|| invalid("An experience read lost its stretch start"))?;
    let read_hour =
        i32::try_from(run.value).map_err(|_| invalid("An experience read hour is not an hour"))?;
    Ok(ExperienceRead::from_values(
        (run.span.first, run.span.last),
        read_hour,
        starts != 0,
        values,
    )
    .map(|read| (read, evidence.identity_first)))
}

/// Answers `product` for the `country-experience` series family.
pub fn query<P: PageRead>(
    product: &str,
    params: Value,
    snapshot: &mut Snapshot<'_, P>,
    directory: &Directory,
    cancelled: &AtomicBool,
) -> io::Result<Value> {
    let params: SeriesParams = serde_json::from_value(params)?;
    let Family::Experience { measure: name } = &params.family;
    // A reply charts a branch's banked, gain or change; its reads only navigate.
    let measure = measure(name)
        .filter(|&measure| measure != READ_HOUR)
        .ok_or_else(|| invalid("Unknown series measure"))?;
    let name = name.clone();
    params
        .branch_id
        .parse::<u64>()
        .map_err(|_| invalid("Invalid series branch"))?;
    let last = snapshot.publication().accepted.last;
    let mut warnings = Vec::new();
    let mut warn = |key: &str| {
        if actor(directory, key).is_none() {
            let tag = serde_json::from_str::<(String, Option<u32>, Option<u32>)>(key)
                .map_or_else(|_| key.to_owned(), |key| key.0);
            let warning = format!("{tag}: no recorded experience in this branch");
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
                let found = actor(directory, &read.tag);
                let key = found.and_then(|actor| directory.key(actor, measure, None));
                let run = match key {
                    Some(key) => measure_run(snapshot, key, read.hour)?,
                    None => None,
                };
                warn(&read.tag);
                let (experience, segment_first_hour) = match found {
                    Some(actor) => supplying(snapshot, directory, actor, read.hour)?
                        .map_or((None, None), |(read, first)| (Some(read), first)),
                    None => (None, None),
                };
                let available = match found {
                    Some(actor) => directory.scalar(held(actor, measure)?).is_some(),
                    None => false,
                };
                states.push(MeasureState {
                    approximate: false,
                    sample_hour: None,
                    read_hour: experience.as_ref().map(|read| read.read_hour),
                    experience,
                    segment_first_hour,
                    available: Some(available),
                    recorded: Some(found.is_some()),
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
                scale: Some(UNITS),
                day_hours: Some(DAY_HOURS),
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
                let found = actor(directory, &tag).map(|actor| {
                    (
                        actor,
                        directory.actors[actor as usize].first_hour,
                        directory.key(actor, measure, None),
                        directory.key(actor, READ_HOUR, None),
                    )
                });
                let summary = match found {
                    Some((_, first, Some(key), _)) => engine::summary(
                        snapshot,
                        key,
                        (first, last),
                        (from, to, width),
                        DECIMALS,
                        params.within.get(&tag).map(Vec::as_slice),
                    )?,
                    _ => ChartSummary {
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
                        Some((_, _, _, Some(key))) => {
                            engine::clusters(snapshot, key, (from, to, width))?
                        }
                        _ => MarkSummary {
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
                let Some(actor) = actor(directory, tag) else {
                    continue;
                };
                for measure in &selected {
                    if let Some(key) = super::emit::measure(measure)
                        .and_then(|measure| directory.key(actor, measure, None))
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
        _ => return Err(invalid("Unknown series query")),
    })
}
