//! Spending reads share production endpoints, classifications and exact S values.
use super::*;
use crate::prepared::{prefix::Prefix, series::Reader};
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
    // Production prepared on its own answers from its own coverage.
    let coverage = crate::prepared::prefix::read_or_empty(
        source,
        content,
        "production-spending",
        branch,
        None,
        cancel,
    )?
    .coverage;
    let requested = (
        input.hour.unwrap_or(bounds.1),
        input.end.unwrap_or(bounds.1),
    );
    // The amounts are cumulative endpoints, but a later token can reclassify
    // any earlier design. The prefix's grouping is provisional until complete.
    let prefix = Prefix::new(source, branch, coverage, Some(requested.0.max(requested.1)));
    let through = |hour: i32| prefix.through(Some(hour)).unwrap_or(hour);
    let (hour, end) = (through(requested.0), through(requested.1));
    let mut input = input;
    // A range whose baseline lies after the prefix has nothing prepared yet:
    // no page after the prefix is read, and every hour of it is pending.
    if !prefix.coverage.complete
        && input
            .from
            .is_some_and(|from| from.saturating_sub(1) > prefix.coverage.last_hour)
    {
        input.from = None;
    }
    let (store, key) = crate::prepared::files(source, content)?;
    let mut data = match read_pages(&store, &key, branch, bounds, &input, hour, end, cancel)? {
        Some(data) => data,
        None => serde_json::to_value(ProductionSpendingReply {
            branch_id: input.branch_id.clone(),
            tag: input.tag.clone(),
            decimals: 5,
            ..Default::default()
        })?,
    };
    pend(&mut data, &prefix)?;
    prefix.mark_revisable(source, branch, &mut data)?;
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

/// Ends every run list of a prefix reply (each leaf's, the total and the
/// Rocket total) with the pending hours, unknown with the reason
/// `preparing`: later preparation answers them, so a reader at one of them
/// finds no unread hour.
fn pend(data: &mut Value, prefix: &Prefix) -> io::Result<()> {
    if prefix.pending.is_empty() {
        return Ok(());
    }
    let pending = |runs: &mut Value| -> io::Result<()> {
        let runs = runs
            .as_array_mut()
            .ok_or_else(|| crate::invalid("Production spending runs are not a list"))?;
        let after = runs
            .last()
            .and_then(|run| run["lastHour"].as_i64())
            .map_or(i64::MIN, |hour| hour);
        for span in &prefix.pending {
            if i64::from(span.last_hour) <= after {
                continue;
            }
            let first = i64::from(span.first_hour).max(after.saturating_add(1));
            runs.push(serde_json::to_value(ProductionSpendingRun {
                first_hour: i32::try_from(first).expect("between two hours"),
                last_hour: span.last_hour,
                total_raw: None,
                reason: Some(ProductionSpendingUnknown::Preparing),
            })?);
        }
        Ok(())
    };
    for leaf in data["leaves"]
        .as_array_mut()
        .ok_or_else(|| crate::invalid("Production spending leaves are not a list"))?
    {
        pending(&mut leaf["runs"])?;
    }
    pending(&mut data["total"])?;
    pending(&mut data["rocketTotal"])
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
