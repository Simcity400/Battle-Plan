//! A map view's raid offers, cycled off the frame path as the game's player
//! cache cycles them ([`crate::map::raids::cycle`]).
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

use replay::{BranchId, Hour, frame::DecodedFrame};

use crate::{
    document::Content,
    map::{
        perspective,
        raids::{HourLayer, IconCache, RaidIcons, cycle::Cycle, paths::HourPaths, script::World},
    },
    playback::Current,
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
    workers: crate::parallel::Workers,
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
    /// window's presentation reset ([`RaidLane::reset`]).
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

impl Chain {
    /// The next hour the view needs built, if its frame has arrived. Frames
    /// and icons the view no longer needs are dropped.
    fn plan(&mut self, state: &mut State) -> Option<Plan> {
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
        self.after.retain(|hour, _| *hour >= keep);
        let hour = if state.playing {
            last.map_or(start, |last| {
                next_hour(state, branch, last, shown, Hour::MAX)
            })
        } else {
            // Paused, the shown hour is built again with its own sweep, unless
            // it is complete; later hours are the chain's no longer.
            self.after.retain(|hour, _| *hour <= shown);
            match self.after.get(&shown) {
                Some(cycle) if cycle.complete() => return None,
                Some(_) => {
                    self.after.remove(&shown);
                    if shown != start && self.after.range(..shown).next_back().is_none() {
                        // Nothing to build it from: the cycle starts again.
                        self.start = Some(shown);
                        self.after.clear();
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
        state
            .frames
            .retain(|(built_branch, built), _| *built_branch == branch && *built >= floor);
        state.built.retain(|built, _| *built >= shown - 1);
        let frame = state.frames.get(&(branch, hour))?.clone();
        let from = if hour == start {
            None
        } else {
            Some(self.after.range(..hour).next_back()?.1.clone())
        };
        Some(Plan {
            epoch: self.epoch,
            hour,
            frame,
            from,
            paused: !state.playing && hour == shown,
        })
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
    pub(crate) fn use_workers(&self, workers: crate::parallel::Workers) {
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

fn serve(shared: &Shared) {
    let mut chain = Chain::default();
    loop {
        let job = {
            let mut state = lock(shared);
            loop {
                if state.stop {
                    return;
                }
                let sources = (state.content.clone(), state.perspective.clone());
                if let (Some(content), Some(perspective)) = sources
                    && let Some(plan) = chain.plan(&mut state)
                {
                    break Job {
                        content,
                        perspective,
                        plan,
                    };
                }
                state = shared
                    .1
                    .wait(state)
                    .unwrap_or_else(|poisoned| poisoned.into_inner());
            }
        };
        let (epoch, hour) = (job.plan.epoch, job.plan.hour);
        let stamp = job.stamp();
        // A panicking hour must not stop the lane: it is delivered failed,
        // and the cycle starts again at the next hour.
        let icons = &mut chain.icons;
        let built =
            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| build(shared, job, icons)));
        match built {
            Ok(cycle) => settle(&mut chain, &lock(shared), &stamp, hour, cycle),
            Err(_) => {
                let failed = RaidIcons {
                    report: crate::map::raids::RaidReport {
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
    let world = match World::new(&frame, assets.facts()) {
        Ok(world) => world,
        Err(reason) => {
            publish(hour, RaidIcons::unavailable(&frame, ours, reason));
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
mod tests {
    use super::*;

    /// A lane state showing `hour` of branch 1 with the frames of `hours`
    /// (recorded gaps: whether a frame exists is all a plan reads).
    fn state(hour: Hour, playing: bool, hours: std::ops::RangeInclusive<Hour>) -> State {
        State {
            branch: Some(1),
            shown: Some(hour),
            playing,
            epoch: 1,
            frames: hours.map(|hour| ((1, hour), None)).collect(),
            ..State::default()
        }
    }

    /// Builds `plan` as the lane would, with an empty cycle standing for
    /// its result, `complete` or not.
    fn built(chain: &mut Chain, plan: &Plan, complete: bool) {
        let cycle = if complete {
            Cycle::new(1, 0)
        } else {
            Cycle::pending(1, 0)
        };
        chain.after.insert(plan.hour, cycle);
    }

    /// Playing, each hour is built from the one before as its frame
    /// arrives, ahead of the shown hour.
    #[test]
    fn playing_builds_each_hour_from_the_one_before() {
        let mut chain = Chain::default();
        let mut state = state(10, true, 10..=11);
        let first = chain.plan(&mut state).unwrap();
        assert_eq!(
            (first.hour, first.from.is_none(), first.paused),
            (10, true, false)
        );
        built(&mut chain, &first, false);
        let next = chain.plan(&mut state).unwrap();
        assert_eq!((next.hour, next.from.is_some()), (11, true));
        built(&mut chain, &next, false);
        assert!(chain.plan(&mut state).is_none(), "hour 12 has no frame yet");
        state.frames.insert((1, 12), None);
        assert_eq!(chain.plan(&mut state).unwrap().hour, 12);
    }

    /// A reset (a seek, a step back, a branch change), or a shown hour
    /// before the chain's first, starts the cycle again at the shown hour.
    #[test]
    fn a_reset_starts_the_cycle_again_at_the_shown_hour() {
        let mut chain = Chain::default();
        let mut state = state(10, true, 8..=13);
        for _ in 0..3 {
            let plan = chain.plan(&mut state).unwrap();
            built(&mut chain, &plan, false);
        }
        state.epoch += 1;
        state.shown = Some(12);
        let plan = chain.plan(&mut state).unwrap();
        assert_eq!((plan.hour, plan.from.is_none()), (12, true));
        built(&mut chain, &plan, false);
        state.shown = Some(11);
        let plan = chain.plan(&mut state).unwrap();
        assert_eq!(
            (plan.hour, plan.from.is_none()),
            (11, true),
            "before the chain"
        );
    }

    /// Paused, the shown hour is built again from the hour before and
    /// stepped per frame until complete; later hours leave the chain.
    #[test]
    fn paused_the_shown_hour_is_cycled_until_complete() {
        let mut chain = Chain::default();
        let mut state = state(10, true, 10..=12);
        for _ in 0..3 {
            let plan = chain.plan(&mut state).unwrap();
            built(&mut chain, &plan, false);
        }
        state.shown = Some(11);
        state.playing = false;
        let plan = chain.plan(&mut state).unwrap();
        assert_eq!(
            (plan.hour, plan.from.is_some(), plan.paused),
            (11, true, true)
        );
        assert!(!chain.after.contains_key(&12));
        built(&mut chain, &plan, true);
        assert!(chain.plan(&mut state).is_none(), "complete: nothing to do");
        // Played on, the next hour builds from the complete one.
        state.playing = true;
        let plan = chain.plan(&mut state).unwrap();
        assert_eq!((plan.hour, plan.paused), (12, false));
    }

    /// A capture resets the lane at its hour, paused. Until the hour's frame
    /// is fed the lane has nothing to build, and the capture would wait for
    /// ever; once it is, the lane builds the hour from row 0 and cycles it
    /// until complete, and then has nothing more to do: the capture saves.
    #[test]
    fn a_cold_capture_cycles_its_hour_once_its_frame_is_fed() {
        let mut chain = Chain::default();
        let mut state = state(10, false, 10..=10);
        state.frames.clear();
        assert!(
            chain.plan(&mut state).is_none(),
            "no frame: nothing to build"
        );
        state.frames.insert((1, 10), None);
        let plan = chain.plan(&mut state).unwrap();
        assert_eq!(
            (plan.hour, plan.from.is_none(), plan.paused),
            (10, true, true)
        );
        built(&mut chain, &plan, true);
        assert!(chain.plan(&mut state).is_none(), "complete");
    }

    /// Playing, the builders pass hours over to keep up with the clock (a
    /// playback skip, not a seek). Once the window shows a later hour, the
    /// chain builds the next hour that arrived from the cycle of the last
    /// hour it built, in the same reset, instead of waiting for the hours
    /// passed over; paused, it does so up to the shown hour.
    #[test]
    fn a_playback_skip_carries_the_cycle_without_stalling_or_resetting() {
        let mut chain = Chain::default();
        let mut state = state(10, true, 10..=11);
        for _ in 0..2 {
            let plan = chain.plan(&mut state).unwrap();
            built(&mut chain, &plan, false);
        }
        // The cycle after hour 11, told apart by its country.
        chain.after.insert(11, Cycle::pending(1, 7));
        // 12 and 13 are passed over. While the window shows an hour before
        // them they may still come, and the chain waits for 12.
        state.frames.insert((1, 14), None);
        assert!(chain.plan(&mut state).is_none());
        state.shown = Some(14);
        let plan = chain.plan(&mut state).unwrap();
        assert_eq!((plan.hour, plan.epoch), (14, 1), "no reset");
        assert_eq!(plan.from.as_ref().map(Cycle::ours), Some(7), "carried");
        built(&mut chain, &plan, false);
        // An hour passed over that arrives late is not built.
        state.frames.insert((1, 12), None);
        state.frames.insert((1, 15), None);
        let plan = chain.plan(&mut state).unwrap();
        assert_eq!(plan.hour, 15);
        assert!(!state.frames.contains_key(&(1, 12)));
        built(&mut chain, &plan, false);
        // Paused two hours on, with 16 passed over: the shown hour builds
        // from 15 and cycles until complete.
        state.frames.insert((1, 17), None);
        state.shown = Some(17);
        state.playing = false;
        let plan = chain.plan(&mut state).unwrap();
        assert_eq!(
            (plan.hour, plan.paused, plan.from.is_some()),
            (17, true, true)
        );
        built(&mut chain, &plan, true);
        assert!(chain.plan(&mut state).is_none(), "complete");
    }

    /// A lagging lane keeps the frames it still needs, and drops the rest.
    #[test]
    fn a_lagging_lane_keeps_the_frames_it_needs() {
        let mut chain = Chain::default();
        let mut state = state(10, true, 9..=20);
        let plan = chain.plan(&mut state).unwrap();
        built(&mut chain, &plan, false);
        state.shown = Some(18);
        let plan = chain.plan(&mut state).unwrap();
        assert_eq!(plan.hour, 11);
        assert!(state.frames.contains_key(&(1, 11)));
        assert!(!state.frames.contains_key(&(1, 9)));
    }

    fn from(tag: &str) -> perspective::Choice {
        perspective::Choice {
            country: perspective::Country::Tag(tag.into()),
            fog: true,
        }
    }

    /// Icons of hour 10 on branch 1 for canonical country `ours`.
    fn icons_for(ours: u32) -> RaidIcons {
        RaidIcons {
            built_for: Some((1, 10, ours)),
            ..RaidIcons::default()
        }
    }

    /// After a switch of viewing country, nothing built for the previous
    /// one installs, even for one frame: a batch still being built for it is
    /// not kept, and a result kept from before is refused for the new
    /// resolved country. When the window puts the new country's hour on
    /// screen its presentation history resets, and the cycle with it.
    #[test]
    fn a_country_switch_leaves_no_icon_of_the_previous_country() {
        let (ger, sov) = (from("GER"), from("SOV"));
        let mut state = State::default();
        state.view(true, &ger, (1, 10), false);
        let building = state.stamp().unwrap();
        assert!(state.publish(building.clone(), 10, icons_for(3)));
        assert!(state.icons(1, 10, 3).is_some(), "the view's own icons");
        assert!(state.icons(1, 10, 4).is_none(), "another country's");
        state.view(false, &sov, (1, 10), false);
        // The tables for the Soviet view refuse Germany's icons.
        assert!(state.icons(1, 10, 4).is_none());
        assert!(
            !state.publish(building, 10, icons_for(3)),
            "a German batch finishing after the switch"
        );
        // The Soviet cycle's own results install.
        assert!(state.publish(state.stamp().unwrap(), 10, icons_for(4)));
        assert!(state.icons(1, 10, 4).is_some());
        // A fog change alone keeps the cycle: offers do not read the fog.
        let generation = state.epoch;
        let unfogged = perspective::Choice { fog: false, ..sov };
        state.view(false, &unfogged, (1, 10), false);
        assert_eq!(state.epoch, generation);
        assert!(state.icons(1, 10, 4).is_some());
        // The window's reset for the new resolved country drops them all.
        state.restart(1);
        assert!(state.icons(1, 10, 4).is_none());
        // The installer refuses icons of another country or hour.
        let installed = icons_for(4);
        assert!(installed.drawable_at(1, 10, 4));
        assert!(!installed.drawable_at(1, 10, 3), "the previous country's");
        assert!(!installed.drawable_at(1, 11, 4), "another hour's");
        assert!(!RaidIcons::default().drawable_at(1, 10, 4));
    }

    /// A country switch while an hour is in flight. The new view and its
    /// reset arrive under one lock, so every plan after the reset is the new
    /// view's. And in the interleaving the review reproduced (the lane woke
    /// on a reset before the new view arrived, and planned the new
    /// generation for the old country), the old country's result, complete,
    /// is refused at publication and dropped from the chain too: the new
    /// view's paused hour is planned again and cycles until complete.
    #[test]
    fn a_country_switch_with_a_job_in_flight_leaves_the_new_view_to_converge() {
        let (ger, sov) = (from("GER"), from("SOV"));
        let mut chain = Chain::default();
        let mut state = State::default();
        state.present(true, &ger, (1, 10), false, None, true);
        let german = chain.plan(&mut state).unwrap();
        let german = Stamp {
            generation: german.epoch,
            country: ger.country.clone(),
        };
        // The switch: view and reset at once.
        state.present(false, &sov, (1, 10), false, None, true);
        assert_eq!(
            state.stamp().map(|stamp| stamp.country),
            Some(sov.country.clone()),
            "a plan after the reset is the new view's"
        );
        // The German hour finishes, complete, after the switch.
        settle(&mut chain, &state, &german, 10, Cycle::new(1, 3));
        assert!(!state.publish(german, 10, icons_for(3)));
        // The reviewed interleaving: a reset alone, a plan, then the view.
        state.restart(1);
        let planned = chain.plan(&mut state).unwrap();
        let stale = Stamp {
            generation: planned.epoch,
            country: sov.country.clone(),
        };
        state.view(false, &ger, (1, 10), false);
        settle(&mut chain, &state, &stale, 10, Cycle::new(1, 4));
        assert!(!state.publish(stale, 10, icons_for(4)));
        // The German view's paused hour is still to build, and converges.
        let plan = chain.plan(&mut state).expect("not marked complete");
        assert_eq!(
            (plan.hour, plan.from.is_none(), plan.paused),
            (10, true, true)
        );
        let stamp = state.stamp().unwrap();
        settle(&mut chain, &state, &stamp, 10, Cycle::new(1, 3));
        assert!(state.publish(stamp, 10, icons_for(3)));
        assert!(chain.plan(&mut state).is_none(), "complete");
        assert!(state.icons(1, 10, 3).is_some());
    }

    /// Planned ahead, the lane prunes a frame the window still holds built;
    /// a seek back to that hour hands the lane the frame with the reset, and
    /// the paused hour cycles until complete. An hour in flight from before
    /// the seek is dropped.
    #[test]
    fn a_seek_back_to_a_cached_built_hour_brings_its_frame() {
        let recorded = perspective::Choice::default();
        let mut chain = Chain::default();
        let mut state = state(8, true, 8..=10);
        state.perspective = Some(recorded.clone());
        for _ in 0..3 {
            let plan = chain.plan(&mut state).unwrap();
            built(&mut chain, &plan, false);
        }
        state.shown = Some(10);
        state.frames.insert((1, 11), None);
        let ahead = chain.plan(&mut state).unwrap();
        assert_eq!(ahead.hour, 11);
        assert!(!state.frames.contains_key(&(1, 8)), "pruned");
        let ahead = Stamp {
            generation: ahead.epoch,
            country: recorded.country.clone(),
        };
        // The window seeks back to 8 from its cache of built hours.
        state.present(false, &recorded, (1, 8), false, None, true);
        assert!(state.frames.contains_key(&(1, 8)));
        settle(&mut chain, &state, &ahead, 11, Cycle::pending(1, 0));
        assert!(!chain.after.contains_key(&11));
        let plan = chain.plan(&mut state).expect("the frame is there");
        assert_eq!(
            (plan.hour, plan.from.is_none(), plan.paused),
            (8, true, true)
        );
        built(&mut chain, &plan, true);
        assert!(chain.plan(&mut state).is_none(), "complete");
    }

    /// A result stamped with an earlier generation (a reset since) is
    /// refused when published, and one kept from it is not installed.
    #[test]
    fn a_stale_generation_result_is_refused() {
        let ger = from("GER");
        let mut state = State::default();
        state.view(true, &ger, (1, 10), false);
        let stale = state.stamp().unwrap();
        assert!(state.publish(stale.clone(), 10, icons_for(3)));
        state.restart(1);
        assert!(!state.publish(stale.clone(), 10, icons_for(3)));
        assert!(state.icons(1, 10, 3).is_none());
        // Even a stale result that were still held is not installed.
        state.built.insert(
            10,
            Published {
                stamp: stale,
                icons: Arc::new(icons_for(3)),
            },
        );
        assert!(state.icons(1, 10, 3).is_none());
    }

    /// A new viewing country or branch starts the cycle again.
    #[test]
    fn a_new_viewing_country_starts_the_cycle_again() {
        assert_eq!(carried(Some(Cycle::pending(1, 3)), 1, 3).ours(), 3);
        assert!(!carried(Some(Cycle::pending(1, 3)), 1, 3).complete());
        assert!(carried(Some(Cycle::pending(1, 3)), 1, 4).complete());
        assert!(carried(Some(Cycle::pending(1, 3)), 2, 3).complete());
        assert!(carried(None, 1, 3).complete());
    }
}
