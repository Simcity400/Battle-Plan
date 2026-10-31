//! Spending reads production's prepared facts: one country's holder at its
//! dates, its spent series' peaks and the classes content gives its designs.
use super::*;
use crate::prepared::{prefix::Prefix, series::Reader};
use crate::prepared_production::{self as pages, facts, read};
use serde_json::{Value, json};

/// A prepared spending request: one country's amounts at its dates.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct Params {
    pub branch_id: String,
    pub tag: String,
    pub refits: bool,
    pub from: Option<i32>,
    pub hour: Option<i32>,
    pub end: Option<i32>,
}

pub fn query(
    source: &Source,
    params: Value,
    content: Option<&super::super::ContentOptions>,
    cancel: &AtomicBool,
) -> io::Result<Option<Value>> {
    let input: Params = serde_json::from_value(params)?;
    let purposes = Purposes::counting(input.refits);
    query_with(
        source,
        input,
        content,
        "production-spending",
        purposes,
        cancel,
    )
    .map(|(data, _)| Some(data))
}

/// The dates a prepared spending reply answers: those requested, or the
/// prepared prefix's last hour before preparation reaches them.
pub(crate) struct Answered {
    pub branch: u64,
    pub bounds: (i32, i32),
    /// A range's first hour, when its baseline is prepared.
    pub from: Option<i32>,
    pub hour: i32,
    pub end: i32,
}

/// [`query`] for a product gated as `gate`, counting `purposes`. A reply split
/// by purpose has one leaf per design and purpose, and its `peaks` hold new
/// production's only, none for a design with converted or unrecorded-purpose
/// input.
pub(crate) fn query_with(
    source: &Source,
    input: Params,
    content: Option<&super::super::ContentOptions>,
    gate: &str,
    purposes: Purposes,
    cancel: &AtomicBool,
) -> io::Result<(Value, Answered)> {
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
    let coverage =
        crate::prepared::prefix::read_or_empty(source, content, gate, branch, None, cancel)?
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
    let mut data = match pages::read::index(source, content, &store, &key, branch, cancel)? {
        Some((index, classes)) => read_pages(
            Some(source),
            &index,
            &classes,
            &store,
            &key,
            bounds,
            &input,
            hour,
            end,
            purposes,
            cancel,
        )?,
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
    Ok((
        data,
        Answered {
            branch,
            bounds,
            from: input.from,
            hour,
            end,
        },
    ))
}

/// Ends every run list of a prefix reply (each leaf's, the total, the
/// equipment total and the Rocket total) with the pending hours, unknown with the reason
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
        if leaf["unproven"]
            .as_array()
            .is_some_and(|runs| !runs.is_empty())
        {
            pending(&mut leaf["unproven"])?;
        }
    }
    pending(&mut data["total"])?;
    // Only a reply split by purpose carries the equipment total.
    if data["equipmentTotal"]
        .as_array()
        .is_some_and(|runs| !runs.is_empty())
    {
        pending(&mut data["equipmentTotal"])?;
    }
    pending(&mut data["rocketTotal"])
}

/// A holder's source at an endpoint, its reads resolved there.
struct SourceAt<'a> {
    head: &'a facts::Head,
    read_hour: i32,
    source: &'a facts::Source,
}

/// A country's source at an endpoint read for its reply.
fn source_of<'a>(
    index: &read::Index,
    unique: &'a BTreeMap<u64, Option<facts::Holder>>,
    ordinal: u64,
    rocket: bool,
) -> io::Result<Option<SourceAt<'a>>> {
    let Some(Some(state)) = unique.get(&ordinal) else {
        return Ok(None);
    };
    let Some(source) = (if rocket {
        &state.rocket
    } else {
        &state.equipment
    }) else {
        return Ok(None);
    };
    Ok(Some(SourceAt {
        head: &source.head,
        read_hour: index.endpoint(ordinal).read_of(&source.head, rocket)?,
        source,
    }))
}

fn continuous(a: Option<&SourceAt<'_>>, b: Option<&SourceAt<'_>>) -> bool {
    a.zip(b).is_some_and(|(a, b)| {
        (
            a.head.slot,
            a.head.country_id,
            a.head.observation,
            a.head.segment,
        ) == (
            b.head.slot,
            b.head.country_id,
            b.head.observation,
            b.head.segment,
        )
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

/// Each selected design's peak over `first..=last`, keyed as the reply sends it.
pub(crate) fn peaks_of(
    store: &store::Store,
    key: &store::SourceKey,
    branch: u64,
    selected: &[(bool, u64, u64)],
    (first, last): (i32, i32),
    cancel: &AtomicBool,
) -> io::Result<BTreeMap<String, String>> {
    let mut peaks = BTreeMap::new();
    if first > last || selected.is_empty() {
        return Ok(peaks);
    }
    let mut reader = Reader::new(store, key, pages::spent_section(branch), cancel)?;
    for &(rocket, variant, id) in selected {
        if let Some(value) = maximum(&mut reader, id, first, last)? {
            peaks.insert(
                serde_json::to_string(&(variant.to_string(), rocket))?,
                value.to_string(),
            );
        }
    }
    Ok(peaks)
}

/// One country's reply at its dates, from the branch's index. With a source,
/// its peaks are kept for the publication and window.
#[allow(clippy::too_many_arguments)]
pub(crate) fn read_pages(
    source: Option<&Source>,
    index: &read::Index,
    classes: &read::Classes,
    store: &store::Store,
    key: &store::SourceKey,
    bounds: (i32, i32),
    input: &Params,
    hour: i32,
    end: i32,
    purposes: Purposes,
    cancel: &AtomicBool,
) -> io::Result<Value> {
    let holder = index.tags.get(&input.tag).copied();
    let series = holder.map_or(&[][..], |holder| &index.keys[holder as usize][..]);
    let mut inventory: BTreeMap<bool, BTreeSet<u64>> = BTreeMap::new();
    let mut selected = Vec::new();
    // A design with converted or unrecorded-purpose input has no peak of its
    // new leaf alone in a split reply: its series without refits holds them.
    let mut separate = BTreeSet::new();
    if purposes == Purposes::Split {
        for row in series {
            if let read::SeriesKey::Purpose { variant, .. } = row {
                separate.insert(*variant);
            }
        }
    }
    for row in series {
        let &read::SeriesKey::Spent {
            rocket,
            refits,
            variant,
            key: id,
        } = row
        else {
            continue;
        };
        // The combined key lists every design; the key without refits lists
        // those with new or converted input.
        if refits == (rocket || purposes != Purposes::WithoutRefits) {
            inventory.entry(rocket).or_default().insert(variant);
        }
        // A split reply's peaks are new production's, from the series without
        // refits: a refit-only maximum is no difference of two maxima.
        if refits == (rocket || purposes == Purposes::Combined)
            && (rocket || !separate.contains(&variant))
        {
            selected.push((rocket, variant, id));
        }
    }
    let mut scanned = index.diagnostics.spending.scanned(holder, &input.tag);
    let dates: BTreeSet<_> = [
        Some(hour),
        Some(end),
        input.from.and_then(|from| from.checked_sub(1)),
    ]
    .into_iter()
    .flatten()
    .collect();
    let endpoints: BTreeMap<i32, Option<u64>> = dates
        .iter()
        .map(|&date| (date, index.endpoint_at(date)))
        .collect();
    // Each endpoint read once, with the country's holder there.
    let mut unique: BTreeMap<u64, Option<facts::Holder>> = BTreeMap::new();
    for &ordinal in endpoints.values().flatten() {
        if let std::collections::btree_map::Entry::Vacant(entry) = unique.entry(ordinal) {
            entry.insert(match holder {
                Some(holder) => index.holder_at(store, key, holder, ordinal)?,
                None => None,
            });
        }
    }
    for (&rocket, variants) in &inventory {
        let tags = if rocket {
            &mut scanned.rocket.tags
        } else {
            &mut scanned.tags
        };
        let track = tags.entry(input.tag.clone()).or_default();
        for &variant in variants {
            track.cost_changes.entry((variant, Purpose::New)).or_default();
        }
    }
    let mut eq_scope = BTreeMap::new();
    let mut ro_scope = BTreeMap::new();
    for &ordinal in unique.keys() {
        cancelled(cancel)?;
        let page = index.endpoint(ordinal);
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
        for rocket in [false, true] {
            let Some(at) = source_of(index, &unique, ordinal, rocket)? else {
                continue;
            };
            if let Some(origin) = at.head.observation {
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
            reads.push(at.read_hour);
            let track = tags.entry(input.tag.clone()).or_default();
            let sample = u32::try_from(track.hours.len())
                .map_err(|_| crate::invalid("Too many requested production samples"))?;
            track.hours.push(page.first_hour);
            track.last_hours.push(page.last_hour);
            track.starts.push(true);
            track.identities.push((at.head.slot, at.head.country_id, 0));
            track.presence.push(at.head.present);
            scope.insert(page.first_hour, at.head.campaign_reason);
            let mut entries = BTreeMap::new();
            for leaf in &at.source.leaves {
                entries.insert(
                    (index.variants[leaf.variant as usize], leaf.purpose),
                    CostTotal {
                        raw: i128::from(leaf.spent),
                        reason: leaf.reason,
                        unproven_raw: i128::from(leaf.unproven),
                    },
                );
            }
            for &variant in inventory.get(&rocket).into_iter().flatten() {
                if !entries.keys().any(|&(id, _)| id == variant) {
                    entries.insert((variant, Purpose::New), CostTotal::default());
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
    // The publication's types of the designs the reply reads, named as the
    // dates' endpoints recorded them.
    let designs: BTreeSet<u64> = scanned
        .tags
        .values()
        .chain(scanned.rocket.tags.values())
        .flat_map(|track| track.changes.keys().chain(track.cost_changes.keys()))
        .map(|(variant, _)| *variant)
        .collect();
    let mut kinds = BTreeMap::new();
    for variant in designs {
        let Some(id) = index.variant_id(variant) else {
            continue;
        };
        let Some(class) = classes.of.get(id as usize).and_then(Option::as_ref) else {
            continue;
        };
        let mut names: Vec<StockVariantName> = Vec::new();
        for &ordinal in unique.keys() {
            for name in index.names_at(classes, id, ordinal)?.into_iter().flatten() {
                if names
                    .last()
                    .is_none_or(|last| last.first_hour < name.first_hour)
                {
                    names.push(name);
                }
            }
        }
        kinds.insert(
            variant,
            read::Kind {
                key: class.key.clone(),
                ordinal: class.ordinal,
                domain: class.domain,
                name: class.name.clone(),
                reason: class.reason.clone(),
                names,
            },
        );
    }
    let params = ProductionSpendingParams {
        branch_id: input.branch_id.clone(),
        tag: input.tag.clone(),
        refits: input.refits,
    };
    let mut reply = reply_prepared(
        &scanned,
        &params,
        (&kinds, &eq_scope, &ro_scope),
        index.variant_warnings(),
        cancel,
        purposes,
    )?;
    for run in &mut reply.production {
        if run.present == Some(false) {
            let ordinal = unique.keys().copied().find(|&ordinal| {
                let page = index.endpoint(ordinal);
                page.first_hour <= run.first_hour && run.first_hour <= page.last_hour
            });
            run.empty_census_hour = match ordinal {
                Some(ordinal) => source_of(index, &unique, ordinal, false)?.map(|at| at.read_hour),
                None => None,
            };
        }
    }
    let mut continuity = BTreeMap::new();
    if let Some(before) = input
        .from
        .and_then(|from| from.checked_sub(1))
        .and_then(|hour| endpoints.get(&hour))
    {
        for date in [hour, end] {
            let mut joined = [false; 2];
            for (slot, rocket) in joined.iter_mut().zip([false, true]) {
                let earlier = match before {
                    Some(ordinal) => source_of(index, &unique, *ordinal, rocket)?,
                    None => None,
                };
                let later = match endpoints.get(&date).copied().flatten() {
                    Some(ordinal) => source_of(index, &unique, ordinal, rocket)?,
                    None => None,
                };
                *slot = continuous(earlier.as_ref(), later.as_ref());
            }
            continuity.insert(date, joined);
        }
    }
    let window = (input.from.unwrap_or(bounds.0).max(bounds.0), end.min(bounds.1));
    let peaks = match source {
        // Peaks hold for the publication and window, whatever the hour.
        Some(source) => crate::prepared::query::observations::scope(
            source,
            "production:peaks",
            Some(key),
        )
        .get_entry(
            index.branch,
            &(&input.tag, purposes == Purposes::Split, purposes == Purposes::Combined, window),
            &index.publication(),
            cancel,
            crate::prepared::query::observations::serde_bytes,
            || peaks_of(store, key, index.branch, &selected, window, cancel),
        )?
        .as_ref()
        .clone(),
        None => peaks_of(store, key, index.branch, &selected, window, cancel)?,
    };
    let mut data = serde_json::to_value(reply)?;
    data["continuity"] = json!(continuity);
    data["peaks"] = json!(peaks);
    data["exactHours"] = json!(dates);
    Ok(data)
}

#[cfg(test)]
pub(crate) mod tests;
