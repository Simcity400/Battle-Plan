
use dashboard_base::products::{BranchParams, LoadContext, Product};
use dashboard_base::Source;
use dashboard_query::{Query, identity::Holder, series::MissingSpan};
use serde::{Deserialize, Serialize};
use std::{io, sync::atomic::AtomicBool};
#[cfg(any(test, feature = "export-types"))]
use ts_rs::TS;

pub struct CountryIdentities;

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct CountryIdentityRun {
    pub first_hour: i32,
    pub last_hour: i32,
    pub slot: u32,
    pub country_id: u32,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct IdentitySpan {
    pub first_hour: i32,
    pub last_hour: i32,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct TagIdentities {
    pub tag: String,
    pub identities: Vec<CountryIdentityRun>,
    pub unresolved: Vec<IdentitySpan>,
}

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct CountryIdentitiesReply {
    pub branch_id: String,
    #[doc = " Recorded hours where identity is known; country runs coalesce across gaps outside these spans."]
    #[serde(default)]
    pub known: Vec<IdentitySpan>,
    #[doc = " By tag."]
    pub countries: Vec<TagIdentities>,
    #[doc = " Hours the identity table was not available: no tag's holder is known there."]
    pub unknown: Vec<MissingSpan>,
    pub warnings: Vec<String>,
}

dashboard_base::reply_warnings!(CountryIdentitiesReply);

impl Product for CountryIdentities {
    const KEY: &'static str = "country-identities";
    const CONTENT_DEPENDENT: bool = false;
    type Params = BranchParams;
    const FIELDS: dashboard_base::products::Fields = dashboard_base::fields![branchId];
    type Reply = CountryIdentitiesReply;

    #[cfg(any(test, feature = "export-types"))]
    fn types() -> Vec<String> {
        vec![CountryIdentityRun::decl(), IdentitySpan::decl(), TagIdentities::decl(), MissingSpan::decl(), CountryIdentitiesReply::decl()]
    }

    fn validate(params: &Self::Params) -> Result<(), &'static str> {
        params.branch_id.parse::<u64>().map(|_| ()).map_err(|_| "Invalid branch")
    }

    fn load(source: &Source, params: &Self::Params, _context: &LoadContext, cancelled: &AtomicBool) -> io::Result<Self::Reply> {
        dashboard_text::metadata::check_cancel(cancelled)?;
        let branch = params.branch_id.parse::<u64>().map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "Invalid branch"))?;
        let query = dashboard_base::queries::query(source, branch)?;
        Ok(reply(&params.branch_id, &query))
    }
}

fn known_spans(stretches: &[(i32, i32)], unknown: &[(i32, i32)]) -> Vec<IdentitySpan> {
    let mut blocked: Vec<(i32, i32)> = unknown.to_vec();
    blocked.sort_unstable();
    let mut settled_blocked: Vec<(i32, i32)> = Vec::with_capacity(blocked.len());
    for (first, last) in blocked {
        match settled_blocked.last_mut() {
            Some(held) if i64::from(held.1) + 1 >= i64::from(first) => held.1 = held.1.max(last),
            _ => settled_blocked.push((first, last)),
        }
    }
    let mut out: Vec<IdentitySpan> = Vec::new();
    for &(first, last) in stretches {
        let mut cursor = first;
        let start = settled_blocked.partition_point(|&(_, end)| end < first);
        for &(a, b) in &settled_blocked[start..] {
            if a > last {
                break;
            }
            if a > cursor {
                out.push(IdentitySpan { first_hour: cursor, last_hour: a - 1 });
            }
            if b >= last {
                cursor = last.saturating_add(1);
                break;
            }
            cursor = cursor.max(b + 1);
        }
        if cursor <= last {
            out.push(IdentitySpan { first_hour: cursor, last_hour: last });
        }
    }
    out.sort_unstable_by_key(|span| span.first_hour);
    let mut merged: Vec<IdentitySpan> = Vec::with_capacity(out.len());
    for span in out {
        match merged.last_mut() {
            Some(held) if i64::from(held.last_hour) + 1 >= i64::from(span.first_hour) => held.last_hour = held.last_hour.max(span.last_hour),
            _ => merged.push(span),
        }
    }
    merged
}

fn no_known_between(known: &[IdentitySpan], last_hour: i32, next_first: i32) -> bool {
    if i64::from(last_hour) + 1 >= i64::from(next_first) {
        return true;
    }
    let gap_first = last_hour + 1;
    let gap_last = next_first - 1;
    let at = known.partition_point(|span| span.last_hour < gap_first);
    known.get(at).is_none_or(|span| span.first_hour > gap_last)
}

pub fn reply(branch_id: &str, query: &Query<'_>) -> CountryIdentitiesReply {
    let stretches: Vec<(i32, i32)> = query.timeline.stretches.iter().map(|stretch| (stretch.first, stretch.last)).collect();
    let unknown_pairs: Vec<(i32, i32)> = query.holders.unknown.iter().map(|&(first, last, _)| (first, last)).collect();
    let known = known_spans(&stretches, &unknown_pairs);
    build_reply(branch_id, &query.holders, known)
}

fn build_reply(branch_id: &str, holders: &dashboard_query::identity::Holders, known: Vec<IdentitySpan>) -> CountryIdentitiesReply {
    let countries = holders
        .tags
        .iter()
        .map(|(tag, runs)| {
            let mut identities: Vec<CountryIdentityRun> = Vec::new();
            let mut unresolved: Vec<IdentitySpan> = Vec::new();
            for run in runs {
                match run.holder {
                    Holder::Found { slot, id } => match identities.last_mut() {
                        Some(held) if held.slot == slot && held.country_id == id && no_known_between(&known, held.last_hour, run.first) => {
                            held.last_hour = held.last_hour.max(run.last);
                        }
                        _ => identities.push(CountryIdentityRun {
                            first_hour: run.first,
                            last_hour: run.last,
                            slot,
                            country_id: id,
                        }),
                    },
                    Holder::Unresolved => match unresolved.last_mut() {
                        Some(held) if no_known_between(&known, held.last_hour, run.first) => {
                            held.last_hour = held.last_hour.max(run.last);
                        }
                        _ => unresolved.push(IdentitySpan { first_hour: run.first, last_hour: run.last }),
                    },
                }
            }
            TagIdentities { tag: tag.clone(), identities, unresolved }
        })
        .collect();
    CountryIdentitiesReply {
        branch_id: branch_id.to_owned(),
        known,
        countries,
        unknown: holders
            .unknown
            .iter()
            .map(|(first, last, reason)| MissingSpan { first_hour: *first, last_hour: *last, reason: reason.clone() })
            .collect(),
        warnings: Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dashboard_query::identity::{HolderRun, Holders};
    use std::collections::BTreeMap;

    #[test]
    fn sparse_stretches_coalesce_unchanged_identity_runs_independent_of_stretch_count() {
        let stretches: Vec<(i32, i32)> = (0..200).map(|i| (i * 10, i * 10)).collect();
        let mut tags = BTreeMap::new();
        for country in 0..20u32 {
            let runs: Vec<HolderRun> = stretches
                .iter()
                .map(|&(hour, _)| HolderRun { first: hour, last: hour, holder: Holder::Found { slot: country, id: country + 1 }, starts_segment: true })
                .collect();
            tags.insert(format!("C{country:02}"), runs);
        }
        let holders = Holders::new(tags, Vec::new());
        let known = known_spans(&stretches, &[]);
        let built = build_reply("1", &holders, known);
        assert_eq!(built.known.len(), 200);
        for entry in &built.countries {
            assert_eq!(entry.identities.len(), 1);
            assert_eq!(entry.identities[0].first_hour, 0);
            assert_eq!(entry.identities[0].last_hour, 1990);
        }
    }

    #[test]
    fn identity_runs_do_not_coalesce_across_known_hour_where_country_was_absent() {
        let stretches = vec![(10, 10), (20, 20), (30, 30)];
        let mut tags = BTreeMap::new();
        tags.insert(
            "GER".to_owned(),
            vec![
                HolderRun { first: 10, last: 10, holder: Holder::Found { slot: 1, id: 1 }, starts_segment: true },
                HolderRun { first: 30, last: 30, holder: Holder::Found { slot: 1, id: 1 }, starts_segment: true },
            ],
        );
        let holders = Holders::new(tags, Vec::new());
        let built = build_reply("1", &holders, known_spans(&stretches, &[]));
        assert_eq!(built.countries[0].identities.len(), 2);
    }
}
