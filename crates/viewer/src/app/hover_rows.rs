
pub(crate) mod proof;
#[cfg(test)]
mod tests;

use crate::playhead_host::views::{AtHead, Pace, Views};
use replay_data::{BranchId, Hour, rows::TableId};
use replay_frame::frame::{DecodedFrame, discovering};
use std::{
    sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError},
    time::{Duration, Instant},
};
use viewer_counter_products::map::{
    counter_content::Content,
    counter_tooltips::{self, Batch, Style},
    faction_theatre_tooltips, occupation_modifier_tooltips,
    province_tooltips::{self, ModeHover, naval_accidents, naval_base, naval_mines},
};
use viewer_native_gui::counter_hovers::{land::CounterHover, task_force, task_force_port};
use viewer_foundation::{perspective::Vision, tooltip_text::Tooltip};
use viewer_runtime::unit_handle::Unit;
use viewer_raids::map::raids::RaidAssets;

pub(crate) trait Prepare: Send + Sync {
    fn prepare(&self, frame: &Arc<DecodedFrame>);
}

impl Prepare for province_tooltips::Content {
    fn prepare(&self, frame: &Arc<DecodedFrame>) {
        self.prepare_temperature(frame);
    }
}

pub(crate) type Head<'a> = &'a Views;

const CAPTURE_LIMIT: Duration = Duration::from_secs(120);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Family {
    Counters,
    Province,
    Naval,
    Occupation,
    Capture,
}

impl Family {
    pub(crate) const ALL: [Self; 5] = [Self::Counters, Self::Province, Self::Naval, Self::Occupation, Self::Capture];

    pub(crate) fn from_env() -> std::io::Result<Option<Self>> {
        match std::env::var("BATTLEPLAN_HOVER_FAMILY").or_else(|_| std::env::var("HOI4_HOVER_FAMILY")) {
            Ok(name) if !name.trim().is_empty() => Self::ALL.into_iter().find(|family| family.name() == name.trim()).map(Some).ok_or_else(|| std::io::Error::other(format!("BATTLEPLAN_HOVER_FAMILY: no hover family {name}"))),
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

#[derive(Clone, Debug, PartialEq)]
pub(crate) enum What {
    Land(CounterHover),
    TaskForce(task_force::Hover),
    Port(task_force_port::Hover),
    AirBase { province: u32, kind: i32 },
    NavalBase { province: u32, fleet_selected: bool },
    FactionTheatre { faction: u64, ordinal: u32 },
    Location(u32),
    Province { province: u32, mode: ModeHover, routes: Vec<viewer_gpu::ribbon_content::TradeRouteLine> },
    Mines(u32),
    Accidents(u32),
    Occupation { state: u32, token: String },
    Capture,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Key {
    pub what: What,
    pub sources: [usize; 4],
    pub style: (bool, bool),
    pub seen: (u32, bool),
}

const VISION: usize = 1;

impl Key {
    pub(crate) fn new(what: What, sources: [usize; 4], style: (bool, bool)) -> Self {
        Self { what, sources, style, seen: (0, false) }
    }

    fn shows_for(&self, other: &Key) -> bool {
        self.what == other.what
            && self.style == other.style
            && self.seen == other.seen
            && (0..self.sources.len()).all(|at| at == VISION || self.sources[at] == other.sources[at])
    }
}

pub(crate) fn id<T: ?Sized>(source: &Arc<T>) -> usize {
    Arc::as_ptr(source).cast::<()>() as usize
}

pub(crate) fn named(tables: &[TableId]) -> String {
    tables.iter().map(|table| format!("{table:?}")).collect::<Vec<_>>().join(", ")
}

#[derive(Debug, Default)]
pub(crate) struct Loading {
    pending: Vec<TableId>,
    state: Option<String>,
}

impl Loading {
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

    pub(crate) fn no_rows(&mut self) {
        self.state = Some(if self.pending.is_empty() { "rows at the head".to_owned() } else { named(&self.pending) });
    }

    pub(crate) fn joined(&mut self) {
        self.pending.clear();
        self.state = None;
    }

    pub(crate) fn state(&self) -> Option<&str> {
        self.state.as_deref()
    }

    pub(crate) fn reason(&self, name: &str) -> Option<String> {
        self.state.as_ref().map(|state| format!("{name}: loading: {state}"))
    }
}

pub(crate) type Retry = Arc<dyn Fn() + Send + Sync>;
pub(crate) type Wake = Arc<dyn Fn() + Send + Sync>;
pub(crate) type Build = Box<dyn FnMut(&Arc<DecodedFrame>, &Retry) -> Arc<Tooltip> + Send>;

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

#[derive(Clone)]
pub(crate) struct Made {
    pub key: Key,
    pub frame: usize,
    pub text: Option<Arc<Tooltip>>,
    pub branch: BranchId,
    pub hour: Hour,
    pub missed: Vec<TableId>,
}

#[derive(Clone, Default)]
pub(crate) struct Caches {
    pub counters: Arc<Mutex<counter_tooltips::Cache>>,
    pub provinces: Arc<Mutex<province_tooltips::Cache>>,
}

const TEXTS_KEPT: usize = 8;

const AHEAD: i32 = 96;

pub(crate) fn next_hour(presented: Hour, ahead: &[Hour], made: impl Fn(Hour) -> bool) -> Option<Hour> {
    ahead
        .iter()
        .copied()
        .filter(|&hour| hour > presented)
        .find(|&hour| !made(hour))
        .or_else(|| (!made(presented)).then_some(presented))
}

struct Hover {
    rows: AtHead,
    worker: Option<Worker>,
    posted: Option<(Key, usize)>,
    texts: std::collections::VecDeque<Made>,
    loading: Loading,
    pending: Option<Hour>,
    asked: bool,
}

impl Hover {
    fn new(family: Family) -> Self {
        let rows = AtHead::with(family.name(), &[], Pace::Step);
        let rows = if family == Family::Capture { rows } else { rows.keeps_its_answer() };
        Self { rows, worker: None, posted: None, texts: Default::default(), loading: Loading::default(), pending: None, asked: false }
    }

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
            if self.texts.len() == TEXTS_KEPT {
                self.texts.pop_front();
            }
            self.texts.push_back(made);
        }
    }

    fn newest(&self) -> Option<&Made> {
        self.texts.back()
    }

    fn made_at(&self, key: &Key, branch: BranchId, hour: Hour) -> bool {
        self.texts.iter().any(|made| (made.branch, made.hour) == (branch, hour) && made.key.shows_for(key))
    }

    fn ahead(&mut self, views: Head<'_>, key: &Key, (branch, presented): (BranchId, Hour)) -> Option<Arc<DecodedFrame>> {
        let newest = self.rows.frame_through(views, presented.saturating_add(AHEAD))?;
        let mut frames = Vec::new();
        for hour in presented + 1..=newest.hour.min(presented.saturating_add(AHEAD)) {
            if let Some(frame) = self.rows.frame_through(views, hour)
                && frame.hour == hour
                && frame.branch == branch
            {
                frames.push(frame);
            }
        }
        let hours: Vec<Hour> = frames.iter().map(|frame| frame.hour).collect();
        match next_hour(presented, &hours, |hour| self.made_at(key, branch, hour))? {
            hour if hour == presented => self.rows.frame_through(views, presented).filter(|frame| frame.hour == presented && frame.branch == branch),
            hour => frames.into_iter().find(|frame| frame.hour == hour),
        }
    }

    fn shown_for(&self, key: &Key, at: Option<(BranchId, Hour)>) -> Option<Arc<Tooltip>> {
        let made = match at {
            Some(at) => self.texts.iter().rev().find(|made| (made.branch, made.hour) == at && made.key.shows_for(key)),
            None => self.newest().filter(|made| made.key == *key),
        };
        made.and_then(|made| made.text.clone())
    }
}

pub(crate) struct HoverRows {
    hovers: [Hover; 5],
    caches: Caches,
    refused: bool,
    presented: Option<(BranchId, Hour)>,
    timed: Option<(Duration, Option<Hour>, Option<String>)>,
    pub(crate) playing: bool,
}

impl Default for HoverRows {
    fn default() -> Self {
        Self { hovers: Family::ALL.map(Hover::new), caches: Caches::default(), refused: false, presented: None, timed: None, playing: false }
    }
}

impl HoverRows {
    pub(crate) fn seeded(seeded: &dyn Fn(Family) -> Vec<TableId>) -> Self {
        let mut hovers = Self::default();
        for family in Family::ALL {
            hovers.hovers[family as usize].rows = AtHead::new(family.name(), &seeded(family), Pace::PerSecond(4));
        }
        hovers
    }

    pub(crate) fn present(&mut self, at: Option<(BranchId, Hour)>) {
        self.presented = at;
    }

    pub(crate) fn ask(&mut self, family: Family, head: Head<'_>, key: Key, make: impl FnOnce(&Caches) -> Job) -> Option<Arc<Tooltip>> {
        let at = self.presented;
        self.timed_ask(family, |hovers| {
            let (text, frame, _) = hovers.post_head(family, head, key, at, make);
            let hover = &mut hovers.hovers[family as usize];
            hover.pending = at.filter(|_| text.is_none() && frame.is_some() && hover.loading.state().is_none()).map(|(_, hour)| hour);
            text
        })
    }

    fn timed_ask(&mut self, family: Family, ask: impl FnOnce(&mut Self) -> Option<Arc<Tooltip>>) -> Option<Arc<Tooltip>> {
        let before = self.timed.is_some().then(viewer_runtime::profiling::current_thread_cpu_time).flatten();
        let text = ask(self);
        if let (Some((spent, shown, loading)), Some(before)) = (&mut self.timed, before) {
            *spent += viewer_runtime::profiling::current_thread_cpu_time().map_or(Duration::ZERO, |now| now.saturating_sub(before));
            let hover = &self.hovers[family as usize];
            *shown = text.as_ref().and(hover.newest()).map(|made| made.hour);
            *loading = hover.loading.state().map(str::to_owned);
        }
        text
    }

    pub(crate) fn time(&mut self) {
        self.timed = Some((Duration::ZERO, None, None));
    }

    pub(crate) fn take_timed(&mut self) -> Option<serde_json::Value> {
        let (spent, shown, loading) = self.timed.as_mut()?;
        let figures = serde_json::json!({"cpu_us": spent.as_secs_f64() * 1e6, "shown_hour": shown, "loading": loading});
        *spent = Duration::ZERO;
        Some(figures)
    }

    #[cfg(test)]
    pub(crate) fn loading_of(&self, family: Family) -> Option<&str> {
        self.hovers[family as usize].loading.state()
    }

    pub(crate) fn loading(&self) -> Vec<(String, usize)> {
        Family::ALL
            .into_iter()
            .filter_map(|family| {
                let hover = &self.hovers[family as usize];
                hover.loading.reason(&format!("the {} hover", family.name())).or_else(|| hover.pending.map(|hour| format!("the {} hover: loading: its text at hour {hour}", family.name())))
            })
            .map(|reason| (reason, 1))
            .collect()
    }

    #[cfg(test)]
    pub(crate) fn next(&mut self, family: Family, limit: Duration) -> Result<Option<Arc<Tooltip>>, String> {
        let hover = &mut self.hovers[family as usize];
        let worker = hover.worker.as_ref().ok_or_else(|| format!("the {} hover has no worker", family.name()))?;
        let made = worker.wait(|_| true, Some(Instant::now() + limit)).ok_or_else(|| format!("the {} hover's worker made nothing within {limit:?}", family.name()))?;
        let text = made.text.clone();
        hover.keep(made);
        Ok(text)
    }

    #[cfg(test)]
    pub(crate) fn wait_within(&mut self, family: Family, head: Head<'_>, key: Key, make: impl Fn(&Caches) -> Job, limit: Duration) -> Result<Arc<Tooltip>, String> {
        self.wait_head(family, head, key, &make, Instant::now() + limit)
    }

    #[cfg(test)]
    pub(crate) fn rows_wait(&self, family: Family, ready: impl FnMut(&DecodedFrame) -> bool, limit: Duration) -> std::io::Result<Arc<DecodedFrame>> {
        self.hovers[family as usize].rows.wait(ready, limit)
    }

    pub(crate) fn wait(&mut self, family: Family, head: Head<'_>, key: Key, make: impl Fn(&Caches) -> Job) -> Option<Arc<Tooltip>> {
        self.wait_head(family, head, key, &make, Instant::now() + CAPTURE_LIMIT).ok()
    }

    fn wait_head(&mut self, family: Family, head: Head<'_>, key: Key, make: &dyn Fn(&Caches) -> Job, until: Instant) -> Result<Arc<Tooltip>, String> {
        let late = |hover: &Hover| format!("the {} hover is still loading {:?} at its deadline", family.name(), hover.loading.state());
        loop {
            let (text, frame, posted) = self.post_head(family, head, key.clone(), None, make);
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

    fn post_head(&mut self, family: Family, views: Head<'_>, key: Key, presented: Option<(BranchId, Hour)>, make: impl FnOnce(&Caches) -> Job) -> (Option<Arc<Tooltip>>, Option<usize>, bool) {
        let playing = self.playing;
        let hover = &mut self.hovers[family as usize];
        hover.asked = true;
        hover.take();
        let rows = match (playing, presented) {
            (true, Some(at)) => {
                if hover.rows.frame(views).is_none() {
                    None
                } else {
                    hover.loading.joined();
                    let Some(rows) = hover.ahead(views, &key, at) else {
                        return (hover.shown_for(&key, presented), Some(0), false);
                    };
                    Some(rows)
                }
            }
            _ => hover.rows.frame(views),
        };
        let Some(rows) = rows else {
            hover.loading.no_rows();
            return (hover.shown_for(&key, presented), None, false);
        };
        hover.loading.joined();
        let (text, frame, posted) = self.post(family, rows, true, key, presented, make);
        (text, Some(frame), posted)
    }

    fn post(&mut self, family: Family, frame: Arc<DecodedFrame>, at_head: bool, key: Key, presented: Option<(BranchId, Hour)>, make: impl FnOnce(&Caches) -> Job) -> (Option<Arc<Tooltip>>, usize, bool) {
        let hover = &mut self.hovers[family as usize];
        let at = id(&frame);
        let mut posted = false;
        if !hover.posted.as_ref().is_some_and(|(held, held_at)| held.shows_for(&key) && *held_at == at) {
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
        (hover.shown_for(&key, presented), at, posted)
    }

    pub(crate) fn shown(&self, family: Family) -> Option<&Made> {
        self.hovers[family as usize].newest()
    }

    pub(crate) fn end_frame(&mut self, capturing: bool) {
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
            hover.texts.clear();
            hover.loading = Loading::default();
            hover.pending = None;
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
        hover.rows.is_open() || hover.worker.is_some() || !hover.texts.is_empty()
    }

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
    last: Option<Posted>,
    retry: bool,
    made: Option<Made>,
    discard: Vec<Posted>,
    stop: bool,
}

#[derive(Default)]
struct Shared {
    slot: Mutex<Slot>,
    wake: Condvar,
    done: Condvar,
}

struct Worker {
    shared: Arc<Shared>,
}

impl Worker {
    fn spawn(family: Family) -> std::io::Result<Self> {
        let shared = Arc::new(Shared::default());
        let working = Arc::clone(&shared);
        let handle = std::thread::Builder::new().name(format!("{} hover", family.name())).spawn(move || {
            viewer_runtime::worker_priority::background_priority();
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
                Made { key: posted.key.clone(), frame: posted.frame, text: Some(Arc::new(text)), branch: posted.rows.branch, hour: posted.rows.hour, missed: Vec::new() }
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

pub(crate) fn make(posted: &mut Posted, retry: &Retry) -> Made {
    let Posted { key, frame, rows, at_head, job } = posted;
    let (key, frame, branch, hour, at_head) = (key.clone(), *frame, rows.branch, rows.hour, *at_head);
    if !at_head {
        return Made { key, frame, text: Some((job.build)(rows, retry)), branch, hour, missed: Vec::new() };
    }
    if let Some(prepare) = &job.prepare
        && !rows.tables[TableId::WeatherProvinces.index()].is_not_declared()
    {
        prepare.prepare(rows);
    }
    let (text, missed) = discovering(|| (job.build)(rows, retry));
    Made { key, frame, text: missed.is_empty().then_some(text), branch, hour, missed }
}

impl super::Viewer {
    pub(super) fn open_map(&self) -> Option<&viewer_playback::side::SideThread> {
        self.playhead_map.as_ref().filter(|side| side.on_screen().is_some())
    }

    pub(super) fn rows_head(&self) -> Option<Views> {
        Some(self.open_map()?.views().clone())
    }

    pub(super) fn shown_vision(&self) -> Result<Arc<Vision>, String> {
        match self.playhead_map.as_ref().filter(|side| side.on_screen().is_some()) {
            Some(side) => side.products().vision.as_ref().map(|vision| Arc::clone(&**vision)).map_err(Clone::clone),
            None => Err("the playhead's map is not open".into()),
        }
    }

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
            Family::Naval => self.counter_report.naval_region_hovers.iter().find(|hover| matches!(hover.control, viewer_counter_kit::NavalRegionalControl::Mines | viewer_counter_kit::NavalRegionalControl::Accidents)).map(|hover| middle(hover.rect)),
            Family::Occupation => self.overlay_hits.occupation.iter().find(|hit| matches!(hit.hover, viewer_map_icons::occupation_icons::Hover::Modifier { .. })).and_then(|hit| hit.bounds()).map(middle),
            Family::Capture => None,
        }
    }

    pub(super) fn hover_inputs(&self, document: &crate::document::Document, vision: &Arc<Vision>, ctx: &eframe::egui::Context) -> Inputs {
        let ctx = ctx.clone();
        Inputs { content: Arc::clone(&document.counter_content), raids: Arc::clone(&document.icons.raids), vision: Arc::clone(vision), style: self.hover_style(), wake: Arc::new(move || ctx.request_repaint()) }
    }
}

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
        Key { seen: (self.vision.ours(), self.vision.fog()), ..Key::new(what, [id(&self.content), id(&self.vision), more[0], more[1]], (self.style.nato, self.style.diagnostics)) }
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

    pub(crate) fn naval_base(&self, province: u32, fleet_selected: bool) -> Job {
        let inputs = self.clone();
        self.job(Box::new(move |frame, _| Arc::new(naval_base::build(&inputs.content, frame, &inputs.vision, province, fleet_selected))))
    }

    pub(crate) fn faction_theatre(&self, faction: u64, ordinal: u32) -> Job {
        let inputs = self.clone();
        self.job(Box::new(move |frame, _| Arc::new(faction_theatre_tooltips::build(&inputs.content, frame, faction, ordinal))))
    }

    pub(crate) fn location(&self, province: u32) -> Job {
        let inputs = self.clone();
        self.job(Box::new(move |frame, _| Arc::new(Batch::new(&inputs.content, inputs.raids.movement_reads(), frame, &inputs.vision, inputs.style).build_location(province))))
    }

    pub(crate) fn naval(&self, region: u32, mines: bool) -> Job {
        let inputs = self.clone();
        self.job(Box::new(move |frame, _| Arc::new(if mines { naval_mines::build(&inputs.content, frame, &inputs.vision, region, false) } else { naval_accidents::build(&inputs.content, frame, &inputs.vision, region) })))
    }

    pub(crate) fn occupation(&self, state: u32, definition: content_reader_politics::occupation_modifiers::Definition) -> Job {
        let inputs = self.clone();
        self.job(Box::new(move |frame, _| Arc::new(occupation_modifier_tooltips::build(&inputs.content, frame, state, &definition, inputs.vision.ours()))))
    }

    pub(crate) fn province(&self, caches: &Caches, province: u32, mode: ModeHover, routes: Vec<viewer_gpu::ribbon_content::TradeRouteLine>, selected: Arc<Vec<Unit>>, rosters: Arc<province_tooltips::air_region::Sampler>) -> Job {
        let (inputs, cache) = (self.clone(), Arc::clone(&caches.provinces));
        let build: Build = Box::new(move |frame, retry| {
            let retry = Arc::clone(retry);
            lock(&cache).request(&inputs.content, inputs.raids.movement_reads(), frame, province, &inputs.vision, inputs.style.nato, &selected, &rosters, &mode, &routes, move || retry())
        });
        self.job(build).preparing(Arc::clone(&self.content.province_hover) as Arc<dyn Prepare>)
    }

    pub(crate) fn terrain(&self, combat: Arc<viewer_combat::Content>, province: u32, region: bool, routes: Vec<viewer_gpu::ribbon_content::TradeRouteLine>) -> Job {
        let inputs = self.clone();
        self.job(Box::new(move |frame, _| {
            let below = || viewer_combat::terrain_mode::hover(&combat, frame, province, &inputs.vision);
            Arc::new(province_tooltips::under_routes(&inputs.content, frame, inputs.vision.ours(), &routes, region, below))
        }))
    }

    pub(crate) fn capture(&self, land: Vec<CounterHover>, task_forces: Vec<task_force::Hover>, ports: Vec<task_force_port::Hover>) -> Job {
        let inputs = self.clone();
        self.job(Box::new(move |frame, _| {
            let batch = Batch::new(&inputs.content, inputs.raids.movement_reads(), frame, &inputs.vision, inputs.style);
            let tooltips = land.iter().map(|hover| batch.build(hover)).chain(task_forces.iter().map(|hover| batch.build_task_force(hover))).chain(ports.iter().map(|hover| batch.build_task_force_port(hover)));
            Arc::new(Tooltip { refused: counter_tooltips::count_refusals(tooltips), ..Tooltip::default() })
        }))
    }
}
