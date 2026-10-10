//! Exact recorded civilian-factory output, shared with Construction log's scan.
//! ConstructionLineOutput measures each building/repair spend and each
//! railway's first progress call. Its factory channels exclude CIC-bank
//! draws, instant progress and railway carries.
//! Deleted lines keep their final publication; census progress and current cost
//! never price a completion. Amounts appear only at their supplying read.
//! Linked-country output, inexact observations and missing final publications
//! remain named unknowns. Output without any census owner cannot be attributed
//! to a country. Existing recordings without this table remain readable.

pub use super::construction_log::SpendingGap;
use super::{
    CountryParams, LoadContext, Product,
    construction_log::{
        self, BuildingInfo, COST_SCALE, ConstructionProject, ProjectKind, ProjectRun, ScannedLog,
        StateName,
    },
};
use crate::Source;
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    io,
    sync::atomic::AtomicBool,
};
use ts_rs::TS;

pub struct ConstructionSpending;

/// A running total, exact, at a read that adds to it.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
pub struct SpentStep {
    pub known_at: i32,
    /// Fixed5 (`cost_scale` per point of output), as a decimal string.
    pub total_raw: String,
}

/// A stretch of a project's progress that is not counted.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
pub struct SpendingUnknown {
    /// The read that ends it: nothing about it shows before.
    pub known_at: i32,
    pub reason: SpendingGap,
    /// The project's native handle, as in the construction log.
    pub project: String,
    /// A later exact cumulative read resolves this missing observation.
    pub until: Option<i32>,
}

/// When one project of a leaf did its kind of work: from the first read of an
/// unbroken stretch of such reads until the next read that shows it doing
/// something else, or gone (`until`, exclusive; none while it still does). A
/// project that changes kind more than once has an interval per stretch. An
/// inferred interval is established only at `known_at`, even if it covers earlier hours.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
pub struct LeafProject {
    /// The project's native handle, as in the construction log.
    pub project: String,
    pub first_read: i32,
    /// The read that establishes an inferred interval; none when its first read does.
    pub known_at: Option<i32>,
    pub until: Option<i32>,
    /// The interval ended at a break in its reads (hours not recorded, or a read
    /// that failed), at `until`: what it did from then is not known, not nothing.
    pub unread: bool,
    /// When `unread`, the hour its reads resume (the read that showed it gone, for
    /// one that left); none when they never do before the branch ends (the rest of
    /// the branch is not recorded for it).
    pub resumes: Option<i32>,
}

/// The projects of one repair or new kind, building type and state.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
pub struct SpendingLeaf {
    pub repair: bool,
    pub kind: ProjectKind,
    /// Index into [`ConstructionSpendingReply::types`].
    pub building: u32,
    /// The target state (a railway: its route's first province's).
    pub state: Option<u32>,
    /// A railway: the state of its route's last province.
    pub to_state: Option<u32>,
    /// Running total at each read that adds to it, in hour order.
    pub spent: Vec<SpentStep>,
    /// In hour order.
    pub unknown: Vec<SpendingUnknown>,
    /// Its projects, by first read.
    pub projects: Vec<LeafProject>,
}

#[derive(Clone, Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct ConstructionSpendingReply {
    pub branch_id: String,
    pub tag: String,
    /// Units per point of civilian-factory output in `total_raw`.
    pub cost_scale: u32,
    /// Hours the building and railway queues were read, ascending.
    pub reads: Vec<i32>,
    pub railway_reads: Vec<i32>,
    /// Every building type the branch's projects name (a leaf's `building` indexes it).
    pub types: Vec<BuildingInfo>,
    pub repair_icon: Option<String>,
    /// Names of the states the leaves name.
    pub states: Vec<StateName>,
    /// By repair, kind, building type and state.
    pub leaves: Vec<SpendingLeaf>,
    /// Projects whose work before observation began was not captured.
    pub before_first_read: u32,
    /// Missing output and output without a census owner affect every country.
    pub unattributed: Vec<SpendingUnknown>,
    pub warnings: Vec<String>,
}

/// A leaf: repair, railway, building type, state and route end state.
type LeafKey = (bool, bool, u32, Option<u32>, Option<u32>);
/// A leaf's amounts, unknown stretches and projects while grouping.
type LeafParts = (Vec<(i32, i128)>, Vec<SpendingUnknown>, Vec<LeafProject>);

/// When a project was doing one kind of work (`repair`: its runs with levels to
/// repair; else those with new levels to build): one interval per unbroken
/// stretch of such runs, from its first read until the next run that shows it
/// doing something else, or its ending. A break in its reads (its runs are not
/// contiguous: hours not recorded, or a read that failed) ends an interval too,
/// just after the last hour its reads covered, saying where they resume (for one
/// that left, the read that showed it gone); so does the end of its reads before
/// `last_hour` (the branch's last) while it has no ending, and then they never
/// resume.
fn kind_spans(
    project: &ConstructionProject,
    repair: bool,
    last_hour: Option<i32>,
) -> Vec<LeafProject> {
    let doing = |run: &ProjectRun| {
        if repair {
            run.repair_levels > 0
        } else {
            run.remaining > 0 || project.kind == ProjectKind::Railway
        }
    };
    let mut spans = Vec::new();
    let mut runs = project.runs.iter().peekable();
    while let Some(run) = runs.next() {
        if !doing(run) {
            continue;
        }
        let first_read = run.first_hour;
        let mut last = run;
        while let Some(next) = runs.peek().copied() {
            if !doing(next) || next.first_hour > last.last_hour.saturating_add(1) {
                break;
            }
            last = next;
            runs.next();
        }
        let after = last.last_hour.saturating_add(1);
        let (until, unread, resumes) = match runs.peek() {
            Some(next) if next.first_hour > after => (Some(after), true, Some(next.first_hour)),
            Some(next) => (Some(next.first_hour), false, None),
            None => match &project.end {
                Some(end) if end.known_at > after => (Some(after), true, Some(end.known_at)),
                Some(end) => (Some(end.known_at), false, None),
                None if last_hour.is_some_and(|last_hour| last.last_hour < last_hour) => {
                    (Some(after), true, None)
                }
                None => (None, false, None),
            },
        };
        spans.push(LeafProject {
            project: project.id.clone(),
            first_read,
            known_at: None,
            until,
            unread,
            resumes,
        });
    }
    spans
}

/// Adds to a project's repair intervals each stretch it repaired in that no read
/// shows as repair work (damaged and repaired between two reads): from the read
/// before the one the log dates the repair to (`reads`, the queue's successful
/// reads), up to that read, unless an interval already covers it.
fn dated_repairs(project: &ConstructionProject, reads: &[i32], spans: &mut Vec<LeafProject>) {
    let mut dated: Vec<i32> = project
        .built
        .iter()
        .filter(|levels| levels.repair)
        .map(|levels| levels.known_at)
        .collect();
    dated.sort_unstable();
    dated.dedup();
    for known_at in dated {
        let from = reads[..reads.partition_point(|&read| read < known_at)]
            .last()
            .map_or(project.first_read, |&read| read.max(project.first_read));
        let covered = spans.iter().any(|span| {
            span.first_read <= from && span.until.is_none_or(|until| until >= known_at)
        });
        if !covered {
            spans.push(LeafProject {
                project: project.id.clone(),
                first_read: from,
                known_at: Some(known_at),
                until: Some(known_at),
                unread: false,
                resumes: None,
            });
        }
    }
    spans.sort_by_key(|span| span.first_read);
}

/// Group the two factory channels, retaining exact signed differences and every
/// deleted line's last publication. Missing observations resolve when the same
/// line's later cumulative read establishes its total again.
pub(crate) fn leaves(log: &ScannedLog, tag: &str) -> (Vec<SpendingLeaf>, u32) {
    let mut grouped: BTreeMap<LeafKey, LeafParts> = BTreeMap::new();
    let mut before = 0;
    for project in log.projects.iter().filter(|project| project.tag == tag) {
        before += u32::from(
            project
                .output
                .first()
                .is_some_and(|read| read.since_hour.is_some() && !read.observed_since_creation),
        );
        let railway = project.kind == ProjectKind::Railway;
        for repair in [false, true] {
            let mut projects = kind_spans(project, repair, log.last_hour);
            if repair && !railway {
                dated_repairs(project, &log.reads, &mut projects);
            }
            let mut spent = Vec::new();
            let mut unknown: Vec<SpendingUnknown> = Vec::new();
            let mut last = 0_i128;
            let mut repair_ticks = 0;
            for read in &project.output {
                let raw = if repair {
                    &read.repair_raw
                } else {
                    &read.construction_raw
                };
                if let Some(raw) = raw {
                    let total = raw.parse::<i128>().expect("captured i64 output");
                    let amount = total - last;
                    spent.push((read.known_at, amount));
                    last = total;
                    if let Some(gap) = unknown.last_mut()
                        && gap.until.is_none()
                    {
                        gap.until = Some(read.known_at);
                    }
                    if (amount != 0 || (repair && read.repair_ticks > repair_ticks))
                        && projects.iter().all(|span| {
                            span.first_read > read.known_at
                                || span.until.is_some_and(|until| until <= read.known_at)
                        })
                    {
                        // The output knows the kind even when neither census read
                        // shows it, including a line's final repair spend.
                        projects.push(LeafProject {
                            project: project.id.clone(),
                            first_read: read.known_at,
                            known_at: None,
                            until: read.known_at.checked_add(1),
                            unread: false,
                            resumes: None,
                        });
                    }
                    // An unread output supplies no tick count, so only reads update it.
                    repair_ticks = read.repair_ticks;
                } else if let Some(reason) = read.reason
                    && unknown
                        .last()
                        .is_none_or(|gap| gap.until.is_some() || gap.reason != reason)
                {
                    if let Some(gap) = unknown.last_mut()
                        && gap.until.is_none()
                    {
                        gap.until = Some(read.known_at);
                    }
                    unknown.push(SpendingUnknown {
                        known_at: read.known_at,
                        reason,
                        project: project.id.clone(),
                        until: None,
                    });
                }
            }
            if projects.is_empty()
                && spent.iter().all(|(_, amount)| *amount == 0)
                && unknown.is_empty()
            {
                continue;
            }
            if projects.is_empty() {
                projects.push(LeafProject {
                    project: project.id.clone(),
                    first_read: project.first_read,
                    known_at: None,
                    until: project.end.as_ref().map(|end| end.known_at),
                    unread: false,
                    resumes: None,
                });
            }
            let leaf = grouped
                .entry((
                    repair,
                    railway,
                    project.building,
                    project.state,
                    project.to_state.filter(|_| railway),
                ))
                .or_default();
            leaf.0.extend(spent);
            leaf.1.extend(unknown);
            leaf.2.extend(projects);
        }
    }
    let leaves = grouped
        .into_iter()
        .map(
            |(
                (repair, railway, building, state, to_state),
                (mut spent, mut unknown, mut projects),
            )| {
                spent.sort_unstable();
                let mut steps: Vec<SpentStep> = Vec::new();
                let mut total = 0_i128;
                for (hour, amount) in spent {
                    total += amount;
                    match steps.last_mut() {
                        Some(step) if step.known_at == hour => step.total_raw = total.to_string(),
                        _ => steps.push(SpentStep {
                            known_at: hour,
                            total_raw: total.to_string(),
                        }),
                    }
                }
                unknown.sort_by(|a, b| (a.known_at, &a.project).cmp(&(b.known_at, &b.project)));
                projects.sort_by(|a, b| {
                    (a.first_read, a.until, &a.project).cmp(&(b.first_read, b.until, &b.project))
                });
                SpendingLeaf {
                    repair,
                    kind: if railway {
                        ProjectKind::Railway
                    } else {
                        ProjectKind::Building
                    },
                    building,
                    state,
                    to_state,
                    spent: steps,
                    unknown,
                    projects,
                }
            },
        )
        .collect();
    (leaves, before)
}

impl Product for ConstructionSpending {
    const KEY: &'static str = "construction-spending";
    const CONTENT_DEPENDENT: bool = true;
    type Params = CountryParams;
    type Reply = ConstructionSpendingReply;

    fn types() -> Vec<String> {
        vec![
            SpentStep::decl(),
            LeafProject::decl(),
            SpendingUnknown::decl(),
            SpendingLeaf::decl(),
            ConstructionSpendingReply::decl(),
        ]
    }

    fn validate(params: &Self::Params) -> Result<(), &'static str> {
        params.validate()
    }

    fn load(
        source: &Source,
        params: &Self::Params,
        context: &LoadContext,
        cancelled: &AtomicBool,
    ) -> io::Result<Self::Reply> {
        let log = construction_log::scanned(source, params.branch(), cancelled)?;
        Self::from_scanned(source, params, context, &log, cancelled)
    }
}

impl ConstructionSpending {
    fn from_scanned(
        source: &Source,
        params: &CountryParams,
        context: &LoadContext,
        log: &ScannedLog,
        cancelled: &AtomicBool,
    ) -> io::Result<ConstructionSpendingReply> {
        let branch = params.branch();
        let mut warnings = Vec::new();
        let names = construction_log::content_names(
            source,
            context,
            branch,
            log,
            cancelled,
            &mut warnings,
        )?;
        construction_log::scan_warnings(log, &mut warnings);
        let (leaves, before_first_read) = leaves(log, &params.tag);
        if leaves.is_empty() && !(log.reads.is_empty() && log.railway_reads.is_empty()) {
            warnings.push(format!(
                "{}: no recorded construction in this branch",
                params.tag
            ));
        }
        let states: BTreeSet<u32> = leaves
            .iter()
            .flat_map(|leaf| [leaf.state, leaf.to_state])
            .flatten()
            .collect();
        Ok(ConstructionSpendingReply {
            branch_id: params.branch_id.clone(),
            tag: params.tag.clone(),
            cost_scale: COST_SCALE,
            reads: log.reads.clone(),
            railway_reads: log.railway_reads.clone(),
            states: names.states(&states),
            types: names.types,
            repair_icon: names.repair_icon,
            leaves,
            before_first_read,
            unattributed: log
                .output_unknown
                .iter()
                .map(|gap| SpendingUnknown {
                    known_at: gap.known_at,
                    reason: gap.reason,
                    project: gap.line.map_or_else(String::new, |id| id.to_string()),
                    until: gap.until,
                })
                .collect(),
            warnings,
        })
    }
}

#[cfg(test)]
mod tests;
