//! A recording with no native columnar recording beside it. Its rows are not prepared: its store
//! holds a page refusing every family and job by name on each branch, from the branch's first
//! hour, which its queries answer with, and its status rows say why.

use super::{native::status, registry};
use crate::{
    Source,
    config::ContentOptions,
    prepared::{
        Section, State,
        coverage::{self, Coverage, Page, Refusal},
        pipeline::Accepted,
    },
};
use std::{collections::BTreeMap, io, sync::atomic::AtomicBool};

/// Why `label` refuses on a recording with no native recording beside it.
fn reason(label: &str) -> String {
    format!(
        "{label} is prepared only from a native columnar recording, and this recording has none \
         beside it"
    )
}

/// Every family and job, by name, with why it refuses.
fn refused() -> Vec<(&'static str, String)> {
    registry::FAMILIES
        .iter()
        .map(|family| {
            let name = family.id.name();
            (name, reason(status::label(family.status, name)))
        })
        .chain(
            registry::JOBS
                .iter()
                .map(|job| (job.name, reason(status::label(&[job.name], job.name)))),
        )
        .collect()
}

/// Each branch of `source`'s replay, with its first and last hour.
fn branches(source: &Source) -> BTreeMap<u64, (i32, i32)> {
    let replay = source.replay_ref();
    replay
        .branches
        .iter()
        .map(|branch| (branch.id, crate::metadata::branch_bounds(replay, branch)))
        .collect()
}

/// The mark of the refusals published over `branches`: a store holding another publishes them
/// again.
fn mark(refused: &[(&str, String)], branches: &BTreeMap<u64, (i32, i32)>) -> [u8; 32] {
    let mut hash = blake3::Hasher::new();
    hash.update(&coverage::SCHEMA.to_le_bytes());
    hash.update(b"rows\0");
    for (name, why) in refused {
        hash.update(name.as_bytes());
        hash.update(&[0]);
        hash.update(why.as_bytes());
        hash.update(&[0]);
    }
    for (branch, (first, last)) in branches {
        hash.update(&branch.to_le_bytes());
        hash.update(&first.to_le_bytes());
        hash.update(&last.to_le_bytes());
    }
    *hash.finalize().as_bytes()
}

/// Publishes the refusals of `source`'s store, unless it holds them, and what that accepted.
pub(super) fn prepare(
    source: &Source,
    content: Option<&ContentOptions>,
    cancelled: &AtomicBool,
) -> io::Result<Accepted> {
    crate::metadata::check_cancel(cancelled)?;
    let (refused, branches) = (refused(), branches(source));
    let mark = mark(&refused, &branches);
    let (store, key) = crate::prepared::files(source, content)?;
    if status::held(&store)?.as_deref() != Some(&mark[..]) {
        let mut writer = store.replace(key)?;
        for (name, why) in &refused {
            for (&branch, &(first, last)) in &branches {
                let page = Page {
                    coverage: Coverage {
                        schema: coverage::SCHEMA,
                        first_hour: first,
                        last_hour: last,
                        complete: true,
                    },
                    refused: BTreeMap::from([(
                        (*name).to_owned(),
                        Refusal {
                            hour: first,
                            error: crate::QueryError::Refused { text: why.clone() },
                            from: None,
                        },
                    )]),
                };
                writer.write_at(
                    &coverage::family_section(name, branch),
                    0,
                    serde_json::to_vec(&page)?.as_slice(),
                )?;
            }
        }
        writer.write_at(coverage::PLAN, 0, &mark[..])?;
        crate::metadata::check_cancel(cancelled)?;
        writer.commit()?;
    }
    Ok(accepted(&refused, status::stored(&store)?))
}

/// What a store holding `source`'s refusals accepted; None while it holds anything else.
pub(super) fn restore(
    source: &Source,
    content: Option<&ContentOptions>,
    cancelled: &AtomicBool,
) -> io::Result<Option<Accepted>> {
    crate::metadata::check_cancel(cancelled)?;
    let refused = refused();
    let mark = mark(&refused, &branches(source));
    let (store, _) = crate::prepared::files(source, content)?;
    if status::held(&store)?.as_deref() != Some(&mark[..]) {
        return Ok(None);
    }
    Ok(Some(accepted(&refused, status::stored(&store)?)))
}

/// Every status row failed with why its family or job refuses.
fn accepted(refused: &[(&'static str, String)], bytes: u64) -> Accepted {
    let why: BTreeMap<&str, &String> = refused.iter().map(|(name, why)| (*name, why)).collect();
    let sections = registry::STATUS
        .iter()
        .map(|&(id, label)| {
            let name = registry::FAMILIES
                .iter()
                .find(|family| family.status.contains(&id))
                .map_or(id, |family| family.id.name());
            Section {
                id: id.to_owned(),
                label: label.to_owned(),
                state: State::Failed,
                done: "0".to_owned(),
                total: None,
                error: Some(
                    why.get(name)
                        .map_or_else(|| reason(label), |why| (*why).clone()),
                ),
            }
        })
        .collect();
    Accepted {
        sections,
        bytes,
        coverage: BTreeMap::new(),
        families: BTreeMap::new(),
    }
}
