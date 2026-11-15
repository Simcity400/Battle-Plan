//! The map's counter and province hovers, made off the window thread. Each hover family has a
//! reader of rows at the playback head, open while its hover is shown, and a worker of its own
//! that makes the text from the reader's newest rows. While those rows do not hold a table the
//! text reads, the hover shows what it showed and says it is loading, by name; nothing else is
//! read in their place. The window thread only says what is hovered and takes the newest text
//! made for it; it never runs a leaf.

pub(crate) mod proof;
#[cfg(test)]
mod tests;
mod today;

use crate::playhead_host::{
    Entry, MapSide,
    rows::{AtHead, Recording},
};
use replay_data::{Hour, rows::TableId};
use replay_frame::frame::{DecodedFrame, discovering};
use std::{
    sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError},
    time::{Duration, Instant},
};
use viewer_counter_products::map::{
    counter_content::Content,
    counter_tooltips::{self, Batch, Style},
    occupation_modifier_tooltips,
    province_tooltips::{self, ModeHover, naval_accidents, naval_mines},
};
use viewer_foundation::{
    map::{
        counter_hovers::{land::CounterHover, task_force, task_force_port},
        perspective::Vision,
    },
    tooltip_text::Tooltip,
    unit_handle::Unit,
};
use viewer_raids::map::raids::RaidAssets;

/// What prepares a frame's province temperatures, so a hover over it derives none as it is made.
pub(crate) trait Prepare: Send + Sync {
    fn prepare(&self, frame: &Arc<DecodedFrame>);
}

impl Prepare for province_tooltips::Content {
    fn prepare(&self, frame: &Arc<DecodedFrame>) {
        self.prepare_temperature(frame);
    }
}

/// The playhead's map's entry and the branch of the clock, which rows at its head carry.
pub(crate) type Head<'a> = (&'a Arc<Entry>, Recording);

/// How long the capture waits for a hover's rows and text before it names it loading.
const CAPTURE_LIMIT: Duration = Duration::from_secs(120);

/// How often the map's vision is made again for the hovers while it plays: the rows' cadence.
const VISION_CADENCE: Duration = Duration::from_millis(250);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Family {
    /// Land, task force, port and air base counters.
    Counters,
    /// The province in every map mode, and the resistance marker.
    Province,
    /// Regional naval mines and accidents.
    Naval,
    Occupation,
    /// The capture's inventory of every presented counter hover.
    Capture,
}

impl Family {
    pub(crate) const ALL: [Self; 5] = [Self::Counters, Self::Province, Self::Naval, Self::Occupation, Self::Capture];

    /// The family the paced harness points at (`BATTLE_PLAN_HOVER_FAMILY`), none when unset.
    pub(crate) fn from_env() -> std::io::Result<Option<Self>> {
        match std::env::var("BATTLE_PLAN_HOVER_FAMILY").or_else(|_| std::env::var("HOI4_HOVER_FAMILY")) {
            Ok(name) if !name.trim().is_empty() => Self::ALL.into_iter().find(|family| family.name() == name.trim()).map(Some).ok_or_else(|| std::io::Error::other(format!("BATTLE_PLAN_HOVER_FAMILY: no hover family {name}"))),
            _ => Ok(None),
        }
    }

    pub(crate) fn name(self) -> &'static str {
        match self {
            Self::Counters => "counters",
            Self::Province => "province",
            Self::Naval => "naval_region",
            Self::Occupation => "occupation",
            Self::Capture => "capture",
        }
    }
}

/// What is hovered.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum What {
    Land(CounterHover),
    TaskForce(task_force::Hover),
    Port(task_force_port::Hover),
    AirBase { province: u32, kind: i32 },
    Province { province: u32, mode: ModeHover },
    Mines(u32),
    Accidents(u32),
    Occupation { state: u32, token: String },
    Capture,
}

/// What is hovered and the identities of the sources its text is made from: a new key is a new
/// text, and the newest text made is shown only for its own key.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Key {
    pub what: What,
    pub sources: [usize; 4],
    pub style: (bool, bool),
}

impl Key {
    pub(crate) fn new(what: What, sources: [usize; 4], style: (bool, bool)) -> Self {
        Self { what, sources, style }
    }
}

/// The address an `Arc` source is known by in a key.
pub(crate) fn id<T: ?Sized>(source: &Arc<T>) -> usize {
    Arc::as_ptr(source).cast::<()>() as usize
}

/// Table names as a loading state names them.
pub(crate) fn named(tables: &[TableId]) -> String {
    tables.iter().map(|table| format!("{table:?}")).collect::<Vec<_>>().join(", ")
}

/// A reader of rows at the head that is loading: the tables a read found missing until they join
/// its rows, or the rows themselves while its reader opens.
#[derive(Debug, Default)]
pub(crate) struct Loading {
    pending: Vec<TableId>,
    state: Option<String>,
}

impl Loading {
    /// A read over its rows lacked `missed`, which are learned and loading until they join.
    pub(crate) fn learned(&mut self, missed: &[TableId]) {
        for table in missed {
            if !self.pending.contains(table) {
                self.pending.push(*table);
            }
        }
        if !self.pending.is_empty() {
            self.state = Some(named(&self.pending));
        }
    }

    /// Its reader holds no rows: the tables still joining, or the rows while it opens.
    pub(crate) fn no_rows(&mut self) {
        self.state = Some(if self.pending.is_empty() { "rows at the head".to_owned() } else { named(&self.pending) });
    }

    /// Its reader holds rows: every table it learned has joined them.
    pub(crate) fn joined(&mut self) {
        self.pending.clear();
        self.state = None;
    }

    pub(crate) fn state(&self) -> Option<&str> {
        self.state.as_deref()
    }

    /// What it is loading, named for the window's refusals: `name` says whose.
    pub(crate) fn reason(&self, name: &str) -> Option<String> {
        self.state.as_ref().map(|state| format!("{name}: loading: {state}"))
    }
}

/// Called by a leaf whose answer is still being read elsewhere (the air roster history): the
/// worker makes the text again once it is answered.
pub(crate) type Retry = Arc<dyn Fn() + Send + Sync>;
pub(crate) type Wake = Arc<dyn Fn() + Send + Sync>;
pub(crate) type Build = Box<dyn FnMut(&Arc<DecodedFrame>, &Retry) -> Arc<Tooltip> + Send>;

/// How a hover's text is made: the leaf over a frame, what prepares its rows first, and what
/// wakes the window when a text is done.
pub(crate) struct Job {
    build: Build,
    prepare: Option<Arc<dyn Prepare>>,
    wake: Wake,
}

impl Job {
    pub(crate) fn new(build: Build, wake: Wake) -> Self {
        Self { build, prepare: None, wake }
    }

    pub(crate) fn preparing(mut self, prepare: Arc<dyn Prepare>) -> Self {
        self.prepare = Some(prepare);
        self
    }
}

/// What a worker made for a key over a frame: its text, or none when the rows lacked a table.
#[derive(Clone)]
pub(crate) struct Made {
    pub key: Key,
    pub frame: usize,
    pub text: Option<Arc<Tooltip>>,
    /// The hour of the frame it was made over.
    pub hour: Hour,
    /// Tables the rows lacked, which the reader declares from now on.
    pub missed: Vec<TableId>,
}

/// The caches the leaves keep between texts, owned by the hovers and used only on their workers.
#[derive(Clone, Default)]
pub(crate) struct Caches {
    pub counters: Arc<Mutex<counter_tooltips::Cache>>,
    pub provinces: Arc<Mutex<province_tooltips::Cache>>,
}

#[derive(Default)]
struct Hover {
    rows: AtHead,
    worker: Option<Worker>,
    posted: Option<(Key, usize)>,
    /// The newest text made, always with a text.
    shown: Option<Made>,
    loading: Loading,
    asked: bool,
}

impl Hover {
    /// Takes in what the worker made since: a text is shown; tables its rows lacked are learned.
    fn take(&mut self) {
        if let Some(made) = self.worker.as_ref().and_then(Worker::take) {
            self.keep(made);
        }
    }

    fn keep(&mut self, made: Made) {
        if !made.missed.is_empty() {
            self.rows.learn(&made.missed);
            self.loading.learned(&made.missed);
        }
        if made.text.is_some() {
            self.shown = Some(made);
        }
    }

    fn shown_for(&self, key: &Key) -> Option<Arc<Tooltip>> {
        self.shown.as_ref().filter(|made| made.key == *key).and_then(|made| made.text.clone())
    }
}

/// The map's vision as the hovers and selections read it, made from the vision layer again when
/// what that layer shows has moved, at most [`VISION_CADENCE`] apart while the head plays. The frame
/// copies the layer's arrays; a thread of its own makes the vision (every province's byte, every
/// unit's level and sight) and the frame takes it when it has arrived, keeping the one before
/// meanwhile: no frame makes a vision.
#[derive(Default)]
pub(crate) struct ShownVision {
    held: Option<(VisionKey, Instant, Result<Arc<Vision>, String>)>,
    asked: bool,
    /// The copy being made, by its key and when it was copied; the maker's thread.
    making: Option<(VisionKey, Instant)>,
    maker: Option<(std::sync::mpsc::Sender<crate::layers::vision::SeenCopy>, std::sync::mpsc::Receiver<Arc<Vision>>)>,
}

/// What moves the vision layer's value: a step applied, its worker's product, its units still
/// settling, and the perspective.
#[derive(Clone, Debug, PartialEq)]
struct VisionKey {
    applied: Option<Hour>,
    published: crate::layers::Published,
    carried: usize,
    choice: viewer_foundation::map::perspective::Choice,
}

impl ShownVision {
    pub(crate) fn shown(&mut self, side: &MapSide) -> Result<Arc<Vision>, String> {
        use crate::layers::Layer as _;
        self.asked = true;
        self.take_made();
        let layer = side.parts().map(|(_, layers)| layers.iter().find_map(|layer| layer.any().downcast_ref::<crate::layers::vision::Vision>()));
        let layer = match layer {
            Some(Some(layer)) => layer,
            Some(None) => return Err("the map draws no vision layer".into()),
            None => return self.held.as_ref().map_or_else(|| Err("the vision layer is composing".into()), |(_, _, held)| held.clone()),
        };
        let key = VisionKey { applied: side.applied(), published: layer.published(), carried: layer.carried().entities, choice: layer.choice().clone() };
        let now = Instant::now();
        let due = match &self.held {
            None => true,
            Some((held, made, value)) => *held != key && (value.is_err() || side.entry().paused() || now.duration_since(*made) >= VISION_CADENCE),
        };
        if due && self.making.is_none() {
            match layer.shown_copy() {
                Err(why) => self.held = Some((key, now, Err(why))),
                Ok(copy) => self.make(key, now, copy),
            }
        }
        self.held.as_ref().map_or_else(|| Err("the vision is being made".into()), |(_, _, held)| held.clone())
    }

    /// `copy` made into the vision on the maker's thread, started on first use.
    fn make(&mut self, key: VisionKey, at: Instant, copy: crate::layers::vision::SeenCopy) {
        if self.maker.is_none() {
            let (jobs, receive) = std::sync::mpsc::channel::<crate::layers::vision::SeenCopy>();
            let (send, made) = std::sync::mpsc::channel();
            let started = std::thread::Builder::new().name("shown vision".into()).spawn(move || {
                viewer_foundation::worker_priority::background_priority();
                for copy in receive {
                    if send.send(Arc::new(copy.make())).is_err() {
                        return;
                    }
                }
            });
            if started.is_ok() {
                self.maker = Some((jobs, made));
            }
        }
        match &self.maker {
            Some((jobs, _)) if jobs.send(copy).is_ok() => self.making = Some((key, at)),
            _ => self.held = Some((key, at, Err("no thread to make the vision on".into()))),
        }
    }

    /// The vision made, once it has arrived.
    fn take_made(&mut self) {
        let Some((_, made)) = &self.maker else { return };
        while let Ok(vision) = made.try_recv() {
            if let Some((key, at)) = self.making.take() {
                self.held = Some((key, at, Ok(vision)));
            }
        }
    }
}

/// Every hover family's reader and worker.
#[derive(Default)]
pub(crate) struct HoverRows {
    hovers: [Hover; 5],
    caches: Caches,
    vision: ShownVision,
    refused: bool,
    /// Timed (the paced harness): this frame's thread CPU spent asking, and the hour of the text
    /// the last ask showed, and what it was loading.
    timed: Option<(Duration, Option<Hour>, Option<String>)>,
}

impl HoverRows {
    /// Readers that declare `seeded` tables from the start (the proof tool, with the tables an
    /// earlier run learned).
    pub(crate) fn seeded(seeded: &dyn Fn(Family) -> Vec<TableId>) -> Self {
        let mut hovers = Self::default();
        for family in Family::ALL {
            hovers.hovers[family as usize].rows = AtHead::new(&seeded(family), crate::playhead_host::rows::Cadence::PerSecond(4));
        }
        hovers
    }

    /// The newest text made for `key`, none before the first; a text is asked of the family's
    /// worker over rows at `head` when the key or the rows changed. Never runs a leaf, and never
    /// reads another frame: while the rows lack a table the text reads, the family is loading.
    pub(crate) fn ask(&mut self, family: Family, head: Head<'_>, key: Key, make: impl FnOnce(&Caches) -> Job) -> Option<Arc<Tooltip>> {
        self.timed_ask(family, |hovers| hovers.post_head(family, head, key, make).0)
    }

    /// `ask`, timed for the paced harness when it times.
    fn timed_ask(&mut self, family: Family, ask: impl FnOnce(&mut Self) -> Option<Arc<Tooltip>>) -> Option<Arc<Tooltip>> {
        let before = self.timed.is_some().then(viewer_foundation::profiling::current_thread_cpu_time).flatten();
        let text = ask(self);
        if let (Some((spent, shown, loading)), Some(before)) = (&mut self.timed, before) {
            *spent += viewer_foundation::profiling::current_thread_cpu_time().map_or(Duration::ZERO, |now| now.saturating_sub(before));
            let hover = &self.hovers[family as usize];
            *shown = text.as_ref().and(hover.shown.as_ref()).map(|made| made.hour);
            *loading = hover.loading.state().map(str::to_owned);
        }
        text
    }

    /// Times every ask from now on, for the paced harness.
    pub(crate) fn time(&mut self) {
        self.timed = Some((Duration::ZERO, None, None));
    }

    /// This frame's timed figures: the window thread's CPU spent asking (microseconds), the hour
    /// of the text shown and what the hover was loading; counted afresh from here.
    pub(crate) fn take_timed(&mut self) -> Option<serde_json::Value> {
        let (spent, shown, loading) = self.timed.as_mut()?;
        let figures = serde_json::json!({"cpu_us": spent.as_secs_f64() * 1e6, "shown_hour": shown, "loading": loading});
        *spent = Duration::ZERO;
        Some(figures)
    }

    /// What `family` is loading, none when it is not.
    #[cfg(test)]
    pub(crate) fn loading_of(&self, family: Family) -> Option<&str> {
        self.hovers[family as usize].loading.state()
    }

    /// Every open hover's loading state and the map's vision's, named for the window's refusals.
    pub(crate) fn loading(&self) -> Vec<(String, usize)> {
        let hovers = Family::ALL.into_iter().filter_map(|family| self.hovers[family as usize].loading.reason(&format!("the {} hover", family.name())));
        let vision = self.vision.held.as_ref().and_then(|(_, _, held)| held.as_ref().err()).map(|why| format!("the hovers' vision: loading: {why}"));
        hovers.chain(vision).map(|reason| (reason, 1)).collect()
    }

    /// The vision the playhead's map `side` shows, as the hovers read it; loading by name while
    /// its vision layer has published nothing.
    pub(crate) fn vision(&mut self, side: &MapSide) -> Result<Arc<Vision>, String> {
        self.vision.shown(side)
    }

    /// The next text `family`'s worker makes, whatever asked it (a leaf's late answer, a prepare
    /// let go); refused by name when none is made within `limit`, none when the rows lacked a
    /// table.
    #[cfg(test)]
    pub(crate) fn next(&mut self, family: Family, limit: Duration) -> Result<Option<Arc<Tooltip>>, String> {
        let hover = &mut self.hovers[family as usize];
        let worker = hover.worker.as_ref().ok_or_else(|| format!("the {} hover has no worker", family.name()))?;
        let made = worker.wait(|_| true, Some(Instant::now() + limit)).ok_or_else(|| format!("the {} hover's worker made nothing within {limit:?}", family.name()))?;
        let text = made.text.clone();
        hover.keep(made);
        Ok(text)
    }

    /// The text over rows at `head`, its rows and tables waited for within `limit`; refused by
    /// name with what it was loading when it is not made.
    #[cfg(test)]
    pub(crate) fn wait_within(&mut self, family: Family, head: Head<'_>, key: Key, make: impl Fn(&Caches) -> Job, limit: Duration) -> Result<Arc<Tooltip>, String> {
        self.wait_head(family, head, key, &make, Instant::now() + limit)
    }

    /// The newest rows at the head `family`'s reader holds that satisfy `ready`, waited for
    /// within `limit`.
    #[cfg(test)]
    pub(crate) fn rows_wait(&self, family: Family, ready: impl FnMut(&DecodedFrame) -> bool, limit: Duration) -> std::io::Result<Arc<DecodedFrame>> {
        self.hovers[family as usize].rows.wait(ready, limit)
    }

    /// As [`Self::ask`], waiting for the rows, the tables the text reads and the text: the
    /// capture, which is not at speed, takes the text of the frame it writes. None, loading by
    /// name, when they are not there within [`CAPTURE_LIMIT`].
    pub(crate) fn wait(&mut self, family: Family, head: Head<'_>, key: Key, make: impl Fn(&Caches) -> Job) -> Option<Arc<Tooltip>> {
        self.wait_head(family, head, key, &make, Instant::now() + CAPTURE_LIMIT).ok()
    }

    fn wait_head(&mut self, family: Family, head: Head<'_>, key: Key, make: &dyn Fn(&Caches) -> Job, until: Instant) -> Result<Arc<Tooltip>, String> {
        let late = |hover: &Hover| format!("the {} hover is still loading {:?} at its deadline", family.name(), hover.loading.state());
        loop {
            let (text, frame, posted) = self.post_head(family, head, key.clone(), make);
            let hover = &mut self.hovers[family as usize];
            let left = until.saturating_duration_since(Instant::now());
            let Some(frame) = frame else {
                if left.is_zero() || hover.rows.wait(|_| true, left).is_err() {
                    return Err(late(hover));
                }
                continue;
            };
            if let Some(text) = text.filter(|_| !posted) {
                return Ok(text);
            }
            let Some(worker) = hover.worker.as_ref() else { return Err(format!("the {} hover has no worker", family.name())) };
            let mut missed = Vec::new();
            let made = worker.wait(
                |made| {
                    missed.extend_from_slice(&made.missed);
                    made.key == key && made.frame == frame
                },
                Some(until),
            );
            if !missed.is_empty() {
                hover.rows.learn(&missed);
                hover.loading.learned(&missed);
            }
            let made = made.ok_or_else(|| late(hover))?;
            let text = made.text.clone();
            hover.keep(made);
            if let Some(text) = text {
                return Ok(text);
            }
        }
    }

    /// The text shown for `key` over rows at `head`, the frame asked over, and whether a text was
    /// asked now; no frame while the rows do not hold what the family reads.
    fn post_head(&mut self, family: Family, (entry, recording): Head<'_>, key: Key, make: impl FnOnce(&Caches) -> Job) -> (Option<Arc<Tooltip>>, Option<usize>, bool) {
        let hover = &mut self.hovers[family as usize];
        hover.asked = true;
        hover.take();
        let Some(rows) = hover.rows.frame(entry, recording) else {
            hover.loading.no_rows();
            return (hover.shown_for(&key), None, false);
        };
        hover.loading.joined();
        let (text, frame, posted) = self.post(family, rows, true, key, make);
        (text, Some(frame), posted)
    }

    /// The text shown for `key`, posting a text over `frame` when the key or the frame changed.
    fn post(&mut self, family: Family, frame: Arc<DecodedFrame>, at_head: bool, key: Key, make: impl FnOnce(&Caches) -> Job) -> (Option<Arc<Tooltip>>, usize, bool) {
        let hover = &mut self.hovers[family as usize];
        let at = id(&frame);
        let mut posted = false;
        if !hover.posted.as_ref().is_some_and(|(held, held_at)| *held == key && *held_at == at) {
            if hover.worker.is_none() && !self.refused {
                match Worker::spawn(family) {
                    Ok(worker) => hover.worker = Some(worker),
                    Err(error) => {
                        eprintln!("The {} hover's worker is refused, so it shows nothing: {error}", family.name());
                        self.refused = true;
                    }
                }
            }
            if let Some(worker) = &hover.worker {
                worker.post(Posted { key: key.clone(), frame: at, rows: frame, at_head, job: make(&self.caches) });
                hover.posted = Some((key.clone(), at));
                posted = true;
            }
        }
        (hover.shown_for(&key), at, posted)
    }

    /// The text shown for `family`, with the hour it was made at: the proof's freshness.
    pub(crate) fn shown(&self, family: Family) -> Option<&Made> {
        self.hovers[family as usize].shown.as_ref()
    }

    /// Closes every family not asked since the last call: its reader's tables are retired, its
    /// worker ends once its text in hand is done, and it holds no text. The capture's closes only
    /// once the capture ends, as it asks on the frames it writes alone.
    pub(crate) fn end_frame(&mut self, capturing: bool) {
        if !std::mem::take(&mut self.vision.asked) {
            self.vision.held = None;
        }
        for family in Family::ALL {
            if family == Family::Capture && capturing {
                continue;
            }
            let hover = &mut self.hovers[family as usize];
            if std::mem::take(&mut hover.asked) {
                hover.rows.end_frame();
                continue;
            }
            hover.rows.end_frame();
            hover.worker = None;
            hover.posted = None;
            hover.shown = None;
            hover.loading = Loading::default();
            if family == Family::Province {
                self.caches.provinces = Arc::default();
            }
            if family == Family::Counters {
                self.caches.counters = Arc::default();
            }
        }
    }

    #[cfg(test)]
    pub(crate) fn is_open(&self, family: Family) -> bool {
        let hover = &self.hovers[family as usize];
        hover.rows.is_open() || hover.worker.is_some() || hover.shown.is_some()
    }

    /// Joins every worker thread `family` has started, which must all be closed: how many ended.
    #[cfg(test)]
    pub(crate) fn join(family: Family) -> usize {
        let handles: Vec<std::thread::JoinHandle<()>> = {
            let mut threads = lock(&THREADS);
            let (of, rest) = std::mem::take(&mut *threads).into_iter().partition(|(of, _)| *of == family);
            *threads = rest;
            of.into_iter().map(|(_, handle)| handle).collect::<Vec<_>>()
        };
        handles.into_iter().map(|handle| handle.join().expect("a hover worker ends")).count()
    }
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

#[cfg(test)]
static THREADS: Mutex<Vec<(Family, std::thread::JoinHandle<()>)>> = Mutex::new(Vec::new());

/// A text to make over a frame: rows at the head, or today's frame (and the proof's reference).
pub(crate) struct Posted {
    pub key: Key,
    pub frame: usize,
    pub rows: Arc<DecodedFrame>,
    pub at_head: bool,
    pub job: Job,
}

#[derive(Default)]
struct Slot {
    wanted: Option<Posted>,
    /// The last text made, kept to be made again when a leaf's answer arrives.
    last: Option<Posted>,
    retry: bool,
    made: Option<Made>,
    /// Texts posted over and never begun: their frames are let go on the worker, never on the
    /// window thread, where the last hold of a table's rows would free them.
    discard: Vec<Posted>,
    stop: bool,
}

#[derive(Default)]
struct Shared {
    slot: Mutex<Slot>,
    wake: Condvar,
    done: Condvar,
}

/// One family's worker thread: makes the newest text posted, never queued behind an older one,
/// and ends when the worker is dropped (after the text in hand).
struct Worker {
    shared: Arc<Shared>,
}

impl Worker {
    fn spawn(family: Family) -> std::io::Result<Self> {
        let shared = Arc::new(Shared::default());
        let working = Arc::clone(&shared);
        let handle = std::thread::Builder::new().name(format!("{} hover", family.name())).spawn(move || {
            viewer_foundation::worker_priority::background_priority();
            run(&working);
        })?;
        #[cfg(test)]
        lock(&THREADS).push((family, handle));
        #[cfg(not(test))]
        drop(handle);
        Ok(Self { shared })
    }

    fn post(&self, posted: Posted) {
        let mut slot = lock(&self.shared.slot);
        if let Some(older) = slot.wanted.replace(posted) {
            slot.discard.push(older);
        }
        drop(slot);
        self.shared.wake.notify_one();
    }

    fn take(&self) -> Option<Made> {
        lock(&self.shared.slot).made.take()
    }

    /// The first text made that satisfies `until`; none once `deadline` passes.
    fn wait(&self, mut until: impl FnMut(&Made) -> bool, deadline: Option<Instant>) -> Option<Made> {
        let mut slot = lock(&self.shared.slot);
        loop {
            if let Some(made) = slot.made.take()
                && until(&made)
            {
                return Some(made);
            }
            slot = match deadline {
                None => self.shared.done.wait(slot).unwrap_or_else(PoisonError::into_inner),
                Some(deadline) => {
                    let left = deadline.checked_duration_since(Instant::now()).filter(|left| !left.is_zero())?;
                    self.shared.done.wait_timeout(slot, left).unwrap_or_else(PoisonError::into_inner).0
                }
            };
        }
    }
}

impl Drop for Worker {
    /// The worker ends and lets go of what it holds on its own thread.
    fn drop(&mut self) {
        lock(&self.shared.slot).stop = true;
        self.shared.wake.notify_one();
    }
}

fn run(shared: &Arc<Shared>) {
    loop {
        let mut posted = {
            let mut slot = lock(&shared.slot);
            loop {
                if !slot.discard.is_empty() {
                    let discard = std::mem::take(&mut slot.discard);
                    drop(slot);
                    drop(discard);
                    slot = lock(&shared.slot);
                    continue;
                }
                if slot.stop {
                    return;
                }
                if let Some(posted) = slot.wanted.take() {
                    slot.retry = false;
                    break posted;
                }
                if std::mem::take(&mut slot.retry)
                    && let Some(posted) = slot.last.take()
                {
                    break posted;
                }
                slot = shared.wake.wait(slot).unwrap_or_else(PoisonError::into_inner);
            }
        };
        let retry: Retry = {
            let (shared, wake) = (Arc::downgrade(shared), Arc::clone(&posted.job.wake));
            Arc::new(move || {
                if let Some(shared) = shared.upgrade() {
                    lock(&shared.slot).retry = true;
                    shared.wake.notify_one();
                }
                wake();
            })
        };
        let made = match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| make(&mut posted, &retry))) {
            Ok(made) => made,
            Err(_) => {
                let mut text = Tooltip::default();
                text.refused.insert("hover text: its worker panicked making it".into(), 1);
                Made { key: posted.key.clone(), frame: posted.frame, text: Some(Arc::new(text)), hour: posted.rows.hour, missed: Vec::new() }
            }
        };
        let mut slot = lock(&shared.slot);
        if slot.stop {
            return;
        }
        slot.made = Some(made);
        if slot.wanted.is_none() {
            let wake = Arc::clone(&posted.job.wake);
            slot.last = Some(posted);
            drop(slot);
            shared.done.notify_all();
            wake();
        } else {
            drop(slot);
            shared.done.notify_all();
        }
    }
}

/// The text of `posted`. Over rows at the head: prepared first, made while learning the tables
/// they lack, and no text when they lack one. Over any other frame (today's, the proof's
/// reference): made as it is.
pub(crate) fn make(posted: &mut Posted, retry: &Retry) -> Made {
    let Posted { key, frame, rows, at_head, job } = posted;
    let (key, frame, hour, at_head) = (key.clone(), *frame, rows.hour, *at_head);
    if !at_head {
        return Made { key, frame, text: Some((job.build)(rows, retry)), hour, missed: Vec::new() };
    }
    if let Some(prepare) = &job.prepare
        && !rows.tables[TableId::WeatherProvinces.index()].is_not_declared()
    {
        prepare.prepare(rows);
    }
    let (text, missed) = discovering(|| (job.build)(rows, retry));
    Made { key, frame, text: missed.is_empty().then_some(text), hour, missed }
}

impl super::Viewer {
    /// The playhead's map while it is open: made, and not stopped at an error.
    pub(super) fn open_map(&self) -> Option<&MapSide> {
        self.playhead_map.as_ref().filter(|side| side.on_screen().is_some())
    }

    /// Rows at the head of the open map with the clock's branch they carry; none with no map open
    /// or no clock.
    pub(super) fn rows_head(&self) -> Option<(Arc<Entry>, Recording)> {
        Some((Arc::clone(self.open_map()?.entry()), Recording { branch: self.playhead.as_ref()?.branch }))
    }

    /// The vision the open map's vision layer shows, as the hovers read it; why there is none,
    /// by name.
    pub(super) fn shown_vision(&mut self) -> Result<Arc<Vision>, String> {
        let side = self.playhead_map.as_ref().filter(|side| side.on_screen().is_some()).ok_or("the playhead's map is not open")?;
        self.hover_rows.vision(side)
    }

    /// A point the paced harness holds the pointer at to keep `family`'s hover open: the first
    /// target the overlay presents of it, or for the province a point that hits no icon, searched
    /// outward from the middle of `pixels`; none until one is presented.
    pub(super) fn hover_target(&self, family: Family, pixels: [u32; 2]) -> Option<[f32; 2]> {
        let middle = |rect: [f32; 4]| [rect[0] + rect[2] * 0.5, rect[1] + rect[3] * 0.5];
        match family {
            Family::Province => {
                let [width, height] = pixels.map(|size| size as f32);
                (0..12).flat_map(|ring| (0..8).map(move |at| (ring, at))).map(|(ring, at)| {
                    let (angle, reach) = (at as f32 * std::f32::consts::FRAC_PI_4, ring as f32 * 0.03);
                    [width * (0.5 + reach * angle.cos()), height * (0.5 + reach * angle.sin())]
                }).find(|&point| self.map_hit(point).is_none())
            }
            Family::Counters => self.counter_report.land_hovers.first().map(|hover| middle(hover.rect)),
            Family::Naval => self.counter_report.naval_region_hovers.iter().find(|hover| matches!(hover.control, viewer_counters::map::counters::NavalRegionalControl::Mines | viewer_counters::map::counters::NavalRegionalControl::Accidents)).map(|hover| middle(hover.rect)),
            Family::Occupation => self.prepared_overlay.as_ref()?.occupation_hits().iter().find(|hit| matches!(hit.hover, viewer_world::map::occupation_icons::Hover::Modifier { .. })).and_then(|hit| hit.bounds()).map(middle),
            Family::Capture => None,
        }
    }

    /// What every hover's leaf reads at this frame, owned, with a wake that repaints the window.
    pub(super) fn hover_inputs(&self, document: &crate::document::Document, vision: &Arc<Vision>, ctx: &eframe::egui::Context) -> Inputs {
        let ctx = ctx.clone();
        Inputs { content: Arc::clone(&document.counter_content), raids: Arc::clone(&document.icons.raids), vision: Arc::clone(vision), style: self.hover_style(), wake: Arc::new(move || ctx.request_repaint()) }
    }
}

/// The owned inputs every hover's leaf reads.
#[derive(Clone)]
pub(crate) struct Inputs {
    pub content: Arc<Content>,
    pub raids: Arc<RaidAssets>,
    pub vision: Arc<Vision>,
    pub style: Style,
    pub wake: Wake,
}

impl Inputs {
    pub(crate) fn key(&self, what: What, more: [usize; 2]) -> Key {
        Key::new(what, [id(&self.content), id(&self.vision), more[0], more[1]], (self.style.nato, self.style.diagnostics))
    }

    fn job(&self, build: Build) -> Job {
        Job::new(build, Arc::clone(&self.wake))
    }

    pub(crate) fn land(&self, caches: &Caches, hover: CounterHover) -> Job {
        let (inputs, cache) = (self.clone(), Arc::clone(&caches.counters));
        self.job(Box::new(move |frame, _| lock(&cache).request(&inputs.content, inputs.raids.movement_reads(), frame, &hover, &inputs.vision, inputs.style)))
    }

    pub(crate) fn task_force(&self, caches: &Caches, hover: task_force::Hover) -> Job {
        let (inputs, cache) = (self.clone(), Arc::clone(&caches.counters));
        self.job(Box::new(move |frame, _| lock(&cache).request_task_force(&inputs.content, inputs.raids.movement_reads(), frame, &hover, &inputs.vision, inputs.style)))
    }

    pub(crate) fn port(&self, caches: &Caches, hover: task_force_port::Hover) -> Job {
        let (inputs, cache) = (self.clone(), Arc::clone(&caches.counters));
        self.job(Box::new(move |frame, _| lock(&cache).request_task_force_port(&inputs.content, inputs.raids.movement_reads(), frame, &hover, &inputs.vision, inputs.style)))
    }

    pub(crate) fn air_base(&self, province: u32, kind: i32, wings: Arc<Vec<u64>>) -> Job {
        let inputs = self.clone();
        self.job(Box::new(move |frame, _| Arc::new(Batch::new(&inputs.content, inputs.raids.movement_reads(), frame, &inputs.vision, inputs.style).build_air_base(province, kind, &wings))))
    }

    pub(crate) fn naval(&self, region: u32, mines: bool) -> Job {
        let inputs = self.clone();
        self.job(Box::new(move |frame, _| Arc::new(if mines { naval_mines::build(&inputs.content, frame, &inputs.vision, region, false) } else { naval_accidents::build(&inputs.content, frame, &inputs.vision, region) })))
    }

    pub(crate) fn occupation(&self, state: u32, definition: installed_content_core::occupation_modifiers::Definition) -> Job {
        let inputs = self.clone();
        self.job(Box::new(move |frame, _| Arc::new(occupation_modifier_tooltips::build(&inputs.content, frame, state, &definition))))
    }

    /// The province hover in `mode`; the roster history's answers make it again when they come.
    pub(crate) fn province(&self, caches: &Caches, province: u32, mode: ModeHover, selected: Arc<Vec<Unit>>, rosters: Arc<province_tooltips::air_region::Sampler>) -> Job {
        let (inputs, cache) = (self.clone(), Arc::clone(&caches.provinces));
        let build: Build = Box::new(move |frame, retry| {
            let retry = Arc::clone(retry);
            lock(&cache).request(&inputs.content, inputs.raids.movement_reads(), frame, province, &inputs.vision, inputs.style.nato, &selected, &rosters, &mode, move || retry())
        });
        self.job(build).preparing(Arc::clone(&self.content.province_hover) as Arc<dyn Prepare>)
    }

    /// Every presented counter hover's refusals, as the capture inventories them.
    pub(crate) fn capture(&self, land: Vec<CounterHover>, task_forces: Vec<task_force::Hover>, ports: Vec<task_force_port::Hover>) -> Job {
        let inputs = self.clone();
        self.job(Box::new(move |frame, _| {
            let batch = Batch::new(&inputs.content, inputs.raids.movement_reads(), frame, &inputs.vision, inputs.style);
            let tooltips = land.iter().map(|hover| batch.build(hover)).chain(task_forces.iter().map(|hover| batch.build_task_force(hover))).chain(ports.iter().map(|hover| batch.build_task_force_port(hover)));
            Arc::new(Tooltip { refused: counter_tooltips::count_refusals(tooltips), ..Tooltip::default() })
        }))
    }
}
