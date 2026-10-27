//! A map view's raid offers, cycled off the frame path as the game's player
//! cache cycles them ([`viewer_raids::map::raids::cycle`]).
//!
//! The window hands the lane the frame of every hour the builders deliver
//! and tells it, every frame, which hour it shows and whether it plays. The
//! cycle is presentation history: the window resets it at the hour it puts
//! on screen exactly when the trains and the weather reset
//! ([`crate::map::presentation::continuity`]): a seek, same-hour ones too, a
//! step back, a branch change, a new recording, a recorded gap or a new
//! resolved viewing country. Playing, the lane builds each hour from the hour
//! before it: the hour's rebuild frame and one batch, as far ahead as the
//! builders have delivered, so a shown hour's icons are usually ready. Hours
//! the builders pass over to keep up with the clock (a playback skip) are
//! not waited for once the window shows a later hour: the cycle is carried
//! to the next hour delivered. Paused, it builds the shown hour again from
//! the hour before and then evaluates one batch per rendered frame until
//! every target is current. An hour whose resolved viewing country is not
//! the cycle's, or a target row that names another target, starts the cycle
//! again at that hour. Every result the lane publishes is stamped with the
//! reset it belongs to and the viewing country it was built for; the lane
//! keeps none built for a view it no longer shows, and the window installs
//! only one of the current reset built for the shown hour and resolved
//! country ([`RaidLane::icons`], [`RaidIcons::drawable_at`]).

use std::{
    collections::BTreeMap,
    sync::{Arc, Condvar, Mutex, MutexGuard},
};
use viewer_foundation::parallel::Workers;
use viewer_foundation::retirement::Retired;

use replay_data::{BranchId, Hour};
use replay_frame::frame::DecodedFrame;

use {
    crate::document::Content, crate::playback::Current, dashboard::triggers::World,
    viewer_foundation::map::perspective, viewer_raids::map::raids::HourLayer,
    viewer_raids::map::raids::IconCache, viewer_raids::map::raids::RaidIcons,
    viewer_raids::map::raids::cycle::Cycle, viewer_raids::map::raids::paths::HourPaths,
};

/// What the window shows and what the lane has built for it.
#[derive(Default)]
struct State {
    content: Option<Arc<Content>>,
    perspective: Option<perspective::Choice>,
    branch: Option<BranchId>,
    /// The frames of the built hours; `None` for a recorded gap.
    frames: BTreeMap<(BranchId, Hour), Option<Arc<DecodedFrame>>>,
    shown: Option<Hour>,
    playing: bool,
    /// Raised by every reset: the lane's generation.
    epoch: u64,
    /// A paused frame was rendered since the last batch.
    asked: bool,
    /// Icons built for `epoch`, by hour.
    built: BTreeMap<Hour, Published>,
    wake: Option<Arc<dyn Fn() + Send + Sync>>,
    /// The threads a batch may use besides the lane's
    /// ([`crate::parallel::Workers`]): the hour builders'.
    workers: Workers,
    stop: bool,
}

/// Whose a published result is: the reset (generation) it belongs to and
/// the viewing country it was built for. Fog is not in it: offers read only
/// the viewing country.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Stamp {
    generation: u64,
    country: perspective::Country,
}

/// A result the lane published, with its stamp.
struct Published {
    stamp: Stamp,
    icons: Arc<RaidIcons>,
}

impl State {
    /// Starts the cycle again: the results published so far are dropped,
    /// and those still being built are refused ([`State::publish`]).
    fn restart(&mut self, branch: BranchId) {
        self.epoch += 1;
        self.built.clear();
        self.branch = Some(branch);
    }

    /// What the window shows this frame. New content or a branch the lane
    /// does not hold starts the cycle again; every other reset is the
    /// window's presentation reset (`RaidLane::reset`).
    fn view(
        &mut self,
        new_content: bool,
        perspective: &perspective::Choice,
        (branch, hour): (BranchId, Hour),
        playing: bool,
    ) {
        if new_content || self.branch != Some(branch) {
            self.restart(branch);
        }
        if self.perspective.as_ref() != Some(perspective) {
            self.perspective = Some(perspective.clone());
        }
        self.shown = Some(hour);
        self.playing = playing;
        self.asked |= !playing;
    }

    /// The hour the window puts on screen with its own frame, and, when its
    /// presentation history resets there, the reset: all at once, so the
    /// lane never plans the new reset for the view before it, and holds the
    /// hour's frame even when it was delivered, and pruned, long before (a
    /// seek to a cached built hour).
    fn present(
        &mut self,
        new_content: bool,
        perspective: &perspective::Choice,
        (branch, hour): (BranchId, Hour),
        playing: bool,
        frame: Option<Arc<DecodedFrame>>,
        reset: bool,
    ) {
        self.frames.insert((branch, hour), frame);
        if reset {
            self.restart(branch);
        }
        self.view(new_content, perspective, (branch, hour), playing);
    }

    /// The stamp of what the window shows now.
    #[cfg(test)]
    fn stamp(&self) -> Option<Stamp> {
        Some(Stamp {
            generation: self.epoch,
            country: self.perspective.as_ref()?.country.clone(),
        })
    }

    /// Whether `stamp` is of the current reset and of the view the window
    /// shows now.
    fn current(&self, stamp: &Stamp) -> bool {
        stamp.generation == self.epoch
            && self
                .perspective
                .as_ref()
                .is_some_and(|shown| shown.country == stamp.country)
    }

    /// Keeps `icons` for `hour` when `stamp` is the current view's; `false`
    /// when it is not: a reset or a new viewing country since.
    fn publish(&mut self, stamp: Stamp, hour: Hour, icons: RaidIcons) -> bool {
        if !self.current(&stamp) {
            return false;
        }
        self.built.insert(
            hour,
            Published {
                stamp,
                icons: Arc::new(icons),
            },
        );
        true
    }

    /// The icons of `hour` of `branch` for canonical viewing country `ours`,
    /// when the lane holds a result of its current reset built for exactly
    /// that. A result of another resolved country is refused even when the
    /// chosen country is unchanged: the recorded player may change.
    fn icons(&self, branch: BranchId, hour: Hour, ours: u32) -> Option<Arc<RaidIcons>> {
        if self.branch != Some(branch) {
            return None;
        }
        let published = self.built.get(&hour)?;
        (published.stamp.generation == self.epoch
            && published.icons.drawable_at(branch, hour, ours))
        .then(|| Arc::clone(&published.icons))
    }
}

type Shared = Arc<(Mutex<State>, Condvar)>;

fn lock(shared: &Shared) -> MutexGuard<'_, State> {
    shared
        .0
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// Keeps `cycle`, the cycle after `hour` built for `stamp`, in `chain` when
/// it is still the current view's. A result built for a view the window
/// left (another reset, another viewing country) is dropped here as well as
/// refused at publication, so it can never mark the new view's hour
/// complete.
fn settle(chain: &mut Chain, state: &State, stamp: &Stamp, hour: Hour, cycle: Cycle) {
    if chain.epoch == stamp.generation && state.current(stamp) {
        chain.after.insert(hour, cycle);
    }
}

/// The cycle after each built hour of one reset, from its first hour on.
#[derive(Default)]
struct Chain {
    epoch: u64,
    start: Option<Hour>,
    after: BTreeMap<Hour, Cycle>,
    /// The offer icons laid out so far.
    icons: IconCache,
    /// A reset whose hour panicked: nothing more is built for it.
    failed: Option<u64>,
}

/// One hour to build: from the cycle after the chain's hour before it
/// (`None` at the chain's first hour), and, paused, stepped once per
/// rendered frame.
#[derive(Debug)]
struct Plan {
    epoch: u64,
    hour: Hour,
    frame: Option<Arc<DecodedFrame>>,
    from: Option<Cycle>,
    paused: bool,
}

struct Planned {
    plan: Plan,
    predecessor: Option<Hour>,
    shown: Hour,
}

fn retire_before<K: Ord + Send + 'static, V: Send + 'static>(
    values: &mut BTreeMap<K, V>,
    first: &K,
    retired: &mut Retired,
) {
    if values.first_key_value().is_some_and(|(key, _)| key < first) {
        let keep = values.split_off(first);
        retired.push(std::mem::replace(values, keep));
    }
}

fn retire_from<K: Ord + Send + 'static, V: Send + 'static>(
    values: &mut BTreeMap<K, V>,
    first: &K,
    retired: &mut Retired,
) {
    if values.last_key_value().is_some_and(|(key, _)| key >= first) {
        retired.push(values.split_off(first));
    }
}

impl Chain {
    /// The next hour the view needs built, if its frame has arrived. Frames
    /// and icons the view no longer needs retire after the state unlocks.
    fn prepare(&mut self, state: &mut State, retired: &mut Retired) -> Option<Planned> {
        let (shown, branch) = (state.shown?, state.branch?);
        if self.failed == Some(state.epoch) {
            return None;
        }
        if self.epoch != state.epoch || self.start.is_none_or(|start| shown < start) {
            *self = Self {
                epoch: state.epoch,
                start: Some(shown),
                ..Self::default()
            };
        }
        let start = self.start.expect("set above");
        let last = self.after.keys().next_back().copied();
        // Hours behind the shown hour are needed only as the one it builds on.
        let keep = last.map_or(shown, |last| {
            let bound = last.min(shown);
            self.after
                .range(..bound)
                .next_back()
                .map_or(bound, |(hour, _)| *hour)
        });
        retire_before(&mut self.after, &keep, retired);
        let hour = if state.playing {
            last.map_or(start, |last| {
                next_hour(state, branch, last, shown, Hour::MAX)
            })
        } else {
            // Paused, the shown hour is built again with its own sweep, unless
            // it is complete; later hours are the chain's no longer.
            if let Some(later) = shown.checked_add(1) {
                retire_from(&mut self.after, &later, retired);
            }
            match self.after.get(&shown) {
                Some(cycle) if cycle.complete() => return None,
                Some(_) => {
                    retired.push(self.after.remove(&shown).expect("incomplete shown cycle"));
                    if shown != start && self.after.range(..shown).next_back().is_none() {
                        // Nothing to build it from: the cycle starts again.
                        self.start = Some(shown);
                        if !self.after.is_empty() {
                            retired.push(std::mem::take(&mut self.after));
                        }
                    }
                    shown
                }
                None => last
                    .map_or(start, |last| next_hour(state, branch, last, shown, shown))
                    .min(shown),
            }
        };
        let start = self.start.expect("set above");
        let floor = hour.min(shown) - 1;
        retire_before(&mut state.frames, &(branch, floor), retired);
        if let Some(later_branch) = branch.checked_add(1) {
            retire_from(&mut state.frames, &(later_branch, Hour::MIN), retired);
        }
        retire_before(&mut state.built, &(shown - 1), retired);
        let frame = state.frames.get(&(branch, hour))?.clone();
        let predecessor = if hour == start {
            None
        } else {
            Some(*self.after.range(..hour).next_back()?.0)
        };
        Some(Planned {
            plan: Plan {
                epoch: self.epoch,
                hour,
                frame,
                from: None,
                paused: !state.playing && hour == shown,
            },
            predecessor,
            shown,
        })
    }

    fn snapshot(&self, planned: Planned) -> Plan {
        let Planned {
            mut plan,
            predecessor,
            shown,
        } = planned;
        let _cost = replay_frame::cpu::scope(
            if plan.hour < shown {
                "raids.plan.snapshot.behind"
            } else if plan.hour == shown {
                "raids.plan.snapshot.shown"
            } else {
                "raids.plan.snapshot.ahead"
            },
            Some("other"),
        );
        plan.from =
            predecessor.map(|hour| self.after.get(&hour).expect("planned predecessor").clone());
        plan
    }

    #[cfg(test)]
    fn plan(&mut self, state: &mut State) -> Option<Plan> {
        let mut retired = Retired::default();
        let planned = self.prepare(state, &mut retired);
        drop(retired);
        planned.map(|planned| self.snapshot(planned))
    }
}

/// The hour the chain builds after `last`: the next one, or, once the window
/// shows a later hour and the next one's frame has not arrived, the next
/// hour whose frame has, up to `limit`. Playing, the builders pass hours
/// over to keep up with the clock (a playback skip, not a seek): the cycle
/// is carried across them, one batch per hour built, rather than waiting for
/// an hour that may never come.
fn next_hour(state: &State, branch: BranchId, last: Hour, shown: Hour, limit: Hour) -> Hour {
    let next = last + 1;
    if next >= shown || next > limit || state.frames.contains_key(&(branch, next)) {
        return next;
    }
    state
        .frames
        .range((branch, next)..=(branch, limit))
        .next()
        .map_or(next, |((_, hour), _)| *hour)
}

/// An hour the window puts on screen, as the raid lane needs it: the view
/// it is shown in and its own frame.
pub(crate) struct Presented<'a> {
    pub content: Option<&'a Arc<Content>>,
    pub perspective: &'a perspective::Choice,
    pub at: (BranchId, Hour),
    pub playing: bool,
    pub current: &'a Current,
}

/// The raid cycle of one map view, on a thread of its own, started on the
/// first hour shown.
#[derive(Default)]
pub(crate) struct RaidLane {
    shared: Shared,
    started: bool,
}

impl RaidLane {
    /// A built hour's frame, which the lane builds its icons from.
    pub(crate) fn feed(&self, branch: BranchId, hour: Hour, current: &Current) {
        let frame = match current {
            Current::Frame(frame) => Some(Arc::clone(frame)),
            Current::Gap { .. } => None,
        };
        lock(&self.shared).frames.insert((branch, hour), frame);
        self.shared.1.notify_all();
    }

    /// The hour the window puts on screen, with its frame, and, when its
    /// presentation history resets there (`reset`), the view it resets to:
    /// installed under one lock before the lane wakes
    /// ([`State::present`]). `presented.content` is `None` only in tests,
    /// which have none: the lane keeps what it holds.
    pub(crate) fn present(&self, presented: Presented<'_>, reset: bool) {
        let frame = match presented.current {
            Current::Frame(frame) => Some(Arc::clone(frame)),
            Current::Gap { .. } => None,
        };
        let mut state = lock(&self.shared);
        let new_content = presented.content.is_some_and(|content| {
            state
                .content
                .as_ref()
                .is_none_or(|held| !Arc::ptr_eq(held, content))
        });
        if new_content {
            state.content = presented.content.cloned();
        }
        state.present(
            new_content,
            presented.perspective,
            presented.at,
            presented.playing,
            frame,
            reset,
        );
        drop(state);
        self.shared.1.notify_all();
    }

    /// What the window shows this frame; a paused frame asks for one batch.
    /// `wake` repaints the window when icons arrive.
    pub(crate) fn show(
        &mut self,
        content: &Arc<Content>,
        perspective: &perspective::Choice,
        (branch, hour): (BranchId, Hour),
        playing: bool,
        wake: impl Fn() + Send + Sync + 'static,
    ) {
        if !self.started {
            let shared = Arc::clone(&self.shared);
            // Without a lane no offer is evaluated, and the diagnostics say
            // the icons are still being built.
            self.started = std::thread::Builder::new()
                .name("raid offers".into())
                .spawn(move || serve(&shared))
                .is_ok();
        }
        let mut state = lock(&self.shared);
        let new_content = state
            .content
            .as_ref()
            .is_none_or(|held| !Arc::ptr_eq(held, content));
        if new_content {
            state.content = Some(Arc::clone(content));
        }
        state.view(new_content, perspective, (branch, hour), playing);
        if state.wake.is_none() {
            state.wake = Some(Arc::new(wake));
        }
        drop(state);
        self.shared.1.notify_all();
    }

    /// Lends the lane's batches the hour builders' threads.
    pub(crate) fn use_workers(&self, workers: Workers) {
        lock(&self.shared).workers = workers;
    }

    /// The icons the lane built for `hour` of `branch` and canonical
    /// viewing country `ours`, when it has: `None` for a result of an
    /// earlier reset or another country.
    pub(crate) fn icons(&self, branch: BranchId, hour: Hour, ours: u32) -> Option<Arc<RaidIcons>> {
        lock(&self.shared).icons(branch, hour, ours)
    }

    /// The lane's generation, raised by every reset.
    #[cfg(test)]
    pub(crate) fn generation(&self) -> u64 {
        lock(&self.shared).epoch
    }

    /// Whether the lane holds `hour`'s frame, so that it can build the hour.
    #[cfg(test)]
    pub(crate) fn has_frame(&self, branch: BranchId, hour: Hour) -> bool {
        lock(&self.shared).frames.contains_key(&(branch, hour))
    }
}

impl Drop for RaidLane {
    fn drop(&mut self) {
        lock(&self.shared).stop = true;
        self.shared.1.notify_all();
    }
}

fn wait_for_plan<T, R>(
    shared: &Shared,
    chain: &mut Chain,
    mut capture: impl FnMut(&mut Chain, &mut State, &mut Retired) -> Option<T>,
    finish: impl FnOnce(&Chain, T) -> R,
) -> Option<R> {
    loop {
        let (planned, retired) = {
            let _other_cost = replay_frame::cpu::scope("raids.plan", Some("other"));
            let mut state = lock(shared);
            let mut retired = Retired::default();
            loop {
                if state.stop {
                    return None;
                }
                let planned = capture(chain, &mut state, &mut retired);
                if planned.is_some() || !retired.is_empty() {
                    break (planned, retired);
                }
                state = shared
                    .1
                    .wait(state)
                    .unwrap_or_else(|poisoned| poisoned.into_inner());
            }
        };
        // Release expired credit before a new snapshot, even when no job is ready.
        {
            let _cost = replay_frame::cpu::scope("raids.plan.retire", Some("other"));
            drop(retired);
        }
        if let Some(planned) = planned {
            return Some(finish(chain, planned));
        }
    }
}

fn serve(shared: &Shared) {
    let _other_cost = replay_frame::cpu::scope("raids.lane", Some("other"));
    let mut chain = Chain::default();
    loop {
        let Some(job) = wait_for_plan(
            shared,
            &mut chain,
            |chain, state, retired| {
                let content = state.content.clone()?;
                let perspective = state.perspective.clone()?;
                let plan = chain.prepare(state, retired)?;
                Some((content, perspective, plan))
            },
            |chain, (content, perspective, planned)| Job {
                content,
                perspective,
                plan: chain.snapshot(planned),
            },
        ) else {
            return;
        };
        let (epoch, hour) = (job.plan.epoch, job.plan.hour);
        let stamp = job.stamp();
        // A panicking hour must not stop the lane: it is delivered failed,
        // and the cycle starts again at the next hour.
        let icons = &mut chain.icons;
        let built =
            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| build(shared, job, icons)));
        let _settle_cost = replay_frame::cpu::scope("raids.settle", Some("other"));
        match built {
            Ok(cycle) => settle(&mut chain, &lock(shared), &stamp, hour, cycle),
            Err(_) => {
                let failed = RaidIcons {
                    report: viewer_raids::map::raids::RaidReport {
                        worker_failed: true,
                        ..Default::default()
                    },
                    ..Default::default()
                };
                publish(shared, stamp, hour, failed);
                chain.failed = Some(epoch);
            }
        }
    }
}

/// Stores `icons` for `hour` when `stamp` is still the current view's, and
/// wakes the window.
fn publish(shared: &Shared, stamp: Stamp, hour: Hour, icons: RaidIcons) {
    let mut state = lock(shared);
    if !state.publish(stamp, hour, icons) {
        return;
    }
    let wake = state.wake.clone();
    drop(state);
    if let Some(wake) = wake {
        wake();
    }
}

/// A [`Plan`] with what building it reads.
struct Job {
    content: Arc<Content>,
    perspective: perspective::Choice,
    plan: Plan,
}

impl Job {
    /// The stamp of every result built for it.
    fn stamp(&self) -> Stamp {
        Stamp {
            generation: self.plan.epoch,
            country: self.perspective.country.clone(),
        }
    }
}

/// `from` when it is the same branch's and viewing country's cycle, else a
/// reset one: a new viewing country starts the cycle again.
fn carried(from: Option<Cycle>, branch: BranchId, ours: u32) -> Cycle {
    match from {
        Some(cycle) if cycle.ours() == ours && cycle.branch() == branch => cycle,
        _ => Cycle::new(branch, ours),
    }
}

/// Builds `job`'s hour; paused, keeps stepping it one batch per rendered
/// frame while it stays the shown, paused hour. The cycle after it.
fn build(shared: &Shared, job: Job, icons: &mut IconCache) -> Cycle {
    let _cost = replay_frame::cpu::scope("viewer", Some("raid responses"));
    let stamp = job.stamp();
    let publish = |hour, built| publish(shared, stamp.clone(), hour, built);
    let Job {
        content,
        perspective,
        plan:
            Plan {
                epoch,
                hour,
                frame,
                from,
                paused,
            },
    } = job;
    // A recorded gap evaluates nothing, and the window shows the gap. The
    // window's presentation history resets at the hour it shows after it,
    // which starts the cycle again there ([`RaidLane::reset`]); the lane
    // needs no reset of its own.
    let Some(frame) = frame else {
        let branch = lock(shared).branch.unwrap_or_default();
        return from.unwrap_or_else(|| Cycle::new(branch, 0));
    };
    let branch = frame.branch;
    let ours = match crate::document::Hour::new(&content, &frame, 0.0).vision(&perspective) {
        Ok(vision) => vision.ours(),
        Err(error) => {
            publish(hour, RaidIcons::unavailable(&frame, 0, error.to_string()));
            return from.unwrap_or_else(|| Cycle::new(branch, 0));
        }
    };
    let assets = &content.icons.raids;
    let world = match World::new(&frame, assets.trigger_content.facts()) {
        Ok(world) => world,
        Err(reason) => {
            publish(
                hour,
                RaidIcons::unavailable(&frame, ours, reason.to_string()),
            );
            return from.unwrap_or_else(|| Cycle::new(branch, ours));
        }
    };
    let paths = HourPaths::new(assets, &frame);
    let mut layer = HourLayer::new(assets, &world, ours, &paths);
    layer.sweep.workers = lock(shared).workers.clone();
    let mut cycle = carried(from, branch, ours);
    if let Err(reason) = cycle.begin(assets, &layer.sweep) {
        cycle = Cycle::restarted(frame.branch, ours, reason);
        // A cycle with no row always begins.
        let _ = cycle.begin(assets, &layer.sweep);
    }
    cycle.step(&layer.sweep);
    publish(hour, layer.icons(&cycle, icons));
    while paused && !cycle.complete() {
        {
            let mut state = lock(shared);
            loop {
                let current = !state.stop
                    && state.epoch == epoch
                    && state.shown == Some(hour)
                    && !state.playing
                    && state
                        .content
                        .as_ref()
                        .is_some_and(|held| Arc::ptr_eq(held, &content))
                    && state.perspective.as_ref() == Some(&perspective);
                if !current {
                    return cycle;
                }
                if state.asked {
                    state.asked = false;
                    break;
                }
                state = shared
                    .1
                    .wait(state)
                    .unwrap_or_else(|poisoned| poisoned.into_inner());
            }
        }
        cycle.step(&layer.sweep);
        publish(hour, layer.icons(&cycle, icons));
    }
    cycle
}

#[cfg(test)]
mod tests;
