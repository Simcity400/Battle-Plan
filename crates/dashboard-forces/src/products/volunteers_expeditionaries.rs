//! Observed land deployments, not dispatch transactions or integrated lifetime totals.
use super::army_size::aid::Catalogue;
use dashboard_base::{
    Source,
    prepared::{
        self, prefix,
        series::{self, Reader, Run},
    },
    products::{LoadContext, Product, check_reply_budget, country_groups::HourSpan},
};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet, btree_map::Entry},
    io,
    sync::atomic::AtomicBool,
};
use ts_rs::TS;

pub struct VolunteersExpeditionaries;
#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
pub enum MilitaryAidKind {
    Land,
    Expeditionary,
}
#[derive(Clone, Debug, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct VolunteersExpeditionariesParams {
    pub branch_id: String,
    pub kind: MilitaryAidKind,
    pub tags: Option<Vec<String>>,
}
#[derive(Clone, Debug, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct MilitaryAidRun {
    pub first_hour: i32,
    pub last_hour: i32,
    pub divisions: Option<f64>,
    pub starts_segment: bool,
}
#[derive(Clone, Debug, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct MilitaryAidLink {
    pub key: String,
    pub sender: String,
    pub sender_slot: Option<u32>,
    pub sender_country_id: Option<u32>,
    pub host: String,
    pub host_slot: Option<u32>,
    pub host_country_id: Option<u32>,
    pub runs: Vec<MilitaryAidRun>,
}
#[derive(Clone, Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct VolunteersExpeditionariesReply {
    pub links: Vec<MilitaryAidLink>,
    pub tallies: Vec<MilitaryAidLink>,
    pub unobserved: Vec<HourSpan>,
    #[serde(flatten)]
    pub prefix: prefix::Prefix,
    pub warnings: Vec<String>,
}
impl Product for VolunteersExpeditionaries {
    const KEY: &'static str = "volunteers-expeditionaries";
    const CONTENT_DEPENDENT: bool = false;
    type Params = VolunteersExpeditionariesParams;
    type Reply = VolunteersExpeditionariesReply;
    fn validate(params: &Self::Params) -> Result<(), &'static str> {
        params
            .branch_id
            .parse::<u64>()
            .map_err(|_| "Invalid aid branch")?;
        for tag in params.tags.iter().flatten() {
            dashboard_core::protocol::validate_tag(tag)?;
        }
        Ok(())
    }
    fn types() -> Vec<String> {
        vec![
            MilitaryAidKind::decl(),
            VolunteersExpeditionariesParams::decl(),
            MilitaryAidRun::decl(),
            MilitaryAidLink::decl(),
            VolunteersExpeditionariesReply::decl(),
        ]
    }
}

fn value(runs: &[Run], hour: i32) -> Option<&Run> {
    runs.get(
        runs.partition_point(|run| run.first_hour <= hour)
            .checked_sub(1)?,
    )
    .filter(|run| run.last_hour >= hour)
}
fn project(values: &[Run], proofs: [&[Run]; 3], first: i32, last: i32) -> Vec<MilitaryAidRun> {
    if first > last {
        return Vec::new();
    }
    let mut cuts = BTreeSet::from([i64::from(first), i64::from(last) + 1]);
    for run in values.iter().chain(proofs.into_iter().flatten()) {
        cuts.insert(i64::from(run.first_hour).max(i64::from(first)));
        cuts.insert((i64::from(run.last_hour) + 1).min(i64::from(last) + 1));
    }
    let cuts: Vec<_> = cuts
        .into_iter()
        .filter(|&hour| i64::from(first) <= hour && hour <= i64::from(last) + 1)
        .collect();
    let mut result: Vec<MilitaryAidRun> = Vec::new();
    for span in cuts.windows(2) {
        let hour = span[0] as i32;
        let known = proofs.iter().all(|runs| value(runs, hour).is_some());
        let raw = known.then(|| value(values, hour).map_or(0, |run| run.value));
        let boundary = proofs
            .iter()
            .flat_map(|runs| value(runs, hour))
            .chain(value(values, hour))
            .any(|run| run.first_hour == hour && run.break_before);
        let starts = known && (boundary || result.last().is_none_or(|run| run.divisions.is_none()));
        let amount = raw.map(|value| value as f64);
        if let Some(run) = result
            .last_mut()
            .filter(|run| run.divisions == amount && !starts)
        {
            run.last_hour = (span[1] - 1) as i32;
        } else {
            result.push(MilitaryAidRun {
                first_hour: hour,
                last_hour: (span[1] - 1) as i32,
                divisions: amount,
                starts_segment: starts,
            });
        }
    }
    result
}
fn merge_spans(spans: &mut Vec<HourSpan>) {
    spans.sort_unstable_by_key(|span| span.first_hour);
    let mut end = 0;
    for at in 0..spans.len() {
        let span = spans[at];
        if end > 0 && i64::from(span.first_hour) <= i64::from(spans[end - 1].last_hour) + 1 {
            spans[end - 1].last_hour = spans[end - 1].last_hour.max(span.last_hour);
        } else {
            spans[end] = span;
            end += 1;
        }
    }
    spans.truncate(end);
}
pub fn query(
    source: &Source,
    params: &VolunteersExpeditionariesParams,
    context: &LoadContext,
    cancelled: &AtomicBool,
) -> io::Result<VolunteersExpeditionariesReply> {
    VolunteersExpeditionaries::validate(params).map_err(dashboard_core::invalid)?;
    let branch = params
        .branch_id
        .parse::<u64>()
        .map_err(|_| dashboard_core::invalid("Invalid aid branch"))?;
    let prefix = prefix::read_or_empty(
        source,
        context.content.as_ref(),
        VolunteersExpeditionaries::KEY,
        branch,
        None,
        cancelled,
    )?;
    let (store, key) = prepared::files(source, context.content.as_ref())?;
    let catalogue = series::read_catalogue::<Catalogue>(&store, &key, "army-aid", branch)?
        .map(|(_, catalogue)| catalogue)
        .unwrap_or_default();
    let through = prefix.coverage.last_hour;
    let mut reply = read(&store, &key, branch, params, catalogue, prefix, cancelled)?;
    reply
        .unobserved
        .extend(prefix::gaps_after(source, branch, through));
    merge_spans(&mut reply.unobserved);
    check_reply_budget(&reply)?;
    Ok(reply)
}
pub(crate) fn read(
    store: &store::Store,
    key: &store::SourceKey,
    branch: u64,
    params: &VolunteersExpeditionariesParams,
    catalogue: Catalogue,
    prefix: prefix::Prefix,
    cancelled: &AtomicBool,
) -> io::Result<VolunteersExpeditionariesReply> {
    dashboard_text::metadata::check_cancel(cancelled)?;
    let (first, last) = (prefix.coverage.first_hour, prefix.coverage.last_hour);
    if first > last {
        return Ok(VolunteersExpeditionariesReply {
            links: Vec::new(),
            tallies: Vec::new(),
            unobserved: Vec::new(),
            prefix,
            warnings: Vec::new(),
        });
    }
    let channel = if params.kind == MilitaryAidKind::Land {
        0
    } else {
        1
    };
    let mut values = Reader::new(store, key, format!("army-aid-values-{branch}"), cancelled)?;
    let mut proof = Reader::new(store, key, format!("army-aid-proof-{branch}"), cancelled)?;
    let mut proofs = BTreeMap::<u64, Vec<Run>>::new();
    let mut links = Vec::new();
    let mut tallies = Vec::new();
    let mut unobserved = Vec::new();
    let mut bytes = 0usize;
    let wanted = params
        .tags
        .as_ref()
        .map(|tags| tags.iter().map(String::as_str).collect::<BTreeSet<_>>());
    for (at, link) in catalogue.links.iter().enumerate() {
        dashboard_text::metadata::check_cancel(cancelled)?;
        if link.channels & ((1 << channel) | if channel == 0 { 1 << 2 } else { 0 }) == 0 {
            continue;
        }
        let sender = &catalogue.actors[link.sender as usize];
        let host = &catalogue.actors[link.host as usize];
        if wanted.as_ref().is_some_and(|tags| {
            !tags.contains(sender.tag.as_str()) && !tags.contains(host.tag.as_str())
        }) {
            continue;
        }
        for (target, selected) in [(&mut links, channel), (&mut tallies, 2)] {
            if selected == 2 && params.kind != MilitaryAidKind::Land {
                continue;
            }
            let keys = [
                4 + link.sender * 4 + selected,
                4 + link.sender * 4 + 3,
                4 + link.host * 4 + 3,
            ];
            for id in keys {
                if let Entry::Vacant(entry) = proofs.entry(id) {
                    entry.insert(proof.runs(id, first, last)?);
                }
            }
            let runs = values.runs(at as u64 * 3 + selected, first, last)?;
            let source_presence = &proofs[&keys[1]];
            let host_presence = &proofs[&keys[2]];
            let left = source_presence
                .first()
                .zip(host_presence.first())
                .map(|(a, b)| a.first_hour.max(b.first_hour))
                .unwrap_or(last.saturating_add(1));
            let right = source_presence
                .last()
                .zip(host_presence.last())
                .map(|(a, b)| a.last_hour.min(b.last_hour))
                .unwrap_or(first.saturating_sub(1));
            let runs = project(
                &runs,
                keys.map(|key| proofs[&key].as_slice()),
                left.max(first),
                right.min(last),
            );
            for run in &runs {
                if run.divisions.is_none() {
                    unobserved.push(HourSpan {
                        first_hour: run.first_hour,
                        last_hour: run.last_hour,
                    });
                }
            }
            let link_key = format!("army-aid:{branch}:{at}:{selected}");
            bytes = bytes.saturating_add(
                size_of::<MilitaryAidLink>()
                    + runs.len() * size_of::<MilitaryAidRun>()
                    + link_key.len()
                    + sender.tag.len()
                    + host.tag.len(),
            );
            if bytes > dashboard_base::products::MAX_PRODUCT_REPLY_BYTES {
                return Err(io::Error::other(
                    "Military aid history exceeds the reply budget; narrow the country selection",
                ));
            }
            // Catalogue pairs stay distinct when either endpoint's tag is reused.
            target.push(MilitaryAidLink {
                key: link_key,
                sender: sender.tag.clone(),
                sender_slot: Some(sender.slot),
                sender_country_id: Some(sender.id),
                host: host.tag.clone(),
                host_slot: Some(host.slot),
                host_country_id: Some(host.id),
                runs,
            });
        }
    }
    // A sparse empty link catalogue is not proof of an empty native tally
    // roster. Preserve both source channels even when neither has a pair yet.
    for source_channel in [channel, 2]
        .into_iter()
        .take(if channel == 0 { 2 } else { 1 })
    {
        let known = proof.runs(source_channel, first, last)?;
        let mut from = first;
        for run in known {
            if from < run.first_hour {
                unobserved.push(HourSpan {
                    first_hour: from,
                    last_hour: run.first_hour - 1,
                });
            }
            from = run.last_hour.saturating_add(1);
        }
        if from <= last {
            unobserved.push(HourSpan {
                first_hour: from,
                last_hour: last,
            });
        }
    }
    merge_spans(&mut unobserved);
    let warnings = catalogue.warnings[channel as usize]
        .iter()
        .chain(if channel == 0 {
            catalogue.warnings[2].iter()
        } else {
            catalogue.warnings[1].iter()
        })
        .map(|(reason, count)| format!("{count} {reason}"))
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect();
    let reply = VolunteersExpeditionariesReply {
        links,
        tallies,
        unobserved,
        prefix,
        warnings,
    };
    check_reply_budget(&reply)?;
    Ok(reply)
}

#[cfg(test)]
mod tests;
