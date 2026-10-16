use super::*;
use crate::prepared::series::{self, MeasureKey, Navigation, Reader};
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};

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
    #[serde(default)]
    share: bool,
}
#[derive(Deserialize)]
struct Read {
    hour: i32,
}

pub(crate) fn query(
    source: &Source,
    product: &str,
    value: &Value,
    store: &store::Store,
    key: &store::SourceKey,
    history: &super::super::country_history::RecordedHistories,
    cancelled: &AtomicBool,
) -> io::Result<Value> {
    let params: BranchParams = serde_json::from_value(json!({"branchId":value["branchId"]}))?;
    params.validate().map_err(crate::invalid)?;
    let requested: Requested = serde_json::from_value(value.clone())?;
    let reached_hours: Vec<_> = requested.reads.iter().map(|read| read.hour).collect();
    let navigation = if product == "series-next-change" {
        Some(serde_json::from_value::<Navigation>(value.clone())?)
    } else {
        None
    };
    let (from, to, width) = if product == "series-summary" {
        series::viewport(requested.from, requested.to, requested.width)?
    } else if let Some(navigation) = &navigation {
        (navigation.first_hour, navigation.last_hour, 1)
    } else {
        let hour = f64::from(reached_hours.first().copied().unwrap_or(0));
        (hour, hour, 1)
    };
    series::summary_budget(from, to, width, requested.within.len() + 2)?;
    let share = requested.share;
    let branch = params.branch();
    let Some((keys, mut scan)) =
        series::read_catalogue::<Scan>(store, key, "world-tension", branch)?
    else {
        return Ok(Value::Null);
    };
    let mut reader = Reader::new(store, key, format!("tension-series-{branch}"), cancelled)?;
    let bounds = source
        .metadata()
        .branches
        .iter()
        .find(|item| item.id.parse::<u64>().ok() == Some(branch))
        .map(|item| (item.first_hour, item.last_hour))
        .ok_or_else(|| crate::invalid("Unknown tension branch"))?;
    let reached: BTreeSet<_> = reached_hours.iter().copied().collect();
    let mut hours = reached.clone();
    if !requested.within.is_empty()
        && to.floor().min(f64::from(bounds.1)) - from.floor().max(f64::from(bounds.0)) + 1.0
            < 2.0 * f64::from(width)
    {
        hours.extend(
            reader
                .runs(3, from.floor() as i32, to.floor() as i32)?
                .into_iter()
                .map(|run| run.first_hour),
        );
    }
    scan.result.reads.clear();
    for &hour in &hours {
        if let Some(read) = read_at(&mut reader, hour)? {
            scan.result.reads.push(read);
        }
    }
    scan.result.reads.sort_by_key(|read| read.first_hour);
    scan.result.reads.dedup();
    let mut order: Vec<_> = scan.building.iter().enumerate().collect();
    let selected_keys: std::collections::HashSet<usize> = requested
        .within
        .values()
        .flatten()
        .map(|(key, _, _, _)| {
            key.parse()
                .map_err(|_| crate::invalid("Invalid tension series key"))
        })
        .collect::<io::Result<_>>()?;
    order.sort_by_key(|(_, country)| (&country.tag, country.slot));
    scan.result.first_contributions = order
        .iter()
        .map(|(_, country)| country.first_hour)
        .collect();
    scan.result.countries =
        order
            .iter()
            .map(|&(index, country)| {
                crate::metadata::check_cancel(cancelled)?;
                let mut runs = Vec::new();
                let selected = selected_keys.contains(&index);
                let mut windows: Vec<(i32, i32)> = Vec::new();
                for &hour in if selected { &hours } else { &reached } {
                    if hour < country.first_hour || hour > bounds.1 {
                        continue;
                    }
                    match windows.last_mut() {
                        Some((_, last)) if last.checked_add(1) == Some(hour) => *last = hour,
                        _ => windows.push((hour, hour)),
                    }
                }
                let key = 5 + index as u64 * 2;
                for (first, last) in windows {
                    // Read each requested window once. Unreached gaps and the time
                    // before this contributor first exists never cause history reads.
                    let mut values = reader.runs(key, first, last)?;
                    if let Some(value) = values.first_mut()
                        && value.first_hour == first
                        && first > country.first_hour
                        && let Some(canonical) = reader.value_at(key, first)?
                    {
                        *value = canonical;
                    }
                    if let Some(value) = values.last_mut()
                        && value.last_hour == last
                        && last < bounds.1
                        && let Some(canonical) = reader.value_at(key, last)?
                    {
                        *value = canonical;
                    }
                    runs.extend(values.into_iter().filter(|run| run.value != 0).map(|run| {
                        FixedRun {
                            first_hour: run.first_hour,
                            last_hour: run.last_hour,
                            value_raw: run.value.to_string(),
                            starts_segment: run.break_before,
                        }
                    }));
                }
                runs.sort_by_key(|run| run.first_hour);
                runs.dedup();
                Ok(TensionCountry {
                    tag: country.tag.clone(),
                    slot: country.slot,
                    runs,
                    identities: None,
                })
            })
            .collect::<io::Result<_>>()?;
    scan.result
        .events
        .sort_by_key(|event| (event.hour, event.first_read));
    let mut summaries = BTreeMap::new();
    let view = (from, to, width);
    if product == "series-summary" {
        summaries.insert("world".to_owned(), reader.summary(0, bounds, view, 5)?);
        summaries.insert(
            "sum".to_owned(),
            reader.summary(if share { 0 } else { 1 }, bounds, view, 5)?,
        );
    }
    for (id, selected_spans) in &requested.within {
        let mut spans = Vec::new();
        let mut first = i64::from(bounds.0);
        for (tag, span_first, span_last, _) in selected_spans {
            let index: usize = tag
                .parse()
                .map_err(|_| crate::invalid("Invalid tension series key"))?;
            let country = scan
                .building
                .get(index)
                .ok_or_else(|| crate::invalid("Unknown tension series key"))?;
            let lo = (*span_first).max(bounds.0);
            let hi = (*span_last).min(bounds.1);
            if lo <= hi {
                if first < i64::from(lo) {
                    spans.push((4, first as i32, lo - 1, None));
                }
                let known = lo.max(country.first_hour);
                if lo < known {
                    spans.push((4, lo, hi.min(known - 1), None));
                }
                if known <= hi {
                    spans.push((5 + index as u64 * 2 + u64::from(share), known, hi, None));
                }
                first = i64::from(hi) + 1;
            }
        }
        if first <= i64::from(bounds.1) {
            spans.push((4, first as i32, bounds.1, None));
        }
        if product == "series-summary" {
            summaries.insert(id.clone(), reader.summary_spans(&spans, bounds, view, 5)?);
        }
    }
    let hour = if let Some(navigation) = &navigation {
        let origin = navigation.origin()?;
        let forward = matches!(navigation.key, MeasureKey::Home | MeasureKey::ArrowRight);
        let read = series::navigate(
            &mut [(&mut reader, vec![(3, Some(scan.coverage.as_slice()))])],
            &Navigation {
                from: Some(origin),
                first_hour: from,
                last_hour: to,
                key: if forward {
                    MeasureKey::ArrowRight
                } else {
                    MeasureKey::ArrowLeft
                },
            },
        )?
        .filter(|hour| {
            if forward {
                *hour > origin
            } else {
                *hour < origin
            }
        })
        .map(|hour| hour as i32);
        let event = scan
            .result
            .events
            .iter()
            .map(|event| event.hour)
            .filter(|hour| {
                if forward {
                    f64::from(*hour) > origin
                } else {
                    f64::from(*hour) < origin
                }
            })
            .reduce(|a, b| if forward { a.min(b) } else { a.max(b) });
        [read, event]
            .into_iter()
            .flatten()
            .filter(|hour| from <= f64::from(*hour) && f64::from(*hour) <= to)
            .reduce(|a, b| if forward { a.min(b) } else { a.max(b) })
    } else {
        None
    };
    if product == "series-next-change" {
        return Ok(json!({"hour":hour}));
    }
    let reply = scan.result.reply(&params, history)?;
    let mut value = serde_json::to_value(reply)?;
    value["catalogue"] = json!(order.iter().map(|(_,country)|json!({"key":keys[&series::field_key(&country.tag,&country.slot.to_string())].to_string(),"tag":country.tag,"slot":country.slot,"firstHour":country.first_hour})).collect::<Vec<_>>());
    value["summaries"] = serde_json::to_value(summaries)?;
    value["exactHours"] = serde_json::to_value(reached_hours)?;
    value["available"] = json!(scan.result.read_count > 0);
    Ok(value)
}

fn read_at(reader: &mut Reader<'_>, hour: i32) -> io::Result<Option<TensionRead>> {
    let Some(run) = reader.value_at(3, hour)? else {
        return Ok(None);
    };
    let mut values = Vec::new();
    for key in 0..3 {
        values.push(
            reader
                .value_at(key, hour)?
                .ok_or_else(|| crate::invalid("Missing tension value at a known read"))?
                .value,
        );
    }
    Ok(Some(TensionRead {
        first_hour: i32::try_from(run.value)
            .map_err(|_| crate::invalid("Invalid tension read hour"))?,
        last_hour: run.last_hour,
        starts_segment: run.break_before,
        tension_raw: (values[0] / i128::from(cworld_threat::SCALAR_DIVISOR)).to_string(),
        sum_raw: values[1].to_string(),
        unnamed_raw: values[2].to_string(),
    }))
}
