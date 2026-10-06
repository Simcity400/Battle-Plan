//! Where one country's construction effort went, for Construction spending: the
//! civilian-factory output its construction queue turned into progress, per
//! project, from the same branch scan as the Construction log
//! (`construction_log::scanned`). The widget groups it by new building or
//! repair, building type (from content) and state.
//!
//! # The measure: `produced` increments
//!
//! `CBuildingProductionLine::AdvanceProduction@140f55590` runs once a day per line
//! (from the country's daily production update): it adds the day's progress
//! (`GetProductionSpeed`, `+0x30`) to `produced` (`+0x38`), recomputes `cost`
//! (`+0x28`, `CalculateBuildingCost`) and, for each whole `cost` it holds, takes
//! `cost` off `produced`, lowers `amount` by one and raises the building a level.
//! So the progress a line made between two of its reads `a` and `b` is exact:
//!
//! > `produced(b) − produced(a) + cost × levels finished`
//!
//! where the levels finished are the new levels the Construction log confirms from
//! the building's hourly record (`built`, not `repair`): every rise the line can
//! claim, dated by rise or by read, even beyond how far `amount` fell (levels added
//! meanwhile keep it up, the log's `added`). Every rise of the building in the
//! stretch must be settled: a rise the log leaves unclaimed (the line does not show
//! it built the level) may still be a completion whose cost is unresolved, so that
//! stretch is unknown (`UnclaimedRise`); only a stretch whose rises are all
//! claimed, or that has none, counts. A fall in `amount` its building did not rise
//! for is a level dropped from the line, which adds no progress. Where the record
//! cannot settle the rises (an hour not recorded, another line at the building, the
//! log's `unaccounted`), how many it finished is unknown, and so is the stretch.
//! Amounts are in fixed5 (`COST_SCALE` per point of civilian-factory output, the
//! game's "IC").
//! A railway line's `produced` is its whole route's progress since it was created
//! (`cost − remaining_cost`), so its progress is `produced(b) − produced(a)`.
//!
//! The recorded `speed` is not the measure. In the PR132 part-M recording (branch
//! 1, 17 reads) building lines' `produced` grew by the next read's `speed` in 3208
//! stretches and by the previous read's in 125; in 21 it grew by neither, even with
//! the same `speed` at both reads (a line whose reads both show no factories and
//! no speed gained 10), and 19 lines with a speed held still (a line that cannot
//! build). A read's `speed` is the line's state then, not what the update applied.
//!
//! # What is counted, and when
//!
//! Each amount shows from the read that establishes it (`known_at`), like the
//! log's levels, so with Live on the total grows read by read.
//!
//! - **Between two reads** of a line with new levels to build (per stretch: a line
//!   can repair, then build): the formula above, when every hour between them was
//!   recorded and every scheduled read of the queue between them succeeded (a
//!   failed read could hide a cost change), the rises settle how many new levels it finished, its `cost` did not
//!   change while it finished a level, no other
//!   line without repair work targeted the same building at the later read (a
//!   linked line shares `produced` with its partner: `AdvanceProduction` writes it
//!   to both), it had no repair work at either read (the repair branch sets
//!   `produced` to 0), and the result is not negative.
//! - **Before its first read**: nothing when it was already in the queue at the
//!   queue's first read in the branch (`before_first_read`: its progress before
//!   the recording's first read is not counted). Otherwise its first read's
//!   `produced`, when the read before it was recorded without a gap since, the
//!   line was created after that read, and its building's level held at every
//!   recorded hour between the two reads (the hourly record: a level finished and
//!   then demolished leaves the endpoints equal, so equal endpoints prove nothing),
//!   and the log's opening-stretch rule finds no daily repair or possible completion
//!   since its creation: a repair resets progress even without a level change.
//!   A railway's `produced` at its first read is all its progress.
//! - **Finishing**: after a finished line's last read, the update that finished it
//!   recomputed `cost` first (`CalculateBuildingCost`, then `+0x28`), so the cost it
//!   finished at is not the one its last read shows, and no read after shows it:
//!   the stretch is unknown (`FinishedAfterRead`), never `cost × amount − produced`
//!   at the last read's cost. A railway's route cost is recomputed the same way.
//! - A line removed with levels unbuilt, or whose ending is unknown, may have
//!   progressed after its last read: that stretch is unknown.
//!
//! Every stretch that is not counted is listed with its reason
//! ([`SpendingGap`]), never as zero.
//!
//! # Repairs are not measured
//!
//! A line with repair work (`repair_levels > 0`) takes the repair branch of
//! `(1 + repair modifiers) × (BASE_FACTORY_REPAIR + speed × 100 / cost)`, scaled by
//! the building type's and the building's repair factors and capped at a whole
//! level, and `CBuilding::AdvanceRepair@1410c5c90` adds it to the building's
//! partial health (`CBuilding+0x48`, 10,000,000 a level), raising its healthy level
//! when full; the line's `produced` is set to 0. So a repair's progress is a share
//! of a level, part of it free (`BASE_FACTORY_REPAIR`), kept on the building and
//! not recorded; the civilian-factory output it took is the day's `speed`, which
//! (above) no read shows. Repair projects are listed with no amount, and the
//! widget says why.
//!
//! # Dating and grouping
//!
//! A project belongs to the tag its queue had at its first read (as in the log),
//! and to its target state (a railway: the states of its route's first and last
//! provinces). Leaves group projects by repair, kind, building type and state;
//! each leaf's `spent` is a running total at every read that adds to it.

use super::{
    CountryParams, LoadContext, Product,
    construction_log::{
        self, BuildingInfo, COST_SCALE, ConstructionProject, EndKind, ProjectKind, ProjectRun,
        ScannedLog, StateName,
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

/// Why a stretch of a project's progress is not counted.
#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq, PartialOrd, Ord, TS)]
#[serde(rename_all = "camelCase")]
pub enum SpendingGap {
    /// An hour between two reads of it was not recorded.
    NotRecorded,
    /// A scheduled read of its queue between two reads failed: its cost could
    /// have changed unseen.
    ReadFailed,
    /// It was created before the read before the one that first lists it.
    ListedLate,
    /// Its building's level changed at an hour between the read before and its
    /// first read, or its opening stretch repaired or possibly completed a level,
    /// so progress it may have made then is not known.
    BuiltBeforeRead,
    /// Another line without repair work targeted the same building.
    SharedTarget,
    /// It lost levels its building's record does not account for: built or
    /// dropped, the recording does not tell (the log's `unaccounted`).
    LevelsUnaccounted,
    /// Its building rose in the stretch without the line showing it built the
    /// level (a focus, event or decision, or a completion masked by an added
    /// level): whether a level's cost was spent is not known.
    UnclaimedRise,
    /// Its cost changed while it finished a level.
    CostChanged,
    /// It had repair work, which sets its progress to zero.
    Repairing,
    /// Its progress fell without a level being finished.
    Unexplained,
    /// It left the queue unfinished: what it did after its last read is not known.
    LeftUnfinished,
    /// It left the queue and whether it was finished is not known.
    EndUnknown,
    /// It finished after its last read, at a cost no read shows.
    FinishedAfterRead,
}

/// A project's spending as the scan establishes it.
#[derive(Clone, Debug, Default)]
pub struct ProjectSpending {
    /// Progress (fixed5) with the read that establishes it; none of zero.
    pub spent: Vec<(i32, i64)>,
    /// Stretches not counted, with the read that ends them.
    pub unknown: Vec<(i32, SpendingGap)>,
    /// It was in the queue at the branch's first read: its progress before is not counted.
    pub before_first_read: bool,
}

impl ProjectSpending {
    /// Records one stretch ending at `hour`.
    pub(crate) fn record(&mut self, hour: i32, stretch: Result<i64, SpendingGap>) {
        match stretch {
            Ok(0) => (),
            Ok(amount) => {
                self.spent.push((hour, amount));
            }
            Err(gap) => {
                self.unknown.push((hour, gap));
            }
        }
    }
}

/// What a read shows of a line's progress.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Progress {
    pub produced: i64,
    pub cost: i64,
    /// Levels (railway: segments) still to build (`amount`).
    pub remaining: i32,
    pub repair_levels: i32,
}

/// Progress a non-negative amount, else unexplained.
fn progress(amount: Option<i64>) -> Result<i64, SpendingGap> {
    amount
        .filter(|amount| *amount >= 0)
        .ok_or(SpendingGap::Unexplained)
}

/// A building line's progress between two of its reads, given the new levels its
/// building's rises show it finished (`finished`, or why that is not known). A level
/// it lost without a rise was dropped and adds nothing.
pub(crate) fn building_stretch(
    was: Progress,
    now: Progress,
    missed: Option<SpendingGap>,
    shared: bool,
    finished: Result<u32, SpendingGap>,
) -> Result<i64, SpendingGap> {
    if let Some(gap) = missed {
        return Err(gap);
    }
    if was.repair_levels > 0 || now.repair_levels > 0 {
        return Err(SpendingGap::Repairing);
    }
    if shared {
        return Err(SpendingGap::SharedTarget);
    }
    let finished = i64::from(finished?);
    if finished > 0 && was.cost != now.cost {
        return Err(SpendingGap::CostChanged);
    }
    progress(
        now.produced
            .checked_sub(was.produced)
            .and_then(|delta| delta.checked_add(finished.checked_mul(now.cost)?)),
    )
}

/// A railway line's progress between two of its reads.
pub(crate) fn railway_stretch(
    was: Progress,
    now: Progress,
    missed: Option<SpendingGap>,
) -> Result<i64, SpendingGap> {
    if let Some(gap) = missed {
        return Err(gap);
    }
    if was.cost != now.cost {
        return Err(SpendingGap::CostChanged);
    }
    progress(now.produced.checked_sub(was.produced))
}

/// The stretch after a line's last read, by how it left the queue: never counted,
/// since no read shows what it did after (a finish: at what cost), only why.
pub(crate) fn ending(kind: EndKind, missed: Option<SpendingGap>) -> SpendingGap {
    match kind {
        _ if missed.is_some() => missed.unwrap_or(SpendingGap::NotRecorded),
        EndKind::Finished => SpendingGap::FinishedAfterRead,
        EndKind::Removed => SpendingGap::LeftUnfinished,
        EndKind::Unknown => SpendingGap::EndUnknown,
    }
}

/// A new building line's progress before its first read: its first read's
/// `produced`, when its building's level held at every recorded hour since the read
/// before (`changed`: whether it changed; none when an hour's record is missing),
/// and the log's opening stretch has no repair or possible completion.
pub(crate) fn before_first_read(
    changed: Option<bool>,
    earlier_unknown: bool,
    produced: i64,
) -> Result<i64, SpendingGap> {
    match changed {
        Some(false) if !earlier_unknown => progress(Some(produced)),
        Some(_) => Err(SpendingGap::BuiltBeforeRead),
        None => Err(SpendingGap::NotRecorded),
    }
}

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
    /// Repair projects: their effort is not measured, so `spent` is empty.
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
    /// Projects already in the queue at its first read in the branch: their
    /// progress before that read is not counted.
    pub before_first_read: u32,
    pub warnings: Vec<String>,
}

/// A leaf: repair, railway, building type, state and route end state.
type LeafKey = (bool, bool, u32, Option<u32>, Option<u32>);
/// A leaf's amounts, unknown stretches and projects while grouping.
type LeafParts = (Vec<(i32, i64)>, Vec<SpendingUnknown>, Vec<LeafProject>);

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

/// One tag's leaves from a branch scan. A project joins the repair leaf of its
/// type and place for the reads it had levels to repair (and the stretches it
/// repaired in between two reads), and the new leaf for
/// those it had new levels to build (a line can repair, then build): its spending
/// is all new-level work, and repairs are not measured.
pub(crate) fn leaves(log: &ScannedLog, tag: &str) -> (Vec<SpendingLeaf>, u32) {
    let mut grouped: BTreeMap<LeafKey, LeafParts> = BTreeMap::new();
    let mut before = 0;
    for (project, spending) in log.projects.iter().zip(&log.spending) {
        if project.tag != tag {
            continue;
        }
        let railway = project.kind == ProjectKind::Railway;
        let key = |repair: bool| {
            (
                repair,
                railway,
                project.building,
                project.state,
                project.to_state.filter(|_| railway),
            )
        };
        let mut repairs = kind_spans(project, true, log.last_hour);
        if project.kind == ProjectKind::Building {
            dated_repairs(project, &log.reads, &mut repairs);
        }
        if !repairs.is_empty() {
            grouped.entry(key(true)).or_default().2.extend(repairs);
        }
        let new = kind_spans(project, false, log.last_hour);
        if !new.is_empty() || !spending.spent.is_empty() || !spending.unknown.is_empty() {
            let leaf = grouped.entry(key(false)).or_default();
            leaf.0.extend(spending.spent.iter().copied());
            leaf.1.extend(
                spending
                    .unknown
                    .iter()
                    .map(|&(known_at, reason)| SpendingUnknown {
                        known_at,
                        reason,
                        project: project.id.clone(),
                    }),
            );
            if new.is_empty() {
                leaf.2.push(LeafProject {
                    project: project.id.clone(),
                    first_read: project.first_read,
                    known_at: None,
                    until: project.end.as_ref().map(|end| end.known_at),
                    unread: false,
                    resumes: None,
                });
            }
            leaf.2.extend(new);
        }
        before += u32::from(spending.before_first_read);
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
                let mut total: i128 = 0;
                for (hour, amount) in spent {
                    total += i128::from(amount);
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
            SpendingGap::decl(),
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
            warnings,
        })
    }
}

#[cfg(test)]
mod tests;
