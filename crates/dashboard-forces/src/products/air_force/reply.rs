use super::*;
use dashboard_base::{
    products::equipment::{UNRESOLVED_MARK, archetype_of},
    ranking,
};
use std::{
    collections::{BTreeSet, HashMap},
    sync::atomic::Ordering,
};

pub(super) fn cancel(cancelled: &AtomicBool) -> io::Result<()> {
    if cancelled.load(Ordering::Acquire) {
        Err(io::Error::new(
            io::ErrorKind::Interrupted,
            "Air force request cancelled",
        ))
    } else {
        Ok(())
    }
}

fn equipment_part(
    raw: &str,
    units: Option<&installed_content::equipment::Database>,
    localise: &impl Fn(&str) -> Option<String>,
    refusals: &mut BTreeSet<String>,
) -> AirForcePart {
    if raw == UNIDENTIFIED {
        return AirForcePart {
            key: raw.into(),
            name: Some("Other / not identified".into()),
            unresolved: true,
        };
    }
    let (variant, token) = raw.split_once('/').unwrap_or((raw, ""));
    if let Some(units) = units {
        match archetype_of(units, token) {
            Ok((key, _, _)) if !key.starts_with(UNRESOLVED_MARK) => {
                return AirForcePart {
                    name: localise(&key),
                    key,
                    unresolved: false,
                };
            }
            Ok((key, _, _)) => {
                refusals.insert(format!("{token}: archetype {key} uses the reserved unresolved mark; its variants remain separate."));
            }
            Err(reason) if !token.is_empty() => {
                refusals.insert(format!("{token}: {reason}; its variants remain separate."));
            }
            Err(_) => {}
        }
    }
    AirForcePart {
        // A later spelling must not name an earlier unresolved observation.
        key: if token.is_empty() {
            format!("{UNRESOLVED_MARK}variant/{variant}")
        } else {
            format!("{UNRESOLVED_MARK}variant/{variant}/{token}")
        },
        name: Some(format!("Variant {variant}")),
        unresolved: true,
    }
}

fn wing_part(keys: &[String], parts: &BTreeMap<String, AirForcePart>) -> AirForcePart {
    let resolved: BTreeMap<_, _> = keys
        .iter()
        .map(|key| {
            let part = &parts[key];
            (part.key.clone(), part)
        })
        .collect();
    if resolved.len() == 1 {
        return (**resolved.values().next().expect("one wing type")).clone();
    }
    AirForcePart {
        key: format!(
            "mixed:{}",
            serde_json::to_string(&resolved.keys().collect::<Vec<_>>()).expect("string keys")
        ),
        name: Some(format!(
            "Mixed types: {}",
            resolved
                .values()
                .map(|part| part
                    .name
                    .as_deref()
                    .unwrap_or("Equipment type name unavailable"))
                .collect::<Vec<_>>()
                .join(" / ")
        )),
        unresolved: resolved.values().any(|part| part.unresolved),
    }
}

#[derive(Default)]
pub(super) struct Parts {
    pub(super) tokens:
        dashboard_text::catalogue::KeyedResults<String, (AirForcePart, BTreeSet<String>)>,
    pub(super) signatures: dashboard_text::catalogue::KeyedResults<Vec<String>, AirForcePart>,
}

#[cfg(test)]
pub(super) fn reply(
    scanned: &ScannedAir,
    params: &AirForceParams,
    units: Option<&installed_content::equipment::Database>,
    localise: impl Fn(&str) -> Option<String>,
    warnings: Vec<String>,
    cancelled: &AtomicBool,
) -> io::Result<AirForceReply> {
    reply_cached(scanned, params, units, localise, warnings, cancelled, None)
}

/// Project a cached census, preserving unknown variants and whole mixed wings.
pub(super) fn reply_cached(
    scanned: &ScannedAir,
    params: &AirForceParams,
    units: Option<&installed_content::equipment::Database>,
    localise: impl Fn(&str) -> Option<String>,
    warnings: Vec<String>,
    cancelled: &AtomicBool,
    cache: Option<&Parts>,
) -> io::Result<AirForceReply> {
    cancel(cancelled)?;
    let membership = params
        .top
        .map(|count| {
            ranking::top_membership(
                &scanned.series,
                count as usize,
                |run| (run.first_hour, run.last_hour),
                |run| run.total(params),
                cancelled,
            )
        })
        .transpose()?;
    selected(
        scanned, params, units, localise, warnings, cancelled, membership, cache,
    )
}

#[allow(clippy::too_many_arguments)]
pub(super) fn selected(
    scanned: &ScannedAir,
    params: &AirForceParams,
    units: Option<&installed_content::equipment::Database>,
    localise: impl Fn(&str) -> Option<String>,
    mut warnings: Vec<String>,
    cancelled: &AtomicBool,
    membership: Option<BTreeMap<String, Vec<HourSpan>>>,
    cache: Option<&Parts>,
) -> io::Result<AirForceReply> {
    let tags: BTreeSet<&str> = match &membership {
        Some(membership) => membership.keys().map(String::as_str).collect(),
        None => params.tags.iter().map(String::as_str).collect(),
    };
    let mut tokens = BTreeSet::new();
    let mut signatures = BTreeSet::new();
    for tag in &tags {
        for run in scanned.series.get(*tag).into_iter().flatten() {
            if let Some(counts) = run.counts(params.ownership) {
                tokens.extend(counts.planes_by_token.keys().cloned());
                for signature in counts.wings_by_tokens.keys() {
                    tokens.extend(signature.iter().cloned());
                    signatures.insert(signature.clone());
                }
            }
        }
    }
    let mut refusals = BTreeSet::new();
    let mut token_parts = BTreeMap::new();
    for raw in &tokens {
        let resolve = || {
            let mut notes = BTreeSet::new();
            let part = equipment_part(raw, units, &localise, &mut notes);
            Ok((part, notes))
        };
        let resolved = match cache {
            Some(cache) => cache.tokens.get_or_init(raw, |_| resolve())?,
            None => Arc::new(resolve()?),
        };
        token_parts.insert(raw.clone(), resolved.0.clone());
        refusals.extend(resolved.1.iter().cloned());
    }
    warnings.extend(refusals);
    let mut signature_parts = BTreeMap::new();
    for keys in signatures {
        let part = match cache {
            Some(cache) => cache
                .signatures
                .get_or_init(&keys, |keys| Ok(wing_part(keys, &token_parts)))?,
            None => Arc::new(wing_part(&keys, &token_parts)),
        };
        signature_parts.insert(keys, part.as_ref().clone());
    }
    let mut catalogue = BTreeMap::new();
    match params.measure {
        AirForceMeasure::Planes => {
            for part in token_parts.values() {
                catalogue.insert(part.key.clone(), part.clone());
            }
        }
        AirForceMeasure::Wings => {
            for part in signature_parts.values() {
                catalogue.insert(part.key.clone(), part.clone());
            }
        }
    }
    let parts: Vec<_> = catalogue.into_values().collect();
    let positions: HashMap<_, _> = parts
        .iter()
        .enumerate()
        .map(|(index, part)| (part.key.as_str(), index))
        .collect();
    let mut series = Vec::new();
    for tag in tags {
        cancel(cancelled)?;
        let Some(source) = scanned.series.get(tag) else {
            continue;
        };
        let segments = scanned
            .segments
            .get(tag)
            .map(Vec::as_slice)
            .unwrap_or_default();
        let spans = membership
            .as_ref()
            .and_then(|membership| membership.get(tag));
        let mut runs: Vec<AirForceRun> = Vec::new();
        let mut project = |run: &RawRun, first_hour: i32, last_hour: i32| {
            let counts = run.counts(params.ownership);
            let total = run.total(params);
            let mut values = vec![total.map(|_| 0i64); parts.len()];
            let mut add = |part: &AirForcePart, amount: Option<i64>| -> io::Result<()> {
                let value = &mut values[positions[part.key.as_str()]];
                *value = match (*value, amount) {
                    (Some(previous), Some(amount)) => Some(
                        previous
                            .checked_add(amount)
                            .filter(|sum| *sum <= MAX_EXACT)
                            .ok_or_else(|| {
                                io::Error::other("Air force count exceeds exact integer range")
                            })?,
                    ),
                    _ => None,
                };
                Ok(())
            };
            if let Some(counts) = counts {
                match params.measure {
                    AirForceMeasure::Planes => {
                        for (token, amount) in &counts.planes_by_token {
                            add(&token_parts[token], *amount)?;
                        }
                    }
                    AirForceMeasure::Wings => {
                        for (signature, amount) in &counts.wings_by_tokens {
                            add(&signature_parts[signature], Some(*amount))?;
                        }
                    }
                }
            }
            let projected = AirForceRun {
                first_hour,
                last_hour,
                starts_segment: run.starts_segment && first_hour == run.first_hour,
                total,
                values,
            };
            match runs.last_mut() {
                Some(previous)
                    if !projected.starts_segment
                        && previous.last_hour.checked_add(1) == Some(first_hour)
                        && previous.total == total
                        && previous.values == projected.values =>
                {
                    previous.last_hour = last_hour;
                }
                _ => runs.push(projected),
            }
            Ok(())
        };
        match spans {
            Some(spans) => ranking::for_each_piece(
                source,
                spans,
                |run| (run.first_hour, run.last_hour),
                cancelled,
                &mut project,
            )?,
            None => {
                for run in source {
                    project(run, run.first_hour, run.last_hour)?;
                }
            }
        }
        series.push(AirForceSeries {
            tag: tag.into(),
            identities: Vec::new(),
            runs,
            segments: segments.to_vec(),
            membership: spans.cloned(),
        });
    }
    let unresolved = parts.iter().filter(|part| part.unresolved).count();
    for (tag, count) in &scanned.undisplayable_tags {
        warnings.push(format!(
            "Country tag bytes {tag:?}: {count} observations are not UTF-8 and cannot be displayed."
        ));
    }
    if unresolved != 0 {
        warnings.push(format!("{unresolved} air equipment parts could not be resolved through the recorded content; each remains visible."));
    }
    for (count, reason) in [
        (
            scanned.unreadable_owners[params.ownership.index()],
            "changed hours have an incomplete wing census or unresolved ownership for this setting; country totals stay unknown",
        ),
        (
            scanned.equipment_missing,
            "changed hours have wings without a complete equipment bank; their planes remain Other / not identified",
        ),
        (
            scanned.equipment_refused,
            "changed hours have wings with invalid or non-reconciling equipment counts; their split is unknown",
        ),
        (
            scanned.equipment_remainders,
            "changed hours have wings with plane counts beyond their equipment bank; the remainder stays Other / not identified",
        ),
    ] {
        if count != 0 {
            warnings.push(format!("{count} {reason}."));
        }
    }
    let reply = AirForceReply {
        branch_id: params.branch_id.clone(),
        parts,
        series,
        warnings,
    };
    Ok(reply)
}
