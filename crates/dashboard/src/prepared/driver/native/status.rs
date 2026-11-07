//! What a native preparation reports, and what its coordinator publishes beside the families:
//! each status row and family coverage, and the pages marking the store as a native
//! preparation's, which refuse by name every family and recording-wide job with no native
//! backend.

use super::shape::Shape;
use crate::prepared::{
    Section, State,
    coverage::{self, Coverage, Page, Refusal},
    driver::{
        plan::{Backend, Plan},
        registry::{self, FamilyId},
    },
    pipeline::Accepted,
};
use dashboard_base::prepared::roots::{self, End, Frontier, Held};
use std::{
    collections::BTreeMap,
    io::{self, Read},
};

/// The recording-wide jobs a native preparation runs before its run: the campaign map, which the
/// viewer reads from the recording's runs in place ([`replay::columnar_natives`]). Those its run
/// prepares among the families are the plan's ([`Plan::jobs`]).
pub(crate) const JOBS: &[&str] = &["map"];

/// Whether a native preparation of `plan` runs the recording-wide job `name`.
fn runs(plan: &Plan, name: &str) -> bool {
    JOBS.contains(&name) || plan.jobs().contains(&name)
}

/// The names a native preparation of `plan` prepares in its run: the families it prepares
/// natively, and the jobs among them.
fn members(plan: &Plan) -> impl Iterator<Item = &'static str> + '_ {
    plan.families()
        .iter()
        .filter(|family| plan.backend(family.id.name()) == Some(Backend::Segments))
        .map(|family| family.id.name())
        .chain(plan.jobs().iter().copied())
}

/// The label of the status row `name` reports to first, else its name.
pub(crate) fn label(status: &[&str], name: &'static str) -> &'static str {
    status
        .first()
        .and_then(|row| registry::STATUS.iter().find(|(id, _)| id == row))
        .map_or(name, |&(_, label)| label)
}

/// Every family and recording-wide job `plan` refuses, by name, with why.
pub(crate) fn refused(plan: &Plan) -> Vec<(&'static str, crate::QueryError)> {
    let unavailable = dashboard_base::prepared::query::native::unprepared;
    plan.families()
        .iter()
        .filter(|family| plan.backend(family.id.name()) == Some(Backend::Unavailable))
        .map(|family| {
            let name = family.id.name();
            (name, unavailable(label(family.status, name)).into())
        })
        .chain(
            registry::JOBS
                .iter()
                .filter(|job| !runs(plan, job.name))
                .map(|job| (job.name, unavailable(label(&[job.name], job.name)).into())),
        )
        .collect()
}

/// The mark of what the coordinator publishes for `plan` over `shape`'s branches: a store
/// holding another goes through every refusal again.
pub(crate) fn mark(plan: &Plan, shape: &Shape) -> [u8; 32] {
    let mut hash = blake3::Hasher::new();
    hash.update(&coverage::SCHEMA.to_le_bytes());
    for (name, reason) in refused(plan) {
        hash.update(name.as_bytes());
        hash.update(&[0]);
        hash.update(reason.to_string().as_bytes());
        hash.update(&[0]);
    }
    for (branch, extent) in shape.branches() {
        hash.update(&branch.to_le_bytes());
        hash.update(&extent.first_hour.to_le_bytes());
        hash.update(&extent.last_hour.to_le_bytes());
    }
    *hash.finalize().as_bytes()
}

/// The mark the publication of `store` holds, whatever key it was committed with; None for a
/// store no native preparation published to.
pub(crate) fn held(store: &store::Store) -> io::Result<Option<Vec<u8>>> {
    let Some(snapshot) = store.current()? else {
        return Ok(None);
    };
    let Some(mut range) = snapshot.read(coverage::PLAN, 0)? else {
        return Ok(None);
    };
    let mut bytes = Vec::new();
    range.read_to_end(&mut bytes)?;
    Ok(Some(bytes))
}

/// Writes the coordinator's pages for `plan` over `shape` with `mark`: a page refusing each name
/// `plan` refuses on every branch from its first hour, and, of each family and job `plan`'s run
/// prepares, no coverage page of a branch it publishes no root of: a refusal of it before it had
/// a native backend.
pub(crate) fn coordinate(
    writer: &mut store::Writer,
    plan: &Plan,
    shape: &Shape,
    mark: &[u8; 32],
) -> io::Result<()> {
    for (name, reason) in refused(plan) {
        for (&branch, extent) in shape.branches() {
            let page = Page {
                coverage: Coverage {
                    schema: coverage::SCHEMA,
                    first_hour: extent.first_hour,
                    last_hour: extent.last_hour,
                    complete: true,
                },
                refused: BTreeMap::from([(
                    name.to_owned(),
                    Refusal {
                        hour: extent.first_hour,
                        error: reason.clone(),
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
    for name in members(plan) {
        for &branch in shape.branches().keys() {
            let page = coverage::family_section(name, branch);
            if writer.range_count(&page)?.is_some()
                && writer
                    .range_count(&roots::root_section(name, branch))?
                    .is_none()
            {
                writer.clear(&page)?;
            }
        }
    }
    writer.write_at(coverage::PLAN, 0, &mark[..])
}

/// The bytes `store`'s publication holds.
pub(crate) fn stored(store: &store::Store) -> io::Result<u64> {
    let Some(snapshot) = store.current()? else {
        return Ok(0);
    };
    let mut bytes = 0;
    for section in snapshot.section_names() {
        for range in 0..snapshot.range_count(section).unwrap_or(0) {
            if let Some(id) = snapshot.range_id(section, range)? {
                bytes += id.len();
            }
        }
    }
    Ok(bytes)
}

/// Why a family's branch ended where its frontier says.
fn ended(branch: u64, end: &End) -> String {
    let how = match end.held {
        Held::Lost => "lost",
        Held::Prepared | Held::Refused => "refused",
    };
    format!(
        "Branch {branch} {how} from hour {}: {}",
        end.hour, end.reason
    )
}

/// What a native preparation of `shape` accepted: each family of `prepared` as `frontier` gives
/// its committed frontier on a branch, and `failed` those that failed with why; the campaign map,
/// prepared before the run, is ready. Every other family and job `plan` refuses by name.
pub(crate) fn accepted(
    plan: &Plan,
    shape: &Shape,
    prepared: &[FamilyId],
    frontier: &dyn Fn(FamilyId, u64) -> Option<Frontier>,
    failed: &BTreeMap<FamilyId, crate::QueryError>,
    bytes: u64,
) -> Accepted {
    let total = shape.len().to_string();
    let refused: BTreeMap<&str, crate::QueryError> = refused(plan).into_iter().collect();
    let mut families = BTreeMap::new();
    let mut sections = Vec::with_capacity(registry::STATUS.len());
    for &(id, label) in registry::STATUS {
        let section = |state: State, done: u64, error: Option<String>| Section {
            id: id.to_owned(),
            label: label.to_owned(),
            state,
            done: done.to_string(),
            total: Some(total.clone()),
            error,
        };
        let family = plan
            .families()
            .iter()
            .find(|family| family.status.contains(&id));
        let name = family.map_or(id, |family| family.id.name());
        if let Some(reason) = refused.get(name) {
            sections.push(section(State::Failed, 0, Some(reason.to_string())));
            continue;
        }
        let Some(family) = family.filter(|family| prepared.contains(&family.id)) else {
            sections.push(if family.is_some() {
                section(State::Idle, 0, None)
            } else {
                section(State::Ready, shape.len() as u64, None)
            });
            continue;
        };
        let (state, done, errors, coverage) = progress(shape, family.id, frontier, failed);
        sections.push(section(
            state,
            done,
            (!errors.is_empty()).then(|| errors.join("; ")),
        ));
        families.insert(family.id.name().to_owned(), coverage);
    }
    // A job has no status row; its coverage is listed beside the families'.
    for &job in prepared
        .iter()
        .filter(|id| plan.jobs().contains(&id.name()))
    {
        let (_, _, _, coverage) = progress(shape, job, frontier, failed);
        families.insert(job.name().to_owned(), coverage);
    }
    // A job holds back no hour of the timeline.
    let working: Vec<FamilyId> = prepared
        .iter()
        .copied()
        .filter(|family| !failed.contains_key(family) && plan.family(family.name()).is_some())
        .collect();
    Accepted {
        sections,
        bytes,
        coverage: shared(shape, &working, frontier),
        families,
    }
}

/// Where `id`'s publication stands on every branch of `shape`, as `frontier` gives its committed
/// frontiers and `failed` its failure: its state, the segments it processed, why it failed or
/// ended a branch, and its coverage by branch.
fn progress(
    shape: &Shape,
    id: FamilyId,
    frontier: &dyn Fn(FamilyId, u64) -> Option<Frontier>,
    failed: &BTreeMap<FamilyId, crate::QueryError>,
) -> (State, u64, Vec<String>, BTreeMap<String, Coverage>) {
    let (mut done, mut terminal) = (0, true);
    let mut errors: Vec<String> = failed
        .get(&id)
        .map(ToString::to_string)
        .into_iter()
        .collect();
    let mut coverage = BTreeMap::new();
    for &branch in shape.branches().keys() {
        let Some(frontier) = frontier(id, branch) else {
            terminal = false;
            continue;
        };
        done += frontier.segments;
        let complete = terminal_in(shape, branch, &frontier);
        terminal &= complete;
        if let Some(end) = &frontier.end {
            errors.push(ended(branch, end));
        }
        coverage.insert(
            branch.to_string(),
            Coverage {
                schema: coverage::SCHEMA,
                first_hour: frontier.first_hour,
                last_hour: frontier.last_hour,
                complete,
            },
        );
    }
    let state = match (errors.is_empty(), terminal) {
        (false, _) => State::Failed,
        (true, true) => State::Ready,
        (true, false) => State::Preparing,
    };
    (state, done, errors, coverage)
}

/// Whether `frontier` leaves its family nothing of `branch` in the recording `shape` places.
fn terminal_in(shape: &Shape, branch: u64, frontier: &Frontier) -> bool {
    frontier.end.is_some() || (frontier.complete && !shape.after(branch, frontier))
}

/// The hours of each branch every family of `families` published alike, for the controls'
/// timeline: none of a branch some family has not published yet. A family whose branch ended
/// has nothing more to prepare there, and limits none.
fn shared(
    shape: &Shape,
    families: &[FamilyId],
    frontier: &dyn Fn(FamilyId, u64) -> Option<Frontier>,
) -> BTreeMap<String, Coverage> {
    let mut walk = BTreeMap::new();
    'branches: for &branch in shape.branches().keys() {
        let mut held: Option<Coverage> = None;
        for &family in families {
            let Some(frontier) = frontier(family, branch) else {
                continue 'branches;
            };
            if frontier.end.is_some() {
                continue;
            }
            let complete = terminal_in(shape, branch, &frontier);
            let coverage = held.get_or_insert(Coverage {
                schema: coverage::SCHEMA,
                first_hour: frontier.first_hour,
                last_hour: frontier.last_hour,
                complete,
            });
            coverage.first_hour = coverage.first_hour.max(frontier.first_hour);
            coverage.last_hour = coverage.last_hour.min(frontier.last_hour);
            coverage.complete &= complete;
        }
        if let Some(coverage) = held.filter(|coverage| coverage.first_hour <= coverage.last_hour) {
            walk.insert(branch.to_string(), coverage);
        }
    }
    walk
}
