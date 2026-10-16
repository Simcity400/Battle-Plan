//! Spending reads share production endpoints, classifications and exact S values.
use super::*;
use crate::prepared::series::Reader;
use crate::prepared_production::{self as pages, EndpointPage, SourceEndpoint};
use serde_json::{Value, json};

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Params {
    branch_id: String,
    tag: String,
    refits: bool,
    from: Option<i32>,
    hour: Option<i32>,
    end: Option<i32>,
}

pub fn query(
    source: &Source,
    params: Value,
    content: Option<&super::super::ContentOptions>,
    cancel: &AtomicBool,
) -> io::Result<Option<Value>> {
    let input: Params = serde_json::from_value(params)?;
    let branch: u64 = input
        .branch_id
        .parse()
        .map_err(|_| crate::invalid("Invalid production branch"))?;
    validate_tag(&input.tag).map_err(crate::invalid)?;
    let Some(bounds) = source
        .replay_ref()
        .branches
        .iter()
        .find(|row| row.id == branch)
        .map(|row| crate::metadata::branch_bounds(source.replay_ref(), row))
    else {
        return Err(crate::invalid("Unknown production branch"));
    };
    let Some(coverage) = crate::prepared::coverage::coverage(source, content, branch, cancel)?
    else {
        return Ok(None);
    };
    let hour = input.hour.unwrap_or(bounds.1);
    let end = input.end.unwrap_or(bounds.1);
    // The original reply's complete design inventory and later learned type
    // facts require the branch catalogue, even for a pinned historical range.
    if !coverage.complete {
        return Ok(None);
    }
    let (store, key) = crate::prepared::files(source, content)?;
    let Some(mut data) = read_pages(&store, &key, branch, bounds, &input, hour, end, cancel)?
    else {
        return Ok(None);
    };
    let context = LoadContext {
        content: content.cloned(),
    };
    let mut warnings = Vec::new();
    let (catalogue, _) = production::content(source, &context, &mut warnings);
    data["warnings"]
        .as_array_mut()
        .expect("spending warnings")
        .splice(0..0, warnings.drain(..).map(Value::String));
    if let Some(catalogue) = catalogue {
        if data["leaves"]
            .as_array()
            .is_some_and(|leaves| leaves.iter().any(|leaf| leaf["rocket"] == true))
        {
            match cached_rocket_name(source, &catalogue) {
                Ok((key, name)) => {
                    data["rocketKey"] = json!(key);
                    data["rocketName"] = json!(name);
                }
                Err(reason) => warnings.push(format!(
                    "Rocket production source name is unavailable: {reason}."
                )),
            }
        }
        warnings.extend(catalogue.refusal_lines());
    }
    data["warnings"]
        .as_array_mut()
        .expect("spending warnings")
        .extend(warnings.into_iter().map(Value::String));
    Ok(Some(data))
}

fn country<'a>(page: &'a EndpointPage, tag: &str) -> Option<&'a pages::CountryEndpoint> {
    page.countries
        .binary_search_by(|row| row.tag.as_str().cmp(tag))
        .ok()
        .map(|index| &page.countries[index])
}

fn source_at<'a>(
    page: Option<&'a EndpointPage>,
    tag: &str,
    rocket: bool,
) -> Option<&'a SourceEndpoint> {
    let row = country(page?, tag)?;
    if rocket {
        row.rocket.as_ref()
    } else {
        row.equipment.as_ref()
    }
}

fn continuous(a: Option<&SourceEndpoint>, b: Option<&SourceEndpoint>) -> bool {
    a.zip(b).is_some_and(|(a, b)| {
        (a.slot, a.country_id, a.observation, a.segment)
            == (b.slot, b.country_id, b.observation, b.segment)
    })
}

/// Shared summaries find the maximum's date; its value is read as i128. A wide
/// bucket resolves tied/rounded floats from its exact runs, as required by D-12.
fn maximum(reader: &mut Reader<'_>, key: u64, first: i32, last: i32) -> io::Result<Option<i128>> {
    let summary = reader.summary_within(
        key,
        (first, last),
        (f64::from(first), f64::from(last), 1),
        0,
        None,
    )?;
    let mut maximum: Option<i128> = None;
    for bucket in summary.buckets.into_iter().flatten() {
        let candidate =
            if bucket.max_value.abs().max(bucket.min_value.abs()) >= 9_007_199_254_740_992.0 {
                reader
                    .runs(
                        key,
                        bucket.first_hour.max(first),
                        bucket.last_hour.min(last),
                    )?
                    .into_iter()
                    .map(|run| run.value)
                    .max()
            } else {
                reader.value_at(key, bucket.max_hour)?.map(|run| run.value)
            };
        if let Some(value) = candidate {
            maximum = Some(maximum.map_or(value, |before| before.max(value)));
        }
    }
    Ok(maximum)
}

#[allow(clippy::too_many_arguments)]
fn read_pages(
    store: &store::Store,
    key: &store::SourceKey,
    branch: u64,
    bounds: (i32, i32),
    input: &Params,
    hour: i32,
    end: i32,
    cancel: &AtomicBool,
) -> io::Result<Option<Value>> {
    // One reader serves the endpoint reads and the peaks; each endpoint read
    // restores only the requested country's leaves.
    let count = pages::endpoint_count(store, key, branch)?;
    let mut reads = pages::Endpoints::new(store, key, branch, count, cancel);
    let Some(catalogue) = reads.catalogue()? else {
        return Ok(None);
    };
    let mut inventory: BTreeMap<bool, BTreeSet<u64>> = BTreeMap::new();
    let mut selected = Vec::new();
    for (token, &id) in &catalogue.spent_keys {
        let (tag, rocket, refits, variant): (String, bool, bool, u64) =
            serde_json::from_str(token)?;
        if tag == input.tag && refits == (rocket || input.refits) {
            inventory.entry(rocket).or_default().insert(variant);
            selected.push((rocket, variant, id));
        }
    }
    let mut scanned = catalogue.spending.scanned(&input.tag);
    let mut classes = catalogue.classifications.clone();
    let variant_warnings = catalogue.variant_warnings.clone();
    let dates: BTreeSet<_> = [
        Some(hour),
        Some(end),
        input.from.and_then(|from| from.checked_sub(1)),
    ]
    .into_iter()
    .flatten()
    .collect();
    let mut endpoints = BTreeMap::new();
    for &date in &dates {
        endpoints.insert(date, reads.page(date, pages::Leaves::Country(&input.tag))?);
    }
    for (&rocket, variants) in &inventory {
        let tags = if rocket {
            &mut scanned.rocket.tags
        } else {
            &mut scanned.tags
        };
        let track = tags.entry(input.tag.clone()).or_default();
        for &variant in variants {
            track.cost_changes.entry((variant, false)).or_default();
        }
    }
    for kind in classes.values_mut() {
        kind.names.clear();
    }
    let mut eq_scope = BTreeMap::new();
    let mut ro_scope = BTreeMap::new();
    let unique: BTreeMap<_, _> = endpoints
        .values()
        .flatten()
        .map(|page| ((page.first_hour, page.last_hour), page))
        .collect();
    for page in unique.values() {
        cancelled(cancel)?;
        scanned.observations.extend(
            [page.equipment_origin, page.equipment_restart]
                .into_iter()
                .flatten(),
        );
        scanned.rocket.observations.extend(
            [page.rocket_origin, page.rocket_restart]
                .into_iter()
                .flatten(),
        );
        for (variant, kind) in &page.classifications {
            if let Some(class) = classes.get_mut(variant) {
                for name in &kind.names {
                    if class
                        .names
                        .last()
                        .is_none_or(|last| last.first_hour < name.first_hour)
                    {
                        class.names.push(name.clone());
                    }
                }
            }
        }
        for rocket in [false, true] {
            let Some(source) = source_at(Some(page), &input.tag, rocket) else {
                continue;
            };
            if let Some(origin) = source.observation {
                if rocket {
                    scanned.rocket.observations.push(origin);
                } else {
                    scanned.observations.push(origin);
                }
            }
            let (tags, reads, scope) = if rocket {
                (
                    &mut scanned.rocket.tags,
                    &mut scanned.rocket.reads,
                    &mut ro_scope,
                )
            } else {
                (&mut scanned.tags, &mut scanned.reads, &mut eq_scope)
            };
            reads.push(source.read_hour);
            let track = tags.entry(input.tag.clone()).or_default();
            let sample = u32::try_from(track.hours.len())
                .map_err(|_| crate::invalid("Too many requested production samples"))?;
            track.hours.push(page.first_hour);
            track.last_hours.push(page.last_hour);
            track.starts.push(true);
            track.identities.push((source.slot, source.country_id, 0));
            track.presence.push(source.present);
            scope.insert(page.first_hour, source.campaign_spending_reason);
            let mut entries = BTreeMap::new();
            for amount in &source.amounts {
                let variant = amount
                    .variant
                    .parse::<u64>()
                    .map_err(|_| crate::invalid("Invalid production variant"))?;
                entries.insert(
                    (variant, amount.refit),
                    CostTotal {
                        raw: amount
                            .spent
                            .parse()
                            .map_err(|_| crate::invalid("Invalid production spending"))?,
                        reason: amount.spending_reason,
                    },
                );
            }
            for &variant in inventory.get(&rocket).into_iter().flatten() {
                if !entries.keys().any(|&(id, _)| id == variant) {
                    entries.insert((variant, false), CostTotal::default());
                }
            }
            for (entry, value) in entries {
                track
                    .cost_changes
                    .entry(entry)
                    .or_default()
                    .push((sample, value));
            }
        }
    }
    for reads in [&mut scanned.reads, &mut scanned.rocket.reads] {
        reads.sort_unstable();
        reads.dedup();
    }
    for observations in [&mut scanned.observations, &mut scanned.rocket.observations] {
        observations.sort_unstable();
        observations.dedup();
    }
    let params = ProductionSpendingParams {
        branch_id: input.branch_id.clone(),
        tag: input.tag.clone(),
        refits: input.refits,
    };
    let mut reply = reply_prepared(
        &scanned,
        &params,
        (&classes, &eq_scope, &ro_scope),
        variant_warnings,
        cancel,
    )?;
    for run in &mut reply.production {
        if run.present == Some(false) {
            run.empty_census_hour = unique
                .values()
                .find(|page| page.first_hour <= run.first_hour && run.first_hour <= page.last_hour)
                .and_then(|page| source_at(Some(page), &input.tag, false))
                .map(|source| source.read_hour);
        }
    }
    let mut continuity = BTreeMap::new();
    if let Some(before) = input
        .from
        .and_then(|from| from.checked_sub(1))
        .and_then(|hour| endpoints.get(&hour))
    {
        for date in [hour, end] {
            continuity.insert(
                date,
                [false, true].map(|rocket| {
                    continuous(
                        source_at(before.as_ref(), &input.tag, rocket),
                        source_at(
                            endpoints.get(&date).and_then(Option::as_ref),
                            &input.tag,
                            rocket,
                        ),
                    )
                }),
            );
        }
    }
    let reader = reads.reader()?;
    let first = input.from.unwrap_or(bounds.0).max(bounds.0);
    let last = end.min(bounds.1);
    let mut peaks = BTreeMap::new();
    if first <= last {
        for (rocket, variant, id) in selected {
            if let Some(value) = maximum(reader, id, first, last)? {
                peaks.insert(
                    serde_json::to_string(&(variant.to_string(), rocket))?,
                    value.to_string(),
                );
            }
        }
    }
    let mut data = serde_json::to_value(reply)?;
    data["continuity"] = json!(continuity);
    data["peaks"] = json!(peaks);
    data["exactHours"] = json!(dates);
    Ok(Some(data))
}

#[cfg(test)]
mod tests;
