use super::*;
use dashboard_base::prepared::query::{keyframes, progressive};
use dashboard_base::prepared::series::{self, MeasureKey, Navigation, Reader};
use dashboard_base::prepared_chart::{ChartSummary, merge};
use serde_json::Value;
use std::{cell::RefCell, collections::BTreeMap};

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Requested {
    from: Option<f64>,
    to: Option<f64>,
    width: Option<u32>,
    #[serde(default)]
    reads: Vec<Read>,
    #[serde(default)]
    within: BTreeMap<String, Vec<(String, i32, i32, bool)>>,
    tag: Option<String>,
}
#[derive(Deserialize)]
struct Read {
    hour: i32,
}

#[derive(Serialize, Deserialize)]
struct Observation {
    scan: ScannedAutonomy,
    #[serde(with = "dashboard_core::map_pairs")]
    values: progressive::Samples,
}

/// Bind the widget's native request to the shared progressive reader. The
/// existing prepared projection remains the prefix; transient observations
/// use the same level/overlord observer and reply builder without publication.
pub fn progressive_query(
    source: &Source,
    product: &str,
    value: &Value,
    store: &store::Store,
    key: &store::SourceKey,
    context: &LoadContext,
    cancelled: &AtomicBool,
) -> io::Result<Value> {
    let params: AutonomyParams = serde_json::from_value(serde_json::json!({
        "branchId":value["branchId"],"subjects":value.get("subjects").unwrap_or(&serde_json::json!([])),
        "overlords":value.get("overlords").unwrap_or(&serde_json::json!([])),
    }))?;
    Autonomy::validate(&params).map_err(dashboard_core::invalid)?;
    let requested: Requested = serde_json::from_value(value.clone())?;
    let branch = params.branch_id.parse().expect("validated autonomy branch");
    // Its family's coverage for the hours the request names, ending before its first hole.
    let accepted = match dashboard_base::prepared::coverage::product_coverage(
        source,
        context.content.as_ref(),
        branch,
        "autonomy",
        cancelled,
    ) {
        Err(error) if error.kind() == io::ErrorKind::InvalidData => None,
        other => other?,
    };
    let mut sampled = (BTreeMap::new(), Vec::new(), BTreeMap::new());
    let mut history_range = None;
    let mut result;
    if accepted.as_ref().is_some_and(|coverage| coverage.complete) {
        result = query(source, product, value, store, key, context, cancelled)?;
    } else {
        let bounds = source
            .with_metadata(|metadata| {
                metadata
                    .branches
                    .iter()
                    .find(|item| item.id.parse::<u64>().ok() == Some(branch))
                    .map(|item| (item.first_hour, item.last_hour))
            })
            .ok_or_else(|| dashboard_core::invalid("Unknown autonomy branch"))?;
        let (from, to, width) = if product == "series-summary" {
            series::viewport(requested.from, requested.to, requested.width)?
        } else if product == "series-next-change" {
            let navigation: Navigation = serde_json::from_value(value.clone())?;
            (navigation.first_hour, navigation.last_hour, 1)
        } else {
            (f64::from(bounds.0), f64::from(bounds.1), 1)
        };
        history_range = Some((from.floor() as i32, to.floor() as i32));
        series::summary_budget(from, to, width, requested.within.len())?;
        let (keys, mut metadata) = if accepted.is_some() {
            series::read_catalogue::<ScannedAutonomy>(store, key, "autonomy", branch)?
                .unwrap_or_default()
        } else {
            Default::default()
        };
        let mut prefix = if accepted.is_some() {
            query(source, product, value, store, key, context, cancelled)?
        } else {
            Value::Null
        };
        let tail = RefCell::new(ScannedAutonomy {
            levels: metadata.levels.clone(),
            lowest_lower: metadata.lowest_lower.clone(),
            level_indices: metadata.level_indices.clone(),
            by_tag: metadata.by_tag.clone(),
            last: metadata.last.clone(),
            subjects: metadata
                .subjects
                .iter()
                .map(|(tag, _)| (tag.clone(), Vec::new()))
                .collect(),
            overlord_subjects: metadata.overlord_subjects.clone(),
            ..Default::default()
        });
        let categories = RefCell::new(BTreeMap::<String, Vec<AutonomyRun>>::new());
        let selected: BTreeSet<_> = requested
            .within
            .values()
            .flatten()
            .map(|(tag, _, _, _)| tag.clone())
            .collect();
        let fine = (to.floor() - from.floor() + 1.0) < 2.0 * f64::from(width);
        let fields = if product == "series-summary" && !fine {
            &FIELDS[4..]
        } else {
            &FIELDS[..]
        };
        let wanted: Vec<_> = selected
            .iter()
            .flat_map(|tag| {
                fields
                    .iter()
                    .map(move |field| (tag.clone(), (*field).to_owned()))
            })
            .collect();
        let observe = |frame: &DecodedFrame, _: Option<&[(String, String)]>| {
            dashboard_text::metadata::check_cancel(cancelled)?;
            let mut native = tail.borrow_mut();
            let observe = accepted
                .as_ref()
                .is_none_or(|coverage| frame.hour > coverage.last_hour);
            let mut values = native.observe_sample(frame, observe)?;
            if observe {
                let changed: HashSet<_> = native.changed.iter().map(|(index, _)| *index).collect();
                let mut bands = categories.borrow_mut();
                // The shared tail reader owns numeric history; native change
                // decisions retain only categorical bands after each observation.
                for (index, (tag, rows)) in native.subjects.iter_mut().enumerate() {
                    let bands = bands.entry(tag.clone()).or_default();
                    for row in rows.drain(..) {
                        if let Some(last) = bands.last_mut().filter(|last| {
                            !changed.contains(&index)
                                && last.last_hour.checked_add(1) == Some(row.first_hour)
                        }) {
                            last.last_hour = row.last_hour;
                        } else {
                            bands.push(row);
                        }
                    }
                }
            }
            let chosen = selection(&params, &native.overlord_subjects);
            values.retain(|(tag, field), (_, value)| {
                let numeric = if product == "series-state" {
                    chosen(tag)
                } else {
                    selected.contains(tag) && fields.contains(&field.as_str())
                };
                if !numeric {
                    *value = None;
                }
                // One existing field suffices to join every subject/overlord holder.
                numeric || field == "relative"
            });
            Ok(values)
        };
        sampled = if product == "series-state" {
            progressive::state_tail(
                source,
                branch,
                accepted.as_ref(),
                TABLES,
                &wanted,
                &requested
                    .reads
                    .iter()
                    .map(|read| read.hour)
                    .collect::<Vec<_>>(),
                &observe,
                cancelled,
            )?
        } else if product == "series-summary" && !fine {
            // A coarse chart needs one observation per block, even when the
            // entire campaign happens to fit inside the exact zoom budget.
            let mut result = progressive::TailData::default();
            keyframes::after(
                source,
                "autonomy:keyframes",
                branch,
                accepted
                    .as_ref()
                    .map_or(bounds.0.saturating_sub(1), |c| c.last_hour)
                    .max((from.floor() as i32).saturating_sub(1)),
                Some(to.floor() as i32),
                TABLES,
                cancelled,
                &|frame| {
                    let mut scan = ScannedAutonomy::default();
                    scan.observe(frame)?;
                    Ok(Some(serde_json::to_vec(&Observation {
                        scan,
                        values: sample(frame, None)?,
                    })?))
                },
                &|bytes: &Vec<u8>| std::mem::size_of::<Vec<u8>>() + bytes.capacity(),
                |spans, bytes| {
                    let observation: Observation = serde_json::from_slice(bytes)?;
                    let mut native = tail.borrow_mut();
                    let mut bands = categories.borrow_mut();
                    let mut tokens = vec![None; observation.scan.levels.len()];
                    for (token, &index) in &observation.scan.level_indices {
                        tokens[index] = Some(token);
                    }
                    let mut levels = Vec::new();
                    for (at, id) in observation.scan.levels.iter().enumerate() {
                        let token = tokens[at].expect("observed autonomy level");
                        let index = if let Some(&index) = native.level_indices.get(token) {
                            index
                        } else {
                            let index = native.levels.len();
                            native.level_indices.insert(token.clone(), index);
                            native.levels.push(id.clone());
                            native.lowest_lower.push(observation.scan.lowest_lower[at]);
                            index
                        };
                        native.lowest_lower[index] =
                            native.lowest_lower[index].min(observation.scan.lowest_lower[at]);
                        levels.push(index as u32);
                    }
                    for (tag, rows) in observation.scan.subjects {
                        if !native.by_tag.contains_key(&tag) {
                            let index = native.subjects.len();
                            native.by_tag.insert(tag.clone(), index);
                            native.subjects.push((tag.clone(), Vec::new()));
                        }
                        bands
                            .entry(tag)
                            .or_default()
                            .extend(rows.into_iter().map(|mut row| {
                                row.level = levels[row.level as usize];
                                row
                            }));
                    }
                    for (overlord, subjects) in observation.scan.overlord_subjects {
                        native
                            .overlord_subjects
                            .entry(overlord)
                            .or_default()
                            .extend(subjects);
                    }
                    native.recorded.extend(observation.scan.recorded);
                    native.unidentified.extend(observation.scan.unidentified);
                    native.shared_subjects += observation.scan.shared_subjects;
                    let mut values = observation.values;
                    for ((tag, field), (_, value)) in &mut values {
                        if !selected.contains(tag) || !fields.contains(&field.as_str()) {
                            *value = None;
                        }
                    }
                    values.retain(|(_, field), (_, value)| value.is_some() || field == "relative");
                    progressive::sampled_spans(&mut result, spans, values.iter());
                    Ok(())
                },
            )?;
            result
        } else {
            progressive::tail(
                source,
                branch,
                accepted.as_ref(),
                TABLES,
                &wanted,
                (from.floor() as i32, to.floor() as i32),
                &observe,
                cancelled,
            )?
        };
        let mut tail = tail.into_inner();
        let mut categories = categories.into_inner();
        let mut expansion = BTreeMap::<i32, Vec<HourSpan>>::new();
        if !sampled.1.is_empty() {
            for block in source
                .replay_ref()
                .index
                .iter()
                .filter(|block| block.branch == branch)
            {
                let at = sampled
                    .1
                    .partition_point(|span| span.last_hour < block.first_hour);
                let spans: Vec<_> = sampled.1[at..]
                    .iter()
                    .take_while(|span| span.first_hour <= block.last_hour)
                    .map(|span| HourSpan {
                        first_hour: span.first_hour.max(block.first_hour),
                        last_hour: span.last_hour.min(block.last_hour),
                    })
                    .collect();
                if !spans.is_empty() {
                    expansion.insert(block.first_hour, spans);
                }
            }
        }
        let extend = |first: i32, last: i32| -> Vec<HourSpan> {
            if sampled.1.is_empty() {
                let (first, last) = if product == "series-state" {
                    (first, last)
                } else {
                    (first.max(from.floor() as i32), last.min(to.floor() as i32))
                };
                if first > last {
                    return Vec::new();
                }
                vec![HourSpan {
                    first_hour: first,
                    last_hour: last,
                }]
            } else {
                expansion
                    .range(first..=last)
                    .flat_map(|(_, spans)| spans.iter().copied())
                    .collect()
            }
        };
        for rows in categories.values_mut() {
            *rows = rows
                .drain(..)
                .flat_map(|row| {
                    extend(row.first_hour, row.last_hour)
                        .into_iter()
                        .map(move |span| AutonomyRun {
                            first_hour: span.first_hour,
                            last_hour: span.last_hour,
                            starts_segment: row.starts_segment && span.first_hour == row.first_hour,
                            ..row.clone()
                        })
                })
                .collect();
        }
        tail.recorded = tail
            .recorded
            .iter()
            .flat_map(|span| extend(span.first_hour, span.last_hour))
            .collect();
        tail.unidentified = tail
            .unidentified
            .iter()
            .flat_map(|span| extend(span.first_hour, span.last_hour))
            .collect();
        if product == "series-next-change" {
            let navigation: Navigation = serde_json::from_value(value.clone())?;
            let mut prepared = BTreeMap::new();
            let mut reader =
                Reader::new(store, key, format!("autonomy-series-{branch}"), cancelled)?;
            let mut within: BTreeMap<String, Vec<HourSpan>> = BTreeMap::new();
            for spans in requested.within.values() {
                for (tag, first, last, _) in spans {
                    within.entry(tag.clone()).or_default().push(HourSpan {
                        first_hour: *first,
                        last_hour: *last,
                    });
                }
            }
            for key_pair in &wanted {
                if let Some(index) = keys.get(&key_pair.0) {
                    let field = FIELDS
                        .iter()
                        .position(|field| *field == key_pair.1)
                        .expect("autonomy field") as u64;
                    let series_key = *index * 6 + field;
                    let hour = series::navigate(
                        &mut [(
                            &mut reader,
                            vec![(series_key, within.get(&key_pair.0).map(Vec::as_slice))],
                        )],
                        &navigation,
                    )?;
                    let preceding = sampled
                        .0
                        .get(key_pair)
                        .and_then(|runs| runs.first())
                        .and_then(|run| run.first_hour.checked_sub(1))
                        .map(|hour| reader.value_at(series_key, hour))
                        .transpose()?
                        .flatten();
                    prepared.insert(key_pair.clone(), (hour, preceding));
                }
            }
            let numeric =
                progressive::navigate(&prepared, &sampled.0, &wanted, &within, &navigation)?;
            let origin = navigation.origin()?;
            let forward = matches!(navigation.key, MeasureKey::Home | MeasureKey::ArrowRight);
            let category = categories
                .iter()
                .filter(|(tag, _)| selected.contains(*tag))
                .flat_map(|(tag, rows)| {
                    rows.iter()
                        .filter(|row| {
                            within.get(tag).is_some_and(|spans| {
                                spans.iter().any(|span| {
                                    span.first_hour <= row.first_hour
                                        && row.first_hour <= span.last_hour
                                })
                            })
                        })
                        .map(|row| f64::from(row.first_hour))
                })
                .filter(|hour| {
                    navigation.first_hour <= *hour
                        && *hour <= navigation.last_hour
                        && if forward {
                            *hour > origin
                        } else {
                            *hour < origin
                        }
                })
                .reduce(|a, b| if forward { a.min(b) } else { a.max(b) });
            result = serde_json::json!({"hour":numeric.into_iter().chain(category).reduce(|a,b| if forward { a.min(b) } else { a.max(b) })});
        } else {
            // Prefix rows index the resolved level order; the observer indexes
            // native state tokens. Convert once before the existing reply builder.
            let prefix_levels: Vec<AutonomyLevel> = if prefix.is_null() {
                Vec::new()
            } else {
                serde_json::from_value(prefix["levels"].clone())?
            };
            let native_levels: HashMap<_, _> = metadata
                .levels
                .iter()
                .enumerate()
                .map(|(index, id)| (id.as_str(), index as u32))
                .collect();
            let levels: Vec<_> = prefix_levels
                .iter()
                .map(|level| native_levels[&level.id.as_str()])
                .collect();
            if !prefix.is_null() {
                for subject in
                    serde_json::from_value::<Vec<AutonomySubject>>(prefix["subjects"].clone())?
                {
                    if let Some(&index) = metadata.by_tag.get(&subject.tag) {
                        metadata.subjects[index].1 = subject
                            .runs
                            .into_iter()
                            .map(|mut run| {
                                run.level = levels[run.level as usize];
                                run
                            })
                            .collect();
                    }
                }
                for subject in prefix["bands"].as_array_mut().into_iter().flatten() {
                    for run in subject["runs"].as_array_mut().into_iter().flatten() {
                        run["level"] = Value::from(
                            levels[run["level"].as_u64().expect("prepared level") as usize],
                        );
                    }
                }
            }
            metadata.levels = std::mem::take(&mut tail.levels);
            metadata.lowest_lower = std::mem::take(&mut tail.lowest_lower);
            metadata.level_indices = std::mem::take(&mut tail.level_indices);
            for (overlord, subjects) in tail.overlord_subjects {
                metadata
                    .overlord_subjects
                    .entry(overlord)
                    .or_default()
                    .extend(subjects);
            }
            let mut bands = prefix
                .get("bands")
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default();
            let mut band_indices: HashMap<_, _> = bands
                .iter()
                .enumerate()
                .map(|(index, subject)| {
                    (
                        subject["tag"]
                            .as_str()
                            .expect("prepared subject")
                            .to_owned(),
                        index,
                    )
                })
                .collect();
            let state_hours: BTreeSet<_> = requested.reads.iter().map(|read| read.hour).collect();
            for (tag, _) in tail.subjects {
                let rows = categories.remove(&tag).unwrap_or_default();
                let index = *metadata.by_tag.entry(tag.clone()).or_insert_with(|| {
                    metadata.subjects.push((tag.clone(), Vec::new()));
                    metadata.subjects.len() - 1
                });
                let band = *band_indices.entry(tag.clone()).or_insert_with(|| {
                    bands.push(serde_json::json!({"tag":tag,"runs":[]}));
                    bands.len() - 1
                });
                let fields: [Vec<series::Run>; 4] = std::array::from_fn(|index| {
                    sampled
                        .0
                        .get(&(tag.clone(), FIELDS[index].into()))
                        .cloned()
                        .unwrap_or_default()
                });
                let mut points = state_hours.clone();
                if product == "series-summary" && fine && selected.contains(&tag) {
                    points.insert(from.floor() as i32);
                    points.extend(fields.iter().flatten().map(|run| run.first_hour));
                    points.extend(rows.iter().map(|run| run.first_hour));
                }
                for hour in points {
                    if let Some(run) = point(&fields, &rows, hour) {
                        metadata.subjects[index].1.push(run);
                    }
                }
                for run in rows {
                    let mut categorical = serde_json::to_value(run)?;
                    for field in ["scoreRaw", "lowerRaw", "upperRaw", "ratioRaw"] {
                        categorical.as_object_mut().unwrap().remove(field);
                    }
                    bands[band]["runs"]
                        .as_array_mut()
                        .unwrap()
                        .push(categorical);
                }
            }
            for span in tail.unidentified {
                mark(&mut metadata.unidentified, span.first_hour);
                metadata.unidentified.last_mut().unwrap().last_hour = span.last_hour;
            }
            metadata.shared_subjects += tail.shared_subjects;
            for span in tail.recorded {
                mark(&mut metadata.recorded, span.first_hour);
                if let Some(last) = metadata.recorded.last_mut() {
                    last.last_hour = last.last_hour.max(span.last_hour);
                }
            }
            for (_, runs) in &mut metadata.subjects {
                runs.sort_by_key(|run| run.first_hour);
                runs.dedup();
            }
            let (reply, places) = metadata.reply(source, &params, context)?;
            let chosen: HashSet<_> = reply
                .subjects
                .iter()
                .map(|subject| subject.tag.clone())
                .collect();
            bands.retain(|subject| chosen.contains(subject["tag"].as_str().expect("autonomy tag")));
            for subject in &mut bands {
                for run in subject["runs"].as_array_mut().unwrap() {
                    run["level"] = Value::from(places[run["level"].as_u64().unwrap() as usize]);
                }
            }
            result = serde_json::to_value(reply)?;
            result["bands"] = serde_json::to_value(bands)?;
            result["exactHours"] = if product == "series-state" {
                serde_json::to_value(
                    requested
                        .reads
                        .iter()
                        .filter(|read| {
                            !sampled.1.iter().any(|span| {
                                span.first_hour <= read.hour && read.hour <= span.last_hour
                            })
                        })
                        .map(|read| read.hour)
                        .collect::<Vec<_>>(),
                )?
            } else {
                prefix
                    .get("exactHours")
                    .cloned()
                    .unwrap_or_else(|| serde_json::json!([]))
            };
            let mut summaries = prefix
                .get("summaries")
                .and_then(Value::as_object)
                .cloned()
                .unwrap_or_default();
            let mut reader = if accepted.is_some() {
                Reader::new(store, key, format!("autonomy-series-{branch}"), cancelled)?
            } else {
                Reader::from_first_file(
                    store,
                    key,
                    format!("autonomy-series-{branch}"),
                    0,
                    None,
                    cancelled,
                )
            };
            for (id, spans) in &requested.within {
                if product != "series-summary" {
                    break;
                }
                let mut summary: Option<ChartSummary> = None;
                let mut membership = Vec::new();
                for (tag, first, last, _) in spans {
                    let within = [HourSpan {
                        first_hour: *first,
                        last_hour: *last,
                    }];
                    membership.extend(within);
                    let series_key = keys
                        .get(tag)
                        .map_or(metadata.subjects.len() as u64 * 6, |index| *index * 6 + 4);
                    let mut part = reader.summary_with_tail(
                        series_key,
                        bounds,
                        (from, to, width),
                        0,
                        sampled
                            .0
                            .get(&(tag.clone(), "relative".into()))
                            .map_or(&[][..], Vec::as_slice),
                        Some(&within),
                    )?;
                    progressive::clip_approximate(&mut part, &sampled.1, Some(&within));
                    if let Some(summary) = &mut summary {
                        for (bucket, next) in summary.buckets.iter_mut().zip(part.buckets) {
                            *bucket = merge(*bucket, next);
                        }
                        if let (Some(runs), Some(next)) = (&mut summary.runs, part.runs) {
                            runs.extend(next);
                        }
                    } else {
                        summary = Some(part);
                    }
                }
                if let Some(mut summary) = summary {
                    progressive::clip_approximate(&mut summary, &sampled.1, Some(&membership));
                    summaries.insert(id.clone(), serde_json::to_value(summary)?);
                }
            }
            result["summaries"] = Value::Object(summaries);
        }
    }
    // Categorical bands and an empty sampled census are approximate even
    // when the request asked for no numeric summaries.
    if !sampled.1.is_empty() {
        result["approximate"] = serde_json::to_value(&sampled.1)?;
    }
    let no_identities = BTreeMap::new();
    let exact_identities = if product == "series-state" && sampled.1.is_empty() {
        &sampled.2
    } else {
        &no_identities
    };
    let requested_tags: Vec<_> = params
        .subjects
        .iter()
        .chain(&params.overlords)
        .chain(
            requested
                .within
                .values()
                .flatten()
                .map(|(tag, _, _, _)| tag),
        )
        .filter(|tag| !tag.is_empty())
        .cloned()
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect();
    let _cpu = replay_frame::cpu::scope_phase("autonomy:query_identities_finish");
    let history = progressive::histories(
        store,
        key,
        branch,
        accepted.as_ref(),
        &sampled.2,
        exact_identities,
        source.replay_ref(),
        &requested_tags,
        cancelled,
    )?;
    result["identities"] = serde_json::to_value(
        history
            .countries
            .iter()
            .map(|(tag, country)| serde_json::json!({"tag":tag,"identities":country.identities}))
            .collect::<Vec<_>>(),
    )?;
    // Preparation proves the history's census. A reached observation proves
    // only that hour; every unsampled recorded hour after the prefix stays
    // pending, including blocks whose keyframe could not be read.
    let mut prefix = dashboard_base::prepared::prefix::Prefix::new(
        source,
        branch,
        dashboard_base::prepared::prefix::extent(source, branch, accepted)?,
        history_range.map(|(_, last)| last),
    );
    if let Some((first, _)) = history_range {
        prefix.pending.retain(|span| span.last_hour >= first);
        for span in &mut prefix.pending {
            span.first_hour = span.first_hour.max(first);
        }
    }
    prefix.sampled(&sampled.1);
    prefix.mark(&mut result)?;
    Ok(result)
}

pub(crate) fn query(
    source: &Source,
    product: &str,
    value: &Value,
    store: &store::Store,
    key: &store::SourceKey,
    context: &LoadContext,
    cancelled: &AtomicBool,
) -> io::Result<Value> {
    let params: AutonomyParams = serde_json::from_value(serde_json::json!({
        "branchId":value["branchId"],"subjects":value.get("subjects").unwrap_or(&serde_json::json!([])),
        "overlords":value.get("overlords").unwrap_or(&serde_json::json!([])),
    }))?;
    Autonomy::validate(&params).map_err(dashboard_core::invalid)?;
    let requested: Requested = serde_json::from_value(value.clone())?;
    let hours: Vec<_> = requested.reads.iter().map(|read| read.hour).collect();
    let navigation = if product == "series-next-change" {
        let navigation: Navigation = serde_json::from_value(value.clone())?;
        Some((
            requested.tag.clone().unwrap_or_default(),
            navigation.origin()?,
            matches!(navigation.key, MeasureKey::Home | MeasureKey::ArrowRight),
        ))
    } else {
        None
    };
    let (from, to, width) = if product == "series-summary" {
        series::viewport(requested.from, requested.to, requested.width)?
    } else if product == "series-next-change" {
        let navigation: Navigation = serde_json::from_value(value.clone())?;
        (navigation.first_hour, navigation.last_hour, 1)
    } else {
        let hour = f64::from(hours.first().copied().unwrap_or(0));
        (hour, hour, 1)
    };
    series::summary_budget(from, to, width, requested.within.len())?;
    let branch: u64 = params
        .branch_id
        .parse()
        .map_err(|_| dashboard_core::invalid("Invalid autonomy branch"))?;
    let Some((indices, mut scan)) =
        series::read_catalogue::<ScannedAutonomy>(store, key, "autonomy", branch)?
    else {
        return Ok(Value::Null);
    };
    let mut reader = Reader::new(store, key, format!("autonomy-series-{branch}"), cancelled)?;
    let wanted = selection(&params, &scan.overlord_subjects);
    let mut chosen = params.clone();
    chosen.overlords.clear();
    chosen.subjects = scan
        .subjects
        .iter()
        .filter(|(tag, _)| wanted(tag))
        .map(|(tag, _)| tag.clone())
        .collect();
    let mut bands: Vec<Vec<AutonomyRun>> = vec![Vec::new(); scan.subjects.len()];
    let chosen_tags: HashSet<_> = chosen.subjects.iter().map(String::as_str).collect();
    let selected_aliases: HashSet<_> = requested
        .within
        .values()
        .flatten()
        .map(|(tag, _, _, _)| tag.as_str())
        .collect();
    let mut summaries = BTreeMap::new();
    let bounds = source
        .with_metadata(|metadata| {
            metadata
                .branches
                .iter()
                .find(|start| start.id.parse::<u64>().ok() == Some(branch))
                .map(|start| (start.first_hour, start.last_hour))
        })
        .ok_or_else(|| dashboard_core::invalid("Unknown autonomy branch"))?;
    if let Some((selected, origin, forward)) = &navigation {
        let mut candidates = Vec::new();
        for (tag, first, last, _) in requested.within.get(selected).into_iter().flatten() {
            let Some(&index) = indices
                .get(tag.as_str())
                .filter(|_| chosen_tags.contains(tag.as_str()))
            else {
                continue;
            };
            let lo = (*first).max(from.ceil() as i32);
            let hi = (*last).min(to.floor() as i32);
            if lo > hi {
                continue;
            }
            if f64::from(lo) > *origin && *forward {
                candidates.push(lo);
            }
            if let Some(hour) = series::navigate(
                &mut [(
                    &mut reader,
                    (0..4)
                        .map(|field| (index * 6 + field, Some(scan.recorded.as_slice())))
                        .collect(),
                )],
                &Navigation {
                    from: Some(*origin),
                    first_hour: f64::from(lo),
                    last_hour: f64::from(hi),
                    key: if *forward {
                        MeasureKey::ArrowRight
                    } else {
                        MeasureKey::ArrowLeft
                    },
                },
            )?
            .filter(|hour| {
                if *forward {
                    *hour > *origin
                } else {
                    *hour < *origin
                }
            }) {
                candidates.push(hour as i32);
            }
        }
        let reached = candidates
            .into_iter()
            .reduce(|a, b| if *forward { a.min(b) } else { a.max(b) });
        return Ok(serde_json::json!({"hour":reached}));
    }
    let reached_hours: BTreeSet<_> = hours.iter().copied().collect();
    let mut windows = Vec::new();
    if product == "series-state" {
        for &hour in &reached_hours {
            if hour < bounds.0 || hour > bounds.1 {
                continue;
            }
            match windows.last_mut() {
                Some((_, last)) if i32::checked_add(*last, 1) == Some(hour) => *last = hour,
                _ => windows.push((hour, hour)),
            }
        }
    } else {
        let lo = (from.floor() as i32).max(bounds.0);
        let hi = (to.floor() as i32).min(bounds.1);
        if lo <= hi {
            windows.push((lo, hi));
        }
    }
    for (index, (tag, _)) in scan.subjects.iter().enumerate() {
        dashboard_text::metadata::check_cancel(cancelled)?;
        if !chosen_tags.contains(tag.as_str()) {
            continue;
        }
        let band_key = index as u64 * 6 + 5;
        let (last, _, sample, (first, starts)) = scan.last[index].as_ref().expect("known actor");
        let latest = sample.run(*first, *last, *starts);
        for &(lo, hi) in &windows {
            if *first <= lo
                || scan
                    .batch
                    .state(band_key)
                    .is_some_and(|state| state.events == 1)
            {
                if latest.first_hour <= hi && lo <= latest.last_hour {
                    bands[index].push(latest.clone());
                }
                continue;
            }
            if let Some((_, row)) = reader.latest_event(band_key, lo)? {
                let event: MetaEvent = serde_json::from_value(row)?;
                bands[index].push(AutonomyRun {
                    last_hour: *last,
                    ..event.run
                });
            }
            if lo < hi {
                for row in reader.events(&[band_key], lo + 1, hi, 0, u32::MAX)?.rows {
                    let event: MetaEvent = serde_json::from_value(row.row)?;
                    if let Some(previous) = bands[index].last_mut() {
                        previous.last_hour =
                            event.previous_hour.unwrap_or(row.hour.saturating_sub(1));
                    }
                    bands[index].push(AutonomyRun {
                        last_hour: *last,
                        ..event.run
                    });
                }
            }
        }
        bands[index].sort_by_key(|run| run.first_hour);
        bands[index].dedup_by(|next, held| {
            if next.first_hour != held.first_hour {
                return false;
            }
            held.last_hour = held.last_hour.max(next.last_hour);
            true
        });
        for at in 1..bands[index].len() {
            let end = bands[index][at].first_hour.saturating_sub(1);
            bands[index][at - 1].last_hour = bands[index][at - 1].last_hour.min(end);
        }
    }
    let mut amounts = BTreeMap::new();
    if product == "series-state" {
        for &hour in &reached_hours {
            let mut keys = Vec::new();
            for (index, (tag, _)) in scan.subjects.iter().enumerate() {
                if !chosen_tags.contains(tag.as_str()) {
                    continue;
                }
                let band = bands[index]
                    .partition_point(|run| run.first_hour <= hour)
                    .checked_sub(1)
                    .and_then(|at| bands[index].get(at))
                    .filter(|run| hour <= run.last_hour);
                if let Some(band) = band {
                    let window = windows
                        .partition_point(|(first, _)| *first <= hour)
                        .checked_sub(1)
                        .map(|at| windows[at]);
                    let Some((first, last)) = window else {
                        continue;
                    };
                    let first = first.max(bands[index].first().unwrap().first_hour);
                    if hour != first || scan.last[index].as_ref().unwrap().3.0 != first {
                        continue;
                    }
                    for field in 0..3 + u64::from(band.ratio_raw.is_some()) {
                        let key = index as u64 * 6 + field;
                        if scan
                            .batch
                            .state(key)
                            .and_then(|state| state.last)
                            .is_some_and(|run| {
                                hour.checked_add(1) == Some(run.first_hour) && last <= run.last_hour
                            })
                        {
                            keys.push(key);
                        }
                    }
                }
            }
            // One supplying-batch lookup per reached hour, for all uncached amounts.
            for (key, value) in keys.iter().copied().zip(reader.values_at(&keys, hour)?) {
                amounts.insert((key, hour), value);
            }
        }
    }
    for (index, (tag, runs)) in scan.subjects.iter_mut().enumerate() {
        dashboard_text::metadata::check_cancel(cancelled)?;
        if !chosen_tags.contains(tag.as_str()) {
            continue;
        }
        let key = index as u64 * 6;
        let mut fields: [Vec<series::Run>; 4] = Default::default();
        for (field, values) in fields.iter_mut().enumerate() {
            let state = scan
                .batch
                .state(key + field as u64)
                .and_then(|state| state.last);
            for &(lo, hi) in &windows {
                if !bands[index]
                    .iter()
                    .any(|run| run.first_hour <= hi && lo <= run.last_hour)
                {
                    continue;
                }
                let lo = lo.max(
                    bands[index]
                        .first()
                        .expect("intersecting autonomy band")
                        .first_hour,
                );
                let hi = hi.min(
                    bands[index]
                        .last()
                        .expect("intersecting autonomy band")
                        .last_hour,
                );
                // At an open band's first read followed by the current run, the
                // band's start and the next run's start prove both clipped bounds.
                if product == "series-state"
                    && scan.last[index].as_ref().is_some_and(|last| last.3.0 == lo)
                    && let Some(last) = state.filter(|last| {
                        lo.checked_add(1) == Some(last.first_hour) && hi <= last.last_hour
                    })
                {
                    if let Some(value) = amounts.get(&(key + field as u64, lo)).copied().flatten() {
                        values.push(series::Run::new(
                            lo,
                            lo,
                            value,
                            bands[index].last().unwrap().starts_segment,
                        ));
                    }
                    values.push(last);
                } else {
                    values.extend(numeric_runs(
                        &mut reader,
                        key + field as u64,
                        state,
                        (lo, hi),
                        bounds,
                    )?);
                }
            }
            values.sort_by_key(|run| run.first_hour);
            values.dedup();
        }
        let mut points = reached_hours.clone();
        if selected_aliases.contains(tag.as_str())
            && (to.floor().min(f64::from(bounds.1)) - from.floor().max(f64::from(bounds.0)) + 1.0)
                < 2.0 * f64::from(width)
        {
            points.insert(from.floor() as i32);
            points.extend(
                fields
                    .iter()
                    .flatten()
                    .map(|run| run.first_hour)
                    .filter(|hour| from <= f64::from(*hour) && f64::from(*hour) <= to),
            );
            points.extend(
                bands[index]
                    .iter()
                    .map(|run| run.first_hour)
                    .filter(|hour| from <= f64::from(*hour) && f64::from(*hour) <= to),
            );
        }
        if product == "series-state" {
            bands[index].retain_mut(|band| {
                let last = fields[0]
                    .iter()
                    .filter(|run| {
                        let lo = run.first_hour.max(band.first_hour);
                        let hi = run.last_hour.min(band.last_hour);
                        lo <= hi && reached_hours.range(lo..=hi).next().is_some()
                    })
                    .map(|run| run.last_hour.min(band.last_hour))
                    .max();
                if let Some(last) = last {
                    band.last_hour = last;
                    true
                } else {
                    false
                }
            });
        } else {
            for band in &mut bands[index] {
                band.last_hour = band.last_hour.min(to.floor() as i32);
            }
        }
        for hour in points {
            if let Some(run) = point(&fields, &bands[index], hour) {
                runs.push(run);
            }
        }
        runs.sort_by_key(|run| run.first_hour);
        runs.dedup();
    }
    for (id, selected_spans) in &requested.within {
        let spans: Vec<_> = selected_spans
            .iter()
            .filter_map(|(tag, first, last, starts)| {
                indices
                    .get(tag.as_str())
                    .filter(|_| chosen_tags.contains(tag.as_str()))
                    .map(|index| (*index * 6 + 4, *first, *last, Some(*starts)))
            })
            .collect();
        if product == "series-summary" {
            summaries.insert(
                id.clone(),
                reader.summary_spans(&spans, bounds, (from, to, width), 0)?,
            );
        }
    }
    let (mut reply, places) = scan.reply(source, &chosen, context)?;
    reply
        .subjects
        .retain(|subject| chosen_tags.contains(subject.tag.as_str()));
    let mut value = serde_json::to_value(reply)?;
    value["bands"] = serde_json::to_value(scan.subjects.iter().zip(bands).filter(|((tag, _), _)| chosen_tags.contains(tag.as_str())).map(|((tag, _), rows)|
        serde_json::json!({"tag":tag,"runs":rows.into_iter().map(|run| serde_json::json!({"firstHour":run.first_hour,"lastHour":run.last_hour,"startsSegment":run.starts_segment,
            "overlord":run.overlord,"level":places[run.level as usize],"lowest":run.lowest,"highest":run.highest})).collect::<Vec<_>>() })).collect::<Vec<_>>())?;
    value["summaries"] = serde_json::to_value(summaries)?;
    value["exactHours"] = serde_json::to_value(hours)?;
    Ok(value)
}

fn numeric_runs(
    reader: &mut Reader<'_>,
    key: u64,
    last: Option<series::Run>,
    (from, to): (i32, i32),
    bounds: (i32, i32),
) -> io::Result<Vec<series::Run>> {
    let covers = |hour| last.filter(|run| run.first_hour <= hour && hour <= run.last_hour);
    if let Some(run) = covers(from).filter(|run| to <= run.last_hour) {
        return Ok(vec![run]);
    }
    let mut runs = reader.runs(key, from, to)?;
    if let Some(run) = runs.first_mut()
        && run.first_hour == from
        && from > bounds.0
        && let Some(canonical) = covers(from)
            .map(|run| Ok(Some(run)))
            .unwrap_or_else(|| reader.value_at(key, from))?
    {
        *run = canonical;
    }
    if let Some(run) = runs.last_mut()
        && run.last_hour == to
        && to < bounds.1
        && let Some(canonical) = covers(to)
            .map(|run| Ok(Some(run)))
            .unwrap_or_else(|| reader.value_at(key, to))?
    {
        *run = canonical;
    }
    Ok(runs)
}

fn point(fields: &[Vec<series::Run>; 4], bands: &[AutonomyRun], hour: i32) -> Option<AutonomyRun> {
    let mut run = bands
        .partition_point(|run| run.first_hour <= hour)
        .checked_sub(1)
        .and_then(|index| bands.get(index))
        .filter(|run| hour <= run.last_hour)?
        .clone();
    let mut values = Vec::new();
    for rows in fields.iter().take(3 + usize::from(run.ratio_raw.is_some())) {
        values.push(
            rows.partition_point(|run| run.first_hour <= hour)
                .checked_sub(1)
                .and_then(|at| rows.get(at))
                .filter(|run| hour <= run.last_hour)?,
        );
    }
    let first = values
        .iter()
        .fold(run.first_hour, |first, value| first.max(value.first_hour));
    run.starts_segment &= first == run.first_hour;
    run.first_hour = first;
    run.last_hour = values
        .iter()
        .fold(run.last_hour, |last, value| last.min(value.last_hour));
    run.score_raw = values[0].value.to_string();
    run.lower_raw = values[1].value.to_string();
    run.upper_raw = values[2].value.to_string();
    run.ratio_raw = values.get(3).map(|value| value.value.to_string());
    Some(run)
}
