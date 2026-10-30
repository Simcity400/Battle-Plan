//! Observed land deployments, not dispatch transactions or integrated lifetime totals, read
//! from the army family's publication ([`super::army_size::emit`]): its aid links, each
//! sender's resolved channels and each party's presence.
use super::army_size::{
    emit::{AID_PROOF, AID_READ, AID_VALUE, AID_WARNINGS, FAMILY, LINKS, party_of},
    query::reasons,
};
use dashboard_base::{
    Source,
    prepared::{engine, prefix},
    products::{Product, check_reply_budget, country_groups::HourSpan},
};
use dashboard_series::{PageRead, Snapshot, family::Directory};
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
    const SERVED: Option<&'static dashboard_base::prepared::served::Plan> = Some(&served::PLAN);
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

/// A run of a key as the reply reads it.
#[derive(Clone, Copy)]
struct Run {
    first_hour: i32,
    last_hour: i32,
    value: i128,
    break_before: bool,
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

/// `key`'s runs over `first..=last`; none of a key the family never wrote.
fn runs<P: PageRead>(
    snapshot: &mut Snapshot<'_, P>,
    key: Option<u64>,
    first: i32,
    last: i32,
) -> io::Result<Vec<Run>> {
    let Some(key) = key else {
        return Ok(Vec::new());
    };
    Ok(engine::runs(snapshot, key, first, last)?
        .into_iter()
        .map(|run| Run {
            first_hour: run.span.first,
            last_hour: run.span.last,
            value: run.value,
            break_before: run.break_before,
        })
        .collect())
}

/// The branch a request names.
pub fn branch(params: &VolunteersExpeditionariesParams) -> io::Result<u64> {
    VolunteersExpeditionaries::validate(params).map_err(dashboard_core::invalid)?;
    params
        .branch_id
        .parse::<u64>()
        .map_err(|_| dashboard_core::invalid("Invalid aid branch"))
}

/// The reply before the army family publishes the branch: every hour the prefix accepts is
/// unobserved.
pub fn unpublished(prefix: prefix::Prefix) -> VolunteersExpeditionariesReply {
    let (first, last) = (prefix.coverage.first_hour, prefix.coverage.last_hour);
    VolunteersExpeditionariesReply {
        links: Vec::new(),
        tallies: Vec::new(),
        unobserved: if first <= last {
            vec![HourSpan {
                first_hour: first,
                last_hour: last,
            }]
        } else {
            Vec::new()
        },
        prefix,
        warnings: Vec::new(),
    }
}

/// The recorded gaps after the prefix joined into the reply's unobserved hours, within its
/// budget.
pub fn finish(
    source: &Source,
    branch: u64,
    mut reply: VolunteersExpeditionariesReply,
) -> io::Result<VolunteersExpeditionariesReply> {
    let through = reply.prefix.coverage.last_hour;
    reply
        .unobserved
        .extend(prefix::gaps_after(source, branch, through));
    merge_spans(&mut reply.unobserved);
    check_reply_budget(&reply)?;
    Ok(reply)
}

/// The reply the publication gives through `prefix`.
pub fn read<P: PageRead>(
    snapshot: &mut Snapshot<'_, P>,
    directory: &Directory,
    params: &VolunteersExpeditionariesParams,
    prefix: prefix::Prefix,
    cancelled: &AtomicBool,
) -> io::Result<VolunteersExpeditionariesReply> {
    dashboard_text::metadata::check_cancel(cancelled)?;
    let branch = branch(params)?;
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
    let channel: u64 = if params.kind == MilitaryAidKind::Land {
        0
    } else {
        1
    };
    let party = |actor: u64| {
        directory
            .actors
            .get(actor as usize)
            .and_then(|actor| party_of(&actor.name))
            .ok_or_else(|| dashboard_core::invalid("A prepared aid link names an unknown party"))
    };
    let proof = |actor: u64, channel: u64| directory.key(actor, AID_PROOF, Some(channel));
    let mut proofs = BTreeMap::<(u64, u64), Vec<Run>>::new();
    let mut links = Vec::new();
    let mut tallies = Vec::new();
    let mut unobserved = Vec::new();
    let mut bytes = 0usize;
    let wanted = params
        .tags
        .as_ref()
        .map(|tags| tags.iter().map(String::as_str).collect::<BTreeSet<_>>());
    let mut index = 0;
    while let Some(link) = directory.scalar(LINKS + index) {
        dashboard_text::metadata::check_cancel(cancelled)?;
        let at = index;
        index += 1;
        let fields: Vec<u64> = link
            .list()
            .iter()
            .filter_map(|field| u64::try_from(field.integer()?).ok())
            .collect();
        let &[sender_actor, host_actor, channels] = fields.as_slice() else {
            return Err(dashboard_core::invalid("A prepared aid link is not valid"));
        };
        if channels & ((1 << channel) | if channel == 0 { 1 << 2 } else { 0 }) == 0 {
            continue;
        }
        let sender = party(sender_actor)?;
        let host = party(host_actor)?;
        if wanted
            .as_ref()
            .is_some_and(|tags| !tags.contains(sender.2) && !tags.contains(host.2))
        {
            continue;
        }
        for (target, selected) in [(&mut links, channel), (&mut tallies, 2)] {
            if selected == 2 && params.kind != MilitaryAidKind::Land {
                continue;
            }
            let keys = [(sender_actor, selected), (sender_actor, 3), (host_actor, 3)];
            for id in keys {
                if let Entry::Vacant(entry) = proofs.entry(id) {
                    entry.insert(runs(snapshot, proof(id.0, id.1), first, last)?);
                }
            }
            let values = runs(
                snapshot,
                directory.key(sender_actor, AID_VALUE, Some(at * 3 + selected)),
                first,
                last,
            )?;
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
                &values,
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
                    + sender.2.len()
                    + host.2.len(),
            );
            if bytes > dashboard_base::products::MAX_PRODUCT_REPLY_BYTES {
                return Err(io::Error::other(
                    "Military aid history exceeds the reply budget; narrow the country selection",
                ));
            }
            // Link pairs stay distinct when either endpoint's tag is reused.
            target.push(MilitaryAidLink {
                key: link_key,
                sender: sender.2.to_owned(),
                sender_slot: Some(sender.0),
                sender_country_id: Some(sender.1),
                host: host.2.to_owned(),
                host_slot: Some(host.0),
                host_country_id: Some(host.1),
                runs,
            });
        }
    }
    // An empty link list is not proof of an empty native tally roster. Preserve both source
    // channels even when neither has a pair yet.
    for source_channel in [channel, 2]
        .into_iter()
        .take(if channel == 0 { 2 } else { 1 })
    {
        let known = runs(
            snapshot,
            directory.key(FAMILY, AID_READ, Some(source_channel)),
            first,
            last,
        )?;
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
    let other = if channel == 0 { 2 } else { 1 };
    let warnings = reasons(directory, AID_WARNINGS + channel)
        .into_iter()
        .chain(reasons(directory, AID_WARNINGS + other))
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

pub mod served;
#[cfg(test)]
mod tests;
