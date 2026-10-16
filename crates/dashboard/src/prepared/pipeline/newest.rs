//! Newest block windows first. While the families prepared on their own
//! prepare, a family whose default view is its branch's newest block window
//! publishes that window from the branch's last block and a look-back proven
//! to hold every fact the window's page carries, under names of its own. Its
//! query answers from it until forward preparation covers the branch, which
//! then publishes the same page bytes for the window. Land battles
//! ([`land_battles::newest`]) and naval battles ([`naval_battles::newest`])
//! publish theirs here, each family on a thread of its own beside the
//! families ([`independent::Beside`]), each branch's page in a commit of its
//! own as soon as it is proven, in the families' group commits.

use super::{Checkpoint, independent};
use crate::{
    metadata::check_cancel,
    products::{land_battles, naval_battles},
};
use replay::container::Replay;
use std::{io, time::Instant};

/// The land battle family's name in the checkpoint's refusals.
const LAND: &str = "land";

/// The naval family's name in the checkpoint's refusals and among the
/// families prepared on their own.
const NAVAL: &str = "naval";

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
    published: fn(&store::Store, &store::SourceKey, &Replay, u64) -> io::Result<bool>,
) -> io::Result<Vec<u64>> {
    let mut branches = Vec::new();
    for (&branch, &last) in phase.last_blocks {
        check_cancel(phase.cancelled)?;
        if reached(state, family, branch, last)
            || bound && published(phase.store, phase.key, phase.replay, branch)?
        {
            continue;
        }
        branches.push(branch);
    }
    Ok(branches)
}

/// One family's newest windows: the branches it tries, how it prepares and
/// writes a branch's window, and the name its time is counted under.
struct Family<'a, T> {
    timing: &'static str,
    branches: Vec<u64>,
    prepare: Box<dyn Fn(u64) -> io::Result<Option<T>> + Send + Sync + 'a>,
    write: fn(&T, &mut store::Writer) -> io::Result<()>,
}

impl<T> Family<'_, T> {
    /// Each branch's window in branch order, each published in a commit of
    /// its own as soon as it is proven; counts the family's time as its own.
    fn publish(
        &self,
        cancelled: &std::sync::atomic::AtomicBool,
        publish: &mut independent::Publish<'_>,
        timings: &mut independent::Timings,
        stall: &Stall,
    ) -> io::Result<()> {
        if self.branches.is_empty() {
            return Ok(());
        }
        let started = Instant::now();
        for &branch in &self.branches {
            check_cancel(cancelled)?;
            #[cfg(test)]
            if let Some(stall) = stall {
                stall(self.timing, branch);
            }
            #[cfg(not(test))]
            let Stall = stall;
            if let Some(newest) = (self.prepare)(branch)? {
                publish.commit(&mut |writer: &mut store::Writer| (self.write)(&newest, writer))?;
            }
        }
        if let Some(timings) = timings.as_mut() {
            let family = timings.entry(self.timing).or_default();
            family.observe += started.elapsed();
            family.calls += 1;
        }
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
) -> io::Result<Vec<independent::Beside<'a>>> {
    let bound = saved.as_ref() == Some(phase.key);
    let land = Family {
        timing: "newest.land",
        branches: wanted(phase, state, LAND, bound, land_battles::newest::published)?,
        prepare: Box::new(move |branch| {
            land_battles::newest::prepare(phase.source, branch, phase.context, phase.cancelled)
        }),
        write: land_battles::newest::Newest::write,
    };
    let naval = Family {
        timing: "newest.naval",
        branches: wanted(phase, state, NAVAL, bound, naval_battles::newest::published)?,
        prepare: Box::new(move |branch| {
            naval_battles::newest::prepare(phase.source, branch, phase.cancelled)
        }),
        write: naval_battles::newest::Newest::write,
    };
    let stall = stall();
    let cancelled = phase.cancelled;
    let mut jobs = Vec::new();
    if !together() {
        jobs.push(independent::Beside {
            name: "newest windows",
            run: Box::new(
                move |publish: &mut independent::Publish<'_>,
                      timings: &mut independent::Timings| {
                    land.publish(cancelled, publish, timings, &stall)?;
                    naval.publish(cancelled, publish, timings, &stall)
                },
            ),
        });
        return Ok(jobs);
    }
    if !land.branches.is_empty() {
        let stall = stall.clone();
        jobs.push(independent::Beside {
            name: "newest land windows",
            run: Box::new(
                move |publish: &mut independent::Publish<'_>,
                      timings: &mut independent::Timings| {
                    land.publish(cancelled, publish, timings, &stall)
                },
            ),
        });
    }
    if !naval.branches.is_empty() {
        jobs.push(independent::Beside {
            name: "newest naval windows",
            run: Box::new(
                move |publish: &mut independent::Publish<'_>,
                      timings: &mut independent::Timings| {
                    naval.publish(cancelled, publish, timings, &stall)
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

/// What tests run before each branch's window is prepared, with the family's
/// timing name and the branch, on the family's thread.
#[cfg(test)]
type Stall = Option<std::sync::Arc<dyn Fn(&str, u64) + Send + Sync>>;

/// Preparation runs nothing before a branch's window.
#[cfg(not(test))]
#[derive(Clone)]
struct Stall;

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

/// Run `run` with `stall` called before each branch's newest window is
/// prepared, on the family's thread, with the family's timing name
/// (`newest.land`, `newest.naval`) and the branch.
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
