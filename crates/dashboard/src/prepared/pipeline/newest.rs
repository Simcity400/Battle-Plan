//! Newest block windows first. While the families prepared on their own
//! prepare, a family whose default view is its branch's newest block window
//! publishes that window from the branch's last block and a look-back proven
//! to hold every fact the window's page carries, under names of its own. Its
//! query answers from it until forward preparation covers the branch, which
//! then publishes the same page bytes for the window. Land battles
//! ([`land_battles::newest`]) and naval battles ([`naval_battles::newest`])
//! publish theirs here, each family on a thread of its own beside the
//! families ([`independent::Beside`]), each branch's page in a commit of its
//! own as soon as it is proven, in the families' group commits. The land
//! window's diagnostics are read and committed afterward on the same Beside
//! job; until then the proven page answers with `warningsComplete: false`.
//!
//! What each branch's preparation did ([`Report`]: the earlier blocks read
//! by their changes, every look-back tried with its first block, its blocks,
//! its time and the check that failed with the hour it names, the blocks
//! read for diagnostics, and the outcome) is counted in the family's timing
//! rows (`newest.<family>.earlier`, `.look-back`, `.naming`, and one row per
//! outcome) and reported in a status row of the family's own while
//! preparation runs ([`Reports`]).

use super::super::{Section, State};
use super::{Checkpoint, independent};
use crate::{
    metadata::check_cancel,
    products::{land_battles, naval_battles},
};
use dashboard_base::prepared::newest::{Outcome, Report};
use replay::container::Replay;
use std::{
    collections::BTreeMap,
    io,
    sync::{Mutex, PoisonError},
    time::Instant,
};

/// Status entries advanced by the proven newest-window commits.
pub(crate) fn gate(product: &str) -> Option<&'static str> {
    match product {
        "land-battles" | "land-battles-manifest" => Some("newest.land"),
        "naval-battles" | "naval-battles-manifest" => Some("newest.naval"),
        _ => None,
    }
}

fn extent(phase: &independent::Phase<'_>, branch: u64) -> super::Coverage {
    let window = phase
        .source
        .block_windows(branch)
        .last()
        .cloned()
        .expect("branch window");
    super::Coverage {
        schema: super::SCHEMA,
        first_hour: window.first_hour,
        last_hour: window.last_hour,
        complete: true,
    }
}

/// The land battle family's name in the checkpoint's refusals.
const LAND: &str = "land";

/// The naval family's name in the checkpoint's refusals and among the
/// families prepared on their own.
const NAVAL: &str = "naval";
const LAND_JOB: &str = "newest land windows";
const NAVAL_JOB: &str = "newest naval windows";
const JOINT_JOB: &str = "newest windows";

/// Whether `family`'s forward preparation has accepted or refused the
/// branch's last block, `last`: by its own progress while it is prepared on
/// its own, by the hourly walk's otherwise.
fn reached(state: &Checkpoint, family: &str, branch: u64, last: usize) -> bool {
    let (next_block, refused) = if independent::registered(family) {
        let Some(progress) = state.independent.get(family) else {
            return false;
        };
        (progress.next_block, progress.refused.get(&branch))
    } else {
        (state.cursor.next_block, state.refused.get(&branch))
    };
    next_block > last || refused.is_some_and(|refused| !refused.open(family))
}

/// The branches whose newest `family` window this preparation tries: those
/// whose last block its forward preparation has neither accepted nor
/// refused, and for whose current last block no earlier preparation
/// published one.
fn wanted(
    phase: &independent::Phase<'_>,
    state: &Checkpoint,
    family: &str,
    bound: bool,
    resume: bool,
    published: fn(&store::Store, &store::SourceKey, &Replay, u64) -> io::Result<bool>,
) -> io::Result<Vec<u64>> {
    let mut branches = Vec::new();
    for (&branch, &last) in phase.last_blocks {
        check_cancel(phase.cancelled)?;
        if !resume
            && (reached(state, family, branch, last)
                || bound && published(phase.store, phase.key, phase.replay, branch)?)
        {
            continue;
        }
        branches.push(branch);
    }
    Ok(branches)
}

/// A family's names: its status row, and its timing rows.
struct Names {
    section: &'static str,
    label: &'static str,
    /// The family's whole time, one call per preparation.
    timing: &'static str,
    /// Earlier blocks read by their changes: time, and blocks as calls.
    earlier: &'static str,
    /// Look-backs: time, and blocks observed as calls.
    look_back: &'static str,
    /// Earlier blocks read for diagnostics: time, and blocks as calls.
    naming: &'static str,
    /// One row per outcome, each branch's a call.
    outcomes: [&'static str; 7],
}

impl Names {
    fn outcome(&self, outcome: &Outcome) -> &'static str {
        self.outcomes[match outcome {
            Outcome::Unfinished => 0,
            Outcome::Proven => 1,
            Outcome::BranchStart(_) => 2,
            Outcome::Unproven(_) => 3,
            Outcome::Unavailable => 4,
            Outcome::NamingRefuses => 5,
            Outcome::Unreadable(_) => 6,
        }]
    }
}

const LAND_NAMES: Names = Names {
    section: "land-battles-newest",
    label: "Land battles: newest window",
    timing: "newest.land",
    earlier: "newest.land.earlier",
    look_back: "newest.land.look-back",
    naming: "newest.land.naming",
    outcomes: [
        "newest.land.unfinished",
        "newest.land.proven",
        "newest.land.branch-start",
        "newest.land.unproven",
        "newest.land.unavailable",
        "newest.land.naming-refuses",
        "newest.land.unreadable",
    ],
};

const NAVAL_NAMES: Names = Names {
    section: "naval-battles-newest",
    label: "Naval battles: newest window",
    timing: "newest.naval",
    earlier: "newest.naval.earlier",
    look_back: "newest.naval.look-back",
    naming: "newest.naval.naming",
    outcomes: [
        "newest.naval.unfinished",
        "newest.naval.proven",
        "newest.naval.branch-start",
        "newest.naval.unproven",
        "newest.naval.unavailable",
        "newest.naval.naming-refuses",
        "newest.naval.unreadable",
    ],
};

/// Counts `time` and `calls` in `timings`' row `name`, when there are calls.
fn count(
    timings: &mut independent::Timings,
    name: &'static str,
    time: std::time::Duration,
    calls: usize,
) {
    if let Some(timings) = timings.as_mut()
        && calls > 0
    {
        let row = timings.entry(name).or_default();
        row.observe += time;
        row.calls += calls as u64;
    }
}

/// What each family's newest windows did so far, for the status rows
/// reported while preparation runs.
#[derive(Default)]
pub(super) struct Reports(Mutex<Vec<FamilyReport>>);

/// One family's newest windows: the branches it tries, whether it has
/// finished, and each branch's report so far.
struct FamilyReport {
    names: &'static Names,
    wanted: usize,
    finished: bool,
    branches: BTreeMap<u64, Report>,
}

impl Reports {
    fn update(&self, names: &'static Names, change: impl FnOnce(&mut FamilyReport)) {
        let mut families = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some(family) = families
            .iter_mut()
            .find(|family| family.names.section == names.section)
        {
            change(family);
        }
    }

    fn begin(&self, names: &'static Names, wanted: usize) {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(FamilyReport {
                names,
                wanted,
                finished: false,
                branches: BTreeMap::new(),
            });
    }

    /// Whether no family's newest windows are tried.
    pub(super) fn is_empty(&self) -> bool {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .is_empty()
    }

    /// Appends each family's status row: preparing until it has tried every
    /// branch, the branches proven of those tried, and each branch's report
    /// as a line.
    pub(super) fn annotate(&self, sections: &mut Vec<Section>) {
        let families = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        for family in families.iter() {
            let proven = family
                .branches
                .values()
                .filter(|report| report.outcome == Outcome::Proven)
                .count();
            let lines: Vec<String> = family
                .branches
                .iter()
                .map(|(branch, report)| format!("Branch {branch}: {}", report.describe()))
                .collect();
            sections.push(Section {
                id: family.names.section.to_owned(),
                label: family.names.label.to_owned(),
                state: if family.finished {
                    State::Ready
                } else {
                    State::Preparing
                },
                done: proven.to_string(),
                total: Some(family.wanted.to_string()),
                error: (!lines.is_empty()).then(|| lines.join("; ")),
            });
        }
    }
}

/// One family's newest windows: the branches it tries, how it prepares and
/// publishes a branch's window and reports its work, with its names.
struct Family<'a> {
    names: &'static Names,
    branches: Vec<u64>,
    #[allow(clippy::type_complexity)]
    run: Box<
        dyn Fn(u64, &mut independent::Publish<'_>, &mut independent::Timings) -> io::Result<Report>
            + Send
            + Sync
            + 'a,
    >,
}

impl Family<'_> {
    /// Each branch's window in branch order, each published in a commit of
    /// its own as soon as it is proven; counts the family's time as its own,
    /// and reports what each branch's preparation did.
    fn publish(
        &self,
        cancelled: &std::sync::atomic::AtomicBool,
        publish: &mut independent::Publish<'_>,
        timings: &mut independent::Timings,
        reports: &Reports,
        stall: &Stall,
    ) -> io::Result<()> {
        if self.branches.is_empty() {
            return Ok(());
        }
        let names = self.names;
        reports.begin(names, self.branches.len());
        let started = super::start_timing(timings);
        for &branch in &self.branches {
            check_cancel(cancelled)?;
            #[cfg(test)]
            if let Some(stall) = stall {
                stall(names.timing, branch);
            }
            #[cfg(not(test))]
            let Stall = stall;
            let report = (self.run)(branch, publish, timings)?;
            count(timings, names.earlier, report.earlier_took, report.earlier);
            count(
                timings,
                names.look_back,
                report.tried.iter().map(|tried| tried.took).sum(),
                report.tried.iter().map(|tried| tried.blocks).sum(),
            );
            count(timings, names.naming, report.naming_took, report.naming);
            count(
                timings,
                names.outcome(&report.outcome),
                std::time::Duration::ZERO,
                1,
            );
            reports.update(names, |family| {
                family.branches.insert(branch, report);
            });
        }
        reports.update(names, |family| family.finished = true);
        super::finish_timing(timings, started, names.timing, false);
        Ok(())
    }
}

/// The newest land and naval windows of each branch whose last block that
/// family's forward preparation has neither accepted nor refused, and for
/// whose current last block no earlier preparation published one, as work
/// beside the families: each family's windows on a thread of its own, their
/// look-backs and earlier-block reads within the shared block-read core
/// budget (tests may prepare the two families' in turn on one thread).
pub(super) fn jobs<'a>(
    phase: &'a independent::Phase<'a>,
    state: &Checkpoint,
    saved: &Option<store::SourceKey>,
    reports: &'a Reports,
    pending: &std::collections::BTreeSet<String>,
) -> io::Result<Vec<independent::Beside<'a>>> {
    let bound = saved.as_ref() == Some(phase.key);
    let stall = stall();
    let diagnosing = stall.clone();
    let land = Family {
        names: &LAND_NAMES,
        branches: wanted(
            phase,
            state,
            LAND,
            bound,
            pending.contains(LAND_JOB) || pending.contains(JOINT_JOB),
            land_battles::newest::published,
        )?,
        run: Box::new(move |branch, publish, timings| {
            let (proven, mut report) = land_battles::newest::prove_reported(
                phase.source,
                branch,
                phase.context,
                phase.cancelled,
            )?;
            let Some(proven) = proven else {
                return Ok(report);
            };
            publish.window(
                "newest.land",
                branch,
                extent(phase, branch),
                &mut |writer| proven.write(writer),
            )?;
            // The proof is visible before the branch-wide naming reads;
            // a naming refusal withholds only the diagnostics.
            before(&diagnosing, "newest.land.diagnostics", branch);
            let started = Instant::now();
            let workers = (land_battles::newest::workers() / 4).max(1);
            let diagnosed =
                proven.diagnose_reported(phase.replay, workers, phase.cancelled, &mut report)?;
            if let Some(timings) = timings.as_mut() {
                let family = timings.entry("newest.land.diagnostics").or_default();
                // Naming has its own row in the report, when any block was named.
                family.observe += started.elapsed().saturating_sub(if report.naming > 0 {
                    report.naming_took
                } else {
                    std::time::Duration::ZERO
                });
                family.calls += 1;
            }
            if let Some(diagnosed) = diagnosed {
                publish.commit(&mut |writer| diagnosed.write(writer))?;
            }
            Ok(report)
        }),
    };
    let naval = Family {
        names: &NAVAL_NAMES,
        branches: wanted(
            phase,
            state,
            NAVAL,
            bound,
            pending.contains(NAVAL_JOB) || pending.contains(JOINT_JOB),
            naval_battles::newest::published,
        )?,
        run: Box::new(move |branch, publish, _| {
            let (newest, report) =
                naval_battles::newest::prepare_reported(phase.source, branch, phase.cancelled)?;
            if let Some(newest) = newest {
                publish.window(
                    "newest.naval",
                    branch,
                    extent(phase, branch),
                    &mut |writer| newest.write(writer),
                )?;
            }
            Ok(report)
        }),
    };
    let cancelled = phase.cancelled;
    let mut jobs = Vec::new();
    if land.branches.is_empty() && naval.branches.is_empty() {
        return Ok(jobs);
    }
    if !together() {
        jobs.push(independent::Beside {
            name: JOINT_JOB,
            first_view: true,
            run: Box::new(
                move |publish: &mut independent::Publish<'_>,
                      timings: &mut independent::Timings| {
                    land.publish(cancelled, publish, timings, reports, &stall)?;
                    naval.publish(cancelled, publish, timings, reports, &stall)
                },
            ),
        });
        return Ok(jobs);
    }
    if !land.branches.is_empty() {
        let stall = stall.clone();
        jobs.push(independent::Beside {
            name: LAND_JOB,
            first_view: true,
            run: Box::new(
                move |publish: &mut independent::Publish<'_>,
                      timings: &mut independent::Timings| {
                    land.publish(cancelled, publish, timings, reports, &stall)
                },
            ),
        });
    }
    if !naval.branches.is_empty() {
        jobs.push(independent::Beside {
            name: NAVAL_JOB,
            first_view: true,
            run: Box::new(
                move |publish: &mut independent::Publish<'_>,
                      timings: &mut independent::Timings| {
                    naval.publish(cancelled, publish, timings, reports, &stall)
                },
            ),
        });
    }
    Ok(jobs)
}

/// Whether the land and naval windows are prepared at once; tests may
/// prepare them one family after the other.
fn together() -> bool {
    #[cfg(test)]
    if TEST_IN_TURN.with(std::cell::Cell::get) {
        return false;
    }
    true
}

/// What tests run before each branch's window or diagnostics are prepared,
/// with the timing name and the branch, on the family's thread.
#[cfg(test)]
type Stall = Option<std::sync::Arc<dyn Fn(&str, u64) + Send + Sync>>;

/// Preparation runs nothing before a branch's window.
#[cfg(not(test))]
#[derive(Clone)]
struct Stall;

/// Runs the test hook before a window or its diagnostics are prepared.
fn before(stall: &Stall, family: &str, branch: u64) {
    #[cfg(test)]
    if let Some(stall) = stall {
        stall(family, branch);
    }
    #[cfg(not(test))]
    let _ = (stall, family, branch);
}

/// This thread's stall, for the families' threads.
fn stall() -> Stall {
    #[cfg(test)]
    {
        TEST_STALL.with(|stall| stall.borrow().clone())
    }
    #[cfg(not(test))]
    {
        Stall
    }
}

#[cfg(test)]
thread_local! {
    static TEST_IN_TURN: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    static TEST_STALL: std::cell::RefCell<Stall> = const { std::cell::RefCell::new(None) };
}

/// Run `run` with the land and naval windows prepared one family after the
/// other.
#[cfg(test)]
pub(super) fn with_test_in_turn<T>(run: impl FnOnce() -> T) -> T {
    struct Reset(bool);
    impl Drop for Reset {
        fn drop(&mut self) {
            TEST_IN_TURN.with(|value| value.set(self.0));
        }
    }
    let _reset = Reset(TEST_IN_TURN.with(|value| value.replace(true)));
    run()
}

/// Run `run` with `stall` called before each branch's newest window or
/// diagnostics are prepared, on the family's thread, with the timing name
/// (`newest.land`, `newest.land.diagnostics`, `newest.naval`) and the branch.
#[cfg(test)]
pub(super) fn with_test_stall<T>(
    stall: impl Fn(&str, u64) + Send + Sync + 'static,
    run: impl FnOnce() -> T,
) -> T {
    struct Reset(Stall);
    impl Drop for Reset {
        fn drop(&mut self) {
            TEST_STALL.with(|value| *value.borrow_mut() = self.0.take());
        }
    }
    let _reset = Reset(TEST_STALL.with(|value| value.replace(Some(std::sync::Arc::new(stall)))));
    run()
}
