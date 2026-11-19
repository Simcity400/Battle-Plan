//! The playhead hosted in the map's process: one per open (recording, timeline), shared by every
//! caller that opens the same ([`open`]), with a slot for the stepper's hook another module fills
//! ([`Hooks`]). The map's frame side ([`MapSide`]) is one reader of it: the union of its drawn
//! layers' columns, kept in one mirror per table ([`mirror`]), and the layers that draw from them
//! ([`crate::layers`]). Every frame-side call returns without waiting: a reset's composing runs on
//! the host's worker and is swapped in whole when done.

pub mod details;
pub mod limitations;
pub mod mirror;
pub mod rows;
#[cfg(any(test, feature = "test-support"))]
pub mod shared_maps_tests;
#[cfg(test)]
mod tests;
pub mod units;

use crate::layers::{Read, Upstream, Budget, Gpu, Inputs, Layer, View};
use mirror::{Mirror, Mirrors};
use playhead::{Consumer, Declared, Lanes, Playhead, Reader, Settings, Step, pages::Pages};
use replay_columnar::state::Kind;
use replay_data::{Hour, rows::TableId};
use std::{
    collections::{BTreeMap, BTreeSet, VecDeque},
    io,
    sync::{
        Arc, Mutex, Weak,
        atomic::{AtomicBool, AtomicU64, Ordering},
        mpsc::{self, Receiver, Sender, TryRecvError},
    },
    time::{Duration, Instant},
};
use units::Units;

/// The frame's slice for the layers' settle and upload together, unless set otherwise.
pub const BUDGET: Duration = Duration::from_millis(3);

/// The part of a map's `slice` reserved to its uploads whatever the settles spend: what they cost
/// when last given budget with work to do, at least a third of the slice (the political fill's
/// upload at its p95, about 1 ms of 3) and at most half. The settles have the rest.
fn upload_share(slice: Duration, uploads: &[Duration]) -> Duration {
    uploads.iter().sum::<Duration>().clamp(slice / 3, slice / 2)
}

/// The order a frame visits its layers in: first those passed over last frame with work waiting
/// (visited with the budget spent, or not at all), then the one whose oldest dirty work has waited
/// the most frames, between equals the one whose last turn with budget is the oldest, each only
/// after the layers it reads. So a layer that spends the whole budget goes after the ones it starved.
fn oldest_first(late: &[u64], turn: &[u64], upstream: &[Vec<usize>], frame: u64) -> Vec<usize> {
    let passed = |at: usize| late[at] > 0 && turn[at] + 1 < frame;
    let mut order = Vec::with_capacity(late.len());
    let mut done = vec![false; late.len()];
    while order.len() < late.len() {
        let next = (0..late.len())
            .filter(|&at| !done[at] && upstream[at].iter().all(|&upper| done[upper]))
            .max_by(|&a, &b| passed(a).cmp(&passed(b)).then(late[a].cmp(&late[b])).then(turn[b].cmp(&turn[a])).then(b.cmp(&a)))
            .expect("upstream layers are registered before the layers that read them");
        done[next] = true;
        order.push(next);
    }
    order
}

/// The layers whose uploads go before every other's: what the user looks at first after a seek.
const SEEN_FIRST: &[&str] = &["political"];

/// The order the uploads go in: the layers seen first, then the one whose upload has been carried
/// the most frames, between equals the one registered first. No upload reads another's.
fn uploads_first(late: &[u64], first: &[bool]) -> Vec<usize> {
    let mut order: Vec<usize> = (0..late.len()).collect();
    order.sort_by_key(|&at| (!first[at], std::cmp::Reverse(late[at])));
    order
}

/// The frame thread's one settle and upload budget, shared by every map it draws in a frame: each
/// map not yet drawn takes a share of what is left, weighted by how many frames its oldest dirty
/// work has waited, so the oldest goes first and none starves. A map drawing again starts a frame.
/// Ids are unique across threads, so a map let go on another thread leaves no other map's place.
mod share {
    use std::{
        cell::RefCell,
        sync::atomic::{AtomicU64, Ordering},
        time::Duration,
    };

    static NEXT: AtomicU64 = AtomicU64::new(1);

    #[derive(Default)]
    struct Thread {
        frame: u64,
        spent: Duration,
        maps: Vec<Map>,
    }

    struct Map {
        id: u64,
        late: u64,
        drawn: Option<u64>,
    }

    thread_local! {
        static THREAD: RefCell<Thread> = RefCell::default();
    }

    pub(super) fn join() -> u64 {
        let id = NEXT.fetch_add(1, Ordering::Relaxed);
        THREAD.with(|thread| thread.borrow_mut().maps.push(Map { id, late: 0, drawn: None }));
        id
    }

    pub(super) fn leave(id: u64) {
        let _ = THREAD.try_with(|thread| thread.borrow_mut().maps.retain(|map| map.id != id));
    }

    /// Map `id` begins its frame: one that has had its turn this frame starts the next. Every map
    /// begins every frame, composing or not, so the frame thread's frames stay the window's.
    pub(super) fn begin(id: u64) {
        THREAD.with(|thread| {
            let mut thread = thread.borrow_mut();
            if thread.maps.iter().any(|map| map.id == id && map.drawn == Some(thread.frame)) {
                thread.frame += 1;
                thread.spent = Duration::ZERO;
            }
        })
    }

    /// Map `id`'s share of what this frame has left of `total`.
    pub(super) fn slice(id: u64, total: Duration) -> Duration {
        THREAD.with(|thread| {
            let thread = thread.borrow();
            let frame = thread.frame;
            // The maps still to draw this frame: this one, and those that drew in the last.
            let waiting = |map: &&Map| map.id == id || (map.drawn != Some(frame) && map.drawn.is_some_and(|drawn| drawn + 1 == frame));
            let weight = |map: &Map| map.late + 1;
            let all: u64 = thread.maps.iter().filter(waiting).map(weight).sum();
            let own = thread.maps.iter().find(|map| map.id == id).map_or(1, weight);
            total.saturating_sub(thread.spent).mul_f64(own as f64 / all.max(1) as f64)
        })
    }

    /// Map `id` spent `spent` of this frame; `late` is how many frames its oldest dirty work has
    /// waited, and `drawn` that it has had its turn this frame (composing, it had one too).
    pub(super) fn spent(id: u64, spent: Duration, late: u64, drawn: bool) {
        THREAD.with(|thread| {
            let mut thread = thread.borrow_mut();
            thread.spent += spent;
            let frame = thread.frame;
            if let Some(map) = thread.maps.iter_mut().find(|map| map.id == id) {
                map.late = late;
                if drawn {
                    map.drawn = Some(frame);
                }
            }
        })
    }
}

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
    /// The head the map's frame side took last and the seek of the steps it had applied then
    /// ([`seek_head`]; [`NO_HEAD`] before any).
    head: AtomicU64,
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

    /// One playback anchor for this (recording, timeline), whoever sends it: a seek to the hour it
    /// is already holding at, or already at and not moving, is dropped; so is a pace it already
    /// has. The map and follow send the same anchors; only the first does anything.
    pub fn anchor(&self, hour: Hour, playing: bool, speed: f64, sought: bool) {
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
        }
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

    /// The head the map's frame side took last, while a map draws and only in the seek it has
    /// applied the reset of.
    pub fn map_head(&self) -> Option<Hour> {
        self.map_shown().flatten()
    }

    /// The head the rows follow while a map draws (none: no map draws): the head the map's frame
    /// side took last, of this seek; none inside while the map has not taken this seek's reset.
    /// A head of the seek before is no head: the steps of this one taken to it run past the hour
    /// sought.
    pub fn map_shown(&self) -> Option<Option<Hour>> {
        if !self.drawn() {
            return None;
        }
        let held = self.head.load(Ordering::Acquire);
        Some((held != NO_HEAD && held >> 32 == seek_bits(self.playhead.epoch())).then(|| held as u32 as i32))
    }

    /// Whether a map draws from this playhead.
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
const NO_HEAD: u64 = u64::MAX;

/// A seek's epoch as the high half of [`seek_head`]: seeks 2^32 apart are not told apart, far
/// beyond any session's seeks.
fn seek_bits(epoch: u64) -> u64 {
    epoch & u64::from(u32::MAX)
}

/// A map's head and the seek of the steps it applied, in one word the frame thread swaps and the
/// rows worker loads.
fn seek_head(hour: Hour, epoch: u64) -> u64 {
    (seek_bits(epoch) << 32) | u64::from(hour as u32)
}

/// A playhead's recording, timeline and clock mapping ([`open_clocked`]).
type Key = (String, Vec<(usize, Hour, Hour)>, Hour);

static ENTRIES: Mutex<Vec<(Key, Weak<Entry>)>> = Mutex::new(Vec::new());

/// The playhead of `units`' recording and timeline: the one already open, or a new one with
/// `settings`.
pub fn open(units: Units, settings: Settings) -> io::Result<Arc<Entry>> {
    open_clocked(units, settings, 0)
}

/// [`open`] for a track whose game hour is the UI clock's plus `clock`: each mapping of one
/// timeline is anchored at its own hours, so each has its own playhead. The map is at 0.
pub fn open_clocked(units: Units, settings: Settings, clock: Hour) -> io::Result<Arc<Entry>> {
    let key: Key = (units.name.clone(), units.timeline.pieces().iter().map(|piece| (piece.run, piece.first, piece.last)).collect(), clock);
    let mut entries = lock(&ENTRIES);
    entries.retain(|(_, entry)| entry.strong_count() > 0);
    if let Some(entry) = entries.iter().find(|(held, _)| *held == key).and_then(|(_, entry)| entry.upgrade()) {
        return Ok(entry);
    }
    let hooks = Arc::new(Hooks::default());
    let stepper_hooks = Arc::clone(&hooks);
    let hook: playhead::Hook = Box::new(move |step: &Step, lanes: &Lanes<'_>| stepper_hooks.run(step, lanes));
    let playhead = Playhead::new(Arc::clone(&units.pages), units.timeline, settings, Some(hook))?;
    let entry = Arc::new(Entry { page_time: Default::default(), playhead, pages: units.pages, kinds: units.kinds, hooks, anchors: Mutex::new((None, None)), capture: Default::default(), maps: Default::default(), head: AtomicU64::new(NO_HEAD), rosters: Default::default() });
    entries.push((key, Arc::downgrade(&entry)));
    Ok(entry)
}

/// The playhead's settings for the map: steps on two threads, reads on one, 60 frames a second,
/// and up to eight steps built past the head: at 1,440 hours a second a step is due every frame,
/// and one slow build (tens of milliseconds) must not let the head pass the newest step.
pub fn settings() -> Settings {
    Settings { step_threads: 2, fetch_threads: 1, ring: 8, ..Settings::default() }
}

/// The layers the window registers, by name, when the playback-cost harness names them (to tell
/// their costs apart); every layer otherwise.
static REGISTERED: std::sync::OnceLock<Vec<String>> = std::sync::OnceLock::new();

pub fn register_only(names: Vec<String>) {
    let _ = REGISTERED.set(names);
}

/// The map's reader for `document`, on the playhead of the recording it reads, at clock `at` (the
/// document's hour when none): its rows carry `at`'s branch, and a playhead no other caller has
/// anchored is sought to its hour; one already anchored is joined at its own clock. A refusal is
/// printed by name, and the map runs without it. Made on the caller's thread; the window makes it
/// on a worker instead ([`Opening`]).
pub fn for_document_at(document: &crate::document::Document, at: Option<At>) -> Option<MapSide> {
    ForMap::of(document, at).make()
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

    /// The reader; a refusal is printed by name, and the map runs without it.
    pub fn make(self) -> Option<MapSide> {
        self.made().map_err(|error| eprintln!("The map's playhead is refused: {error}")).ok()
    }

    fn made(self) -> io::Result<MapSide> {
        let Self { content: document, replay, branch: recording, hour, at, weather } = self;
        let units = units::of(&replay, recording)?;
        let (engaged, branch) = (crate::engaged_index::lanes::shared(&replay, &units, recording, &viewer_foundation::map::perspective::Country::Recorded)?, recording);
        let entry = open(units, settings())?;
        entry.set_capture(replay.capture_mode);
        let content = Arc::new(crate::layers::political::Content::new(Arc::clone(&document.map), &document.border_meshes));
        let place_in = Arc::clone(&document);
        let map = &document.map;
        let largest = map.definitions.last_key_value().map_or(0, |(&id, _)| id).max(map.province_ids.iter().copied().max().unwrap_or(0));
        let intel = viewer_foundation::map::textures::IntelCells::new(map).map(|cells| Arc::new(crate::layers::intel::Content::new(cells, largest))).map_err(|error| error.to_string());
        let snow = document.products.snow_content().map(|content| Arc::new(crate::layers::snow::Content::new(content.cells.clone(), content.winter.clone()))).map_err(|error| error.to_string());
        let counters = crate::layers::counters::Content::new(Arc::clone(&document.counters), document.movement.clone(), (viewer_foundation::map::camera::MapSize::of(&document.map), document.map_defines), document.tables);
        let marker_index = || crate::layers::engagements::Index { history: Arc::clone(&engaged), branch };
        let (land_index, naval_index) = (marker_index(), marker_index());
        let guns = crate::layers::gun_counters::Content::new(Arc::clone(&document.counters), Arc::clone(&document.map), document.movement.clone());
        let mut layers: Vec<Box<dyn Layer>> = vec![
            Box::new(crate::layers::political::Political::new(content)),
            // Every stock mode but the default: the fill, colours and borders the political layer draws there.
            Box::new(crate::layers::mode_fills::new(Arc::clone(&document) as Arc<dyn crate::layers::mode_fills::Paint>, false, false)?.in_window(Arc::clone(&entry), rows::Recording { branch: recording })?),
            Box::new(crate::layers::vision::Vision::new(Arc::new(crate::layers::vision::Content::new(document.products.fog.as_ref().clone())))),
            Box::new(crate::layers::intel::Intel::new(intel)),
            Box::new(crate::layers::radar::Radar::new()),
            weather,
            Box::new(crate::layers::snow::Snow::new(snow)),
            Box::new(crate::layers::sounds::SoundsFed::new(crate::layers::sounds::Sounds::new(document.audio.as_ref().map(|source| Arc::clone(&source.content))))),
            Box::new(crate::layers::strikes::Strikes::new(Arc::new(move |province| place_in.products.impact_place(province)))),
            Box::new(crate::layers::map_text::MapText::new(Arc::clone(&document) as Arc<dyn crate::layers::map_text::Source>, recording).with_rows(rows::Rows::of(&entry, rows::Recording { branch: recording })?)),
            Box::new(crate::layers::convoys::Convoys::new(Arc::clone(&document) as Arc<dyn crate::layers::convoys::Routes>).in_window()),
            // Before units, which joins the ships to its models while they stand in for today's.
            Box::new({
                let ships = crate::layers::ships::Ships::new(Arc::clone(&document) as Arc<dyn crate::layers::ships::Naval>, false)?.in_window();
                ships.set_source(Arc::clone(&entry), rows::Recording { branch: recording })?;
                ships
            }),
            Box::new(crate::layers::aircraft::new(Arc::clone(&document), false)?.in_window(Arc::clone(&entry), rows::Recording { branch: recording })?),
            Box::new(
                crate::layers::units::Units::new(Arc::clone(&document) as Arc<dyn crate::layers::units::Ground>)
                    .in_window(Arc::clone(&document) as Arc<dyn crate::layers::units::models::Select>, &rows::Rows::of(&entry, rows::Recording { branch: recording })?),
            ),
            Box::new({
                let orders = crate::layers::orders::Orders::new(Arc::clone(&document) as Arc<dyn crate::layers::orders::Plans>).in_window();
                orders.attach(rows::Rows::of(&entry, rows::Recording { branch: recording })?);
                orders
            }),
            Box::new(crate::layers::buildings::BuildingsFed::new(crate::layers::buildings::Buildings::reading_vision(Arc::clone(&document) as Arc<dyn crate::layers::buildings::Catalogue>))),
            Box::new(crate::layers::naval::Naval::new(Arc::new(document.naval_dominance.clone()))),
            Box::new(crate::layers::engagements::Engagements::new(Some(crate::layers::engagements::Index { history: engaged, branch }), None).reading_vision()),
            Box::new(crate::layers::intel_icons::Icons::new(Arc::clone(&document) as Arc<dyn crate::layers::intel_icons::Evaluate>, Some((Arc::clone(&entry), rows::Recording { branch: recording })), rows::Cadence::Ready)),
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
            Box::new(crate::layers::names::Names::new(Arc::clone(&document) as Arc<dyn crate::layers::names::Atlas>).in_window(Arc::clone(&entry), rows::Recording { branch: recording })?),
            Box::new(crate::layers::labels::Labels::new(Arc::clone(&document.names.localisation), None).reading_vision()),
        ];
        if let Some(names) = REGISTERED.get() {
            layers.retain(|layer| names.iter().any(|name| name == layer.name()));
        }
        let anchored = entry.anchored();
        let mut side = MapSide::new(Arc::clone(&entry), layers, View::default())?;
        side.branch = Some(recording);
        side.set_sight(Arc::clone(&document));
        if !anchored {
            let (hour, playing, speed) = at.map_or((hour, false, 0.0), |at| (at.hour, at.playing, at.speed));
            entry.anchor(hour, playing, speed, true);
        }
        Ok(side)
    }
}

/// A map's reader being made on a worker; the frame that started it goes on drawing.
pub struct Opening {
    made: Receiver<Option<MapSide>>,
}

impl Opening {
    /// The reader for `document` at clock `at`, made off the caller's thread.
    pub fn start(document: &crate::document::Document, at: Option<At>) -> Option<Self> {
        let made = ForMap::of(document, at);
        Self::spawn(move || made.make())
    }

    /// `make` on a thread of its own; none, printed by name, when no thread can be started.
    pub fn spawn(make: impl FnOnce() -> Option<MapSide> + Send + 'static) -> Option<Self> {
        let (send, made) = mpsc::channel();
        let started = std::thread::Builder::new().name(OPENING.into()).spawn(move || {
            viewer_foundation::worker_priority::background_priority();
            // A reader no longer wanted is dropped here, off the frame thread.
            let _ = send.send(make());
        });
        match started {
            Ok(_) => Some(Self { made }),
            Err(error) => {
                eprintln!("The map's playhead is refused: no thread to open it: {error}");
                None
            }
        }
    }

    /// The reader once made: `Some(None)` when it was refused or its thread stopped; none while it
    /// is being made. Never waits.
    pub fn poll(&self) -> Option<Option<MapSide>> {
        match self.made.try_recv() {
            Ok(side) => Some(side),
            Err(TryRecvError::Empty) => None,
            Err(TryRecvError::Disconnected) => Some(None),
        }
    }
}

/// The name of the thread an [`Opening`] makes its reader on.
pub const OPENING: &str = "map reader opening";

/// One frame of the map's reader at the view the map draws, its head from `at`. The first error,
/// the reader's or its playhead's, is printed by name and the reader stops ([`MapSide::failure`]).
pub fn draw_frame(side: &mut MapSide, view: View, at: Head) {
    if side.failure.is_some() {
        return;
    }
    let result = (|| {
        if let Some(error) = side.entry.playhead.error() {
            return Err(io::Error::other(format!("The playhead stopped: {error}")));
        }
        side.set_view(view)?;
        match at {
            Head::Clock(now) => side.frame(side.head(now)).map(drop),
            Head::Paced(head) => side.frame(head).map(drop),
            Head::Stepped(head) => side.catch_up(head),
        }
    })();
    if let Err(error) = result {
        side.fail(format!("The map's playhead stopped: {error}"));
    }
}

/// Today's parts that layers replace whole, each with the layers that together draw it; a part
/// joins once its layers' act 2 has passed (pixels equal to today's).
pub const REPLACES: &[(&str, &[&str])] = &[("political", &["political", "naval"])];

/// Where a frame's head comes from: the playhead's clock in the window, the harness's head paced,
/// or a stepped harness frame that waits until its hour is applied.
#[derive(Clone, Copy, Debug)]
pub enum Head {
    Clock(Instant),
    Paced(Hour),
    Stepped(Hour),
}

/// The drawn layers' upload into the scene after the frame's settle; `stepped` uploads whole.
pub fn upload_frame(side: &mut MapSide, gpu: &mut Gpu<'_>, stepped: bool) {
    if side.failure.is_some() {
        return;
    }
    if let Err(error) = side.upload(gpu, stepped) {
        side.fail(format!("The map's playhead stopped at an upload: {error}"));
    }
}

enum Job {
    Compose(Box<Away>),
    Drop(Box<dyn Send>),
}

/// The mirrors and layers on the worker while a reset composes.
struct Away {
    mirrors: Box<Mirrors>,
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
                viewer_foundation::worker_priority::background_priority();
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
            viewer_foundation::worker_priority::background_priority();
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
    /// Entities dirty after settling, carried to the next frame, and the frames since the oldest
    /// of them was marked.
    pub carried: usize,
    pub late: u64,
    /// Entities dirty before settling.
    pub dirty: usize,
    pub upload_ns: u64,
    pub upload_cycles: u64,
    /// GPU writes and their bytes counted while it uploaded (another thread's writes in that
    /// window included).
    pub uploads: u64,
    pub upload_bytes: u64,
    /// What it settled is not all on the GPU yet (out of budget), after this frame's upload.
    pub upload_carried: bool,
    /// Drawn: declared, composed and not refused.
    pub ready: bool,
    /// The hour its published product is current for, after this frame.
    pub shown: Option<Hour>,
    /// Its settle or its upload alone took more than the whole share it was given.
    pub overran: bool,
    /// It shows a product made since the last compose swapped in: after a seek, the sought picture.
    pub reset_shown: bool,
    /// A layer fed the steps ahead, in a frame that applied a step: whether the product it shows
    /// after its settle is the applied hour's (made ahead and ready), or an older one.
    pub ready_at_hour: Option<bool>,
    /// Handing it the steps ahead, on the frame thread.
    pub ahead_ns: u64,
    /// The entities it settled this frame, and of them those the view shows (paced runs only).
    pub scope: Option<crate::layers::Scope>,
    /// The units of work its settle was given this frame (its budget answered yes to each); none
    /// when it did not settle (composing, or held for a layer it reads).
    pub given: Option<u32>,
}

/// A test's hold on every settle of a map side ([`MapSide::hold_settles`]), released when dropped.
#[cfg(any(test, feature = "test-support"))]
#[must_use = "the settles are released as soon as this is dropped"]
pub struct SettlesHeld(Arc<std::sync::atomic::AtomicBool>);

#[cfg(any(test, feature = "test-support"))]
impl Drop for SettlesHeld {
    fn drop(&mut self) {
        self.0.store(false, Ordering::Release);
    }
}

/// What a frame did.
#[derive(Clone, Debug, Default)]
pub struct Frame {
    pub head: Hour,
    pub steps: u32,
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
    /// The stepper's wall time building the steps taken this frame, summed, and the most one took.
    pub step_ns: u64,
    /// This map's share of the frame thread's budget this frame.
    pub slice_ns: u64,
    pub step_most_ns: u64,
    /// The part of the slice reserved to the uploads this frame.
    pub upload_share_ns: u64,
    /// The layers whose settle or upload alone took more than the whole share it was given.
    pub overran: Vec<&'static str>,
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
        self.mirrors.iter().map(|(_, bytes)| bytes).sum::<usize>() + self.layers.iter().map(|(_, bytes)| bytes).sum::<usize>() + self.states.1
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
    mirrors: Option<Box<Mirrors>>,
    layers: Vec<Box<dyn Layer>>,
    worker: Worker,
    composing: bool,
    pending: VecDeque<Arc<Step>>,
    taken: Vec<Arc<Step>>,
    /// A view asked for while composing, taken once the layers are back.
    view_later: Option<View>,
    /// The sizes last seen, kept while the mirrors and layers are away.
    sizes: (Vec<(TableId, usize)>, Vec<(&'static str, usize)>),
    /// The frame thread's whole budget, which every map it draws in a frame shares.
    budget: Duration,
    /// This map's share of it this frame, and what its settle spent of that.
    slice: Duration,
    settled: Duration,
    /// Its place in the shared budget of the thread that draws it, taken at its first frame: it may
    /// be made on another thread ([`Opening`]).
    share: Option<u64>,
    /// Per layer: its changes are cleared at the next frame's start (every drawn dependent took them).
    clear: Vec<bool>,
    /// Per layer: what its uploads cost lately when given budget with work to do, which the
    /// uploads' share reserves.
    upload_cost: Vec<Duration>,
    /// Per layer: the hour its published product is current for; the map shows the oldest.
    shown: Vec<Option<Hour>>,
    /// Per layer: the frame since which it has carried work, which orders the layers' turns, and
    /// the frame of its last turn with budget left, which orders equals.
    waiting: Vec<Option<u64>>,
    turn: Vec<u64>,
    /// The last hour applied, and the hour the layers last showed whole.
    applied: Option<Hour>,
    drawn: Option<Hour>,
    /// The hour of the oldest product drawn, every drawn layer counted ([`Frame::oldest`]).
    oldest: Option<Hour>,
    /// The naval attributes of today's political part, not yet handed to the political layer.
    naval: Option<Arc<std::collections::BTreeMap<u32, viewer_world::map::gradient::Attributes>>>,
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
    /// Per layer, as last declared: its own declaration, and what of it the camera decides
    /// ([`Layer::placed`]; none: it reads the camera in `inputs` only).
    declared_each: Vec<LayerDeclared>,
    placed: Vec<Option<u64>>,
    /// A test holds every settle while a [`SettlesHeld`] it took lives: the steps' marks are
    /// carried, nothing made.
    #[cfg(any(test, feature = "test-support"))]
    settles_held: Arc<std::sync::atomic::AtomicBool>,
    /// The content the count of settled entities in view reads its provinces and states from.
    sight: Option<Arc<crate::document::Content>>,
    /// Per layer: the frame since which its upload has been carried, which orders the uploads.
    upload_waiting: Vec<Option<u64>>,
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
    /// Per layer fed the steps ahead: the last hour of the newest step of hours handed to it since
    /// it composed, and the newest [`Step::restep`] it was handed.
    offered: Vec<Option<Hour>>,
    resteps: Vec<u64>,
    /// Per layer: the hour applied at each of its settles, newest last, as many as the age of the
    /// oldest mark it owes (`Carried::frames`, in its settles) needs: a layer current but carrying
    /// marks shows the hour of the settle before its oldest mark.
    settled_at: Vec<VecDeque<Option<Hour>>>,
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

impl MapSide {
    /// The map's reader of `entry` with `layers` drawn at `view`; the playhead is sought to `hour`
    /// unless another reader already holds it.
    pub fn new(entry: Arc<Entry>, mut layers: Vec<Box<dyn Layer>>, view: View) -> io::Result<Self> {
        let names = layers.iter().map(|layer| layer.name()).collect();
        let Declaring { union: declared, reads, upstream, each } = declare(&entry, &mut layers, &view);
        let placed = layers.iter_mut().map(|layer| layer.placed(&view)).collect();
        let reader = entry.playhead.declare(to_declared(&declared), Some(mirror::rebuild(Arc::clone(&entry.pages))))?;
        let consumer = entry.playhead.consumer(reader);
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
            mirrors: Some(Box::default()),
            layers,
            worker: Worker::new()?,
            composing: false,
            pending: VecDeque::new(),
            taken: Vec::new(),
            view_later: None,
            sizes: Default::default(),
            budget: BUDGET,
            slice: BUDGET,
            settled: Duration::ZERO,
            share: None,
            clear: Vec::new(),
            waiting: Vec::new(),
            upload_cost: Vec::new(),
            shown: Vec::new(),
            turn: Vec::new(),
            applied: None,
            drawn: None,
            oldest: None,
            naval: None,
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
            upload_waiting: Vec::new(),
            compose_ns: Vec::new(),
            epoch: 0,
            cancel: Arc::default(),
            superseded: false,
            fresh: Vec::new(),
            reset_shown: false,
            offered: Vec::new(),
            resteps: Vec::new(),
            settled_at: Vec::new(),
        })
    }

    /// Each layer's name, in registration order, whether or not its parts are home.
    pub fn names(&self) -> &[&'static str] {
        &self.names
    }

    pub fn entry(&self) -> &Arc<Entry> {
        &self.entry
    }

    pub fn set_budget(&mut self, budget: Duration) {
        self.budget = budget;
    }

    /// The content whose provinces and states the paced count of settled entities in view reads.
    pub fn set_sight(&mut self, content: Arc<crate::document::Content>) {
        self.sight = Some(content);
    }

    /// Whether its layers were made over `content`, the content whose art they draw.
    pub fn made_over(&self, content: *const crate::document::Content) -> bool {
        self.sight.as_ref().is_some_and(|sight| std::ptr::eq(Arc::as_ptr(sight), content))
    }

    /// No layer settles until the guard returned is dropped: a test keeps a layer's marks across
    /// several steps, which a spent budget no longer does (each layer makes one unit a frame
    /// whatever the budget).
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
            let _lap = replay_frame::cpu::scope("frame.refresh.placed", Some("other"));
            self.place(&view)
        } {
            return Ok(());
        }
        let _lap = replay_frame::cpu::scope("frame.refresh.declare", Some("other"));
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
        if declared != self.declared {
            if let Some(mirrors) = self.mirrors.as_deref_mut() {
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
            self.entry.playhead.redeclare(self.reader, to_declared(&self.declared))?;
        }
        Ok(())
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

    /// One frame at `head`: the steps it reached taken, each applied to the mirrors and then to
    /// the layers, and the layers settled within the budget. Never waits.
    pub fn frame(&mut self, head: Hour) -> io::Result<Frame> {
        self.frame += 1;
        let share = *self.share.get_or_insert_with(share::join);
        share::begin(share);
        let mut frame = Frame { head, layers: vec![LayerFrame::default(); self.names.len()], ..Frame::default() };
        let take_lap = replay_frame::cpu::scope("frame.refresh.take", Some("other"));
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
        let apply_lap = replay_frame::cpu::scope("frame.refresh.apply", Some("other"));
        let mut compose = vec![false; self.layers.len()];
        let mut applied = Vec::with_capacity(steps.len());
        let (resets_before, applied_before) = (self.resets_taken, self.applied);
        // Steps wait, in order: those taken with a reset for the frame after its swap (the compose
        // is of the mirrors at the reset's hour, so the first picture after a seek is the hour
        // sought), and any past the head this frame was given for a frame whose head reaches it.
        let (mut waiting, mut after_reset) = (self.composing, false);
        while let Some(step) = steps.pop_front() {
            waiting |= step.last() > head;
            if waiting {
                self.pending.push_back(step);
                continue;
            }
            let reset = step.reset();
            self.take_step(&step, &mut frame, &mut compose);
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
        drop(apply_lap);
        let settles_lap = replay_frame::cpu::scope("frame.refresh.settles", Some("other"));
        let settling = crate::layers::clock::now();
        if !self.composing {
            let mirrors = self.mirrors.as_deref().expect("the mirrors are home while not composing");
            let composing = compose.iter().any(|&wanted| wanted);
            self.slice = share::slice(share, self.budget);
            frame.slice_ns = self.slice.as_nanos() as u64;
            self.upload_cost.resize(self.layers.len(), Duration::ZERO);
            let reserved = upload_share(self.slice, &self.upload_cost);
            frame.upload_share_ns = reserved.as_nanos() as u64;
            let share_of_settles = self.slice.saturating_sub(reserved);
            let mut budget = Budget::new(share_of_settles);
            self.clear.resize(self.layers.len(), false);
            for (layer, clear) in self.layers.iter_mut().zip(&mut self.clear) {
                if std::mem::take(clear) {
                    layer.clear_changed();
                }
            }
            let mut settled = vec![false; self.layers.len()];
            self.waiting.resize(self.layers.len(), None);
            self.offered.resize(self.layers.len(), None);
            self.resteps.resize(self.layers.len(), 0);
            self.settled_at.resize_with(self.layers.len(), VecDeque::new);
            let (mut lent, mut handed): (Option<Vec<Arc<Step>>>, Vec<Arc<Step>>) = (None, Vec::new());
            // The host's count, or the layer's when it knows its work is older (marked before a compose).
            let late: Vec<u64> = self.layers.iter().zip(&self.waiting).map(|(layer, since)| since.map_or(0, |since| self.frame - since).max(layer.carried().frames)).collect();
            self.turn.resize(self.layers.len(), 0);
            // Per layer settled: what it carried after, for its in-view count once every layer settled.
            let mut counted: Vec<(usize, usize)> = Vec::with_capacity(self.layers.len());
            let mut mark = replay_frame::cpu::thread_cycles();
            for at in oldest_first(&late, &self.turn, &self.upstream, self.frame) {
                if !(self.reads[at].is_ok() && self.ready[at]) || compose[at] || self.superseded {
                    // A layer it draws that is not composed yet: drawn late, counted in the flight record.
                    if self.reads[at].is_ok() && (!self.ready[at] || compose[at]) {
                        viewer_foundation::view_record::add("layers", "frames_not_ready", 1);
                    }
                    continue;
                }
                let (before, rest) = self.layers.split_at_mut(at);
                let (layer, upstream) = (&mut rest[0], Upstream::new(before));
                let part = &mut frame.layers[at];
                let started = Instant::now();
                for step in &applied {
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
                // An upstream not composed, or with frame-thread work left, is half-settled; its
                // worker's work is not carried, so a dependent reads its published product.
                let held = self.upstream[at].iter().any(|&upper| !self.ready[upper] || before[upper].carried().entities > 0);
                part.dirty = layer.carried().entities;
                #[cfg(any(test, feature = "test-support"))]
                let held = held || self.settles_held.load(Ordering::Acquire);
                if !composing && !held {
                    settled[at] = true;
                    let given = budget.left();
                    if given {
                        self.turn[at] = self.frame;
                    }
                    let started = crate::layers::clock::now();
                    // One unit of a layer's work goes whatever the budget: another's spending never
                    // leaves it, and the layers held while it carries, standing still.
                    let mut units = budget.first_free();
                    layer.settle_with(mirrors, &upstream, &mut units)?;
                    part.given = Some(units.given());
                    self.settled_at[at].push_back(self.applied);
                    let spent = crate::layers::clock::now().duration_since(started);
                    part.settle_ns = spent.as_nanos() as u64;
                    let now = replay_frame::cpu::thread_cycles();
                    part.settle_cycles = now.saturating_sub(std::mem::replace(&mut mark, now));
                    if given && spent > share_of_settles {
                        part.overran = true;
                        frame.overran.push(self.names[at]);
                    }
                }
                counted.push((at, layer.carried().entities));
                part.ready = true;
                if fed && !applied.is_empty() {
                    part.ready_at_hour = Some(layer.published() == crate::layers::Published::Current);
                }
                let carried = layer.carried();
                if let Some(ring) = self.settled_at.get_mut(at) {
                    let keep = usize::try_from(carried.frames).unwrap_or(usize::MAX).saturating_add(1);
                    while ring.len() > keep {
                        ring.pop_front();
                    }
                }
                // Counted by the host: a layer starved of budget never settles to count its own frames.
                self.waiting[at] = (carried.entities > 0).then(|| self.waiting[at].unwrap_or(self.frame));
                (part.carried, part.late) = (carried.entities, self.waiting[at].map_or(0, |since| self.frame - since).max(carried.frames));
            }
            // The harness's in-view count, under one label for every layer: a label's two reads of
            // the thread's cycle time each, not two per layer.
            {
                let _in_view = replay_frame::cpu::scope("frame.in-view", Some("other"));
                let content = self.sight.clone();
                let sight = crate::layers::Sight::of(content.as_deref(), &self.view);
                for (at, left) in counted {
                    let part = &mut frame.layers[at];
                    part.scope = self.layers[at].scope(&sight).or_else(|| settled[at].then(|| crate::layers::Scope { settled: part.dirty.saturating_sub(left) as u64, in_view: None }));
                }
            }
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
        let _shown_lap = replay_frame::cpu::scope("frame.refresh.shown", Some("other"));
        // A step is taken once the head reaches its last hour; paused inside one, it never is.
        let playhead = &self.entry.playhead;
        if !self.composing
            && !self.superseded
            && !swapped
            && !self.pending.iter().any(|step| step.last() <= head)
            && (self.stepped || self.entry.paused())
            && playhead.holding().is_none()
            && let Some(applied) = self.applied
            && (applied > head || (applied < head && playhead.newest().is_some_and(|newest| newest > head) && !self.consumer.reaches(head)))
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
        self.settled = crate::layers::clock::now().duration_since(settling);
        share::spent(share, self.settled, frame.layers.iter().map(|part| part.late).max().unwrap_or(0), true);
        (frame.drawn, frame.oldest) = (self.drawn, self.oldest);
        // The head the rows follow is the hour this map's picture is at: while a reset composes, at
        // its swap and when steps wait for after it, the reset's hour, not the head it was given.
        let took = if (self.composing && !self.superseded) || swapped || after_reset { self.applied.map_or(head, |hour| hour.min(head)) } else { head };
        // With the seek of the steps applied: until this map takes a seek's reset, its head is
        // the seek before's, which the rows do not follow.
        let held = seek_head(took, self.epoch);
        if self.entry.head.swap(held, Ordering::AcqRel) != held {
            rows::wake(&self.entry);
        }
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
        let mut whole = self.reads.iter().zip(&self.ready).all(|(reads, &ready)| ready || reads.is_err());
        let mut mode_settled = true;
        let mut oldest: Option<Hour> = None;
        let mut every: Option<Hour> = None;
        let mut drawn = 0;
        let mut fresh = whole;
        for (at, layer) in self.layers.iter().enumerate() {
            if !(self.reads[at].is_ok() && self.ready[at]) {
                continue;
            }
            if layer.uploaded() {
                (self.shown[at], self.fresh[at]) = match layer.published() {
                    Published::Current if layer.carried().entities == 0 => (self.applied, true),
                    Published::Current => (Self::before_mark(self.settled_at.get(at), layer.carried().frames).unwrap_or(self.shown[at]), self.fresh[at]),
                    Published::At(hour) | Published::Cadence(hour) => (Some(hour), true),
                    Published::Nothing => (None, false),
                };
            }
            if let Some(hour) = self.shown[at] {
                every = Some(every.map_or(hour, |held| held.min(hour)));
            }
            if matches!(layer.published(), Published::Cadence(_)) {
                continue;
            }
            self.mode_settled[at] |= layer.uploaded() && layer.carried().entities == 0 && !matches!(layer.published(), Published::Nothing);
            mode_settled &= self.mode_settled[at];
            fresh &= self.fresh[at];
            drawn += 1;
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
        self.reset_shown = fresh;
    }

    /// The hour of the settle before a layer's oldest outstanding mark, `age` of its settles ago;
    /// the oldest kept when it reports an age beyond them; none before its first settle.
    fn before_mark(ring: Option<&VecDeque<Option<Hour>>>, age: u64) -> Option<Option<Hour>> {
        let ring = ring.filter(|ring| !ring.is_empty())?;
        let back = usize::try_from(age).unwrap_or(usize::MAX);
        Some(ring[ring.len().saturating_sub(back.saturating_add(1))])
    }

    /// The last frame's record, its upload included.
    pub fn last_frame(&self) -> &Frame {
        &self.last
    }

    /// Each layer's name, whether it draws, what it carried and whether its upload was left, as
    /// the last frame found them: why a frame was not whole.
    pub fn layer_states(&self) -> Vec<(&'static str, bool, usize, bool)> {
        self.names.iter().enumerate().map(|(at, &name)| (name, self.reads[at].is_ok() && self.ready[at], self.last.layers.get(at).map_or(0, |part| part.carried), self.last.layers.get(at).is_some_and(|part| part.upload_carried))).collect()
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

    /// The naval attributes of the sea regions in today's political part, for the political
    /// fill to carry until they are a layer.
    pub fn set_naval(&mut self, attributes: &std::collections::BTreeMap<u32, viewer_world::map::gradient::Attributes>) {
        if self.naval.as_deref() != Some(attributes) {
            self.naval = Some(Arc::new(attributes.clone()));
        }
    }

    /// Today's parts the drawn layers replace whole: those whose every replacing layer draws.
    pub fn drawn_parts(&self) -> impl Iterator<Item = &'static str> + '_ {
        REPLACES.iter().filter(|(_, layers)| layers.iter().all(|layer| self.draws(layer))).map(|&(part, _)| part)
    }

    /// Whether layer `name` is drawn: declared, composed and not refused.
    pub fn draws(&self, name: &str) -> bool {
        self.names.iter().enumerate().any(|(at, held)| *held == name && self.reads[at].is_ok() && self.ready[at])
    }

    /// Frames until every step to `head` is applied and composed, then everything settled whole:
    /// the stepped harness's frame, which waits for its hour as the stepped frame of rows does.
    pub fn catch_up(&mut self, head: Hour) -> io::Result<()> {
        self.stepped = true;
        let started = Instant::now();
        loop {
            let newest = self.entry.playhead.newest().unwrap_or(head);
            self.frame(head.min(newest.max(self.applied.unwrap_or(newest))))?;
            if !self.composing && !self.superseded && self.applied == Some(head) && self.ready.iter().zip(&self.reads).all(|(&ready, reads)| ready || reads.is_err()) {
                return self.settle_all();
            }
            if let Some(error) = self.entry.playhead.error() {
                return Err(io::Error::other(format!("The playhead stopped: {error}")));
            }
            if started.elapsed() > Duration::from_secs(60) {
                let waiting: Vec<&str> = self.names.iter().zip(self.ready.iter().zip(&self.reads)).filter(|(_, (ready, reads))| !**ready && reads.is_ok()).map(|(name, _)| *name).collect();
                let (report, _, counters) = self.entry.playhead.held();
                return Err(io::Error::other(format!(
                    "Hour {head} was not reached by the map's layers in a minute: applied {:?}, newest decoded {:?}, composing {}, not composed {waiting:?}, steps held here {}, ring steps {}, fetches queued {}, jobs running {}, preparing {}, {counters:?}",
                    self.applied,
                    self.entry.playhead.newest(),
                    self.composing,
                    self.pending.len() + self.taken.len(),
                    report.ring_steps,
                    report.fetches_queued,
                    report.jobs_running,
                    report.preparing,
                )));
            }
            std::thread::sleep(Duration::from_micros(200));
        }
    }

    /// Settles every layer whole, whatever the budget: what the proof tool compares.
    pub fn settle_all(&mut self) -> io::Result<()> {
        self.settled_at.resize_with(self.layers.len(), VecDeque::new);
        if let Some(mirrors) = self.mirrors.as_deref().filter(|_| !self.superseded) {
            for at in 0..self.layers.len() {
                if self.reads[at].is_ok() && self.ready[at] {
                    let (before, rest) = self.layers.split_at_mut(at);
                    rest[0].settle_with(mirrors, &Upstream::new(before), &mut Budget::unlimited())?;
                    self.settled_at[at].push_back(self.applied);
                }
            }
        }
        Ok(())
    }

    /// Writes the layers' changed ranges within the uploads' share of the slice, or what the
    /// settles left when that is more (`unlimited`: all).
    pub fn upload(&mut self, gpu: &mut Gpu<'_>, unlimited: bool) -> io::Result<()> {
        if self.composing || self.superseded {
            return Ok(());
        }
        let _lap = replay_frame::cpu::scope("frame.refresh.uploads", Some("other"));
        let today = self.naval.take();
        let naval = match self.names.iter().position(|&name| name == "naval").filter(|_| self.draws("naval")) {
            Some(at) if self.draws("political") => self.layers[at].any_mut().and_then(|layer| layer.downcast_mut::<crate::layers::naval::Naval>()).and_then(crate::layers::naval::Naval::take_changed),
            Some(_) => None,
            None => today,
        };
        if let Some(naval) = &naval {
            for (at, layer) in self.layers.iter_mut().enumerate() {
                if self.reads[at].is_ok()
                    && self.ready[at]
                    && let Some(political) = layer.any_mut().and_then(|layer| layer.downcast_mut::<crate::layers::political::Political>())
                {
                    political.set_naval(naval);
                }
            }
        }
        self.upload_with(unlimited, |layer, budget| layer.upload(gpu, budget))
    }

    /// The upload pass, each drawn layer's upload made by `each`: the uploads carried longest go
    /// first, and the first with work always makes progress, even with the share spent.
    /// The upload pass without a GPU, `each` making each layer's upload: a layer's tests.
    #[cfg(any(test, feature = "test-support"))]
    pub fn upload_each(&mut self, each: impl FnMut(&mut dyn Layer, &mut Budget) -> io::Result<()>) -> io::Result<()> {
        self.upload_with(false, each)
    }

    fn upload_with(&mut self, unlimited: bool, mut each: impl FnMut(&mut dyn Layer, &mut Budget) -> io::Result<()>) -> io::Result<()> {
        use crate::layers::clock;
        let uploading = clock::now();
        self.upload_cost.resize(self.layers.len(), Duration::ZERO);
        self.upload_waiting.resize(self.layers.len(), None);
        let given = upload_share(self.slice, &self.upload_cost).max(self.slice.saturating_sub(self.settled));
        let mut budget = if unlimited { Budget::unlimited() } else { Budget::new(given) };
        let late: Vec<u64> = self.upload_waiting.iter().map(|since| since.map_or(0, |since| self.frame - since)).collect();
        let seen_first: Vec<bool> = self.names.iter().map(|name| SEEN_FIRST.contains(name)).collect();
        let mut first = true;
        // Read once between uploads: each runs from the read before, the loop's bookkeeping in it.
        let mut mark = replay_frame::cpu::thread_cycles();
        for at in uploads_first(&late, &seen_first) {
            if !(self.reads[at].is_ok() && self.ready[at]) {
                continue;
            }
            let layer = &mut self.layers[at];
            let owed = !layer.uploaded();
            let free = owed && std::mem::take(&mut first);
            let budgeted = free || budget.left();
            let started = clock::now();
            let counted = shader_pipeline::gpu::stats::peek();
            if free {
                each(&mut **layer, &mut budget.first_free())?;
            } else {
                each(&mut **layer, &mut budget)?;
            }
            let spent = clock::now().duration_since(started);
            let now = replay_frame::cpu::thread_cycles();
            let cycles = now.saturating_sub(std::mem::replace(&mut mark, now));
            let written = |count: shader_pipeline::gpu::stats::Count| shader_pipeline::gpu::stats::peek()[count as usize].saturating_sub(counted[count as usize]);
            let carried = !layer.uploaded();
            // A starved upload's cost is not a measurement: only one given budget with work to do.
            if !unlimited && owed && budgeted {
                self.upload_cost[at] = (self.upload_cost[at] * 3 + spent) / 4;
            }
            self.upload_waiting[at] = carried.then(|| self.upload_waiting[at].unwrap_or(self.frame));
            if let Some(part) = self.last.layers.get_mut(at) {
                part.upload_ns = spent.as_nanos() as u64;
                part.upload_cycles = cycles;
                part.upload_carried = carried;
                (part.uploads, part.upload_bytes) = (written(shader_pipeline::gpu::stats::Count::Uploads), written(shader_pipeline::gpu::stats::Count::UploadBytes));
                if !unlimited && budgeted && spent > given && !part.overran {
                    part.overran = true;
                    self.last.overran.push(self.names[at]);
                }
            }
        }
        if !unlimited
            && let Some(share) = self.share
        {
            share::spent(share, clock::now().duration_since(uploading), 0, false);
        }
        self.update_shown();
        for ((part, shown), fresh) in self.last.layers.iter_mut().zip(&self.shown).zip(&self.fresh) {
            (part.shown, part.reset_shown) = (*shown, *fresh);
        }
        (self.last.drawn, self.last.oldest, self.last.reset_shown) = (self.drawn, self.oldest, self.reset_shown);
        Ok(())
    }

    /// A step into the mirrors (a reset's rebuilt mirrors swapped in first); the layers reading a
    /// table it resets or renumbers are marked to compose.
    fn take_step(&mut self, step: &Arc<Step>, frame: &mut Frame, compose: &mut [bool]) {
        let mut reset_tables = BTreeSet::new();
        let mirrors = self.mirrors.as_deref_mut().expect("the mirrors are home while not composing");
        for changes in step.tables() {
            if changes.reset()
                && let Some(state) = self.consumer.state(step, changes.table())
                && let Ok(mut rebuilt) = state.downcast::<Mirror>()
            {
                rebuilt.set_capture(self.entry.capture.get().copied());
                if let Some(old) = mirrors.insert(rebuilt) {
                    self.worker.drop_later(old);
                }
                reset_tables.insert(changes.table());
            }
            if changes.renumber().is_some() {
                reset_tables.insert(changes.table());
            }
        }
        let (started, cycles) = (Instant::now(), replay_frame::cpu::thread_cycles());
        mirrors.apply_timed(step, &mut frame.mirror_tables);
        frame.mirror_ns += started.elapsed().as_nanos() as u64;
        frame.mirror_cycles += replay_frame::cpu::thread_cycles().saturating_sub(cycles);
        frame.steps += 1;
        frame.step_ns += step.built().ns;
        frame.step_most_ns = frame.step_most_ns.max(step.built().ns);
        frame.cells += step.built().cells;
        frame.rows += step.built().rows;
        frame.applied = Some(step.last());
        self.applied = Some(step.last());
        self.epoch = step.epoch();
        if step.reset() {
            frame.sought = frame.sought.or(step.built().sought);
            self.resets_taken += 1;
        }
        for (wanted, reads) in compose.iter_mut().zip(&self.reads) {
            *wanted |= reads.as_ref().is_ok_and(|tables| step.reset() || tables.iter().any(|table| reset_tables.contains(table)));
        }
    }

    fn send_away(&mut self, compose: Vec<bool>) {
        let mirrors = self.mirrors.take().expect("the mirrors are home");
        self.sizes = (mirrors.iter().map(|mirror| (mirror.table(), mirror.bytes())).collect(), self.layers.iter().map(|layer| (layer.name(), layer.held())).collect());
        let layers = std::mem::take(&mut self.layers);
        self.compose_ns.resize(layers.len(), 0);
        self.cancel = Arc::default();
        let away = Away { mirrors, layers, compose, upstream: self.upstream.clone(), cost: self.compose_ns.clone(), cancel: Arc::clone(&self.cancel), results: Vec::new() };
        if self.worker.jobs.send(Job::Compose(Box::new(away))).is_ok() {
            self.composing = true;
            self.superseded = false;
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
                self.settled_at.resize_with(self.layers.len(), VecDeque::new);
                self.settled_at[at] = VecDeque::from([self.applied]);
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
        Held { mirrors, layers, pending: self.pending.len(), states: (report.states, report.state_bytes), playhead: report.total }
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

    #[cfg(any(test, feature = "test-support"))]
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
        if let Some(share) = self.share {
            share::leave(share);
        }
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

fn to_declared(declared: &BTreeMap<usize, Read>) -> Vec<Declared> {
    declared.iter().map(|(&table, read)| Declared { table, columns: read.columns.clone(), keyed: read.keyed, passing: false }).collect()
}
