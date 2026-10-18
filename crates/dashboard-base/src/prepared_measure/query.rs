//! Country balance bindings over the shared prepared series/event reader.
use super::{MeasureActor, MeasureHolder, MeasureRun};
pub use crate::prepared::series::MeasureKey;
use crate::{
    metadata::check_cancel,
    prepared::series::{self, Reader},
    prepared_chart::{ChartSummary, MarkSummary},
    products::country_measure::{BalanceDrop, MEASURE_DECIMALS, Measure},
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{collections::BTreeMap, io, sync::atomic::AtomicBool};
use ts_rs::TS;

/// The sample field a sampled stability measure is dated by: the supplying
/// inputs read's hour, kept beside the measures it dates.
pub const READ_HOUR: &str = "readHour";

#[derive(Clone, Debug, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct MeasureRead {
    pub tag: String,
    pub hour: i32,
}
#[derive(Clone, Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct MeasureState {
    pub tag: String,
    pub hour: i32,
    pub approximate: bool,
    pub sample_hour: Option<i32>,
    pub run: Option<MeasureRun>,
    pub drop: Option<BalanceDrop>,
    pub recorded: Option<bool>,
    pub read_hour: Option<i32>,
    pub experience: Option<crate::products::country_experience::ExperienceRead>,
    pub available: Option<bool>,
    pub segment_first_hour: Option<i32>,
}
#[derive(Clone, Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct MeasureStateReply {
    pub coverage: Option<crate::prepared::coverage::Coverage>,
    pub branch_id: String,
    pub measure: String,
    pub decimals: u8,
    pub states: Vec<MeasureState>,
    pub scale: Option<u32>,
    pub day_hours: Option<i32>,
    pub warnings: Vec<String>,
}
#[derive(Clone, Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct MeasureSummaryLine {
    pub tag: String,
    pub summary: ChartSummary,
    pub marks: Option<MarkSummary>,
}
#[derive(Clone, Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct MeasureSummaryReply {
    pub coverage: Option<crate::prepared::coverage::Coverage>,
    pub branch_id: String,
    pub measure: String,
    pub decimals: u8,
    pub series: Vec<MeasureSummaryLine>,
    pub warnings: Vec<String>,
}
#[derive(Clone, Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct MeasureKeyReply {
    pub coverage: Option<crate::prepared::coverage::Coverage>,
    pub hour: Option<f64>,
    pub approximate: bool,
}
#[derive(Clone, Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct MeasureDropReply {
    pub coverage: Option<crate::prepared::coverage::Coverage>,
    pub first_ordinal: String,
    pub total: String,
    pub rows: Vec<MeasureEvent>,
    pub warnings: Vec<String>,
    pub cursor: Option<String>,
}
#[derive(Clone, Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct MeasureEvent {
    pub tag: String,
    pub hour: i32,
    pub ordinal: String,
    #[ts(type = "unknown")]
    pub row: Value,
}
#[derive(Clone, Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct MeasureMarkReply {
    pub coverage: Option<crate::prepared::coverage::Coverage>,
    pub branch_id: String,
    pub measure: String,
    pub tag: String,
    pub summary: MarkSummary,
    pub warnings: Vec<String>,
}

#[derive(Deserialize)]
#[serde(tag = "kind", deny_unknown_fields)]
enum Family {
    #[serde(rename = "country-measure")]
    Balances { measure: Measure },
    #[serde(rename = "country-stability")]
    Stability { measure: String },
    #[serde(rename = "country-experience")]
    Experience { measure: String },
    #[serde(rename = "stability-parts")]
    Parts { measure: String },
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct SeriesParams {
    branch_id: String,
    family: Family,
    #[serde(default)]
    measures: Vec<String>,
    #[serde(default)]
    within: BTreeMap<String, Vec<crate::products::country_groups::HourSpan>>,
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
    cursor: Option<String>,
    page_size: Option<u32>,
}
pub fn query(
    product: &str,
    params: Value,
    store: &store::Store,
    source_key: &store::SourceKey,
    progressive: Option<crate::prepared::query::progressive::AdapterTail<'_>>,
    cancelled: &AtomicBool,
) -> io::Result<Value> {
    let params: SeriesParams = serde_json::from_value(params)?;
    let parts = matches!(params.family, Family::Parts { .. });
    let experience = matches!(params.family, Family::Experience { .. });
    let stability = matches!(params.family, Family::Stability { .. });
    let decimals = if experience { 0 } else { MEASURE_DECIMALS };
    let (measure, balance) = match &params.family {
        Family::Balances { measure } => (
            match measure {
                Measure::PoliticalPower => "politicalPower",
                Measure::CommandPower => "commandPower",
            }
            .to_owned(),
            true,
        ),
        Family::Stability { measure }
            if ["stability", "warSupport", "maxCommandPower"].contains(&measure.as_str()) =>
        {
            (measure.clone(), false)
        }
        Family::Experience { measure }
            if measure.split_once('-').is_some_and(|(branch, mode)| {
                ["army", "navy", "air"].contains(&branch)
                    && ["banked", "gain", "change"].contains(&mode)
            }) =>
        {
            (measure.clone(), false)
        }
        Family::Parts { measure } if ["stability", "warSupport"].contains(&measure.as_str()) => {
            (measure.clone(), false)
        }
        _ => return Err(crate::invalid("Unknown series measure")),
    };
    let branch = params
        .branch_id
        .parse()
        .map_err(|_| crate::invalid("Invalid series branch"))?;
    let prefix = if parts {
        "parts"
    } else if balance {
        "measure"
    } else if experience {
        "experience"
    } else {
        "stability"
    };
    let use_prepared = progressive.is_none_or(|(_, _, _, _, prepared)| prepared);
    let (keys, rows) = if use_prepared {
        match series::read_catalogue::<Vec<MeasureActor>>(store, source_key, prefix, branch)? {
            Some(catalogue) => catalogue,
            None if progressive.is_some() => Default::default(),
            None => return Err(crate::invalid("Prepared series catalogue is missing")),
        }
    } else {
        Default::default()
    };
    let actors: BTreeMap<_, _> = rows
        .into_iter()
        .map(|actor| ((actor.tag.clone(), actor.measure.clone()), actor))
        .collect();
    let section = if balance {
        format!(
            "measure-{}-{branch}",
            if measure == "politicalPower" {
                "pp"
            } else {
                "cp"
            }
        )
    } else {
        format!("{prefix}-{branch}")
    };
    let mut reader = if use_prepared {
        Reader::new(store, source_key, section, cancelled)?
    } else {
        Reader::from_first_file(store, source_key, section, 0, None, cancelled)
    };
    let segment_section = if balance {
        format!("measure-segments-{branch}")
    } else {
        format!("{prefix}-{branch}")
    };
    let mut segments = if use_prepared {
        Reader::new(store, source_key, segment_section, cancelled)?
    } else {
        Reader::from_first_file(store, source_key, segment_section, 0, None, cancelled)
    };
    let actor = |tag: &str| actors.get(&(tag.to_owned(), measure.clone()));
    let event_actor = |tag: &str| {
        if experience {
            actors.get(&(tag.to_owned(), "experienceRead".into()))
        } else {
            actor(tag)
        }
    };
    let id = |actor: &MeasureActor| {
        keys.get(&series::field_key(&actor.tag, &actor.measure))
            .copied()
            .ok_or_else(|| crate::invalid("Invalid series key"))
    };
    let mut warnings = Vec::new();
    let mut warn = |tag: &str| {
        if progressive.is_none() && actor(tag).is_none_or(|actor| balance && actor.run_count == "0")
        {
            let noun = if experience {
                "experience"
            } else if !balance {
                "stability inputs"
            } else if measure == "politicalPower" {
                "political power"
            } else {
                "command power"
            };
            let tag = if experience {
                serde_json::from_str::<(String, Option<u32>, Option<u32>)>(tag)
                    .map(|row| row.0)
                    .unwrap_or_else(|_| tag.to_owned())
            } else {
                tag.to_owned()
            };
            let warning = format!("{tag}: no recorded {noun} in this branch");
            if !warnings.contains(&warning) {
                warnings.push(warning);
            }
        }
    };
    Ok(match product {
        "series-state" => {
            let mut states = Vec::new();
            for read in params.reads {
                check_cancel(cancelled)?;
                crate::products::validate_tag(&read.tag).map_err(crate::invalid)?;
                let (mut run, drop) = if let Some(actor) = actor(&read.tag) {
                    let key = id(actor)?;
                    let page = if balance {
                        Some(reader.events(&[key], read.hour, read.hour, 0, 1)?)
                    } else {
                        None
                    };
                    (
                        reader
                            .value_at(key, read.hour)?
                            .map(|run| {
                                let (hour, holder) =
                                    segments.latest_event(key, read.hour)?.ok_or_else(|| {
                                        crate::invalid("Missing measure holder segment")
                                    })?;
                                Ok::<_, io::Error>(MeasureRun::from_segment(
                                    run,
                                    hour,
                                    serde_json::from_value::<MeasureHolder>(holder)?,
                                ))
                            })
                            .transpose()?,
                        page.into_iter()
                            .flat_map(|page| page.rows)
                            .next()
                            .map(|row| serde_json::from_value::<BalanceDrop>(row.row))
                            .transpose()?,
                    )
                } else {
                    (None, None)
                };
                let approximate = progressive.is_some_and(|(_, _, spans, _, _)| {
                    spans
                        .iter()
                        .any(|span| span.first_hour <= read.hour && span.last_hour >= read.hour)
                });
                // Stability and experience samples date their values by the
                // same frame's supplying read.
                let sampled_read = progressive
                    .filter(|_| stability || experience)
                    .and_then(|(_, values, _, _, _)| {
                        values
                            .get(&(read.tag.clone(), READ_HOUR.to_owned()))?
                            .iter()
                            .find(|run| run.first_hour <= read.hour && run.last_hour >= read.hour)
                    })
                    .map(|run| {
                        i32::try_from(run.value)
                            .map_err(|_| crate::invalid("Invalid sampled read hour"))
                    })
                    .transpose()?;
                if let Some((_, values, _, identities, _)) = progressive {
                    let lookup = (read.tag.clone(), measure.clone());
                    if let Some(sampled) = values.get(&lookup).and_then(|runs| {
                        runs.iter()
                            .find(|run| run.first_hour <= read.hour && run.last_hour >= read.hour)
                    }) && let Some(&(first, _, slot, country_id)) =
                        identities.get(&lookup).and_then(|spans| {
                            spans.iter().find(|&&(first, last, _, _)| {
                                first <= read.hour && last >= read.hour
                            })
                        })
                    {
                        run = Some(MeasureRun::from_segment(
                            *sampled,
                            first,
                            MeasureHolder {
                                slot,
                                country_id,
                                read_hour: sampled_read,
                                segment_first_hour: None,
                                experience: None,
                            },
                        ));
                    }
                }
                warn(&read.tag);
                let mut segment_first_hour = None;
                let supplying = if experience {
                    if let Some(actor) = event_actor(&read.tag) {
                        let key = id(actor)?;
                        if let Some(run) = reader.value_at(key, read.hour)? {
                            let (_, holder) = segments
                                .latest_event(key, read.hour)?
                                .ok_or_else(|| crate::invalid("Missing experience read"))?;
                            let holder = serde_json::from_value::<MeasureHolder>(holder)?;
                            segment_first_hour = holder.segment_first_hour;
                            holder.experience.map(|mut read| {
                                read.first_hour = run.first_hour;
                                read.last_hour = run.last_hour;
                                read
                            })
                        } else {
                            None
                        }
                    } else {
                        None
                    }
                } else {
                    None
                };
                let read_hour = if experience {
                    supplying
                        .as_ref()
                        .map(|read| read.read_hour)
                        .or(sampled_read)
                } else if sampled_read.is_some() {
                    sampled_read
                } else if !balance {
                    actors
                        .get(&(read.tag.clone(), "readHour".into()))
                        .map(|actor| {
                            reader
                                .value_at(id(actor)?, read.hour)
                                .map(|run| run.map(|run| run.value as i32))
                        })
                        .transpose()?
                        .flatten()
                } else {
                    run.as_ref().and_then(|run| run.read_hour)
                };
                states.push(MeasureState {
                    approximate,
                    sample_hour: approximate
                        .then(|| run.as_ref().map_or(read.hour, |run| run.first_hour)),
                    experience: supplying,
                    segment_first_hour,
                    available: experience
                        .then(|| actor(&read.tag).is_some_and(|actor| actor.run_count != "0")),
                    read_hour,
                    // A transient tail proves a recorded country but never its absence.
                    recorded: match progressive {
                        Some((_, _, _, identities, _))
                            if (stability || experience) && actor(&read.tag).is_none() =>
                        {
                            identities
                                .contains_key(&(read.tag.clone(), READ_HOUR.to_owned()))
                                .then_some(true)
                        }
                        _ => (!balance).then(|| actor(&read.tag).is_some()),
                    },
                    tag: read.tag,
                    hour: read.hour,
                    run,
                    drop,
                });
            }
            serde_json::to_value(MeasureStateReply {
                coverage: None,
                branch_id: params.branch_id,
                measure,
                decimals,
                scale: experience.then_some(crate::products::country_experience::UNITS),
                day_hours: experience.then_some(crate::products::country_experience::DAY_HOURS),
                states,
                warnings,
            })?
        }
        "series-summary" => {
            let (from, to, width) = series::viewport(params.from, params.to, params.width)?;
            series::summary_budget(from, to, width, params.tags.len())?;
            let mut series = Vec::new();
            for tag in params.tags {
                check_cancel(cancelled)?;
                crate::products::validate_tag(&tag).map_err(crate::invalid)?;
                let summary = if let Some((bounds, values, approximate, _, _)) = progressive {
                    let key = match actor(&tag) {
                        Some(actor) => id(actor)?,
                        None => keys
                            .values()
                            .max()
                            .copied()
                            .map_or(Some(0), |key| key.checked_add(1))
                            .ok_or_else(|| crate::invalid("Series key allocation overflow"))?,
                    };
                    let tail = values
                        .get(&(tag.clone(), measure.clone()))
                        .map_or(&[][..], Vec::as_slice);
                    let within = params.within.get(&tag).map(Vec::as_slice);
                    let mut summary = reader.summary_with_tail(
                        key,
                        bounds,
                        (from, to, width),
                        decimals,
                        tail,
                        within,
                    )?;
                    crate::prepared::query::progressive::clip_approximate(
                        &mut summary,
                        approximate,
                        within,
                    );
                    summary
                } else if let Some(actor) = actor(&tag) {
                    reader.summary_within(
                        id(actor)?,
                        (actor.first_hour, actor.last_hour),
                        (from, to, width),
                        decimals,
                        params.within.get(&tag).map(Vec::as_slice),
                    )?
                } else {
                    ChartSummary {
                        from,
                        to,
                        width,
                        buckets: Vec::new(),
                        runs: Some(Vec::new()),
                        approximate: Vec::new(),
                    }
                };
                let marks = if params.with_marks {
                    Some(if let Some(actor) = event_actor(&tag) {
                        (if balance { &mut reader } else { &mut segments }).event_clusters(
                            id(actor)?,
                            from,
                            to,
                            width,
                        )?
                    } else {
                        MarkSummary {
                            from,
                            to,
                            width,
                            clusters: Vec::new(),
                        }
                    })
                } else {
                    None
                };
                warn(&tag);
                series.push(MeasureSummaryLine {
                    tag,
                    summary,
                    marks,
                });
            }
            serde_json::to_value(MeasureSummaryReply {
                coverage: None,
                branch_id: params.branch_id,
                measure,
                decimals,
                series,
                warnings,
            })?
        }
        "series-next-change" => {
            let (first_hour, last_hour) = series::range(params.first_hour, params.last_hour)?;
            let navigation = series::Navigation {
                from: params.from,
                first_hour,
                last_hour,
                key: params
                    .key
                    .ok_or_else(|| crate::invalid("Missing navigation direction"))?,
            };
            let mut keys = Vec::new();
            let selected = if params.measures.is_empty() {
                std::slice::from_ref(&measure)
            } else {
                params.measures.as_slice()
            };
            for tag in &params.tags {
                for selected in selected {
                    if let Some(actor) = actors.get(&(tag.clone(), selected.clone())) {
                        keys.push((id(actor)?, params.within.get(tag).map(Vec::as_slice)));
                    }
                }
            }
            let (hour, approximate) = if let Some((_, values, approximate, _, _)) = progressive {
                let requested = params
                    .tags
                    .iter()
                    .flat_map(|tag| {
                        selected
                            .iter()
                            .map(move |label| (tag.clone(), label.clone()))
                    })
                    .collect::<Vec<_>>();
                let mut prepared = BTreeMap::new();
                for lookup in &requested {
                    if let Some(actor) = actors.get(lookup) {
                        let key = id(actor)?;
                        let hour = series::navigate(
                            &mut [(
                                &mut reader,
                                vec![(key, params.within.get(&lookup.0).map(Vec::as_slice))],
                            )],
                            &navigation,
                        )?;
                        let preceding = values
                            .get(lookup)
                            .and_then(|runs| runs.first())
                            .and_then(|run| run.first_hour.checked_sub(1))
                            .map(|hour| reader.value_at(key, hour))
                            .transpose()?
                            .flatten();
                        prepared.insert(lookup.clone(), (hour, preceding));
                    }
                }
                let hour = crate::prepared::query::progressive::navigate(
                    &prepared,
                    values,
                    &requested,
                    &params.within,
                    &navigation,
                )?;
                let approximate = hour.is_some_and(|hour| {
                    approximate.iter().any(|span| {
                        f64::from(span.first_hour) <= hour && f64::from(span.last_hour) >= hour
                    })
                });
                (hour, approximate)
            } else {
                (
                    series::navigate(&mut [(&mut reader, keys)], &navigation)?,
                    false,
                )
            };
            serde_json::to_value(MeasureKeyReply {
                hour,
                approximate,
                coverage: None,
            })?
        }
        "events-page" => {
            let (from, to, cursor, page_size) = series::event_window(
                params.from,
                params.to,
                params.cursor.as_deref(),
                params.page_size,
            )?;
            let mut keys = BTreeMap::new();
            for tag in params.tags {
                crate::products::validate_tag(&tag).map_err(crate::invalid)?;
                warn(&tag);
                if let Some(actor) = event_actor(&tag) {
                    keys.insert(id(actor)?, tag);
                }
            }
            let events = if balance { &mut reader } else { &mut segments };
            let page = events.events(
                &keys.keys().copied().collect::<Vec<_>>(),
                from,
                to,
                cursor,
                page_size,
            )?;
            let rows = page
                .rows
                .into_iter()
                .map(|row| {
                    Ok(MeasureEvent {
                        tag: keys[&row.key].clone(),
                        hour: row.hour,
                        ordinal: row.ordinal.to_string(),
                        row: row.row,
                    })
                })
                .collect::<io::Result<_>>()?;
            serde_json::to_value(MeasureDropReply {
                coverage: None,
                first_ordinal: page.first.to_string(),
                total: page.total.to_string(),
                rows,
                warnings,
                cursor: page.cursor.map(|next| next.to_string()),
            })?
        }
        "events-clusters" => {
            let (from, to, width) = series::viewport(params.from, params.to, params.width)?;
            let tag = params
                .tag
                .ok_or_else(|| crate::invalid("Missing event key"))?;
            crate::products::validate_tag(&tag).map_err(crate::invalid)?;
            let summary = if let Some(actor) = event_actor(&tag) {
                reader.event_clusters(id(actor)?, from, to, width)?
            } else {
                MarkSummary {
                    from,
                    to,
                    width,
                    clusters: Vec::new(),
                }
            };
            warn(&tag);
            serde_json::to_value(MeasureMarkReply {
                coverage: None,
                branch_id: params.branch_id,
                measure,
                tag,
                summary,
                warnings,
            })?
        }
        _ => return Err(crate::invalid("Unknown series query")),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn single_and_multiple_tags_share_decimal_event_cursors_and_reply_shape() {
        let fixture = crate::prepared::series::tests::Fixture::new();
        let flag = AtomicBool::new(false);
        let mut accumulator = crate::prepared_measure::MeasureAccumulator::new(0);
        let mut rows: Vec<_> = ["alpha", "beta"]
            .into_iter()
            .enumerate()
            .map(|(slot, tag)| replay_data::rows::CountryRow {
                canonical_slot: slot as u32,
                id: slot as u32,
                tag: tag.as_bytes().to_vec(),
                political_power_raw: Some(0),
                ..Default::default()
            })
            .collect();
        let links: Vec<_> = rows
            .iter()
            .map(|row| replay_data::rows::CountryLinkRow {
                canonical_slot: row.canonical_slot,
                id: row.id,
            })
            .collect();
        for hour in 0..4 {
            for row in &mut rows {
                row.political_power_raw = Some(-i64::from(hour));
            }
            accumulator
                .observe_rows(hour, Some((&rows, &links)))
                .unwrap();
        }
        let mut writer = fixture.writer();
        accumulator.flush(&mut writer, 3, &flag).unwrap();
        writer.commit().unwrap();
        for tags in [vec!["alpha"], vec!["beta", "alpha"]] {
            let total = tags.len() * 3;
            let mut cursor = "0".to_owned();
            for index in 0..total {
                let reply = query("events-page", serde_json::json!({
                    "branchId":"3", "family":{"kind":"country-measure","measure":"politicalPower"},
                    "tags":tags, "cursor":cursor, "pageSize":1
                }), &fixture.store, &fixture.key, None, &flag).unwrap();
                assert_eq!(reply["firstOrdinal"], index.to_string());
                assert_eq!(reply["total"], total.to_string());
                assert_eq!(reply["rows"].as_array().unwrap().len(), 1);
                let event = &reply["rows"][0];
                assert_eq!(
                    event["tag"],
                    if index % tags.len() == 0 {
                        "alpha"
                    } else {
                        "beta"
                    }
                );
                assert_eq!(event["hour"], index / tags.len() + 1);
                assert_eq!(event["ordinal"], (index / tags.len()).to_string());
                assert_eq!(event["row"]["hour"], event["hour"]);
                if index + 1 == total {
                    assert!(reply["cursor"].is_null());
                } else {
                    cursor = reply["cursor"].as_str().unwrap().to_owned();
                    assert_eq!(cursor, (index + 1).to_string());
                }
            }
        }
    }

    #[test]
    fn generic_family_queries_keep_exact_values_drop_pages_and_missing_tags() {
        let fixture = crate::prepared::series::tests::Fixture::new();
        let flag = AtomicBool::new(false);
        let mut accumulator = crate::prepared_measure::MeasureAccumulator::new(0);
        let mut rows = vec![replay_data::rows::CountryRow {
            canonical_slot: 7,
            id: 11,
            tag: b"opaque / tag".to_vec(),
            political_power_raw: Some(i64::MAX),
            ..Default::default()
        }];
        let links = [replay_data::rows::CountryLinkRow {
            canonical_slot: 7,
            id: 11,
        }];
        accumulator.observe_rows(0, Some((&rows, &links))).unwrap();
        rows[0].political_power_raw = Some(i64::MIN);
        accumulator.observe_rows(1, Some((&rows, &links))).unwrap();
        let mut writer = fixture.writer();
        accumulator.flush(&mut writer, 3, &flag).unwrap();
        writer.commit().unwrap();
        let family = serde_json::json!({"kind":"country-measure","measure":"politicalPower"});
        let reply = query("series-state", serde_json::json!({"branchId":"3","family":family,"reads":[{"tag":"opaque / tag","hour":1},{"tag":"missing","hour":1}]}), &fixture.store, &fixture.key, None, &flag).unwrap();
        assert_eq!(reply["states"][0]["run"]["valueRaw"], i64::MIN.to_string());
        assert_eq!(reply["states"][0]["run"]["slot"], 7);
        assert_eq!(
            reply["states"][0]["drop"]["beforeRaw"],
            i64::MAX.to_string()
        );
        assert!(reply["states"][1]["run"].is_null());
        assert_eq!(reply["warnings"].as_array().unwrap().len(), 1);
        let reply = query("events-page", serde_json::json!({"branchId":"3","family":family,"tags":["opaque / tag"],"cursor":"0","pageSize":1,"to":1}), &fixture.store, &fixture.key, None, &flag).unwrap();
        assert_eq!(reply["total"], "1");
        assert!(reply["cursor"].is_null());
        assert_eq!(reply["rows"].as_array().unwrap().len(), 1);
        let reply = query("events-page", serde_json::json!({"branchId":"3","family":family,"tags":["opaque / tag"],"pageSize":1,"to":1}), &fixture.store, &fixture.key, None, &flag).unwrap();
        assert_eq!(reply["rows"][0]["tag"], "opaque / tag");
        for params in [
            serde_json::json!({"branchId":"3","family":{"kind":"unsupported"},"reads":[]}),
            serde_json::json!({"branchId":"3","family":family,"tags":["opaque / tag"],"cursor":"bad","pageSize":1}),
        ] {
            assert!(
                query(
                    "events-page",
                    params,
                    &fixture.store,
                    &fixture.key,
                    None,
                    &flag
                )
                .is_err()
            );
        }
    }

    #[test]
    fn measure_provenance_events_resume_without_entering_the_balance_drop_list() {
        let fixture = crate::prepared::series::tests::Fixture::new();
        let flag = AtomicBool::new(false);
        let mut accumulator = crate::prepared_measure::MeasureAccumulator::new(0);
        let mut rows = vec![replay_data::rows::CountryRow {
            canonical_slot: 7,
            id: 11,
            tag: b"holder".to_vec(),
            political_power_raw: Some(0),
            ..Default::default()
        }];
        let mut links = [replay_data::rows::CountryLinkRow {
            canonical_slot: 7,
            id: 11,
        }];
        accumulator.observe_rows(0, Some((&rows, &links))).unwrap();
        rows[0].political_power_raw = Some(-1);
        accumulator.observe_rows(1, Some((&rows, &links))).unwrap();
        rows[0].canonical_slot = 8;
        rows[0].id = 12;
        links[0].canonical_slot = 8;
        links[0].id = 12;
        accumulator.observe_rows(2, Some((&rows, &links))).unwrap();
        rows[0].political_power_raw = None;
        accumulator.observe_rows(3, Some((&rows, &links))).unwrap();
        rows[0].political_power_raw = Some(-1);
        accumulator.observe_rows(4, Some((&rows, &links))).unwrap();
        let mut writer = fixture.writer();
        accumulator.flush(&mut writer, 3, &flag).unwrap();
        writer.commit().unwrap();
        accumulator = serde_json::from_slice(&serde_json::to_vec(&accumulator).unwrap()).unwrap();
        rows[0].political_power_raw = Some(-2);
        accumulator.observe_rows(5, Some((&rows, &links))).unwrap();
        let mut writer = fixture.writer();
        accumulator.flush(&mut writer, 3, &flag).unwrap();
        writer.commit().unwrap();
        let family = serde_json::json!({"kind":"country-measure","measure":"politicalPower"});
        let reply = query("series-state", serde_json::json!({"branchId":"3","family":family,"reads":[{"tag":"holder","hour":2},{"tag":"holder","hour":3},{"tag":"holder","hour":5}]}), &fixture.store, &fixture.key, None, &flag).unwrap();
        assert_eq!(reply["states"][0]["run"]["segmentFirstHour"], 2);
        assert!(reply["states"][1]["run"].is_null());
        assert_eq!(reply["states"][2]["run"]["slot"], 8);
        assert_eq!(reply["states"][2]["run"]["countryId"], 12);
        assert_eq!(reply["states"][2]["run"]["segmentFirstHour"], 4);
        assert_eq!(reply["states"][2]["run"]["startsSegment"], false);
        let reply = query(
            "events-page",
            serde_json::json!({"branchId":"3","family":family,"tags":["holder"],"pageSize":10,"to":5}),
            &fixture.store,
            &fixture.key, None,
            &flag,
        )
        .unwrap();
        assert_eq!(reply["total"], "2");
        assert_eq!(reply["rows"][0]["hour"], 1);
        assert_eq!(reply["rows"][1]["hour"], 5);
    }
}
