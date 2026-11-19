//! The playhead hosted in the map's process: one per open (recording, timeline), shared by every
//! caller that opens the same ([`open`]), with a slot for the stepper's hook another module fills
//! ([`Hooks`]). The map's frame side ([`MapSide`]) is one reader of it: the union of its drawn
//! layers' columns, kept in one mirror per table ([`mirror`]), and the layers that draw from them
//! ([`crate::layers`]). Every frame-side call returns without waiting: a reset's composing runs on
//! the host's worker and is swapped in whole when done.

pub mod details;
pub mod limitations;
pub mod mirror;
pub mod overlay;
pub mod piece;
pub mod side;
pub mod standing;
pub mod timeline;
pub mod views;
#[cfg(any(test, feature = "test-support"))]
pub mod shared_maps_tests;
#[cfg(test)]
mod tests;
pub mod units;

use crate::layers::{Read, Upstream, Settling, Gpu, Inputs, Installs, Layer, View};
use mirror::{Mirror, Mirrors};
use playhead::{Consumer, Declared, Lanes, Playhead, Reader, Settings, Step, pages::Pages};
use replay_columnar::state::Kind;
use replay_data::{Hour, rows::TableId};
use std::{
    collections::{BTreeMap, BTreeSet, VecDeque},
    io,
    sync::{
        Arc, Mutex, Weak,
        atomic::{AtomicBool, AtomicU8, AtomicU64, Ordering},
        mpsc::{self, Receiver, Sender, TryRecvError},
    },
    time::Instant,
};
use units::Units;

/// A function the stepper calls after each step is built, before any reader can take it.
pub type Hook = Box<dyn FnMut(&Step, &Lanes<'_>) + Send>;

/// The stepper's hook, filled and emptied by other modules while the playhead runs.
type HookList = Vec<(u64, Arc<Mutex<Hook>>)>;

#[derive(Default)]
pub struct Hooks {
    held: Mutex<Arc<HookList>>,
    next: AtomicU64,
}

impl Hooks {
    /// Adds `hook`; its id removes it.
    pub fn add(&self, hook: Hook) -> u64 {
        let id = self.next.fetch_add(1, Ordering::Relaxed);
        Arc::make_mut(&mut lock(&self.held)).push((id, Arc::new(Mutex::new(hook))));
        id
    }

    pub fn remove(&self, id: u64) {
        let removed = {
            let mut hooks = lock(&self.held);
            hooks.iter().position(|(held, _)| *held == id).map(|at| Arc::make_mut(&mut hooks).remove(at))
        };
        drop(removed);
    }

    fn run(&self, step: &Step, lanes: &Lanes<'_>) {
        let hooks = Arc::clone(&lock(&self.held));
        for (_, hook) in hooks.iter() {
            lock(hook)(step, lanes);
        }
    }
}

fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// One playhead and what its callers share.
pub struct Entry {
    page_time: Mutex<playhead::PageTime>,
    playhead: Playhead,
    pages: Arc<dyn Pages>,
    kinds: Arc<Vec<Vec<Kind>>>,
    hooks: Arc<Hooks>,
    /// The anchors taken, whoever sent them: the hour last sought and the pace last set.
    anchors: Mutex<(Option<Hour>, Option<(bool, u64)>)>,
    /// The recording's capture mode, given once by whoever opened the recording, which says why a
    /// table is not recorded.
    capture: std::sync::OnceLock<replay_data::scope::CaptureMode>,
    /// Maps drawing from this playhead: while any is, the rows follow the head its frame side took.
    maps: std::sync::atomic::AtomicUsize,
    /// The one air roster sampler of this recording and timeline, read by lanes of its own.
    rosters: std::sync::OnceLock<Option<Arc<viewer_counter_products::map::province_tooltips::air_region::Sampler>>>,
}

impl Entry {
    pub fn follow_anchor(&self, anchor: dashboard_follow::registry::Anchor) {
        let at = lock(&self.page_time).composed(anchor.timestamps, Instant::now());
        let mut held = lock(&self.anchors);
        let pace = (anchor.playing && anchor.speed > 0.0, anchor.speed.to_bits());
        let previous_step = self.playhead.step_hours();
        self.playhead.frame_rate(anchor.frame_rate);
        if held.1.is_none_or(|(_, speed)| speed != pace.1) { self.playhead.pace(anchor.speed); }
        // A pause needs its exact hour even if the stepper has already built past it.
        let seek = anchor.sought || self.playhead.step_hours() < previous_step || (!pace.0 && held.1.is_some_and(|(moving, _)| moving));
        held.1 = Some(pace);
        if seek { held.0 = Some(anchor.hour); }
        self.playhead.anchor(anchor.position, pace.0, anchor.speed, at, seek);
    }

    /// One playback anchor for this (side, recording, timeline), whoever sends it: a seek to the
    /// hour it is already holding at, or already at and not moving, is dropped; so is a pace it
    /// already has. A paused anchor at another hour (a step, a pause) places the clock there.
    pub fn anchor(&self, hour: Hour, playing: bool, speed: f64, sought: bool) {
        self.follow_rate();
        let mut held = lock(&self.anchors);
        let playhead = &self.playhead;
        let pace = (playing && speed > 0.0, speed.to_bits());
        let old_step = playhead.step_hours();
        let at = held.0 == Some(hour) && (playhead.holding().is_some() || (playhead.head(Instant::now()) == hour && held.1.is_none_or(|(moving, _)| !moving)));
        // The pace first: the steps a seek builds ahead of the head are sized by it.
        if held.1 != Some(pace) {
            if pace.0 {
                playhead.play(speed);
            } else {
                // Paused, the steps ahead are built for the speed play will resume at.
                playhead.pace(speed);
                playhead.pause();
            }
            held.1 = Some(pace);
        }
        if (sought && !at) || playhead.step_hours() < old_step {
            playhead.seek(hour);
            held.0 = Some(hour);
        } else if !pace.0 && playhead.head(Instant::now()) != playhead.timeline().clamp(hour) {
            playhead.place(hour);
            held.0 = Some(hour);
        }
    }

    /// Sizes the steps for the rate the simulation holds, following a display whose refresh is
    /// below it (a change of display or of its mode); the harness's own frame clock stands for a
    /// display it has none of.
    pub fn follow_rate(&self) {
        self.playhead.frame_rate(step_rate());
    }

    /// The anchors last set a pause: the map then shows exactly the clock's hour.
    pub fn paused(&self) -> bool {
        matches!(lock(&self.anchors).1, Some((false, _)))
    }

    /// A reset at `hour`: the only exact way to an hour inside a step the playhead built past.
    fn exact(&self, hour: Hour) {
        let mut held = lock(&self.anchors);
        self.playhead.seek(hour);
        held.0 = Some(hour);
    }

    /// Whether any caller has anchored it yet; a map joining one already anchored takes its clock.
    pub fn anchored(&self) -> bool {
        lock(&self.anchors).0.is_some()
    }

    /// The recording's capture mode; the first given stands.
    pub fn set_capture(&self, capture: replay_data::scope::CaptureMode) {
        let _ = self.capture.set(capture);
    }

    /// Whether a map draws from this playhead.
    #[cfg(any(test, feature = "test-support"))]
    pub fn drawn(&self) -> bool {
        self.maps.load(Ordering::Acquire) > 0
    }

    pub fn playhead(&self) -> &Playhead {
        &self.playhead
    }

    pub fn hooks(&self) -> &Hooks {
        &self.hooks
    }

    pub fn kinds(&self) -> &[Vec<Kind>] {
        &self.kinds
    }

    pub fn pages(&self) -> &Arc<dyn Pages> {
        &self.pages
    }

    /// The one air roster sampler of this recording and timeline, over lanes of its own, which
    /// every map and tool reading `replay` on this playhead shares; none when no branch of the
    /// recording plays its timeline.
    pub fn rosters(&self, replay: &Arc<replay::campaign::Replay>) -> Option<Arc<viewer_counter_products::map::province_tooltips::air_region::Sampler>> {
        use viewer_counter_products::map::province_tooltips::air_region::Sampler;
        self.rosters
            .get_or_init(|| crate::lane_rows::rosters::Lanes::of(self, replay).map(|lanes| Arc::new(Sampler::new(Arc::clone(replay), Box::new(lanes)))))
            .clone()
    }

}

/// No head stored yet.
/// Who reads a playhead: the maps drawn from it, or the dashboard's followed queries. They never
/// share one, so no followed query's work runs on the stepper a map's frame takes steps from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Side {
    Map,
    Follow,
}

/// A playhead's side, recording, timeline and clock mapping ([`open_clocked`]).
type Key = (Side, String, Vec<(usize, Hour, Hour)>, Hour);

static ENTRIES: Mutex<Vec<(Key, Weak<Entry>)>> = Mutex::new(Vec::new());

/// The map's playhead of `units`' recording and timeline: the one already open, or a new one with
/// `settings`.
pub fn open(units: Units, settings: Settings) -> io::Result<Arc<Entry>> {
    open_clocked(units, settings, 0, Side::Map)
}

/// [`open`] for `side`'s track whose game hour is the UI clock's plus `clock`: each mapping of one
/// timeline is anchored at its own hours, so each has its own playhead. The map is at 0.
pub fn open_clocked(units: Units, settings: Settings, clock: Hour, side: Side) -> io::Result<Arc<Entry>> {
    let key: Key = (side, units.name.clone(), units.timeline.pieces().iter().map(|piece| (piece.run, piece.first, piece.last)).collect(), clock);
    let mut entries = lock(&ENTRIES);
    entries.retain(|(_, entry)| entry.strong_count() > 0);
    if let Some(entry) = entries.iter().find(|(held, _)| *held == key).and_then(|(_, entry)| entry.upgrade()) {
        return Ok(entry);
    }
    let hooks = Arc::new(Hooks::default());
    let stepper_hooks = Arc::clone(&hooks);
    let hook: playhead::Hook = Box::new(move |step: &Step, lanes: &Lanes<'_>| stepper_hooks.run(step, lanes));
    let playhead = Playhead::new(Arc::clone(&units.pages), units.timeline, settings, Some(hook))?;
    let entry = Arc::new(Entry { page_time: Default::default(), playhead, pages: units.pages, kinds: units.kinds, hooks, anchors: Mutex::new((None, None)), capture: Default::default(), maps: Default::default(), rosters: Default::default() });
    entries.push((key, Arc::downgrade(&entry)));
    Ok(entry)
}

/// How long a whole settle tries for the overlay of its hour before handing back to the steps: the
/// mirrors are declared again, and the rows of a table a maker declared since arrive, only in a step.
const HANDS_BACK: std::time::Duration = std::time::Duration::from_secs(2);

/// How long a stepped frame's picture is caught up to before it is refused by name.
const CATCHING_UP: std::time::Duration = std::time::Duration::from_secs(60);

/// The rate the playhead steps at: one step a publication. The hours inside a publication are read
/// from the rows of those hours ([`playhead::Step::hours_of`]), not by stepping the whole side more
/// finely, so a timed layer dates a change at the game hour it happened at while every mirror,
/// every maker and every untimed layer takes one step for the interval.
fn step_rate() -> f64 {
    viewer_foundation::map::simulation_rate::rate()
}

/// The playhead's settings for the map: steps on two threads, reads on one, a step for each
/// publication the simulation holds its rate at, and eight publications' steps built past the
/// head: at 1,440 hours a second a publication is due every 33 ms, the one that applies the step is
/// prepared before the head reaches its last hour, and one slow build must not let the head pass
/// the newest step.
pub fn settings() -> Settings {
    Settings { step_threads: 2, fetch_threads: 1, ring: 8, ahead: 4, rate: step_rate(), entering: Some(entered), ..Settings::default() }
}

/// The class of each thread the playhead starts. The stepper holds the ring the frame takes every
/// frame, and the step threads hold lanes it waits for, so neither runs below the frame's workers;
/// no frame waits for a fetch.
fn entered(thread: playhead::Thread) {
    use viewer_foundation::worker_priority::{Class, enter};
    enter(match thread {
        playhead::Thread::Stepper | playhead::Thread::Step => Class::Kept,
        playhead::Thread::Fetch => Class::Worker,
    });
}

/// The layers the window registers, by name, when the playback-cost harness names them (to tell
/// their costs apart); every layer otherwise.
static REGISTERED: std::sync::OnceLock<Vec<String>> = std::sync::OnceLock::new();

pub fn register_only(names: Vec<String>) {
    let _ = REGISTERED.set(names);
}

/// The map's reader for `document`, on the playhead of the recording it reads, at clock `at` (the
/// document's hour when none): its rows carry `at`'s branch, and a playhead no other caller has
/// anchored is sought to its hour; one already anchored is joined at its own clock. A refusal
/// names what it refused; the map draws nothing without the reader. Made on the caller's thread;
/// the window makes it on a worker instead ([`Opening`]).
pub fn for_document_at(document: &crate::document::Document, at: Option<At>) -> io::Result<MapSide> {
    ForMap::of(document, at).made()
}

/// A map's clock as its reader opens: the branch it reads, the hour, and the pace.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct At {
    pub branch: replay_data::BranchId,
    pub hour: Hour,
    pub playing: bool,
    pub speed: f64,
}

/// What a map's reader is made from, taken from the document on the frame thread at the cost of
/// reference counts, so that the reader itself (the units read, the engaged fold, the layers'
/// content, the declaration) is made anywhere. It reads no frame of rows: the viewing country is
/// the vision layer's, and the counters' strategic layout their mirrors'.
pub struct ForMap {
    content: Arc<crate::document::Content>,
    replay: Arc<replay::campaign::Replay>,
    branch: replay_data::BranchId,
    hour: Hour,
    at: Option<At>,
    /// The weather layer, whose branch only its own constructor sets, from the document: an empty
    /// layer holding no content.
    weather: Box<dyn Layer>,
}

impl ForMap {
    pub fn of(document: &crate::document::Document, at: Option<At>) -> Self {
        Self {
            content: Arc::clone(&document.content),
            replay: document.playback.replay_handle(),
            branch: at.map_or_else(|| document.playback.branch(), |at| at.branch),
            hour: at.map_or_else(|| document.playback.hour(), |at| at.hour),
            at,
            weather: crate::layers::weather::for_document(document),
        }
    }

    fn made(self) -> io::Result<MapSide> {
        let Self { content: document, replay, branch: recording, hour, at, weather } = self;
        let units = units::of(&replay, recording)?;
        let (engaged, branch) = (crate::engaged_index::lanes::shared(&replay, &units, recording, &viewer_foundation::map::perspective::Country::Recorded)?, recording);
        let entry = open(units, settings())?;
        entry.set_capture(replay.capture_mode);
        let content = Arc::new(crate::layers::political::Content::new(Arc::clone(&document.map), Arc::clone(&document.border_meshes)));
        let place_in = Arc::clone(&document);
        let map = &document.map;
        let largest = map.definitions.last_key_value().map_or(0, |(&id, _)| id).max(map.province_ids.iter().copied().max().unwrap_or(0));
        let intel = viewer_foundation::map::textures::IntelCells::new(map).map(|cells| Arc::new(crate::layers::intel::Content::new(cells, largest))).map_err(|error| error.to_string());
        let snow = document
            .products
            .snow_content()
            .map(|content| Arc::new(crate::layers::snow::Content::new(content.cells.clone(), content.winter.clone(), viewer_foundation::map::camera::MapSize::of(&document.map), document.map_defines)))
            .map_err(|error| error.to_string());
        let counters = crate::layers::counters::Content::new(Arc::clone(&document.counters), document.movement.clone(), (viewer_foundation::map::camera::MapSize::of(&document.map), document.map_defines), document.tables);
        let marker_index = || crate::layers::engagements::Index { history: Arc::clone(&engaged), branch };
        let (land_index, naval_index) = (marker_index(), marker_index());
        let guns = crate::layers::gun_counters::Content::new(Arc::clone(&document.counters), Arc::clone(&document.map), document.movement.clone());
        let mut layers: Vec<Box<dyn Layer>> = vec![
            Box::new(crate::layers::political::Political::new(content)),
            // Every stock mode but the default: the fill, colours and borders the political layer draws there.
            Box::new(crate::layers::mode_fills::new(Arc::clone(&document) as Arc<dyn crate::layers::mode_fills::Paint>, false)?.in_window(Arc::clone(&entry), views::Recording { branch: recording })?),
            Box::new(crate::layers::vision::Vision::new(Arc::new(crate::layers::vision::Content::new(document.products.fog.as_ref().clone())))),
            Box::new(crate::layers::intel::Intel::new(intel)),
            Box::new(crate::layers::radar::Radar::new()),
            weather,
            Box::new(crate::layers::snow::Snow::new(snow)),
            Box::new(crate::layers::sounds::SoundsFed::new(crate::layers::sounds::Sounds::new(document.audio.as_ref().map(|source| Arc::clone(&source.content))))),
            Box::new(crate::layers::strikes::Strikes::new(Arc::new(move |province| place_in.products.impact_place(province)))),
            Box::new(crate::layers::map_text::MapText::new(Arc::clone(&document) as Arc<dyn crate::layers::map_text::Source>, recording).with_rows(Arc::clone(&entry))),
            Box::new(crate::layers::convoys::Convoys::new(Arc::clone(&document) as Arc<dyn crate::layers::convoys::Routes>).in_window()),
            // Before units, which joins the ships to its models while they stand in for today's.
            Box::new({
                let ships = crate::layers::ships::Ships::new(Arc::clone(&document) as Arc<dyn crate::layers::ships::Naval>)?.in_window();
                ships.set_source(Arc::clone(&entry), views::Recording { branch: recording })?;
                ships
            }),
            Box::new(crate::layers::aircraft::new(Arc::clone(&document), false)?.in_window(Arc::clone(&entry), views::Recording { branch: recording })?),
            Box::new(
                crate::layers::units::Units::new(Arc::clone(&document) as Arc<dyn crate::layers::units::Ground>)
                    .in_window(Arc::clone(&document) as Arc<dyn crate::layers::units::models::Select>),
            ),
            Box::new({
                let orders = crate::layers::orders::Orders::new(Arc::clone(&document) as Arc<dyn crate::layers::orders::Plans>).in_window();
                orders
            }),
            Box::new(crate::layers::buildings::BuildingsFed::new(crate::layers::buildings::Buildings::reading_vision(Arc::clone(&document) as Arc<dyn crate::layers::buildings::Catalogue>))),
            Box::new(crate::layers::naval::Naval::new(Arc::new(document.naval_dominance.clone()))),
            Box::new(crate::layers::engagements::Engagements::new(Some(crate::layers::engagements::Index { history: engaged, branch }), None).reading_vision()),
            Box::new(crate::layers::intel_icons::Icons::new(Arc::clone(&document) as Arc<dyn crate::layers::intel_icons::Evaluate>, views::Pace::Ready)),
            Box::new(crate::layers::resource_icons::ResourceIcons::new(Arc::clone(&document) as Arc<dyn crate::layers::resource_icons::Source>)),
            Box::new(crate::layers::occupation_marks::OccupationMarks::new(Arc::clone(&document) as Arc<dyn crate::layers::occupation_marks::Source>)),
            Box::new(crate::layers::relations::Relations::new()),
            Box::new(crate::layers::counters::fed::Fed::new(crate::layers::counters::Counters::new(Arc::new(counters)).reading_vision())),
            // P17: the counter kinds after the land stacks, the markers after the engagements.
            Box::new(crate::layers::land_markers::LandMarkers::new(Arc::clone(&document) as Arc<dyn crate::layers::land_markers::Front>, Some(land_index))),
            Box::new(crate::layers::naval_markers::NavalMarkers::new(Arc::clone(&document) as Arc<dyn crate::layers::naval_markers::Shore>, Some(naval_index))),
            Box::new(crate::layers::base_counters::BaseCountersFed::new(crate::layers::base_counters::BaseCounters::new())),
            Box::new(crate::layers::gun_counters::GunCounters::new(Arc::new(guns))),
            Box::new(crate::layers::air_mission_counters::AirMissionCounters::new(Arc::new(crate::layers::air_mission_counters::Content::new(&document.counters)))),
            // P7: the railway strips and bridges, the country names, the state and capital labels.
            Box::new(crate::layers::rails::Rails::new(Arc::clone(&document) as Arc<dyn crate::layers::rails::Geography>)),
            Box::new(crate::layers::trains::Trains::new(Arc::clone(&document) as Arc<dyn crate::layers::trains::Yard>, None).reading_vision()),
            Box::new(crate::layers::names::Names::new(Arc::clone(&document) as Arc<dyn crate::layers::names::Atlas>).in_window(Arc::clone(&entry), views::Recording { branch: recording })?),
            Box::new(crate::layers::labels::Labels::new(Arc::clone(&document.names.localisation), None).reading_vision()),
            // The raid lane's offer cycle and the viewing country's music chances, at a rate of
            // their own: the overlay of the step lays its icons in.
            Box::new(crate::layers::scripts::Scripts::new(&document)),
        ];
        if let Some(names) = REGISTERED.get() {
            layers.retain(|layer| names.iter().any(|name| name == layer.name()));
        }
        let anchored = entry.anchored();
        let mut side = MapSide::new(Arc::clone(&entry), layers, View::default())?;
        side.branch = Some(recording);
        side.set_sight(Arc::clone(&document));
        side.set_overlay(Arc::clone(&document));
        if !anchored {
            let (hour, playing, speed) = at.map_or((hour, false, 0.0), |at| (at.hour, at.playing, at.speed));
            entry.anchor(hour, playing, speed, true);
        }
        Ok(side)
    }
}

/// A map's reader being made on a worker; the frame that started it goes on drawing.
pub struct Opening {
    made: Receiver<io::Result<MapSide>>,
}

impl Opening {
    /// The reader for `document` at clock `at`, made off the caller's thread.
    pub fn start(document: &crate::document::Document, at: Option<At>) -> io::Result<Self> {
        let made = ForMap::of(document, at);
        Self::spawn(move || made.made())
    }

    /// `make` on a thread of its own; a refusal names why when no thread can be started.
    pub fn spawn(make: impl FnOnce() -> io::Result<MapSide> + Send + 'static) -> io::Result<Self> {
        let (send, made) = mpsc::channel();
        std::thread::Builder::new()
            .name(OPENING.into())
            .spawn(move || {
                viewer_foundation::worker_priority::background_priority();
                // A reader no longer wanted is dropped here, off the frame thread.
                let _ = send.send(make());
            })
            .map_err(|error| io::Error::other(format!("no thread to open it: {error}")))?;
        Ok(Self { made })
    }

    /// The reader once made, or the refusal by name; none while it is being made. Never waits.
    pub fn poll(&self) -> Option<io::Result<MapSide>> {
        match self.made.try_recv() {
            Ok(side) => Some(side),
            Err(TryRecvError::Empty) => None,
            Err(TryRecvError::Disconnected) => Some(Err(io::Error::other("its opening thread stopped before it was made"))),
        }
    }
}

/// The name of the thread an [`Opening`] makes its reader on.
pub const OPENING: &str = "map reader opening";

/// Where the head of the step the side's thread prepares comes from: the playhead's clock in the
/// window, the harness's head paced, or a stepped harness frame, which waits for the picture of
/// exactly its hour ([`side::SideThread::stepped_window`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Head {
    Clock,
    Paced(Hour),
    Stepped(Hour),
}

enum Job {
    Compose(Box<Away>),
    Drop(Box<dyn Send>),
}

/// The mirrors and layers on the worker while a reset composes.
struct Away {
    mirrors: Arc<Mirrors>,
    layers: Vec<Box<dyn Layer>>,
    compose: Vec<bool>,
    /// Per layer: the layers before it that it reads.
    upstream: Vec<Vec<usize>>,
    /// Per layer: what its last compose took, which sends the longest work first.
    cost: Vec<u64>,
    /// Set by the frame side once a newer seek supersedes it: no further layer composes.
    cancel: Arc<AtomicBool>,
    /// Per layer: its compose's result and nanoseconds, and whether it ran.
    results: Vec<(io::Result<()>, u64, bool)>,
}

/// One group of layers that read one another, composed in registration order on one thread: a
/// layer is `Send` but not `Sync`, so two layers reading one upstream never share it across threads.
struct Group {
    at: Vec<usize>,
    layers: Vec<Box<dyn Layer>>,
    results: Vec<(io::Result<()>, u64, bool)>,
    cost: u64,
}

impl Group {
    fn compose(&mut self, mirrors: &Mirrors, compose: &[bool], cancel: &AtomicBool) {
        for member in 0..self.layers.len() {
            let at = self.at[member];
            let wanted = compose[at] && !cancel.load(Ordering::Acquire);
            let (before, rest) = self.layers.split_at_mut(member);
            let started = Instant::now();
            let result = if wanted { rest[0].compose_with(mirrors, &Upstream::new(before)) } else { Ok(()) };
            self.results.push((result, started.elapsed().as_nanos() as u64, wanted));
        }
    }
}

/// Per layer, the first layer of its group: layers are joined by what they read of one another.
fn groups(upstream: &[Vec<usize>]) -> Vec<usize> {
    fn root(of: &mut [usize], mut at: usize) -> usize {
        while of[at] != at {
            of[at] = of[of[at]];
            at = of[at];
        }
        at
    }
    let mut of: Vec<usize> = (0..upstream.len()).collect();
    for (at, uppers) in upstream.iter().enumerate() {
        for &upper in uppers {
            let (a, b) = (root(&mut of, at), root(&mut of, upper));
            of[a.max(b)] = a.min(b);
        }
    }
    (0..upstream.len()).map(|at| root(&mut of, at)).collect()
}

/// The name of the threads a reset's groups compose on beside the host's worker.
const COMPOSING: &str = "playhead host compose";

/// What a compose's threads are to the frame: the side's mirrors and every layer are away while
/// they work, so the drawn hour stands still until they are done — the picture waits for them as
/// it waits for the side itself, and a maker nothing waits for must not be scheduled above them
/// ([`viewer_foundation::worker_priority::Class::Kept`]: at a piece boundary the hold is the
/// compose).
const COMPOSE_CLASS: viewer_foundation::worker_priority::Class = viewer_foundation::worker_priority::Class::Kept;

/// Composes the layers asked: the groups at once on as many threads as the machine gives, the
/// longest by the last compose first, each group's layers in registration order.
fn compose_away(away: &mut Away) {
    let count = away.layers.len();
    let first = groups(&away.upstream);
    let mut group_of = vec![usize::MAX; count];
    let mut all: Vec<Group> = Vec::new();
    for (at, layer) in std::mem::take(&mut away.layers).into_iter().enumerate() {
        let root = first[at];
        if group_of[root] == usize::MAX {
            group_of[root] = all.len();
            all.push(Group { at: Vec::new(), layers: Vec::new(), results: Vec::new(), cost: 0 });
        }
        let group = &mut all[group_of[root]];
        group.at.push(at);
        group.layers.push(layer);
        if away.compose[at] {
            group.cost += away.cost.get(at).copied().unwrap_or(0).max(1);
        }
    }
    let (mut waiting, idle): (Vec<Group>, Vec<Group>) = all.into_iter().partition(|group| group.cost > 0);
    waiting.sort_by_key(|group| group.cost);
    let threads = waiting.len().min(std::thread::available_parallelism().map_or(1, std::num::NonZero::get));
    let (queue, done) = (Mutex::new(waiting), Mutex::new(Vec::with_capacity(count)));
    let (mirrors, compose, cancel) = (&*away.mirrors, &away.compose[..], &*away.cancel);
    let work = || {
        loop {
            let next = lock(&queue).pop();
            let Some(mut group) = next else { break };
            group.compose(mirrors, compose, cancel);
            lock(&done).push(group);
        }
    };
    std::thread::scope(|scope| {
        for _ in 1..threads {
            if std::thread::Builder::new().name(COMPOSING.into()).spawn_scoped(scope, || {
                viewer_foundation::worker_priority::enter(COMPOSE_CLASS);
                work();
            }).is_err() {
                break;
            }
        }
        work();
    });
    let mut layers: Vec<Option<Box<dyn Layer>>> = (0..count).map(|_| None).collect();
    let mut results: Vec<Option<(io::Result<()>, u64, bool)>> = (0..count).map(|_| None).collect();
    let idle = idle.into_iter().map(|group| Group { results: group.at.iter().map(|_| (Ok(()), 0, false)).collect(), ..group });
    for group in done.into_inner().unwrap_or_else(std::sync::PoisonError::into_inner).into_iter().chain(idle) {
        for ((at, layer), result) in group.at.into_iter().zip(group.layers).zip(group.results) {
            layers[at] = Some(layer);
            results[at] = Some(result);
        }
    }
    away.layers = layers.into_iter().map(|layer| layer.expect("every layer comes back from its group")).collect();
    away.results = results.into_iter().map(|result| result.expect("every layer of a group has a result")).collect();
}

/// The host's worker: composes at a reset and drops what the frame let go of.
struct Worker {
    jobs: Sender<Job>,
    done: Receiver<Box<Away>>,
}

impl Worker {
    fn new() -> io::Result<Self> {
        let (jobs, receive) = mpsc::channel::<Job>();
        let (send, done) = mpsc::channel();
        std::thread::Builder::new().name("playhead host worker".into()).spawn(move || {
            viewer_foundation::worker_priority::enter(COMPOSE_CLASS);
            for job in receive {
                match job {
                    Job::Compose(mut away) => {
                        compose_away(&mut away);
                        if send.send(away).is_err() {
                            return;
                        }
                    }
                    Job::Drop(held) => drop(held),
                }
            }
        })?;
        Ok(Self { jobs, done })
    }

    fn drop_later(&self, held: impl Send + 'static) {
        let _ = self.jobs.send(Job::Drop(Box::new(held)));
    }
}

/// One layer's part of a frame.
#[derive(Clone, Copy, Debug, Default)]
pub struct LayerFrame {
    pub apply_ns: u64,
    pub settle_ns: u64,
    /// The frame thread's own cycles in them ([`replay_frame::cpu::thread_cycles`]: none while
    /// preempted), zero unless accounting is on. Read once between layers, each runs from the read
    /// before: an apply holds the host's bookkeeping since the last layer's, and a settle its
    /// layer's steps lent and, with no step applied, that bookkeeping.
    pub apply_cycles: u64,
    pub settle_cycles: u64,
    /// Entities dirty before settling; none are left after it.
    pub dirty: usize,
    /// Its install prepared on the side's thread ([`crate::layers::Layer::stage`]).
    pub stage_ns: u64,
    pub stage_cycles: u64,
    pub upload_ns: u64,
    pub upload_cycles: u64,
    /// GPU writes and their bytes counted while it uploaded (another thread's writes in that
    /// window included).
    pub uploads: u64,
    pub upload_bytes: u64,
    /// Of its upload, the nanoseconds inside the queue's write calls and inside buffer and bind
    /// group creation: the rest of it prepared bytes on this thread.
    pub write_ns: u64,
    pub create_ns: u64,
    /// What it settled is not all on the GPU yet (a write too large for one frame), after this frame's upload.
    pub upload_carried: bool,
    /// Which answer [`crate::layers::Published`] gave after this frame: 0 current, 1 at an hour,
    /// 2 a cadence of its own, 3 nothing yet.
    pub published: u8,
    /// Drawn: declared, composed and not refused.
    pub ready: bool,
    /// The hour its published product is current for, after this frame.
    pub shown: Option<Hour>,
    /// It shows a product made since the last compose swapped in: after a seek, the sought picture.
    pub reset_shown: bool,
    /// A layer fed the steps ahead, in a frame that applied a step: whether the product it shows
    /// after its settle is the applied hour's (made ahead and ready), or an older one.
    pub ready_at_hour: Option<bool>,
    /// Handing it the steps ahead, on the frame thread.
    pub ahead_ns: u64,
    /// The entities it settled this frame, and of them those the view shows (paced runs only).
    pub scope: Option<crate::layers::Scope>,
}

/// A test's hold on every settle of a map side ([`MapSide::hold_settles`]), released when dropped:
/// the steps' marks stay owed so a test can see a layer settle to the newest of several steps.
#[cfg(any(test, feature = "test-support"))]
#[must_use = "the settles are released as soon as this is dropped"]
pub struct SettlesHeld(Arc<AtomicBool>);

#[cfg(any(test, feature = "test-support"))]
impl Drop for SettlesHeld {
    fn drop(&mut self) {
        self.0.store(false, Ordering::Release);
    }
}

/// The side's step as stages, in order, each with the side thread's wall time and the thread's
/// own CPU in it ([`replay_frame::cpu::thread_cycles`]: none while it is off a core): a stage
/// whose wall stands far over its CPU waited rather than worked. Kept for every publication so a
/// hold of the drawn hour is accounted for stage by stage instead of guessed at.
#[derive(Clone, Copy, Debug, Default)]
pub struct Stages {
    pub ns: [u64; Stages::NAMES.len()],
    pub cycles: [u64; Stages::NAMES.len()],
}

impl Stages {
    /// The stages, in the order the step runs them: the views the step before posted waited for,
    /// the steps taken from the ring, their apply to the mirrors and the layers, the settles of
    /// the layers that join no maker, the joins of those that do, the harness's in-view count,
    /// the installs prepared, and the shown hours and the overlay's close.
    pub const NAMES: [&str; 8] = ["views", "take", "apply", "settles", "joins", "in_view", "stage", "shown"];
}

/// What the side's thread is doing, as it publishes it for a display frame that could not take the
/// side ([`side::SideThread::window`]): nothing between publications, else the stage of the
/// publication it is inside ([`Stages::NAMES`]), or a whole settle, which no stage covers.
pub struct Doing(AtomicU8);

impl Doing {
    const IDLE: u8 = 0;
    const WHOLE: u8 = Stages::NAMES.len() as u8 + 1;

    /// The name of what it holds now.
    pub fn name(&self) -> &'static str {
        match self.0.load(Ordering::Relaxed) {
            Self::IDLE => "between publications",
            Self::WHOLE => "a whole settle",
            stage => Stages::NAMES.get(stage as usize - 1).copied().unwrap_or("a stage of its own"),
        }
    }

    fn set(&self, doing: u8) {
        self.0.store(doing, Ordering::Relaxed);
    }
}

impl Default for Doing {
    fn default() -> Self {
        Self(AtomicU8::new(Self::IDLE))
    }
}

/// The side thread's wall and CPU between two stages of one step, which it publishes as it opens
/// each ([`Doing`]).
struct Watch<'a> {
    at: Instant,
    cycles: u64,
    doing: &'a Doing,
}

impl<'a> Watch<'a> {
    fn new(doing: &'a Doing) -> Self {
        doing.set(1);
        Self { at: Instant::now(), cycles: replay_frame::cpu::thread_cycles(), doing }
    }

    /// Closes stage `stage` of `stages` and opens the next.
    fn lap(&mut self, stages: &mut Stages, stage: usize) {
        let (at, cycles) = (Instant::now(), replay_frame::cpu::thread_cycles());
        stages.ns[stage] = at.saturating_duration_since(self.at).as_nanos() as u64;
        stages.cycles[stage] = cycles.saturating_sub(self.cycles);
        (self.at, self.cycles) = (at, cycles);
        self.doing.set(stage as u8 + 2);
    }
}

impl Drop for Watch<'_> {
    fn drop(&mut self) {
        self.doing.set(Doing::IDLE);
    }
}

/// What a publication waited before it was made: how long past its due time it started, and of
/// that what it spent waiting for an hour the last one did not show and for the frame to release
/// the side ([`side::advance`]).
#[derive(Clone, Copy, Debug, Default)]
pub struct Waited {
    pub late_ns: u64,
    pub for_an_hour_ns: u64,
    pub for_the_frame_ns: u64,
}

/// What a frame did.
#[derive(Clone, Debug, Default)]
pub struct Frame {
    /// Which publication of this side it is, counted from the first: the rate a run held, and
    /// which display frames drew the same one, are read from it.
    pub made: u64,
    pub head: Hour,
    pub steps: u32,
    /// The first hour of the first step applied, and the last hour of the last: the hours this
    /// publication carries, which its own display frames' heads stand inside.
    pub first: Option<Hour>,
    /// The last hour of the last step applied.
    pub applied: Option<Hour>,
    pub mirror_ns: u64,
    pub mirror_cycles: u64,
    /// Taking the steps the stepper built from the ring it shares with it: wall nanoseconds, and
    /// the frame thread's own cycles (none while it waits for the ring's lock).
    pub take_ns: u64,
    pub take_cycles: u64,
    /// The mirror's apply by table index: wall nanoseconds and the thread's own cycles.
    pub mirror_tables: Vec<(u64, u64)>,
    /// The apply split by whether a drawn layer or a maker reads the table.
    pub mirror_split: mirror::Split,
    pub cells: u64,
    pub rows: u64,
    pub layers: Vec<LayerFrame>,
    /// Layers whose steps lent ahead were void (rebuilt for a new pace) and restarted this frame.
    pub voided: u32,
    /// A reset's composing is under way: the steps taken wait for it.
    pub composing: bool,
    /// Paused inside a step built past the head, it sought the head for an exact picture.
    pub exact: bool,
    /// A composed reset was swapped in this frame, with how long its layers took on the worker.
    pub swapped: Option<Vec<u64>>,
    pub resets: u32,
    /// What the seek of a reset step taken this frame took, on the playhead's threads.
    pub sought: Option<playhead::Seeking>,
    /// The hour the drawn layers show after this frame's upload: the last applied, once every one
    /// is settled and uploaded whole; until then the hour they last showed whole. A layer that
    /// publishes the hour of a product made on a cadence ([`crate::layers::Published::Cadence`])
    /// does not hold it: it is the window clock's.
    pub drawn: Option<Hour>,
    /// The hour of the oldest product on screen, every drawn layer counted, those made on a
    /// cadence too: what the lag is measured from.
    pub oldest: Option<Hour>,
    /// The stepper's wall time building the steps taken this frame, summed, and the most one took,
    /// with the stepper thread's own CPU in that wall time summed beside it.
    pub step_ns: u64,
    pub step_most_ns: u64,
    pub step_cycles: u64,
    /// The side's whole step: taking the steps, applying them and settling every layer with its
    /// products joined, on the side's thread, and the thread's own CPU in it.
    pub side_ns: u64,
    pub side_cycles: u64,
    /// The step's own stages ([`Stages`]).
    pub stages: Stages,
    /// What the publication waited before this step began ([`Waited`]).
    pub waited: Waited,
    /// The tables a step applied this frame renumbered, each with the rows it listed and those it
    /// moved to another slot: a renumber that moves no row is a pack that moved nothing.
    pub renumbers: Vec<(usize, u32, u32)>,
    /// The tables a step applied this frame reset (its mirror rebuilt), and whether the step
    /// itself was a reset (a seek's or a piece's, against a table landed for this reader alone).
    pub reset_tables: Vec<usize>,
    pub reset_step: bool,
    /// From the publication being started on the side's thread to the frame taking it; zero on a
    /// frame that took no new publication.
    pub reached_ns: u64,
    /// A compose came back superseded by a newer seek and was discarded: the old picture stays
    /// until the newer seek's compose.
    pub superseded: bool,
    /// Every drawn layer shows a product made since the last compose swapped in.
    pub reset_shown: bool,
}

/// What the map's frame side holds, in bytes.
#[derive(Clone, Debug, Default)]
pub struct Held {
    pub mirrors: Vec<(TableId, usize)>,
    /// The rows the makers read, made from those mirrors, by table.
    pub views: Vec<(TableId, usize)>,
    pub layers: Vec<(&'static str, usize)>,
    /// Steps taken with a reset, applied once it is composed; while composing nothing more is
    /// taken, so the backlog stays in the playhead's ring and becomes a longer step.
    pub pending: usize,
    /// Rebuilt states the playhead holds until taken: count and bytes.
    pub states: (usize, usize),
    pub playhead: usize,
}

impl Held {
    pub fn total(&self) -> usize {
        self.mirrors.iter().map(|(_, bytes)| bytes).sum::<usize>() + self.views.iter().map(|(_, bytes)| bytes).sum::<usize>() + self.layers.iter().map(|(_, bytes)| bytes).sum::<usize>() + self.states.1
    }
}

/// The map's reader of a playhead: its mirrors and layers, on the frame thread.
pub struct MapSide {
    entry: Arc<Entry>,
    reader: Reader,
    consumer: Consumer,
    view: View,
    /// Per table read, the union of the layers' columns and whether keys are wanted.
    declared: BTreeMap<usize, Read>,
    /// Per layer: the tables it reads, or why it is refused.
    reads: Vec<Result<BTreeSet<usize>, String>>,
    /// Per layer: composed since it was made, so it may apply and settle.
    ready: Vec<bool>,
    /// Per layer: the layers before it that it reads.
    upstream: Vec<Vec<usize>>,
    /// Per layer: accepted again after a refusal, so it composes at the next frame.
    recompose: Vec<bool>,
    names: Vec<&'static str>,
    mirrors: Option<Arc<Mirrors>>,
    /// The rows its makers build from, made from those mirrors inside its step.
    views: views::Store,
    /// Per table: the union of the drawn layers' columns alone, without the makers'.
    declared_layers: BTreeMap<usize, Read>,
    /// Per table index: whether a drawn layer reads it ([`declared_layers`] as a mask).
    layers_awake: Vec<bool>,
    /// Every table the recording holds, read whole for the makers ([`for_the_makers`]).
    for_the_makers: BTreeMap<usize, Read>,
    layers: Vec<Box<dyn Layer>>,
    worker: Worker,
    composing: bool,
    pending: VecDeque<Arc<Step>>,
    taken: Vec<Arc<Step>>,
    /// A view asked for while composing, taken once the layers are back.
    view_later: Option<View>,
    /// The sizes last seen, kept while the mirrors and layers are away.
    sizes: (Vec<(TableId, usize)>, Vec<(&'static str, usize)>),
    /// Per layer: its changes are cleared at the next frame's start (every drawn dependent took them).
    clear: Vec<bool>,
    /// Per layer: the hour its published product is current for; the map shows the oldest.
    shown: Vec<Option<Hour>>,
    /// The last hour applied, and the hour the layers last showed whole.
    applied: Option<Hour>,
    drawn: Option<Hour>,
    /// The hour of the oldest product drawn, every drawn layer counted ([`Frame::oldest`]).
    oldest: Option<Hour>,
    /// [`Self::drawn`] and [`Self::oldest`] counting only the layers whose publication is one
    /// picture: what the display frame takes the oldest of with its timed layers' own installers,
    /// which advance between publications ([`side::SideThread::install`]). The side says these
    /// once a publication; the hour on screen is the frame's own.
    untimed: (Option<Hour>, Option<Hour>),
    /// The naval attributes of today's political part, not yet handed to the political layer.
    last: Frame,
    frame: u64,
    /// Why the window's reader stopped, printed once.
    failure: Option<String>,
    /// The branch its rows are stamped with; none for a reader not opened for a document's map.
    branch: Option<replay_data::BranchId>,
    /// Reset steps taken, and those whose composed picture is on the frame side.
    resets_taken: u64,
    resets_shown: u64,
    /// The map mode its drawn picture was settled for, and per layer whether it has settled with
    /// no work left since the view's map mode last changed.
    mode: Option<viewer_world::map::map_mode::MapMode>,
    mode_settled: Vec<bool>,
    /// Its frames come from the stepped harness, which holds each frame's hour until it is drawn.
    stepped: bool,
    /// A test holds every settle while a [`SettlesHeld`] it took lives.
    #[cfg(any(test, feature = "test-support"))]
    settles_held: Arc<AtomicBool>,
    /// Per layer, as last declared: its own declaration, and what of it the camera decides
    /// ([`Layer::placed`]; none: it reads the camera in `inputs` only).
    declared_each: Vec<LayerDeclared>,
    placed: Vec<Option<u64>>,
    /// The content the count of settled entities in view reads its provinces and states from.
    sight: Option<Arc<crate::document::Content>>,
    /// The open map's overlay, made within the step ([`overlay`]); none for a map with no
    /// overlay (a dashboard map, a tool).
    overlay: Option<Box<overlay::Overlay>>,
    /// The vision its layers show, copied out of the vision layer in the step that settled it,
    /// which the overlay's tables and the counter layers are then made over.
    vision: overlay::ShownVision,
    /// Per layer: what its last compose took on the worker.
    compose_ns: Vec<u64>,
    /// The epoch of the newest step taken: a seek since supersedes a compose of its steps.
    epoch: u64,
    cancel: Arc<AtomicBool>,
    /// A compose came back superseded: the mirrors and layers are home, half composed for a seek
    /// no longer wanted, and nothing is settled, uploaded or shown until the newer seek composes.
    superseded: bool,
    /// Per layer: it shows a product made since the last compose swapped in; and every drawn layer.
    fresh: Vec<bool>,
    reset_shown: bool,
    /// Per layer, under `BATTLE_PLAN_VIEWER_CHECK_PUBLICATION`: the hour it showed when the judge last
    /// looked, and the resets taken by then, so a seek's older picture is not read as going
    /// backwards.
    judged_shown: Vec<Option<Hour>>,
    judged_resets: u64,
    /// Per layer fed the steps ahead: the last hour of the newest step of hours handed to it since
    /// it composed, and the newest [`Step::restep`] it was handed.
    offered: Vec<Option<Hour>>,
    resteps: Vec<u64>,
    /// Per layer: how far behind the step's hour its product stands, which decides what this step
    /// waits for and what the run reports ([`standing`]).
    standing: standing::Standings,
    /// What the publication being made waited before it began ([`Waited`]), for its record.
    waited: Waited,
    /// What its thread is doing now, read by a display frame that could not take it ([`Doing`]).
    doing: Arc<Doing>,
    /// Pictures it has sent its layers away to compose: a timed layer's change sets are dated with
    /// it, and a frame writes none that an older picture was composed for
    /// ([`crate::layers::Installs::install`]).
    composed: Arc<AtomicU64>,
}

/// Of the steps applied this frame and those not applied yet (`later`: taken and waiting, then made
/// past the head), what a layer fed ahead is handed, each once and in order. Not applied yet: only
/// the epoch applied, up to a reset or a step that resets or renumbers a table it reads (it composes
/// there), and up to a step of no hours (a join's, or a stalled table's catch-up, which must reach
/// the worker before the steps after it): such a step is handed when applied.
fn offer(applied: &[Arc<Step>], later: &[Arc<Step>], epoch: u64, reads: &BTreeSet<usize>, offered: &mut Option<Hour>, out: &mut Vec<Arc<Step>>) {
    out.clear();
    for step in applied {
        if step.first() > step.last() {
            out.push(Arc::clone(step));
        } else if offered.is_none_or(|offered| step.last() > offered) {
            *offered = Some(step.last());
            out.push(Arc::clone(step));
        }
    }
    for step in later {
        if step.first() > step.last() || step.epoch() != epoch || step.reset() || step.tables().iter().any(|changes| (changes.reset() || changes.renumber().is_some()) && reads.contains(&changes.table())) {
            break;
        }
        if offered.is_none_or(|offered| step.last() > offered) {
            *offered = Some(step.last());
            out.push(Arc::clone(step));
        }
    }
}

/// The overlay of the step joined and its tables given to the counter layers in `layers`, which
/// settle over them: the pieces of the side the join and the gift need, apart, so the mirrors stay
/// borrowed around it. Whether the overlay joined stands at the step's hour.
fn join_overlay(overlay: Option<&mut overlay::Overlay>, layers: &[Box<dyn Layer>], perspective: &viewer_foundation::map::perspective::Choice, branch: Option<replay_data::BranchId>, settling: Settling) -> bool {
    let Some(overlay) = overlay else { return false };
    let at_hour = overlay.join(settling);
    if let (Some(branch), Some((tables, rosters))) = (branch, overlay.tables().map(Arc::clone).zip(overlay.rosters())) {
        crate::layers::counter_kinds::give(layers, perspective, tables.vision(), tables.counters(), branch, move || rosters as crate::layers::air_mission_counters::Passes);
    }
    at_hour
}

impl MapSide {
    /// The map's reader of `entry` with `layers` drawn at `view`; the playhead is sought to `hour`
    /// unless another reader already holds it.
    pub fn new(entry: Arc<Entry>, mut layers: Vec<Box<dyn Layer>>, view: View) -> io::Result<Self> {
        let names = layers.iter().map(|layer| layer.name()).collect();
        // The store is given before the first declaration: a layer whose maker starts as it is
        // placed opens its reader there, so its tables are in the first declaration.
        let views = views::Store::new(&entry);
        for layer in &mut layers {
            layer.views(&views)?;
        }
        let Declaring { union: declared, reads, upstream, each } = declare(&entry, &mut layers, &view);
        let placed = layers.iter_mut().map(|layer| layer.placed(&view)).collect();
        // The makers' tables are declared beside the layers', never as a layer's reads: a table
        // only a maker reads must not make its layer compose when it arrives.
        let declared_layers = declared;
        let for_the_makers = for_the_makers(&entry);
        let declared = merged(&declared_layers, &for_the_makers);
        let reader = entry.playhead.declare(to_declared(&declared), Some(mirror::rebuild(Arc::clone(&entry.pages))))?;
        let consumer = entry.playhead.consumer(reader);
        let reads_len = reads.len();
        // A timed layer settles between the mirrors' steps, where every other layer's product
        // stands at another hour: one that reads another layer, or that another reads, is refused
        // here rather than showing a mixture.
        let timed = layers.iter().any(|layer| layer.timed());
        if timed {
            for (at, layer) in layers.iter().enumerate() {
                let read_by_others = upstream.iter().enumerate().any(|(other, uppers)| other != at && uppers.contains(&at));
                if layer.timed() && (!upstream[at].is_empty() || read_by_others) {
                    return Err(io::Error::other(format!("The map's layer {} is timed and reads or is read by another layer", layer.name())));
                }
            }
        }
        entry.maps.fetch_add(1, Ordering::AcqRel);
        Ok(Self {
            entry,
            reader,
            consumer,
            view,
            declared,
            declared_each: each,
            placed,
            ready: vec![false; reads.len()],
            recompose: vec![false; reads.len()],
            upstream,
            reads,
            names,
            mirrors: Some(Arc::default()),
            views,
            layers_awake: awake_mask(&declared_layers),
            declared_layers,
            for_the_makers,
            layers,
            worker: Worker::new()?,
            composing: false,
            pending: VecDeque::new(),
            taken: Vec::new(),
            view_later: None,
            sizes: Default::default(),
            clear: Vec::new(),
            shown: Vec::new(),
            applied: None,
            drawn: None,
            oldest: None,
            untimed: (None, None),
            last: Frame::default(),
            frame: 0,
            failure: None,
            branch: None,
            resets_taken: 0,
            resets_shown: 0,
            mode: None,
            mode_settled: Vec::new(),
            stepped: false,
            #[cfg(any(test, feature = "test-support"))]
            settles_held: Arc::default(),
            sight: None,
            overlay: None,
            vision: overlay::ShownVision::default(),
            compose_ns: Vec::new(),
            epoch: 0,
            cancel: Arc::default(),
            superseded: false,
            fresh: Vec::new(),
            reset_shown: false,
            judged_shown: Vec::new(),
            judged_resets: 0,
            offered: Vec::new(),
            resteps: Vec::new(),
            standing: standing::Standings::new(reads_len),
            waited: Waited::default(),
            doing: Arc::default(),
            composed: Arc::default(),
        })
    }

    /// Every timed layer's installer, taken once as the side goes to its thread: the display
    /// frame owns them from here on and writes each hour's change set as its head reaches it,
    /// whether or not this side's thread holds the window ([`crate::layers::Installs`]).
    pub fn take_installs(&mut self) -> Vec<Box<dyn Installs>> {
        self.layers.iter_mut().filter_map(|layer| layer.installs()).collect()
    }

    /// What its thread is doing, for the frame that could not take it ([`Doing`]).
    pub fn doing(&self) -> &Arc<Doing> {
        &self.doing
    }

    /// Pictures it has composed, which date a timed layer's change sets: the display frame writes
    /// none of a picture composed before the one it draws.
    pub fn composed(&self) -> &Arc<AtomicU64> {
        &self.composed
    }

    /// Each layer's name, in registration order, whether or not its parts are home.
    pub fn names(&self) -> &[&'static str] {
        &self.names
    }

    pub fn entry(&self) -> &Arc<Entry> {
        &self.entry
    }

    /// The content whose provinces and states the paced count of settled entities in view reads.
    pub fn set_sight(&mut self, content: Arc<crate::document::Content>) {
        self.sight = Some(content);
    }

    /// The content whose art its layers draw.
    pub(super) fn sight(&self) -> Option<&Arc<crate::document::Content>> {
        self.sight.as_ref()
    }

    /// The open map's overlay is made within this side's steps, over `content`.
    pub fn set_overlay(&mut self, content: Arc<crate::document::Content>) {
        self.overlay = Some(Box::new(overlay::Overlay::new(content)));
    }

    pub fn overlay(&self) -> Option<&overlay::Overlay> {
        self.overlay.as_deref()
    }

    /// The vision its layers show, as the step that settled the vision layer copied it out; why
    /// there is none, by name.
    pub fn vision(&self) -> Result<Arc<viewer_foundation::map::perspective::Vision>, String> {
        self.vision.kept()
    }

    /// The frame's newest ask of the overlay, taken at the next step.
    pub fn ask_overlay(&mut self, ask: overlay::Ask) {
        if let Some(overlay) = self.overlay.as_deref_mut() {
            overlay.ask(ask);
        }
    }

    /// The overlay the last step made; the frame holds it from then on.
    pub fn take_overlay(&mut self) -> Option<overlay::Made> {
        self.overlay.as_deref_mut().and_then(overlay::Overlay::take)
    }

    /// The step asks the overlay's maker for the overlay of its hour, at its apply: its rows, its
    /// tables and its layout are then made while the layers of this step settle. `whole`: the rows
    /// of exactly this hour, waited for however long they take. `lay`: the overlay laid out over
    /// the tables asked for, which a whole settle asks for only once the counter layers have
    /// settled over them. Whether anything was asked for.
    fn ask_overlay_step(&mut self, whole: bool, lay: bool, moved: Arc<viewer_foundation::map::counter_move::MovedHours>) -> bool {
        if self.overlay.is_none() || self.composing || self.superseded {
            return false;
        }
        let keys = side::LayInKeys::of(self);
        let lay_ins = Arc::new(side::LayIns::of(self));
        let published = crate::layers::counter_kinds::published(self);
        let draws = crate::layers::counter_kinds::draws(self);
        let left = crate::document::LeftToLayers::of(Some(self));
        let draws_labels = self.draws("labels");
        let engaged = self
            .layers
            .iter()
            .find_map(|layer| layer.any().downcast_ref::<crate::layers::engagements::Engagements>())
            .and_then(crate::layers::engagements::Engagements::history)
            .map(Arc::clone);
        // A saved frame's own hour: the vision layer's as it stands, waited for. A paced step's is
        // the one last copied out, which the settle of its own vision layer asks for.
        let vision = if whole { self.copy_vision(true) } else { self.vision.kept_for(&self.view.perspective) };
        let (applied, branch) = (self.applied, self.branch);
        let (perspective, views) = (&self.view.perspective, &self.views);
        let overlay = self.overlay.as_deref_mut().expect("held above");
        overlay.ask_step(overlay::Lend { views, applied, branch, perspective, vision, engaged, draws_labels, lay_ins, keys, published, draws, left, moved, whole, lay })
    }

    /// The hours of `step` at which the recording moved the movers the drawn layers place, and the
    /// province each hour left each in, added to `into`: the overlay's layout then gives every mover
    /// a place for each hour its own rows moved it, where this publication's rows alone give one
    /// for the interval's end. Empty while no drawn layer reads a table's hours
    /// ([`crate::layers::Inputs::hourly`]), and for a seek's reset, a piece's crossing and a
    /// catch-up, which carry no hours.
    ///
    /// Read against the mirrors as they stand before `step` is applied: its hourly writes name
    /// slots as they were inside it, before a later step reuses a slot or the block's packing moves
    /// a row.
    fn moved_before(&self, step: &Step, into: &mut viewer_foundation::map::counter_move::MovedHours) {
        if let Some(mirrors) = self.mirrors.as_deref() {
            for (at, layer) in self.layers.iter().enumerate() {
                if self.reads[at].is_ok() && self.ready[at] {
                    layer.moved_at(step, mirrors, into);
                }
            }
        }
    }

    /// The counter kinds' first layer, for the step that gives the overlay's tables to those
    /// layers just before they settle; none for a map drawing no counter kind.
    fn overlay_counters_at(&mut self) -> Option<usize> {
        let (overlay, names) = (self.overlay.as_deref_mut(), &self.names);
        overlay.and_then(|overlay| overlay.counters_at(names))
    }

    /// [`join_overlay`] outside a step's settle: a stepped frame's whole overlay.
    fn join_overlay(&mut self, settling: Settling) -> bool {
        let at = self.overlay_counters_at().unwrap_or(0);
        join_overlay(self.overlay.as_deref_mut(), &self.layers[at..], &self.view.perspective, self.branch, settling)
    }

    /// The overlay laid out again over the art that arrived while a saved frame drew: the maker is
    /// asked for the hour's layout once more and waited for. Refused by name, so a stopped picture
    /// is never saved with the layout the art moved under.
    pub fn lay_overlay_again(&mut self) -> Result<overlay::Made, String> {
        // What the ask waits on is made elsewhere and arrives on its own: the vision of the step
        // being copied out of its layer, a compose, a step that superseded this one. A whole
        // settle waits for them too ([`Self::overlay_whole`]).
        let until = Instant::now() + HANDS_BACK;
        while !self.ask_overlay_step(true, true, Arc::default()) {
            if Instant::now() >= until {
                let loading = self.overlay.as_deref().map(overlay::Overlay::loading).unwrap_or_default().join("; ");
                return Err(format!(
                    "the overlay's maker was not asked for hour {:?}: composing {}, superseded {}, branch {:?}, an overlay {}{}",
                    self.applied,
                    self.composing,
                    self.superseded,
                    self.branch,
                    self.overlay.is_some(),
                    if loading.is_empty() { String::new() } else { format!("; {loading}") },
                ));
            }
            std::thread::sleep(std::time::Duration::from_micros(200));
        }
        self.join_overlay(Settling::Whole);
        let hour = self.applied;
        self.take_overlay().ok_or_else(|| {
            let overlay = self.overlay.as_deref().expect("asked above");
            format!(
                "the overlay's maker laid nothing out for hour {hour:?}: it stands at {:?}, missing {:?}{}",
                overlay.hour(),
                overlay.missed(),
                match overlay.loading().join("; ") {
                    loading if loading.is_empty() => String::new(),
                    loading => format!("; {loading}"),
                },
            )
        })
    }

    /// No layer settles until the guard returned is dropped: a test then keeps a layer's marks
    /// across several steps and sees the settle after them make the newest values.
    #[cfg(any(test, feature = "test-support"))]
    pub fn hold_settles(&self) -> SettlesHeld {
        self.settles_held.store(true, Ordering::Release);
        SettlesHeld(Arc::clone(&self.settles_held))
    }

    /// The map's playback anchors: a seek, play at `speed` game hours a second, or a pause.
    pub fn anchor(&self, hour: Hour, playing: bool, speed: f64, sought: bool) {
        self.entry.anchor(hour, playing, speed, sought);
    }

    /// The clock's head now.
    pub fn head(&self, now: Instant) -> Hour {
        self.entry.playhead.head(now)
    }

    /// The hour a publication begun now is for, its head standing at `at`: the hour the head will
    /// have reached `ahead` from now, so that the picture stands at the head's own hour by the
    /// time it is drawn rather than a whole making behind it. None while the head stands still
    /// (paused, held at a seek, at the recording's last hour), where that hour is `at` itself.
    pub fn head_from(&self, at: Hour, ahead: std::time::Duration) -> Option<Hour> {
        self.entry.playhead.head_from(at, ahead)
    }

    /// Whether a publication at `head` would show an hour the one before did not: a step it has
    /// taken and not applied, or one in the playhead's ring, ends at or before `head`. The ring
    /// held by another thread answers yes: the publication finds out.
    pub fn reaches(&self, head: Hour) -> bool {
        self.pending.iter().any(|step| step.last() <= head) || self.consumer.reaches(head) != Some(false)
    }

    /// Per table the map mirrors at its view: the columns and keys its drawn layers declared,
    /// and those layers by name.
    pub fn readers(&self) -> Vec<(TableId, usize, bool, Vec<&'static str>)> {
        self.declared
            .iter()
            .map(|(&table, read)| {
                let layers = self.names.iter().zip(&self.reads).filter(|(_, reads)| reads.as_ref().is_ok_and(|tables| tables.contains(&table))).map(|(name, _)| *name).collect();
                (TableId::ALL[table], read.columns.len(), read.keyed, layers)
            })
            .collect()
    }

    /// Why each layer is refused, by name; none for a layer drawn.
    pub fn refusals(&self) -> impl Iterator<Item = (&'static str, &str)> + '_ {
        self.names.iter().zip(&self.reads).filter_map(|(name, reads)| reads.as_ref().err().map(|why| (*name, why.as_str())))
    }

    /// Draws at `view`: layers whose columns change are declared again, and a table whose columns
    /// are new arrives as a reset of that table.
    pub fn set_view(&mut self, view: View) -> io::Result<()> {
        if view == self.view {
            return Ok(());
        }
        if self.composing || self.superseded {
            self.view_later = Some(view);
            return Ok(());
        }
        if view.same_declaration(&self.view) && {
            let _lap = replay_frame::cpu::scope("side.placed", Some("other"));
            self.place(&view)
        } {
            return Ok(());
        }
        let _lap = replay_frame::cpu::scope("side.declare", Some("other"));
        let Declaring { union: declared, reads, upstream, each } = declare(&self.entry, &mut self.layers, &view);
        if view.map_mode != self.view.map_mode {
            self.mode_settled.clear();
        }
        self.view = view;
        for (at, read) in reads.iter().enumerate() {
            if read.is_ok() && self.reads[at].is_err() {
                self.ready[at] = false;
                self.recompose[at] = true;
            }
        }
        self.reads = reads;
        self.upstream = upstream;
        self.declared_each = each;
        let placed_at = &self.view;
        self.placed = self.layers.iter_mut().map(|layer| layer.placed(placed_at)).collect();
        if let Some(mirrors) = self.mirrors.as_deref() {
            for (at, layer) in self.layers.iter_mut().enumerate() {
                if self.reads[at].is_ok() && self.ready[at] {
                    layer.viewed(&self.view, mirrors);
                }
            }
        }
        // A layer whose own columns changed composes again, whatever the mirrors already hold for
        // the makers: its inputs are new even where its table is not.
        let moved: BTreeSet<usize> = declared.iter().filter(|(table, read)| self.declared_layers.get(table) != Some(read)).map(|(&table, _)| table).chain(self.declared_layers.keys().filter(|table| !declared.contains_key(table)).copied()).collect();
        self.declared_layers = declared;
        self.layers_awake = awake_mask(&self.declared_layers);
        self.remirror(&moved)
    }

    /// The mirrors declared for the drawn layers' columns and the makers' tables together: a
    /// table no longer read by a layer is let go (the makers' stay), and a layer whose own
    /// declaration changed waits for its reset.
    fn remirror(&mut self, moved: &BTreeSet<usize>) -> io::Result<()> {
        // Its columns are already in the mirrors, so no reset brings them: the layer is composed
        // again at the next frame instead, from the mirrors as they stand.
        for at in 0..self.reads.len() {
            if self.reads[at].as_ref().is_ok_and(|tables| tables.iter().any(|table| moved.contains(table))) {
                (self.ready[at], self.recompose[at]) = (false, true);
            }
        }
        let declared = merged(&self.declared_layers, &self.for_the_makers);
        if declared == self.declared {
            return Ok(());
        }
        self.views.settled();
        if let Some(mirrors) = self.mirrors.as_mut().and_then(Arc::get_mut) {
            for table in self.declared.keys().filter(|table| !declared.contains_key(table)) {
                if let Some(old) = mirrors.remove(*table) {
                    self.worker.drop_later(old);
                }
            }
        }
        let changed: BTreeSet<usize> = declared.iter().filter(|(table, read)| self.declared.get(table) != Some(read)).map(|(&table, _)| table).collect();
        for (at, reads) in self.reads.iter().enumerate() {
            if reads.as_ref().is_ok_and(|tables| tables.iter().any(|table| changed.contains(table))) {
                self.ready[at] = false;
            }
        }
        self.declared = declared;
        self.entry.playhead.redeclare(self.reader, to_declared(&self.declared))
    }

    /// The camera moved and nothing else of the view did: each layer takes the new placement and what
    /// of its declaration the camera decides is compared with the last; a layer that reads the
    /// camera in `inputs` only declares again alone, against its own last declaration. False when any
    /// declaration changed: the view is then declared whole.
    fn place(&mut self, view: &View) -> bool {
        let kinds = self.entry.kinds();
        for (at, layer) in self.layers.iter_mut().enumerate() {
            match layer.placed(view) {
                Some(key) => {
                    if self.placed.get(at) != Some(&Some(key)) {
                        return false;
                    }
                }
                None => {
                    let mut inputs = Inputs::new(kinds, layer.name());
                    if layer.inputs(view, &mut inputs).is_err() {
                        return false;
                    }
                    let after = inputs.upstream().to_vec();
                    if self.declared_each.get(at) != Some(&Some((inputs.into_tables(), after))) {
                        return false;
                    }
                }
            }
        }
        self.view = view.clone();
        if let Some(mirrors) = self.mirrors.as_deref() {
            for (at, layer) in self.layers.iter_mut().enumerate() {
                if self.reads[at].is_ok() && self.ready[at] {
                    layer.viewed(&self.view, mirrors);
                }
            }
        }
        true
    }

    /// One step at `head`: the steps it reached taken, each applied to the mirrors and then to the
    /// layers, every layer settled to its last entity and the products made from rows at the head
    /// joined within one budget, so the picture it leaves stands whole at one hour.
    pub fn frame(&mut self, head: Hour) -> io::Result<Frame> {
        self.frame_within(head, crate::layers::Settling::step(), true)
    }

    /// [`Self::frame`] for a publication the head stands still for: it waits for every product its
    /// layers owe, because no later hour is coming and the picture a watcher is left looking at
    /// must be whole ([`crate::layers::Settling::still`]).
    pub fn frame_still(&mut self, head: Hour) -> io::Result<Frame> {
        self.frame_within(head, crate::layers::Settling::still(), true)
    }

    /// [`Self::frame`] within `settling`: the hours between the head and a stepped frame's own are
    /// caught up with no budget, because only the picture at that frame's hour is settled whole.
    /// `lay`: whether the overlay of the step is laid out as well as made. A caught-up step makes
    /// the tables its counter layers settle over and no layout, so the hour a saved frame shows is
    /// laid out once, as a step's is.
    fn frame_within(&mut self, head: Hour, settling: crate::layers::Settling, lay: bool) -> io::Result<Frame> {
        self.frame += 1;
        let stepping = Instant::now();
        let doing = Arc::clone(&self.doing);
        let mut watch = Watch::new(&doing);
        timeline::step();
        // The views the step before posted are made before the mirrors are written again: the
        // side holds them alone while it applies.
        self.views.settled();
        timeline::views_settled();
        let mut frame = Frame { made: self.frame, head, waited: self.waited, layers: vec![LayerFrame::default(); self.names.len()], ..Frame::default() };
        watch.lap(&mut frame.stages, 0);
        let take_lap = replay_frame::cpu::scope("side.take", Some("other"));
        let playhead = &self.entry.playhead;
        self.consumer.advance(head);
        if self.composing {
            if playhead.epoch() != self.epoch {
                self.cancel.store(true, Ordering::Release);
            }
            match self.worker.done.try_recv() {
                Ok(away) if away.cancel.load(Ordering::Acquire) => {
                    frame.superseded = true;
                    self.take_home(*away);
                }
                Ok(away) => {
                    frame.swapped = Some(away.results.iter().map(|(_, ns, _)| *ns).collect());
                    self.swap_in(*away)?;
                }
                Err(TryRecvError::Empty) => {}
                Err(TryRecvError::Disconnected) => return Err(io::Error::other("The playhead host's worker stopped")),
            }
        }
        // The swap frame shows the composed picture alone, at the hour its reset was composed for:
        // the steps since (the clock ran on meanwhile) are taken and applied from the next frame.
        let swapped = frame.swapped.is_some();
        // Composing, the steps are still taken (applied after the swap) while another map shares
        // this playhead; alone, they wait in the ring, kept for this map, and hold no other reader.
        if !swapped && (!self.composing || self.entry.maps.load(Ordering::Acquire) > 1) {
            let (started, cycles) = (Instant::now(), replay_frame::cpu::thread_cycles());
            self.consumer.take(head, &mut self.taken);
            frame.take_ns += started.elapsed().as_nanos() as u64;
            frame.take_cycles += replay_frame::cpu::thread_cycles().saturating_sub(cycles);
        }
        let mut steps: VecDeque<Arc<Step>> = VecDeque::new();
        if !swapped {
            // A step of a seek since superseded is never applied: the newer seek's reset replaces it.
            let epoch = self.entry.playhead.epoch();
            self.pending.retain(|step| step.epoch() == epoch);
            steps = std::mem::take(&mut self.pending);
            steps.extend(self.taken.drain(..));
        }
        drop(take_lap);
        watch.lap(&mut frame.stages, 1);
        let apply_lap = replay_frame::cpu::scope("side.apply", Some("other"));
        let mut compose = vec![false; self.layers.len()];
        let mut applied = Vec::with_capacity(steps.len());
        let (resets_before, applied_before) = (self.resets_taken, self.applied);
        // Steps wait, in order: those taken with a reset for the frame after its swap (the compose
        // is of the mirrors at the reset's hour, so the first picture after a seek is the hour
        // sought), and any past the head this frame was given for a frame whose head reaches it.
        let (mut waiting, mut after_reset, mut crossing) = (self.composing, false, false);
        let mut moved = viewer_foundation::map::counter_move::MovedHours::default();
        while let Some(step) = steps.pop_front() {
            waiting |= step.last() > head;
            if waiting {
                self.pending.push_back(step);
                continue;
            }
            let reset = step.reset();
            if self.overlay.is_some() {
                self.moved_before(&step, &mut moved);
            }
            crossing |= self.take_step(&step, &mut frame, &mut compose);
            applied.push(step);
            (waiting, after_reset) = (waiting || reset, after_reset || reset);
        }
        // Superseded, only the newer seek's reset composes: the mirrors are not there before it.
        if self.superseded && self.resets_taken == resets_before {
            compose.fill(false);
        } else if !self.composing {
            for (wanted, again) in compose.iter_mut().zip(&mut self.recompose) {
                *wanted |= std::mem::take(again);
            }
            // A layer that reads no table waits for no reset (a map joining an anchored playhead
            // gets joins only): once the map has taken a step, it composes.
            if self.applied.is_some() {
                for (at, wanted) in compose.iter_mut().enumerate() {
                    *wanted |= !self.ready[at] && self.reads[at].as_ref().is_ok_and(BTreeSet::is_empty);
                }
            }
        }
        // Tables declared together can arrive in different steps (one joined from the file, one
        // landed from memory): a layer composes once every table it reads is home, never with one
        // missing, and so once.
        let home = |compose: &mut [bool], mirrors: Option<&Mirrors>| {
            if let Some(mirrors) = mirrors {
                for (wanted, reads) in compose.iter_mut().zip(&self.reads) {
                    *wanted &= reads.as_ref().is_ok_and(|tables| tables.iter().all(|&table| mirrors.get(TableId::ALL[table]).is_some()));
                }
            }
        };
        home(&mut compose[..], self.mirrors.as_deref());
        for at in 0..compose.len() {
            if self.reads[at].is_ok() && self.upstream[at].iter().any(|&before| compose[before]) {
                compose[at] = true;
            }
        }
        home(&mut compose[..], self.mirrors.as_deref());
        let composed_here = crossing && !after_reset && !self.composing && !self.superseded && compose.contains(&true);
        if composed_here {
            self.compose_here(&compose, &mut frame);
        }
        drop(apply_lap);
        watch.lap(&mut frame.stages, 2);
        timeline::applied();
        timeline::settling();
        // Per layer, how far behind the step's hour its product stood after the step before: the
        // one rule this step's settles and joins wait by.
        if self.composing || self.superseded || self.resets_taken != resets_before {
            self.standing.reset();
        } else {
            self.standing.step(self.applied, Instant::now());
        }
        let settles_lap = replay_frame::cpu::scope("side.settles", Some("other"));
        if !self.composing {
            // Composed here, the layers composed are left out of this step as a reset's are, and
            // settle at the next publication; everything else settles and the hour advances.
            let composing = compose.iter().any(|&wanted| wanted) && !composed_here;
            // The overlay is a maker of this step: asked here, with the apply done and the hour
            // known, so its rows, its tables and its layout are made while the layers settle, and
            // joined at the counter kinds' first layer, which settles over the tables it published.
            let overlay_at = (!composing && self.overlay.is_some()).then(|| self.overlay_counters_at().unwrap_or(0));
            if overlay_at.is_some() {
                if viewer_foundation::view_record::on() {
                    let (movers, hours) = moved.counts();
                    viewer_foundation::view_record::add("moved", "movers", movers as u64);
                    viewer_foundation::view_record::add("moved", "hours", hours as u64);
                }
                self.ask_overlay_step(settling.strict(), lay, Arc::new(std::mem::take(&mut moved)));
            }
            // The sets a maker reads whose rows the steps moved, posted to the view store's make
            // threads: they are made while the layers that join no maker settle below, and each
            // maker is given its frame the moment its own sets are made. Composing, nothing is
            // made and no maker is asked, as nothing is settled.
            if !composing && let Some(hour) = self.applied {
                let _views = replay_frame::cpu::scope("side.views", Some("other"));
                let (store, held) = (&mut self.views, self.mirrors.as_ref().expect("the mirrors are home"));
                store.post(held, hour, self.branch.unwrap_or_default(), self.epoch, false);
            }
            let mirrors = self.mirrors.as_deref().expect("the mirrors are home while not composing");
            self.clear.resize(self.layers.len(), false);
            for (layer, clear) in self.layers.iter_mut().zip(&mut self.clear) {
                if std::mem::take(clear) {
                    layer.clear_changed();
                }
            }
            let mut settled = vec![false; self.layers.len()];
            self.offered.resize(self.layers.len(), None);
            self.resteps.resize(self.layers.len(), 0);
            let (mut lent, mut handed): (Option<Vec<Arc<Step>>>, Vec<Arc<Step>>) = (None, Vec::new());
            // Per layer settled: what it carried after, for its in-view count once every layer settled.
            let mut counted: Vec<(usize, usize)> = Vec::with_capacity(self.layers.len());
            let mut mark = replay_frame::cpu::thread_cycles();
            let applied_now = self.applied;
            let composed = self.composed.load(Ordering::Acquire);
            // Per layer: a maker's product joins its settle, so it asked its worker at the apply
            // and is settled in the pass after. A layer that reads one of those is settled there
            // too: it never reads a half-settled upstream.
            let mut joins = vec![false; self.layers.len()];
            // The timed layers settled: dated once every layer settled, so what they take from
            // another layer's product ([`hand_naval`]) is that of this step's hour.
            let mut dated = vec![false; self.layers.len()];
            // Registration order: a layer that reads another is registered after it.
            for at in 0..self.layers.len() {
                if overlay_at == Some(at) {
                    join_overlay(self.overlay.as_deref_mut(), &self.layers[at..], &self.view.perspective, self.branch, settling);
                }
                if !(self.reads[at].is_ok() && self.ready[at]) || compose[at] || self.superseded {
                    // A layer it draws that is not composed yet: drawn late, counted in the flight record.
                    if self.reads[at].is_ok() && (!self.ready[at] || compose[at]) {
                        viewer_foundation::view_record::add("layers", "frames_not_ready", 1);
                    }
                    continue;
                }
                let bound = self.layers[at].behind_bound();
                let settling = settling.behind_bound(self.standing.behind(at), bound);
                let (before, rest) = self.layers.split_at_mut(at);
                let (layer, upstream) = (&mut rest[0], Upstream::new(before));
                let part = &mut frame.layers[at];
                let timed = layer.timed();
                let started = Instant::now();
                for step in applied.iter() {
                    layer.apply_with(step, mirrors, &upstream);
                }
                part.apply_ns += started.elapsed().as_nanos() as u64;
                if !applied.is_empty() {
                    let now = replay_frame::cpu::thread_cycles();
                    part.apply_cycles += now.saturating_sub(std::mem::replace(&mut mark, now));
                }
                let fed = layer.wants_ahead();
                if fed && self.applied.is_some() {
                    let started = Instant::now();
                    let lent = lent.get_or_insert_with(|| {
                        let mut steps: Vec<Arc<Step>> = self.pending.iter().cloned().collect();
                        self.consumer.ahead(&mut steps);
                        steps
                    });
                    // A step rebuilt for a new pace voids the steps lent before it past the hour applied.
                    // A lent step may span this frame's hour while the frame applied rebuilt ones: the
                    // steps are handed again from the hour applied before this frame.
                    let restep = applied.iter().chain(lent.iter()).map(|step| step.restep()).max().unwrap_or(0);
                    if restep > self.resteps[at] {
                        if self.offered[at].is_some_and(|offered| self.applied.is_none_or(|applied| offered > applied)) {
                            layer.void_ahead(mirrors, applied_before);
                            frame.voided += 1;
                            self.offered[at] = applied_before;
                        }
                        self.resteps[at] = restep;
                    }
                    let reads = self.reads[at].as_ref().expect("a drawn layer reads its tables");
                    offer(&applied, lent, self.epoch, reads, &mut self.offered[at], &mut handed);
                    if !handed.is_empty() {
                        layer.ahead(&handed);
                        handed.clear();
                    }
                    part.ahead_ns = started.elapsed().as_nanos() as u64;
                }
                // An upstream not composed is half-made: a dependent reads its published product.
                let held = self.upstream[at].iter().any(|&upper| !self.ready[upper]);
                part.dirty = layer.carried().entities;
                #[cfg(any(test, feature = "test-support"))]
                let held = held || self.settles_held.load(Ordering::Acquire);
                if timed {
                    // Settled once for the publication, as every layer is; what it changed is then
                    // dated hour by hour from the rows of those hours, and a publication that
                    // applied no step still dates its own, so a view change or a held product is
                    // answered at the publication's hour.
                    settled[at] = true;
                    let started = Instant::now();
                    layer.settle_with(mirrors, &upstream, settling)?;
                    dated[at] = applied_now.is_some();
                    part.settle_ns += started.elapsed().as_nanos() as u64;
                    let now = replay_frame::cpu::thread_cycles();
                    part.settle_cycles = now.saturating_sub(std::mem::replace(&mut mark, now));
                } else if !composing && !held {
                    joins[at] = layer.ask(mirrors, &upstream)? || self.upstream[at].iter().any(|&upper| joins[upper]);
                    if !joins[at] {
                        settled[at] = true;
                        let started = Instant::now();
                        layer.settle_with(mirrors, &upstream, settling)?;
                        part.settle_ns = started.elapsed().as_nanos() as u64;
                        let now = replay_frame::cpu::thread_cycles();
                        part.settle_cycles = now.saturating_sub(std::mem::replace(&mut mark, now));
                    }
                }
                // The vision the rest of the picture is made over: copied out of the layer in
                // the step that settled it, before the overlay's tables and the counter layers
                // read it.
                if self.names[at] == "vision" {
                    let shown = layer.any().downcast_ref::<crate::layers::vision::Vision>();
                    drop(self.vision.shown(shown, applied_now, true, false));
                }
                part.ready = true;
                if !joins[at] {
                    counted.push((at, layer.carried().entities));
                    if fed && !applied.is_empty() {
                        part.ready_at_hour = Some(layer.published() == crate::layers::Published::Current);
                    }
                    let published = layer.published();
                    self.standing.settled(at, self.names[at], published, Instant::now(), bound)?;
                }
            }
            if let Some(hour) = applied_now.filter(|_| dated.contains(&true)) {
                hand_naval(&self.names, &mut self.layers, &self.reads, &self.ready);
                for at in (0..self.layers.len()).filter(|&at| dated[at]) {
                    let started = Instant::now();
                    self.layers[at].timed_at(hour, composed)?;
                    frame.layers[at].settle_ns += started.elapsed().as_nanos() as u64;
                }
            }
            timeline::joining();
            watch.lap(&mut frame.stages, 3);
            // The settles a maker's product joins, in registration order: their builds ran while
            // the layers above settled, so each takes what its maker has published and ends. One
            // whose product has stood at the bound waits for it; the rest join at the step after.
            for at in 0..self.layers.len() {
                if !joins[at] {
                    continue;
                }
                let bound = self.layers[at].behind_bound();
                let settling = settling.behind_bound(self.standing.behind(at), bound);
                let (before, rest) = self.layers.split_at_mut(at);
                let (layer, upstream) = (&mut rest[0], Upstream::new(before));
                let part = &mut frame.layers[at];
                settled[at] = true;
                let started = Instant::now();
                layer.join(mirrors, &upstream, settling)?;
                part.settle_ns = started.elapsed().as_nanos() as u64;
                let now = replay_frame::cpu::thread_cycles();
                part.settle_cycles = now.saturating_sub(std::mem::replace(&mut mark, now));
                counted.push((at, layer.carried().entities));
                if layer.wants_ahead() && !applied.is_empty() {
                    part.ready_at_hour = Some(layer.published() == crate::layers::Published::Current);
                }
                let published = layer.published();
                self.standing.settled(at, self.names[at], published, Instant::now(), bound)?;
            }
            // The hours have stopped for longer than a starving product could explain: every
            // drawn layer must stand at the hour the picture shows ([`standing::Standings`]).
            if !self.stepped && self.standing.hours_stopped() {
                let drawn: Vec<(usize, &'static str)> = (0..self.layers.len()).filter(|&at| self.reads[at].is_ok() && self.ready[at] && !compose[at]).map(|at| (at, self.names[at])).collect();
                self.standing.stopped_whole(&drawn)?;
            }
            watch.lap(&mut frame.stages, 4);
            // The harness's in-view count, under one label for every layer: a label's two reads of
            // the thread's cycle time each, not two per layer.
            {
                let _in_view = replay_frame::cpu::scope("side.in-view", Some("other"));
                let content = self.sight.clone();
                let sight = crate::layers::Sight::of(content.as_deref(), &self.view);
                for (at, left) in counted {
                    let part = &mut frame.layers[at];
                    part.scope = self.layers[at].scope(&sight).or_else(|| settled[at].then(|| crate::layers::Scope { settled: part.dirty.saturating_sub(left) as u64, in_view: None }));
                }
            }
            watch.lap(&mut frame.stages, 5);
            // Every drawn layer's install prepared here, on this thread, so the frame's own window
            // only hands the bytes to the queue. Asked whether or not the layer settled this step:
            // one whose settle was held still owes the install what it settled before.
            {
                let _staging = replay_frame::cpu::scope("side.stage", Some("other"));
                for at in 0..self.layers.len() {
                    if !(self.reads[at].is_ok() && self.ready[at]) || compose[at] || self.superseded {
                        continue;
                    }
                    let started = Instant::now();
                    self.layers[at].stage()?;
                    frame.layers[at].stage_ns = started.elapsed().as_nanos() as u64;
                    let now = replay_frame::cpu::thread_cycles();
                    frame.layers[at].stage_cycles = now.saturating_sub(std::mem::replace(&mut mark, now));
                }
            }
            watch.lap(&mut frame.stages, 6);
            // What every drawn dependent has settled after is cleared at the next frame's start, so
            // between frames a layer's changes still read as this frame's.
            for upper in 0..self.layers.len() {
                let drawn = |at: usize| self.reads[at].is_ok() && self.ready[at];
                self.clear[upper] = (0..self.layers.len()).filter(|&at| drawn(at) && self.upstream[at].contains(&upper)).all(|at| settled[at]);
            }
            if composing {
                frame.resets += 1;
                self.send_away(compose);
            }
        }
        drop(settles_lap);
        let _shown_lap = replay_frame::cpu::scope("side.shown", Some("other"));
        // A step is taken once the head reaches its last hour; paused inside one, it never is.
        let playhead = &self.entry.playhead;
        if !self.composing
            && !self.superseded
            && !swapped
            && !self.pending.iter().any(|step| step.last() <= head)
            && (self.stepped || self.entry.paused())
            && playhead.holding().is_none()
            && let Some(applied) = self.applied
            && (applied > head || (applied < head && playhead.newest().is_some_and(|newest| newest > head) && self.consumer.reaches(head) == Some(false)))
        {
            self.entry.exact(head);
            frame.exact = true;
        }
        frame.composing = self.composing || self.superseded;
        if !frame.composing {
            self.resets_shown = self.resets_taken;
            self.update_shown();
        }
        for ((part, shown), fresh) in frame.layers.iter_mut().zip(&self.shown).zip(&self.fresh) {
            (part.shown, part.reset_shown) = (*shown, *fresh);
        }
        frame.reset_shown = self.reset_shown;
        (frame.drawn, frame.oldest) = (self.drawn, self.oldest);
        frame.side_ns = stepping.elapsed().as_nanos() as u64;
        watch.lap(&mut frame.stages, 7);
        frame.side_cycles = frame.stages.cycles.iter().sum();
        if let Some(overlay) = self.overlay.as_deref_mut() {
            overlay.stepped();
        }
        timeline::close(frame.composing);
        self.last = frame.clone();
        Ok(frame)
    }

    /// Each drawn layer's shown hour, and the map's drawn hour: the oldest of them, once every
    /// layer it declares is drawn (a layer waiting for the reset of a table it declared again is
    /// not). Paused or after a seek, the map is drawn at the head only when every layer shows it.
    /// The drawn hour leaves out the layers whose product is made on a cadence; the oldest hour on
    /// screen counts them too.
    fn update_shown(&mut self) {
        use crate::layers::Published;
        self.shown.resize(self.layers.len(), None);
        self.fresh.resize(self.layers.len(), false);
        self.mode_settled.resize(self.layers.len(), false);
        let ready = self.reads.iter().zip(&self.ready).all(|(reads, &ready)| ready || reads.is_err());
        let (mut whole, mut untimed_whole) = (ready, ready);
        let mut mode_settled = true;
        let (mut oldest, mut untimed_oldest): (Option<Hour>, Option<Hour>) = (None, None);
        let (mut every, mut untimed_every): (Option<Hour>, Option<Hour>) = (None, None);
        let (mut drawn, mut untimed_drawn) = (0, 0);
        let mut fresh = whole;
        for (at, layer) in self.layers.iter().enumerate() {
            if !(self.reads[at].is_ok() && self.ready[at]) {
                continue;
            }
            if layer.uploaded() {
                (self.shown[at], self.fresh[at]) = match layer.published() {
                    // A timed layer shows the hour of the newest change set the frames installed,
                    // which is the head's own, not the publication's.
                    Published::Current => (layer.shown_through().or(self.applied), true),
                    Published::At(hour) | Published::Cadence(hour) => (Some(hour), true),
                    Published::Nothing => (None, false),
                };
            }
            let timed = layer.timed();
            if let Some(hour) = self.shown[at] {
                every = Some(every.map_or(hour, |held| held.min(hour)));
                if !timed {
                    untimed_every = Some(untimed_every.map_or(hour, |held| held.min(hour)));
                }
            }
            if matches!(layer.published(), Published::Cadence(_)) {
                continue;
            }
            self.mode_settled[at] |= layer.uploaded() && layer.carried().entities == 0 && !matches!(layer.published(), Published::Nothing);
            mode_settled &= self.mode_settled[at];
            fresh &= self.fresh[at];
            drawn += 1;
            untimed_drawn += u32::from(!timed);
            match (self.shown[at], timed) {
                (Some(hour), false) => untimed_oldest = Some(untimed_oldest.map_or(hour, |held| held.min(hour))),
                (None, false) => untimed_whole = false,
                (Some(_), true) => {}
                (None, true) => {}
            }
            match self.shown[at] {
                Some(hour) => oldest = Some(oldest.map_or(hour, |held| held.min(hour))),
                None => whole = false,
            }
        }
        if whole {
            self.drawn = if drawn == 0 { self.applied } else { oldest };
            self.oldest = match (self.drawn, every) {
                (Some(drawn), Some(every)) => Some(drawn.min(every)),
                (drawn, every) => drawn.or(every),
            };
            if mode_settled && self.drawn.is_some() {
                self.mode = Some(self.view.map_mode.clone());
            }
        }
        if untimed_whole {
            let shown = if untimed_drawn == 0 { self.applied } else { untimed_oldest };
            self.untimed = (
                shown,
                match (shown, untimed_every) {
                    (Some(shown), Some(every)) => Some(shown.min(every)),
                    (shown, every) => shown.or(every),
                },
            );
        }
        self.reset_shown = fresh;
    }

    /// The hour the picture shows and the oldest hour on it, counting only the layers whose
    /// publication is one picture: the display frame takes the oldest of these and its own timed
    /// installers ([`Self::untimed`]).
    pub fn untimed_hours(&self) -> (Option<Hour>, Option<Hour>) {
        self.untimed
    }

    /// How long the publication the frame is taking took to reach it, for the frame's record.
    pub fn reached(&mut self, took: Option<std::time::Duration>) {
        self.last.reached_ns = took.map_or(0, |took| took.as_nanos() as u64);
    }

    /// What the publication about to be made waited for, for its record ([`Waited`]).
    pub fn waited(&mut self, waited: Waited) {
        self.waited = waited;
    }

    /// The last frame's record, its upload included.

    pub fn last_frame(&self) -> &Frame {
        &self.last
    }

    /// Each layer's name, whether it draws and whether its upload was left, as the last frame
    /// found them: why a frame was not whole.
    pub fn layer_states(&self) -> Vec<(&'static str, bool, bool)> {
        self.names.iter().enumerate().map(|(at, &name)| (name, self.reads[at].is_ok() && self.ready[at], self.last.layers.get(at).is_some_and(|part| part.upload_carried))).collect()
    }

    /// Why the map's drawn hour is what it is: the hour applied, the drawn hour, whether a reset
    /// composes, and per declared layer not refused its name, whether it is composed, the hour it
    /// shows, what it publishes, what it carries and whether its upload is done.
    pub fn shown_states(&self) -> String {
        let layers: Vec<String> = (0..self.layers.len())
            .filter(|&at| self.reads[at].is_ok())
            .map(|at| {
                let layer = &self.layers[at];
                format!("{} ready {} shown {:?} {:?} carried {} uploaded {}", self.names[at], self.ready[at], self.shown.get(at).copied().flatten(), layer.published(), layer.carried().entities, layer.uploaded())
            })
            .collect();
        format!("applied {:?}, drawn {:?}, composing {}, superseded {}, last head {}, exact {}; {}", self.applied, self.drawn, self.composing, self.superseded, self.last.head, self.last.exact, layers.join("; "))
    }

    /// Whether layer `name` is drawn: declared, composed and not refused.
    pub fn draws(&self, name: &str) -> bool {
        self.names.iter().enumerate().any(|(at, held)| *held == name && self.reads[at].is_ok() && self.ready[at])
    }

    /// Steps until every step to `head` is applied and composed, then everything settled whole:
    /// the one picture a stepped frame draws. Run by whoever owns the side (its own thread, a tool
    /// or a test), never by a frame: a frame waits for the picture and advances nothing.
    pub fn catch_up(&mut self, head: Hour) -> io::Result<()> {
        self.stepped = true;
        let started = Instant::now();
        loop {
            let newest = self.entry.playhead.newest().unwrap_or(head);
            self.frame_within(head.min(newest.max(self.applied.unwrap_or(newest))), crate::layers::Settling::step(), false)?;
            if !self.composing && !self.superseded && self.applied == Some(head) && self.ready.iter().zip(&self.reads).all(|(&ready, reads)| ready || reads.is_err()) {
                self.settle_all()?;
                // A layer that makes its product from rows at the head asks for the rows of this
                // hour in the settle above; the step after this one makes them, and the settle
                // after that makes its product. The picture is not whole until it has.
                if self.head_products_reached(head) && self.overlay_whole(head)? {
                    // The settles of the overlay's two passes moved what the layers show: the
                    // picture handed to the frame is the one those settles left.
                    self.update_shown();
                    return Ok(());
                }
            }
            if let Some(error) = self.entry.playhead.error() {
                return Err(io::Error::other(format!("The playhead stopped: {error}")));
            }
            if started.elapsed() > CATCHING_UP {
                return Err(io::Error::other(self.waiting_state(head)));
            }
            std::thread::sleep(std::time::Duration::from_micros(200));
        }
    }

    /// Why `head`'s picture has not arrived: what the layers, the overlay, this reader and the
    /// playhead hold. A layer not composed names the tables it reads whose mirror is not home: a
    /// compose waits for every one of them, so one that never lands holds the picture by itself.
    pub fn waiting_state(&self, head: Hour) -> String {
        let waiting: Vec<String> = self
            .names
            .iter()
            .enumerate()
            .filter(|(at, _)| !self.ready[*at] && self.reads[*at].is_ok())
            .map(|(at, name)| {
                let missing: Vec<String> = self.reads[at]
                    .iter()
                    .flatten()
                    .filter(|&&table| self.mirrors.as_deref().is_none_or(|mirrors| mirrors.get(TableId::ALL[table]).is_none()))
                    .map(|&table| format!("{:?}", TableId::ALL[table]))
                    .collect();
                match missing.is_empty() {
                    true => (*name).to_owned(),
                    false => format!("{name} (no mirror of {})", missing.join(", ")),
                }
            })
            .collect();
        let overlay = self.overlay.as_deref().map_or_else(String::new, |overlay| format!("the overlay stands at {:?}, missing {:?}, {}; ", overlay.hour(), overlay.missed(), overlay.loading().join("; ")));
        let (report, _, counters) = self.entry.playhead.held();
        format!(
            "Hour {head} was not reached by the map's layers in a minute: {overlay}applied {:?}, newest decoded {:?}, composing {}, not composed {waiting:?}, steps held here {}, ring steps {}, fetches queued {}, jobs running {}, preparing {}, {counters:?}",
            self.applied,
            self.entry.playhead.newest(),
            self.composing,
            self.pending.len() + self.taken.len(),
            report.ring_steps,
            report.fetches_queued,
            report.jobs_running,
            report.preparing,
        )
    }

    /// The vision its layers show, copied out of the vision layer: the settle loop takes it for
    /// the layer it has just settled, and a stepped frame takes it again, `exact`, while it waits
    /// for the one of its own hour.
    fn copy_vision(&mut self, exact: bool) -> Result<Arc<viewer_foundation::map::perspective::Vision>, String> {
        let applied = self.applied;
        let home = self.mirrors.is_some() && !self.composing && !self.superseded;
        let at = self.names.iter().position(|name| *name == "vision").filter(|&at| self.reads[at].is_ok() && self.ready[at]);
        let layer = at.and_then(|at| self.layers[at].any().downcast_ref::<crate::layers::vision::Vision>());
        self.vision.shown(layer, applied, home, exact)
    }

    /// The overlay of a stepped frame: the rows of `head` waited for however long they take, the
    /// tables made whole (every table they read learned), given to the counter layers, every layer
    /// settled over them, and only then the overlay laid out over the art of the hour — the one
    /// layout the hour has, as a step's is. Refused by name.
    /// False when the overlay of the hour is not there yet: its rows do not hold a table its maker
    /// reads. That is answered by a step — the rows of a table a maker learned arrive in steps —
    /// so the settle is handed back for one and asked again after it, within [`CATCHING_UP`].
    fn overlay_whole(&mut self, head: Hour) -> io::Result<bool> {
        if self.overlay.is_none() {
            return Ok(true);
        }
        let until = Instant::now() + HANDS_BACK;
        for lay in [false, true] {
            loop {
                // The maker's rows are made from the mirrors by the view store, which a step posts
                // to at its apply. This settle advances no step, so it posts to it itself: before
                // the ask for the hour's sets, and again after it, because the ask is what opens
                // the maker's reader.
                self.post_makers();
                let asked = self.ask_overlay_step(true, lay, Arc::default());
                self.post_makers();
                if asked && self.join_overlay(Settling::Whole) {
                    break;
                }
                if Instant::now() >= until {
                    return Ok(false);
                }
                std::thread::sleep(std::time::Duration::from_micros(200));
            }
            self.settle_all()?;
        }
        Ok(self.overlay.as_deref().expect("held above").stands_at(head))
    }

    /// Whether every drawn layer that makes its product from rows at the head has made it for
    /// `head` ([`Layer::made_at`]): what a stepped frame's whole picture waits for beside the
    /// overlay.
    fn head_products_reached(&self, head: Hour) -> bool {
        (0..self.layers.len()).filter(|&at| self.reads[at].is_ok() && self.ready[at]).all(|at| self.layers[at].made_at(head))
    }

    /// The sets the makers read, made from the mirrors as the steps left them. A step posts them
    /// at its apply; a whole settle advances no step, so it posts them itself.
    fn post_makers(&mut self) {
        if self.composing || self.superseded {
            return;
        }
        let Some(hour) = self.applied else { return };
        let (store, held) = (&mut self.views, self.mirrors.as_ref().expect("the mirrors are home while not composing"));
        store.post(held, hour, self.branch.unwrap_or_default(), self.epoch, true);
    }

    /// Settles every layer whole, the products made from rows at the head waited for: what a
    /// stepped frame and the proof tool compare.
    pub fn settle_all(&mut self) -> io::Result<()> {
        self.doing.set(Doing::WHOLE);
        // The rows the makers read at the hour applied, made before anything settles: a step posts
        // them at its apply, and a whole settle advances none.
        self.post_makers();
        self.views.made();
        if let Some(mirrors) = self.mirrors.as_deref().filter(|_| !self.superseded) {
            for at in 0..self.layers.len() {
                if self.reads[at].is_ok() && self.ready[at] {
                    let (before, rest) = self.layers.split_at_mut(at);
                    rest[0].settle_with(mirrors, &Upstream::new(before), Settling::Whole)?;
                    self.standing.settled_whole(at, rest[0].published());
                }
            }
            let applied = self.applied;
            hand_naval(&self.names, &mut self.layers, &self.reads, &self.ready);
            for at in 0..self.layers.len() {
                if self.reads[at].is_ok() && self.ready[at] {
                    self.layers[at].stage()?;
                    // A whole settle leaves a timed layer's changes as the hour settled, which the
                    // picture it is taken for stands at.
                    if let (true, Some(hour)) = (self.layers[at].timed(), applied) {
                        let composed = self.composed.load(Ordering::Acquire);
                        self.layers[at].timed_at(hour, composed)?;
                    }
                }
            }
        }
        self.doing.set(Doing::IDLE);
        Ok(())
    }

    /// Writes every drawn layer's changed ranges, in registration order. A timed layer writes
    /// nothing here: its hours' change sets are the display frame's own, written outside this
    /// window ([`crate::layers::Installs`]), so that no frame repeats an hour because the side's
    /// thread was busy.
    pub fn upload(&mut self, gpu: &mut Gpu<'_>) -> io::Result<()> {
        if self.composing || self.superseded {
            return Ok(());
        }
        let _lap = replay_frame::cpu::scope("frame.refresh.uploads", Some("other"));
        self.upload_with(|layer| layer.upload(gpu))
    }

    /// The naval layer's attributes, when they changed, handed to the political fill, which draws
    /// them: on the side, before the fill dates its hour, so its set carries that hour's.
    /// The upload pass without a GPU, `each` making each layer's upload: a layer's tests.
    #[cfg(any(test, feature = "test-support"))]
    pub fn upload_each(&mut self, each: impl FnMut(&mut dyn Layer) -> io::Result<()>) -> io::Result<()> {
        self.upload_with(each)
    }

    /// Each drawn layer's upload made by `each`, in registration order. Every drawn layer that
    /// owes the GPU something takes its install here, whatever the frame's clock says: a
    /// publication is one moment on screen, and an install left behind held the hour that layer
    /// showed where it was until a frame had room for it.
    fn upload_with(&mut self, mut each: impl FnMut(&mut dyn Layer) -> io::Result<()>) -> io::Result<()> {
        // Read once between uploads: each runs from the read before, the loop's bookkeeping in it.
        let mut mark = replay_frame::cpu::thread_cycles();
        for at in 0..self.layers.len() {
            if !(self.reads[at].is_ok() && self.ready[at]) {
                continue;
            }
            let layer = &mut self.layers[at];
            let started = Instant::now();
            let counted = shader_pipeline::gpu::stats::peek();
            each(&mut **layer)?;
            let spent = started.elapsed();
            let now = replay_frame::cpu::thread_cycles();
            let cycles = now.saturating_sub(std::mem::replace(&mut mark, now));
            let written = |count: shader_pipeline::gpu::stats::Count| shader_pipeline::gpu::stats::peek()[count as usize].saturating_sub(counted[count as usize]);
            if let Some(part) = self.last.layers.get_mut(at) {
                part.upload_ns = spent.as_nanos() as u64;
                part.upload_cycles = cycles;
                part.upload_carried = !layer.uploaded();
                (part.uploads, part.upload_bytes) = (written(shader_pipeline::gpu::stats::Count::Uploads), written(shader_pipeline::gpu::stats::Count::UploadBytes));
                (part.write_ns, part.create_ns) = (written(shader_pipeline::gpu::stats::Count::WriteNs), written(shader_pipeline::gpu::stats::Count::CreateNs));
            }
        }
        self.update_shown();
        self.judge_installs();
        let published: Vec<u8> = self
            .layers
            .iter()
            .map(|layer| match layer.published() {
                crate::layers::Published::Current => 0,
                crate::layers::Published::At(_) => 1,
                crate::layers::Published::Cadence(_) => 2,
                crate::layers::Published::Nothing => 3,
            })
            .collect();
        for (((part, shown), fresh), published) in self.last.layers.iter_mut().zip(&self.shown).zip(&self.fresh).zip(published) {
            (part.shown, part.reset_shown, part.published) = (*shown, *fresh, published);
        }
        (self.last.drawn, self.last.oldest, self.last.reset_shown) = (self.drawn, self.oldest, self.reset_shown);
        Ok(())
    }

    /// `BATTLE_PLAN_VIEWER_CHECK_PUBLICATION`: every drawn layer's product is on the GPU when the window
    /// ends, and no layer's picture goes backwards.
    fn judge_installs(&mut self) {
        use viewer_foundation::map::publication_check as check;
        if !check::on() {
            return;
        }
        for at in 0..self.layers.len() {
            if !(self.reads[at].is_ok() && self.ready[at]) {
                continue;
            }
            check::judged();
            if !self.layers[at].uploaded() {
                check::found(self.names[at], "its install was left for a later frame");
            }
        }
        self.judged_shown.resize(self.layers.len(), None);
        let sought = std::mem::replace(&mut self.judged_resets, self.resets_taken) != self.resets_taken;
        for at in 0..self.layers.len() {
            // Only a layer whose shown hour is the hour applied: one that publishes its own
            // worker's product says an older hour when a restep voids what that worker folded,
            // which is the worker's cadence and not the budget's carry.
            if !matches!(self.layers[at].published(), crate::layers::Published::Current) {
                continue;
            }
            check::judged();
            let shown = self.shown.get(at).copied().flatten();
            let before = self.judged_shown[at];
            if !sought
                && let (Some(before), Some(shown)) = (before, shown)
                && shown < before
            {
                check::backwards(self.names[at], before, shown);
            }
            self.judged_shown[at] = shown.or(before);
        }
    }

    /// A step into the mirrors (a reset's rebuilt mirrors swapped in first); the layers reading a
    /// table it resets or renumbers are marked to compose. Whether it reset a table without being a
    /// whole reset: a piece's crossing, whose few layers compose on this thread
    /// ([`Self::compose_here`]).
    fn take_step(&mut self, step: &Arc<Step>, frame: &mut Frame, compose: &mut [bool]) -> bool {
        let mut reset_tables = BTreeSet::new();
        // The judge's reference: a crossing's tables keep their mirrors, so the states the prepare
        // rebuilt for them are taken here alone, and compared once the crossing is applied.
        let mut judged: Vec<(usize, Box<Mirror>)> = Vec::new();
        if piece::on() && !step.reset() {
            for changes in step.tables().iter().filter(|changes| !changes.reset()) {
                if let Some(state) = self.consumer.state(step, changes.table())
                    && let Ok(rebuilt) = state.downcast::<Mirror>()
                {
                    judged.push((changes.table(), rebuilt));
                }
            }
        }
        let mirrors = self.mirrors.as_mut().and_then(Arc::get_mut).expect("the mirrors are home and the step's views made");
        // A crossing resets or renumbers tables the map already holds; a table that joins brings a
        // mirror it did not hold, and its first compose goes to the worker as a reset's does.
        let mut replaced = false;
        for changes in step.tables() {
            if changes.reset()
                && let Some(state) = self.consumer.state(step, changes.table())
                && let Ok(mut rebuilt) = state.downcast::<Mirror>()
            {
                rebuilt.set_capture(self.entry.capture.get().copied());
                if let Some(old) = mirrors.insert(rebuilt) {
                    self.worker.drop_later(old);
                    replaced = true;
                }
                reset_tables.insert(changes.table());
            }
            if let Some(old) = changes.renumber() {
                let moved = old.iter().enumerate().filter(|&(new, &from)| from as usize != new).count();
                frame.renumbers.push((changes.table(), old.len() as u32, moved as u32));
                reset_tables.insert(changes.table());
                replaced = true;
            }
        }
        let crossing = !step.reset() && replaced;
        frame.reset_tables.extend(reset_tables.iter().copied());
        frame.reset_step |= step.reset();
        let (started, cycles) = (Instant::now(), replay_frame::cpu::thread_cycles());
        mirrors.apply_timed(step, &mut frame.mirror_tables, &mut frame.mirror_split, &mut self.views.noting(&self.layers_awake));
        frame.mirror_ns += started.elapsed().as_nanos() as u64;
        for (table, mut rebuilt) in judged {
            piece::judged();
            rebuilt.set_capture(self.entry.capture.get().copied());
            let id = TableId::ALL[table];
            match mirrors.get(id) {
                None => piece::found(id, step.last(), "the crossing left no mirror of it"),
                Some(mirror) => {
                    if let Some(why) = mirror.differs(&rebuilt, self.entry.kinds().get(table).map_or(&[][..], |kinds| &kinds[..])) {
                        piece::found(id, step.last(), &why);
                    }
                }
            }
        }
        frame.mirror_cycles += replay_frame::cpu::thread_cycles().saturating_sub(cycles);
        frame.steps += 1;
        frame.step_ns += step.built().ns;
        frame.step_most_ns = frame.step_most_ns.max(step.built().ns);
        frame.step_cycles += step.built().cycles;
        frame.cells += step.built().cells;
        frame.rows += step.built().rows;
        if step.first() <= step.last() {
            frame.first = frame.first.filter(|first| *first <= step.first()).or_else(|| Some(step.first()));
        }
        frame.applied = Some(step.last());
        self.applied = Some(step.last());
        self.epoch = step.epoch();
        if step.reset() {
            frame.sought = frame.sought.or(step.built().sought);
            self.resets_taken += 1;
            self.vision.forget();
        }
        for (wanted, reads) in compose.iter_mut().zip(&self.reads) {
            *wanted |= reads.as_ref().is_ok_and(|tables| step.reset() || tables.iter().any(|table| reset_tables.contains(table)));
        }
        crossing
    }

    /// A piece's crossing composes on this thread, before anything settles. A boundary resets only
    /// the tables no change set carries over ([`playhead::boundary::crossed`]: a log's, whose rows
    /// are its own run's ordinals), so the layers that read one are composed here in registration
    /// order with the mirrors in hand, and nothing is sent away: every other layer settles over the
    /// crossing and the drawn hour advances through the boundary. A seek's whole compose, which
    /// rebuilds the whole picture, still goes to the worker ([`Self::send_away`]).
    fn compose_here(&mut self, compose: &[bool], frame: &mut Frame) {
        self.compose_ns.resize(self.layers.len(), 0);
        self.fresh.resize(self.layers.len(), false);
        self.offered.resize(self.layers.len(), None);
        let mut spent = vec![0; self.layers.len()];
        for at in 0..self.layers.len() {
            if !compose[at] {
                continue;
            }
            let mirrors = self.mirrors.as_deref().expect("the mirrors are home while not composing");
            let (before, rest) = self.layers.split_at_mut(at);
            let started = Instant::now();
            let result = rest[0].compose_with(mirrors, &Upstream::new(before));
            let ns = started.elapsed().as_nanos() as u64;
            (self.compose_ns[at], spent[at]) = (ns, ns);
            (self.fresh[at], self.offered[at]) = (false, None);
            match result {
                Err(error) => self.reads[at] = Err(error.to_string()),
                Ok(()) => self.ready[at] = true,
            }
        }
        frame.resets += 1;
        frame.swapped = Some(spent);
    }

    fn send_away(&mut self, compose: Vec<bool>) {
        self.composed.fetch_add(1, Ordering::AcqRel);
        self.views.settled();
        let mirrors = self.mirrors.take().expect("the mirrors are home");
        self.sizes = (mirrors.iter().map(|mirror| (mirror.table(), mirror.bytes())).collect(), self.layers.iter().map(|layer| (layer.name(), layer.held())).collect());
        let layers = std::mem::take(&mut self.layers);
        self.compose_ns.resize(layers.len(), 0);
        self.cancel = Arc::default();
        let away = Away { mirrors, layers, compose, upstream: self.upstream.clone(), cost: self.compose_ns.clone(), cancel: Arc::clone(&self.cancel), results: Vec::new() };
        if self.worker.jobs.send(Job::Compose(Box::new(away))).is_ok() {
            self.composing = true;
            self.superseded = false;
            if let Some(overlay) = self.overlay.as_deref_mut() {
                overlay.reset();
            }
        }
    }

    fn swap_in(&mut self, away: Away) -> io::Result<()> {
        let Away { mirrors, layers, results, .. } = away;
        self.mirrors = Some(mirrors);
        self.layers = layers;
        self.composing = false;
        self.fresh.resize(self.layers.len(), false);
        self.offered.resize(self.layers.len(), None);
        for (at, (result, ns, composed)) in results.into_iter().enumerate() {
            if composed {
                self.compose_ns[at] = ns;
                self.fresh[at] = false;
                self.offered[at] = None;
            }
            match result {
                Err(error) => self.reads[at] = Err(error.to_string()),
                Ok(()) if composed => self.ready[at] = true,
                Ok(()) => {}
            }
        }
        if let Some(view) = self.view_later.take() {
            self.set_view(view)?;
        }
        Ok(())
    }

    /// A compose a newer seek superseded comes home unshown: its layers are half composed for an
    /// hour no longer wanted, so nothing is settled, uploaded or shown until the newer seek's reset
    /// composes them all, and the old picture stays meanwhile.
    fn take_home(&mut self, away: Away) {
        let Away { mirrors, layers, results, .. } = away;
        self.mirrors = Some(mirrors);
        self.layers = layers;
        self.composing = false;
        self.superseded = true;
        for (at, (result, _, _)) in results.into_iter().enumerate() {
            if let Err(error) = result {
                self.reads[at] = Err(error.to_string());
            }
        }
    }

    /// Per layer, how far behind the step's hour its product stood over the run: what the step
    /// never waited for, and how often the bound made it wait after all ([`standing`]).
    pub fn standing(&self) -> serde_json::Value {
        self.standing.report(&self.names)
    }

    /// The store its makers read the rows of the step's hour from.
    pub fn views(&self) -> &views::Store {
        &self.views
    }

    /// The mirrors and layers, while they are on the frame side and whole (not a superseded compose).
    pub fn parts(&self) -> Option<(&Mirrors, &[Box<dyn Layer>])> {
        self.mirrors.as_deref().filter(|_| !self.superseded).map(|mirrors| (mirrors, &self.layers[..]))
    }

    /// The layers, to be changed by a test as a frame would leave them.
    #[cfg(any(test, feature = "test-support"))]
    pub fn layers_mut(&mut self) -> &mut [Box<dyn Layer>] {
        &mut self.layers
    }

    /// A reset is composing, or a superseded one waits for the newer seek's.
    pub fn composing(&self) -> bool {
        self.composing || self.superseded
    }

    /// The view it draws at.
    pub fn view(&self) -> &View {
        &self.view
    }

    /// The last hour of the last step applied: the hour its mirrors are at.
    pub fn applied(&self) -> Option<Hour> {
        self.applied
    }

    pub fn held(&self) -> Held {
        let (report, _, _) = self.entry.playhead.held();
        let (mirrors, layers) = match &self.mirrors {
            Some(mirrors) => (mirrors.iter().map(|mirror| (mirror.table(), mirror.bytes())).collect(), self.layers.iter().map(|layer| (layer.name(), layer.held())).collect()),
            None => self.sizes.clone(),
        };
        Held { mirrors, views: self.views.held(), layers, pending: self.pending.len(), states: (report.states, report.state_bytes), playhead: report.total }
    }

    /// What its drawn layers show, for the window's report; none once its reader stopped at an error.
    pub fn on_screen(&self) -> Option<OnScreen> {
        use crate::layers::Published;
        if self.failure.is_some() {
            return None;
        }
        let drawn = (0..self.layers.len()).filter(|&at| self.reads[at].is_ok() && self.ready[at] && !matches!(self.layers[at].published(), Published::Cadence(_)));
        let whole = |at: usize| self.layers[at].carried().entities == 0 && self.layers[at].uploaded();
        Some(OnScreen {
            head: self.last.head,
            drawn: self.drawn,
            sought: lock(&self.entry.anchors).0,
            composing: self.composing(),
            settled: !self.composing() && drawn.clone().all(whole),
            furthest: drawn.min_by_key(|&at| (self.shown.get(at).copied().flatten(), whole(at))).map(|at| self.names[at]),
            resets: self.resets_shown,
        })
    }

    /// Why its reader stopped, by name; none while it draws.
    pub fn failure(&self) -> Option<&str> {
        self.failure.as_deref()
    }

    fn fail(&mut self, failure: String) {
        eprintln!("{failure}");
        self.failure = Some(failure);
    }

    /// The branch its rows at the head are stamped with; none for a reader not opened for a
    /// document's map (a test's, the harness's extra maps).
    pub fn branch(&self) -> Option<replay_data::BranchId> {
        self.branch
    }

    /// The branch its rows are stamped with, for a side made without the open map's recording.
    pub fn set_branch(&mut self, branch: replay_data::BranchId) {
        self.branch = Some(branch);
    }

    /// The map mode its drawn picture was composed and settled for: while a newly chosen mode is
    /// being settled, the one before; none before its first picture.
    pub fn mode_on_screen(&self) -> Option<&viewer_world::map::map_mode::MapMode> {
        self.mode.as_ref()
    }
}

/// What a map's drawn layers show ([`MapSide::on_screen`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct OnScreen {
    /// The head its last frame took.
    pub head: Hour,
    /// The map's drawn hour: the oldest its drawn layers show, layers on a cadence aside.
    pub drawn: Option<Hour>,
    /// The hour the playhead was last sought to, by any of its callers.
    pub sought: Option<Hour>,
    /// A reset is composing: the picture is the one before it.
    pub composing: bool,
    /// No reset is composing and every drawn layer is settled and uploaded whole.
    pub settled: bool,
    /// The drawn layer furthest behind: showing the oldest hour (none first), then not settled;
    /// none while a reset composes or no layer is drawn. Layers on a cadence aside.
    pub furthest: Option<&'static str>,
    /// Reset steps whose composed picture is on the frame side: a new one is a picture composed
    /// for a seek (the window's, another caller's, or the host's own exact head).
    pub resets: u64,
}

impl Drop for MapSide {
    fn drop(&mut self) {
        self.entry.maps.fetch_sub(1, Ordering::AcqRel);
        self.entry.playhead.retire(self.reader);
    }
}

/// Every layer's inputs at `view`, joined: per table, the union of columns and of keys; per layer,
/// its tables or why it is refused, and the layers before it that it reads.
fn declare(entry: &Entry, layers: &mut [Box<dyn Layer>], view: &View) -> Declaring {
    let mut union: BTreeMap<usize, Read> = BTreeMap::new();
    let mut reads: Vec<Result<BTreeSet<usize>, String>> = Vec::with_capacity(layers.len());
    let mut upstream = Vec::with_capacity(layers.len());
    let mut each = Vec::with_capacity(layers.len());
    let names: Vec<&'static str> = layers.iter().map(|layer| layer.name()).collect();
    for (at, layer) in layers.iter_mut().enumerate() {
        let mut inputs = Inputs::new(entry.kinds(), layer.name());
        let declared = layer.inputs(view, &mut inputs).and_then(|()| {
            inputs
                .upstream()
                .iter()
                .map(|name| match names[..at].iter().position(|held| held == name) {
                    Some(before) if reads[before].is_ok() => Ok(before),
                    Some(_) => Err(io::Error::other(format!("The {} layer is refused: the {name} layer it reads is refused", names[at]))),
                    None => Err(io::Error::other(format!("The {} layer is refused: it reads the {name} layer, which is not registered before it", names[at]))),
                })
                .collect::<io::Result<Vec<usize>>>()
        });
        upstream.push(declared.as_ref().map_or_else(|_| Vec::new(), Clone::clone));
        match declared {
            Ok(_) => {
                let after = inputs.upstream().to_vec();
                let tables = inputs.into_tables();
                each.push(Some((tables.clone(), after)));
                reads.push(Ok(tables.keys().copied().collect()));
                for (table, read) in tables {
                    let held = union.entry(table).or_default();
                    held.columns.extend(read.columns);
                    held.columns.sort_unstable();
                    held.columns.dedup();
                    held.keyed |= read.keyed;
                    held.hourly |= read.hourly;
                }
            }
            Err(error) => {
                each.push(None);
                reads.push(Err(error.to_string()));
            }
        }
    }
    Declaring { union, reads, upstream, each }
}

/// One layer's own declaration: its tables and the layers it reads by name; none when refused.
type LayerDeclared = Option<(BTreeMap<usize, Read>, Vec<&'static str>)>;

struct Declaring {
    union: BTreeMap<usize, Read>,
    reads: Vec<Result<BTreeSet<usize>, String>>,
    upstream: Vec<Vec<usize>>,
    each: Vec<LayerDeclared>,
}

/// Every table of the recording's schema, read whole and keyed: what the side's mirrors hold for
/// its makers and for the window's readers of the hour, whatever any of them has declared yet.
///
/// A maker's builder reads the content's scripts, which read any table, so the tables a maker
/// comes to read are not known before it reads them. Holding them all from the start is what makes
/// a table learned mid-play already at the head: a table newly declared to the playhead joins it
/// at a step past its head, which a stepped frame holding the head at its own hour never reaches.
/// What a maker declares then decides which views are made, never which mirrors exist. A table the
/// recording holds nothing of is held too, so a reader that declares one is answered with its
/// mirror's "not recorded" instead of waiting for ever for a mirror that never comes. A table no
/// run wrote at all, not even its status, has no first restore to seek to, so it is left out.
fn for_the_makers(entry: &Entry) -> BTreeMap<usize, Read> {
    let kinds = entry.kinds();
    (0..kinds.len()).filter(|&table| written(entry, table)).map(|table| (table, Read { columns: (0..kinds[table].len()).collect(), keyed: true, hourly: false })).collect()
}

/// The naval layer's attributes, when they changed, handed to the political fill, which draws
/// them: on the side, before the fill dates its hour, so its set carries that hour's.
fn hand_naval(names: &[&'static str], layers: &mut [Box<dyn Layer>], reads: &[Result<BTreeSet<usize>, String>], ready: &[bool]) {
    let drawn = |at: usize| reads[at].is_ok() && ready[at];
    let draws = |name: &str| names.iter().enumerate().any(|(at, held)| *held == name && drawn(at));
    if !(draws("naval") && draws("political")) {
        return;
    }
    let Some(naval) = names
        .iter()
        .position(|&name| name == "naval")
        .and_then(|at| layers[at].any_mut().and_then(|layer| layer.downcast_mut::<crate::layers::naval::Naval>()))
        .and_then(crate::layers::naval::Naval::take_changed)
    else {
        return;
    };
    for (at, layer) in layers.iter_mut().enumerate() {
        if drawn(at)
            && let Some(political) = layer.any_mut().and_then(|layer| layer.downcast_mut::<crate::layers::political::Political>())
        {
            political.set_naval(&naval);
        }
    }
}

/// Whether the recording writes `table` from a run's start: the tables a side's mirrors hold.
fn written(entry: &Entry, table: usize) -> bool {
    entry.pages.runs().iter().any(|run| run.pages.first().and_then(|page| page.tables.get(table)).is_some_and(|held| held.first.is_some()))
}

/// Per table index: whether `declared` holds it.
fn awake_mask(declared: &BTreeMap<usize, Read>) -> Vec<bool> {
    let mut mask = vec![false; declared.keys().next_back().map_or(0, |&table| table + 1)];
    for &table in declared.keys() {
        mask[table] = true;
    }
    mask
}

/// The drawn layers' reads and the makers' tables as one union, per table: the columns of
/// both and the keys either wants.
fn merged(layers: &BTreeMap<usize, Read>, makers: &BTreeMap<usize, Read>) -> BTreeMap<usize, Read> {
    let mut union = layers.clone();
    for (&table, read) in makers {
        let held = union.entry(table).or_default();
        held.columns.extend(read.columns.iter().copied());
        held.columns.sort_unstable();
        held.columns.dedup();
        held.keyed |= read.keyed;
        held.hourly |= read.hourly;
    }
    union
}

fn to_declared(declared: &BTreeMap<usize, Read>) -> Vec<Declared> {
    declared.iter().map(|(&table, read)| Declared { table, columns: read.columns.clone(), keyed: read.keyed, passing: false, hourly: read.hourly }).collect()
}
