//! The window's hour builders: everything the map draws for a recorded hour,
//! built on worker threads ahead of the playhead.
//!
//! The game simulates an hour in about 25 ms and draws every frame of it in
//! a few, so a replay that rebuilt its political fill, railways, buildings,
//! unit models, battle plan, fog, snow and mud, names and icons on the frame
//! that crosses into an hour stalled that frame for hundreds of milliseconds.
//! Here none of that runs on the frame path. The window says what it wants
//! ([`Want`]: the playhead, how far ahead it is heading, the perspective and
//! the camera) and every frame takes whatever hours have
//! finished ([`Built`]); the frame that shows an hour only uploads it.
//!
//! A window's hours are decoded on a lane of their own ([`DecodeLane`]), a
//! window of hours ahead of the builders, through the replay's shared
//! [`BlockReader`], which reads the next block before playback reaches it; a
//! dashboard view uses the source-shared decoder instead. For each decoded
//! hour, the coordinator hands the independent parts of that hour to a pool of
//! worker threads and assembles them as they come back. Hours are dispatched
//! in the order the window will need them — the playhead, the hours ahead of
//! it, then the hours just behind it — and at most one pool's worth at a
//! time, so a seek is answered as soon as the pool drains rather than after a
//! backlog of hours nobody wants any more. An hour the window no longer wants
//! is cancelled before its parts start.
//!
//! Most hours change few of the layers the GPU holds: the political fill and
//! the border classes change when a province changes hands, the railways and
//! buildings when one is built. A part whose value equals the previous hour's
//! shares that hour's value ([`Shared`], [`Political`], [`Borders`]), so the
//! window, which remembers the id of what it uploaded, uploads nothing for it;
//! political workers hold compact region tables. The scene diffs those tables and
//! computes the gradient only when a displayed state changes. Each such part
//! waits for the preceding built hour's value, which was dispatched first and
//! so is already being built, even when the playhead skips past the builders.
//! For the political part the wait is not about the cheap
//! region derivation: an equal hour must keep the same political id, because
//! the border classes are shared, and their upload skipped, only by that id.

use crate::map_views::CounterView;
use std::cell::Cell;
use std::sync::MutexGuard;
use std::{
    collections::{BTreeMap, BTreeSet, HashMap, VecDeque},
    io,
    sync::{
        Arc, Condvar, Mutex, Weak,
        atomic::{AtomicBool, AtomicU64, Ordering},
        mpsc::{self, Receiver, Sender},
    },
    thread,
    time::Instant,
};
use viewer_foundation::builder_pool::Pool;
use viewer_foundation::parallel::Workers;
use viewer_foundation::worker_priority::background_priority;

use replay::container::Replay;
use replay_data::{BranchId, Hour};

use {
    crate::document::Content, crate::document::Hour as HourView, crate::document::OverlayTables,
    crate::document::PreparedOverlay, crate::engaged_index::EngagedIndex, crate::playback::Current,
    crate::playback::Playback, viewer_foundation::blocks::BlockReader,
    viewer_foundation::map::camera::Camera, viewer_foundation::map::memo::Keyed,
    viewer_foundation::map::memo::Recent, viewer_foundation::map::name_draw::Placements,
    viewer_foundation::map::perspective, viewer_foundation::map::ribbon_content::OrderField,
    viewer_foundation::map::tables::Limitations, viewer_foundation::map::textures,
    viewer_foundation::map::textures::IntelMap, viewer_foundation::retirement::builder_count,
    viewer_world::map::aircraft::AircraftPlan, viewer_world::map::border_pass::ClassPlan,
    viewer_world::map::demilitarized::Zone, viewer_world::map::demilitarized::ZonePlan,
    viewer_world::map::map_buildings::PreparedBuildings, viewer_world::map::models::ModelField,
    viewer_world::map::name_pass::NameAreas, viewer_world::map::name_pass::PlacedAreas,
    viewer_world::map::political, viewer_world::map::political::FrameKey,
    viewer_world::map::political::PoliticalLayers, viewer_world::map::railways::RailMesh,
    viewer_world::map::railways::RailNetwork, viewer_world::map::trains::TrainPlan,
    viewer_world::map::unit_models::PreparedModels, viewer_world::map::weather,
    viewer_world::map::weather::SnowMud, viewer_world::map::weather_effects,
};

/// The camera the icon layer and the names are built for.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct View {
    pub camera: Camera,
    pub viewport: [u32; 2],
    /// Device pixels per interface pixel; see [`crate::map::ui_scale`].
    pub ui_scale: f32,
}

/// What the window wants built.
#[derive(Clone, Debug, PartialEq)]
pub struct Want {
    pub branch: BranchId,
    /// The playhead's recorded hour.
    pub hour: Hour,
    /// Hours past the playhead to build before they are reached.
    pub ahead: i32,
    /// Hours before the playhead to keep built, for a step back.
    pub behind: i32,
    /// Playing: an hour in flight at or before the playhead is finished, not
    /// cancelled, because the window may show it while the playhead's own hour
    /// is still being built.
    pub playing: bool,
    pub perspective: perspective::Choice,
    /// The view's map mode, which chooses the political fill's country channel.
    pub map_mode: viewer_world::map::map_mode::MapMode,
    pub show_allied_plans: bool,
    /// The window's counter options and memory, which its hours are laid out with.
    pub counters: CounterView,
    pub view: View,
    /// The hours of this branch the window holds built, the one on screen
    /// included; the builders do not build them again.
    pub have: Vec<Hour>,
    /// How many built hours the window had received when it asked. Hours sent
    /// after that are on their way and not built again either.
    pub received: u64,
}

impl Want {
    /// The hours this want keeps built, in the order they are dispatched.
    fn hours(&self, range: (Hour, Hour)) -> Vec<Hour> {
        let (first, last) = range;
        let mut hours = vec![self.hour];
        let end = self.hour.saturating_add(self.ahead).min(last);
        // Consecutive hours share the preceding hour's parts and let the
        // decoder advance in place. A frame may skip what was built, but the
        // builders do not sample a predicted frame grid.
        if self.hour < end {
            hours.extend(self.hour + 1..=end);
        }
        hours.extend(
            (1..=self.behind)
                .map(|step| self.hour.saturating_sub(step))
                .filter(|hour| *hour >= first),
        );
        hours.retain(|hour| (first..=last).contains(hour));
        hours
    }

    /// Whether a change from `self` to `next` invalidates what was built.
    fn invalidates(&self, next: &Want) -> bool {
        // Selection and the active GUI object are viewer-local overlay inputs.
        // Their prepared snapshot is checked when installed; pointer motion
        // must not cancel simulation-hour builders or restart their epoch.
        self.map_mode != next.map_mode || self.invalidates_parts(next)
    }

    /// Whether mode-independent parts must also be rebuilt.
    fn invalidates_parts(&self, next: &Want) -> bool {
        self.perspective != next.perspective
            || self.show_allied_plans != next.show_allied_plans
            || self.counters.settings != next.counters.settings
            || !Arc::ptr_eq(&self.counters.presentation, &next.counters.presentation)
    }

    fn keeps_reusable(&self, (branch, hour): (BranchId, Hour)) -> bool {
        branch == self.branch
            && (self.hour.saturating_sub(self.behind)..=self.hour.saturating_add(self.ahead))
                .contains(&hour)
    }

    /// Finish hours already in flight at/before the playing clock, even when
    /// it has passed their request window: they may still be the newest built
    /// fallback. Future hours finish only while they remain wanted.
    fn keeps_pending(&self, key: (BranchId, Hour), keep: &BTreeSet<(BranchId, Hour)>) -> bool {
        keep.contains(&key)
            || (self.playing
                && key.0 == self.branch
                && key.1 <= self.hour)
            // A paused catch-up still needs the scripted colour chain from
            // preceding in-flight hours. Explicit seeks invalidate the epoch
            // first; ordinary pauses must not cold-start that history.
            || (matches!(self.map_mode, viewer_world::map::map_mode::MapMode::Scripted(_))
                && key.0 == self.branch
                && key.1 <= self.hour)
    }

    /// Explicit seeks, including same-hour seeks, restart history just like
    /// a changed mode or perspective. No ready or pending result may cross it.
    fn discard_stale_have(&mut self, previous: Option<&Self>, sought: bool) -> bool {
        let invalidates = sought || previous.is_some_and(|old| old.invalidates(self));
        if invalidates {
            self.have.clear();
        }
        invalidates
    }
}

/// Request one pool of consecutive hours, including the playhead. Cold build
/// latency must not turn into a speculative recording-wide decode window.
fn playback_ahead(parallel: usize) -> i32 {
    i32::try_from(parallel.saturating_sub(1)).unwrap_or(i32::MAX)
}

/// The nearest earlier/later built candidates, not necessarily adjacent
/// recorded hours. Only already-dispatched slots are considered, so waits
/// follow dispatch order and cannot form a cycle.
fn neighbour_slots<T>(
    slots: &BTreeMap<(BranchId, Hour), T>,
    branch: BranchId,
    hour: Hour,
) -> Vec<((BranchId, Hour), &T)> {
    slots
        .range((branch, i32::MIN)..(branch, hour))
        .next_back()
        .into_iter()
        .chain(
            slots
                .range((branch, hour)..=(branch, i32::MAX))
                .find(|(key, _)| key.1 > hour),
        )
        .map(|(key, value)| (*key, value))
        .collect()
}

/// Advance only scripted colour history through recorded hours that have no
/// full layer build. An unavailable hour breaks the retained-border history.
fn carry_colour(
    mut colour: Option<Arc<viewer_world::map::map_mode::ColourState>>,
    frames: Vec<Current>,
    mut fill: impl FnMut(
        &replay_frame::frame::DecodedFrame,
        Option<&viewer_world::map::map_mode::ColourState>,
    ) -> io::Result<Option<Arc<viewer_world::map::map_mode::ColourState>>>,
) -> io::Result<Option<Arc<viewer_world::map::map_mode::ColourState>>> {
    for current in frames {
        colour = match current {
            Current::Frame(frame) => fill(&frame, colour.as_deref())?,
            Current::Gap { .. } => None,
        };
    }
    Ok(colour)
}

static NEXT_ID: AtomicU64 = AtomicU64::new(1);

fn next_id() -> u64 {
    NEXT_ID.fetch_add(1, Ordering::Relaxed)
}

/// A layer value consecutive hours share while it does not change. `id` names
/// the value: two hours with the same id hold the same value.
pub struct Shared<T> {
    pub id: u64,
    pub value: T,
}

impl<T: PartialEq> Shared<T> {
    /// A neighbour that holds `value`, otherwise `value` under a new id.
    fn share(neighbours: Vec<Arc<Self>>, value: T) -> Arc<Self> {
        match neighbours
            .into_iter()
            .find(|neighbour| neighbour.value == value)
        {
            Some(same) => same,
            None => Arc::new(Self {
                id: next_id(),
                value,
            }),
        }
    }
}

/// The railway mesh and the bridges placed on it.
pub type Rails = Shared<(RailMesh, ModelField)>;

/// An hour's political fill, shared by the hours whose [`FrameKey`] is equal.
pub struct Political {
    pub id: u64,
    key: FrameKey,
    pub layers: PoliticalLayers,
}

/// An hour's border buckets, shared by the hours with the same political fill
/// and demilitarised zone.
pub struct Borders {
    pub id: u64,
    political: u64,
    pub(crate) zone: Zone,
    pub classes: ClassPlan,
    pub highlighted: ZonePlan,
}

/// The viewing country's own tag at an hour, which presentation continuity
/// compares. It walks the hour's country tables, so it is read here and not
/// on a frame.
#[derive(Debug, Default)]
pub struct Header {
    pub viewing: Option<String>,
}

/// Everything the map draws for one recorded hour, built off the frame path.
pub struct Layers {
    pub political: Arc<Political>,
    pub borders: Arc<Borders>,
    pub rails: Arc<Rails>,
    pub buildings: Arc<Shared<PreparedBuildings>>,
    /// The building field's refused type tokens
    /// ([`viewer_world::map::map_buildings::MapBuildingSummary::warnings`]), worded
    /// with the hour: an hour sharing its neighbour's field still reports its own.
    pub building_warnings: Vec<String>,
    pub trains: TrainPlan,
    pub aircraft: AircraftPlan,
    pub units: Arc<PreparedModels>,
    /// Shared by the hours whose plan tables are the same
    /// ([`viewer_counter_products::map::orders::TABLES`]), so the window uploads it once.
    pub orders: Arc<OrderField>,
    pub effects: weather_effects::Prepared,
    /// The songs' chances for the viewing country at this hour.
    pub music: Option<Arc<viewer_audio::Chances>>,
    /// The hour's battles, for the battle ambience.
    pub battles: Option<Arc<viewer_audio::Battles>>,
    /// This hour's nuclear impacts, which start their province effects when presented.
    pub impacts: Arc<viewer_world::map::strike_effects::Impacts>,
    /// This hour's map events for the viewing country, presented once each.
    pub map_text: Arc<viewer_world::map::animated_map_text::events::Prepared>,
    /// Shared by the hours whose fog is the same, so the window writes it once.
    pub intel: Arc<IntelMap>,
    /// Shared so the window can hold the grid it uploaded and compare.
    pub snow_mud: Arc<SnowMud>,
    /// Worded at fraction 0; the window sets its own fraction before reading.
    pub limitations: Limitations,
    pub header: Arc<Header>,
    /// Shared by the hours whose named areas are the same.
    pub names: Arc<Placements>,
    pub overlay: PreparedOverlay,
    /// The camera-independent tables `overlay` was laid out from, raid icons
    /// included, so the window lays out other cameras without rebuilding them.
    pub overlay_tables: OverlayTables,
}

/// One built hour.
pub struct Built {
    /// Which [`Want::invalidates`] generation it belongs to.
    pub epoch: u64,
    pub branch: BranchId,
    pub hour: Hour,
    /// The view its names and icons were built for.
    pub view: View,
    pub current: Current,
    /// The hour's layers, or why it has none: a recorded gap or a failed part.
    pub layers: Result<Layers, String>,
    /// CPU milliseconds of each part, and wall milliseconds from dispatch.
    pub laps: Vec<(&'static str, f64)>,
    pub wall: f64,
}

enum Part {
    Political(io::Result<(Arc<Political>, Arc<Borders>)>),
    Names(io::Result<Arc<Placements>>),
    Overlay(io::Result<Box<(PreparedOverlay, OverlayTables)>>),
    Rails(io::Result<Arc<Rails>>),
    Buildings(io::Result<(Arc<Shared<PreparedBuildings>>, Vec<String>)>),
    Movers(io::Result<Arc<Movers>>),
    Fog(io::Result<Arc<FogPart>>),
    Snow(io::Result<Arc<SnowMud>>),
}

#[derive(Clone)]
struct FogPart {
    intel: Arc<IntelMap>,
    limitations: Limitations,
    header: Arc<Header>,
    map_text: Arc<viewer_world::map::animated_map_text::events::Prepared>,
    /// The viewing country's song chances.
    music: Option<Arc<viewer_audio::Chances>>,
    battles: Option<Arc<viewer_audio::Battles>>,
}

#[derive(Clone)]
struct Movers {
    trains: TrainPlan,
    aircraft: AircraftPlan,
    units: Arc<PreparedModels>,
    orders: Arc<OrderField>,
    effects: weather_effects::Prepared,
    impacts: Arc<viewer_world::map::strike_effects::Impacts>,
}

/// Mode-independent results of a built hour, retained only inside the wanted window.
struct Reusable {
    rails: Arc<Rails>,
    buildings: (Arc<Shared<PreparedBuildings>>, Vec<String>),
    movers: Arc<Movers>,
    fog: Arc<FogPart>,
    snow: Arc<SnowMud>,
}

impl Part {
    /// The error an hour carries when its builder panicked.
    ///
    /// The failed part still arrives, so [`Pending::take`] can finish the hour
    /// and choose the highest-priority error after all parts report. Its shared
    /// slots are already filled by the dropped [`Fill`].
    fn failed(name: &'static str) -> Self {
        let error = || io::Error::other(format!("{name}: the hour builder panicked"));
        match name {
            "political" => Self::Political(Err(error())),
            "names" => Self::Names(Err(error())),
            "overlay" => Self::Overlay(Err(error())),
            "rails" => Self::Rails(Err(error())),
            "buildings" => Self::Buildings(Err(error())),
            "movers" => Self::Movers(Err(error())),
            "fog" => Self::Fog(Err(error())),
            _ => Self::Snow(Err(error())),
        }
    }
}

const PARTS: usize = 8;

/// One part's value for one hour, which the neighbouring hours' parts compare with.
struct Slot<T> {
    value: Mutex<Option<Option<Arc<T>>>>,
    done: Condvar,
}

impl<T> Slot<T> {
    fn new() -> Arc<Self> {
        Arc::new(Self {
            value: Mutex::new(None),
            done: Condvar::new(),
        })
    }

    /// The value, waiting for the part building it; `None` when it failed or
    /// was cancelled.
    fn wait(&self) -> Option<Arc<T>> {
        let Ok(mut value) = self.value.lock() else {
            return None;
        };
        loop {
            if let Some(value) = value.as_ref() {
                return value.clone();
            }
            value = match self.done.wait(value) {
                Ok(value) => value,
                Err(_) => return None,
            };
        }
    }
}

/// Fills a slot however its part ends, so no later part waits for ever.
struct Fill<T>(Arc<Slot<T>>);

impl<T> Fill<T> {
    fn set(&self, value: Option<Arc<T>>) {
        if let Ok(mut slot) = self.0.value.lock()
            && slot.is_none()
        {
            *slot = Some(value);
            self.0.done.notify_all();
        }
    }
}

impl<T> Drop for Fill<T> {
    fn drop(&mut self) {
        self.set(None);
    }
}

/// One part's slots of several hours.
fn slots_of<T>(hours: &[Slots], part: impl Fn(&Slots) -> &Arc<Slot<T>>) -> Vec<Arc<Slot<T>>> {
    hours.iter().map(|slots| Arc::clone(part(slots))).collect()
}

/// The values of every slot that has one, waiting for each.
fn wait_all<T>(slots: Vec<Arc<Slot<T>>>) -> Vec<Arc<T>> {
    slots.iter().filter_map(|slot| slot.wait()).collect()
}

/// The slots of one hour's shared parts.
#[derive(Clone)]
struct Slots {
    political: Arc<Slot<Political>>,
    borders: Arc<Slot<Borders>>,
    rails: Arc<Slot<Rails>>,
    buildings: Arc<Slot<Shared<PreparedBuildings>>>,
}

impl Slots {
    fn new() -> Self {
        Self {
            political: Slot::new(),
            borders: Slot::new(),
            rails: Slot::new(),
            buildings: Slot::new(),
        }
    }
}

/// Hour layers the next hours usually rebuild unchanged, kept by what they
/// were built from (`viewer_foundation::map::memo`). A hit hands back the same allocation, so the
/// window, which remembers what it uploaded, uploads nothing for it.
struct Memos {
    /// Country names, by the areas and names they place.
    names: Recent<NameAreas, Placements>,
    /// Each country area's placed name, so a moved front re-places only the
    /// areas it changed.
    placed: PlacedAreas,
    /// The railway, by the recorded network it is drawn from.
    rails: Recent<RailNetwork, Rails>,
    /// The snow and mud grid, by the `WeatherProvinces` table it rasterises.
    snow: Keyed<SnowMud>,
    /// The intel map, by the viewing country's rebuilt province bytes.
    intel: Recent<Option<BTreeMap<u32, u8>>, IntelMap>,
}

/// How many recent values each memo keeps: the pool builds several hours at
/// once, and a step back or a perspective switch returns to a recent one.
const MEMO_ENTRIES: usize = 8;

impl Memos {
    fn new() -> Self {
        Self {
            names: Recent::new(MEMO_ENTRIES),
            placed: PlacedAreas::default(),
            rails: Recent::new(MEMO_ENTRIES),
            snow: Keyed::new(weather::TABLES, MEMO_ENTRIES),
            intel: Recent::new(MEMO_ENTRIES),
        }
    }
}

enum Event {
    Shutdown,
    Want,
    Part {
        ticket: u64,
        epoch: u64,
        branch: BranchId,
        hour: Hour,
        name: &'static str,
        ms: f64,
        part: Box<Part>,
    },
}

/// One worker budget for every map in a native root. Per-view coordinators
/// retain independent cancellation and layer state, but do not multiply pools.
pub struct Resources {
    pool: Arc<Pool>,
    decoders: Mutex<BTreeMap<usize, Weak<SharedDecoder>>>,
    pub dropper: Arc<Dropper>,
}

impl Resources {
    pub fn new() -> io::Result<Arc<Self>> {
        Ok(Arc::new(Self {
            pool: Arc::new(Pool::new(builder_count())?),
            decoders: Mutex::new(BTreeMap::new()),
            dropper: Arc::new(Dropper::new()?),
        }))
    }

    /// A bounded nonblocking admission path for scene creation. The caller can
    /// retry next frame when hour work already occupies the shared queue.
    pub fn try_submit(&self, work: impl FnOnce() + Send + 'static) -> bool {
        self.pool.try_submit(Box::new(work)).is_ok()
    }

    pub fn decoder(&self, replay: Arc<Replay>) -> io::Result<Arc<SharedDecoder>> {
        let key = Arc::as_ptr(&replay) as usize;
        let mut decoders = self.decoders.lock().map_err(|_| stopped())?;
        decoders.retain(|_, decoder| decoder.strong_count() > 0);
        if let Some(decoder) = decoders.get(&key).and_then(Weak::upgrade) {
            return Ok(decoder);
        }
        // The decoder owns Replay, so its address cannot be reused while this
        // weak entry can upgrade. No bytes or decoder are copied per map.
        let decoder = Arc::new(SharedDecoder(Mutex::new(DecoderState {
            playback: Playback::unloaded(replay)?,
            frames: BTreeMap::new(),
        })));
        decoders.insert(key, Arc::downgrade(&decoder));
        Ok(decoder)
    }
}

pub struct SharedDecoder(Mutex<DecoderState>);

struct DecoderState {
    playback: Playback,
    // Weak entries share frames already held by visible/in-flight maps without
    // retaining a second historical payload cache. One block/current per source
    // is held by Playback; the registry itself owns only bounded metadata.
    frames: BTreeMap<(BranchId, Hour), Weak<replay_frame::frame::DecodedFrame>>,
}

impl SharedDecoder {
    pub fn seed(&self, mut playback: Playback) -> io::Result<()> {
        // Asset/source loaders call this off the UI. Install the host's
        // committed checkpoint reader before this source serves hour workers.
        if let Some(source) = playback.checkpoint_source() {
            match source.open() {
                Ok(Some(checkpoints)) => playback = playback.with_checkpoints(checkpoints),
                Ok(None) => {}
                Err(error) => eprintln!(
                    "Map checkpoint binding unavailable: {error}; using the replay reader"
                ),
            }
        }
        let mut decoder = self.0.lock().map_err(|_| stopped())?;
        if let Current::Frame(frame) = playback.current() {
            decoder
                .frames
                .insert((frame.branch, frame.hour), Arc::downgrade(frame));
        }
        decoder.playback = playback;
        Ok(())
    }

    fn frame(&self, branch: BranchId, hour: Hour) -> io::Result<Current> {
        let mut decoder = self.0.lock().map_err(|_| stopped())?;
        if let Some(frame) = decoder.frames.get(&(branch, hour)).and_then(Weak::upgrade) {
            return Ok(Current::Frame(frame));
        }
        decoder.playback.seek(branch, hour)?;
        let current = decoder.playback.current().clone();
        decoder.frames.retain(|_, frame| frame.strong_count() > 0);
        if decoder.frames.len() >= FRAME_CACHE {
            decoder.frames.pop_first();
        }
        if let Current::Frame(frame) = &current {
            decoder.frames.insert((branch, hour), Arc::downgrade(frame));
        }
        Ok(current)
    }
}

/// An hour dispatched and not yet assembled.
struct Pending {
    ticket: u64,
    current: Current,
    alive: Arc<AtomicBool>,
    started: Instant,
    political: Option<(Arc<Political>, Arc<Borders>)>,
    names: Option<Arc<Placements>>,
    overlay: Option<(PreparedOverlay, OverlayTables)>,
    rails: Option<Arc<Rails>>,
    buildings: Option<(Arc<Shared<PreparedBuildings>>, Vec<String>)>,
    movers: Option<Arc<Movers>>,
    fog: Option<Arc<FogPart>>,
    snow: Option<Arc<SnowMud>>,
    received: usize,
    error: Option<String>,
    /// Rank of the part `error` names, so concurrent failures report the
    /// same part in the same fixed order whatever arrives first.
    error_rank: Option<u8>,
    laps: Vec<(&'static str, f64)>,
    view: View,
}

/// Fixed part order, shared by error priority and lap reporting, so
/// whatever the pool finishes first the hour reads the same.
fn part_rank(part: &Part) -> u8 {
    match part {
        Part::Political(_) => 0,
        Part::Names(_) => 1,
        Part::Overlay(_) => 2,
        Part::Rails(_) => 3,
        Part::Buildings(_) => 4,
        Part::Movers(_) => 5,
        Part::Fog(_) => 6,
        Part::Snow(_) => 7,
    }
}

/// Lap order: the serial decode first, then parts in fixed order.
fn lap_rank(name: &str) -> u8 {
    match name {
        "decode" => 0,
        "political" => 1,
        "names" => 2,
        "overlay" => 3,
        "rails" => 4,
        "buildings" => 5,
        "movers" => 6,
        "fog" => 7,
        "snow" => 8,
        _ => 9,
    }
}

impl Pending {
    /// Records a part failure only when no higher-priority part has failed,
    /// so concurrent failures name the same part every run.
    fn fail_part(&mut self, name: &'static str, rank: u8, error: io::Error) {
        if self.error_rank.is_none_or(|best| rank < best) {
            self.error_rank = Some(rank);
            self.error = Some(format!("{name}: {error}"));
        }
    }

    fn take(&mut self, ticket: u64, name: &'static str, ms: f64, part: Part) -> bool {
        // A seek may cancel and then request the same hour within one epoch.
        // Results from the cancelled dispatch must not fill its replacement.
        if ticket != self.ticket {
            return false;
        }
        self.received += 1;
        self.laps.push((name, ms));
        let rank = part_rank(&part);
        match part {
            Part::Political(result) => match result {
                Ok(value) => self.political = Some(value),
                Err(error) => self.fail_part(name, rank, error),
            },
            Part::Names(result) => match result {
                Ok(value) => self.names = Some(value),
                Err(error) => self.fail_part(name, rank, error),
            },
            Part::Overlay(result) => match result {
                Ok(value) => self.overlay = Some(*value),
                Err(error) => self.fail_part(name, rank, error),
            },
            Part::Rails(result) => match result {
                Ok(value) => self.rails = Some(value),
                Err(error) => self.fail_part(name, rank, error),
            },
            Part::Buildings(result) => match result {
                Ok(value) => self.buildings = Some(value),
                Err(error) => self.fail_part(name, rank, error),
            },
            Part::Movers(result) => match result {
                Ok(value) => self.movers = Some(value),
                Err(error) => self.fail_part(name, rank, error),
            },
            Part::Fog(result) => match result {
                Ok(value) => self.fog = Some(value),
                Err(error) => self.fail_part(name, rank, error),
            },
            Part::Snow(result) => match result {
                Ok(value) => self.snow = Some(value),
                Err(error) => self.fail_part(name, rank, error),
            },
        }
        // Wait for every part so error priority is independent of arrival order.
        self.received == PARTS
    }

    fn reusable(&self) -> Option<Arc<Reusable>> {
        if self.error.is_some() {
            return None;
        }
        Some(Arc::new(Reusable {
            rails: Arc::clone(self.rails.as_ref()?),
            buildings: self.buildings.as_ref()?.clone(),
            movers: Arc::clone(self.movers.as_ref()?),
            fog: Arc::clone(self.fog.as_ref()?),
            snow: Arc::clone(self.snow.as_ref()?),
        }))
    }

    fn reuse(&mut self, parts: &Reusable) {
        for (name, part) in [
            ("rails", Part::Rails(Ok(Arc::clone(&parts.rails)))),
            ("buildings", Part::Buildings(Ok(parts.buildings.clone()))),
            ("movers", Part::Movers(Ok(Arc::clone(&parts.movers)))),
            ("fog", Part::Fog(Ok(Arc::clone(&parts.fog)))),
            ("snow", Part::Snow(Ok(Arc::clone(&parts.snow)))),
        ] {
            self.take(self.ticket, name, 0.0, part);
        }
    }

    fn layers(self) -> Result<Layers, String> {
        if let Some(error) = self.error {
            return Err(error);
        }
        let missing = || "an hour part did not arrive".to_owned();
        let (political, borders) = self.political.ok_or_else(missing)?;
        let movers = (*self.movers.ok_or_else(missing)?).clone();
        let FogPart {
            intel,
            limitations,
            header,
            map_text,
            music,
            battles,
        } = (*self.fog.ok_or_else(missing)?).clone();
        let (overlay, overlay_tables) = self.overlay.ok_or_else(missing)?;
        let (buildings, building_warnings) = self.buildings.ok_or_else(missing)?;
        Ok(Layers {
            political,
            borders,
            rails: self.rails.ok_or_else(missing)?,
            buildings,
            building_warnings,
            trains: movers.trains,
            aircraft: movers.aircraft,
            units: movers.units,
            orders: movers.orders,
            effects: movers.effects,
            impacts: movers.impacts,
            map_text,
            music,
            battles,
            intel,
            snow_mud: self.snow.ok_or_else(missing)?,
            limitations,
            header,
            names: self.names.ok_or_else(missing)?,
            overlay,
            overlay_tables,
        })
    }
}

/// Recently decoded frames the coordinator keeps, so a step back or a
/// rebuild for a new perspective decodes nothing. Also the bound of the weak
/// frame and snapshot registries, which hold no frame data themselves.
const FRAME_CACHE: usize = 96;

/// Decoded bytes the coordinator's own frame cache may hold.
///
/// A decoded hour grows with the recording's countries and units, so a fixed
/// count of hours is not a fixed amount of memory. Tables are counted at their
/// own row sizes ([`replay_data::rows::TableRows::allocated_bytes`]), without the
/// strings and lists a row owns on the heap. The cache keeps at most
/// [`FRAME_CACHE`] hours and, past this budget, only the hours the current
/// want still reaches (`FrameCache::evict`).
const FRAME_CACHE_BYTES: u64 = 2 << 30;

/// A decoded table's row allocation in bytes.
fn table_bytes(table: &replay_frame::frame::TableView) -> u64 {
    match table {
        replay_frame::frame::TableView::Complete(rows)
        | replay_frame::frame::TableView::Truncated { rows, .. } => rows.allocated_bytes() as u64,
        replay_frame::frame::TableView::Unavailable { .. } => 0,
    }
}

/// The coordinator's recently decoded hours, least recently used first,
/// bounded by count and by row bytes.
///
/// Hours share every table that did not change between them
/// ([`replay_frame::frame::SharedView`]), so the byte count is of distinct tables:
/// each allocation is counted once however many cached hours hold it.
#[derive(Default)]
struct FrameCache {
    frames: BTreeMap<(BranchId, Hour), Current>,
    /// Every cached hour, least recently used first.
    order: VecDeque<(BranchId, Hour)>,
    /// Distinct cached tables by allocation: holders and row bytes.
    tables: HashMap<usize, (usize, u64)>,
    bytes: u64,
}

impl FrameCache {
    /// A cached hour; the hit makes it the most recently used.
    fn get(&mut self, key: &(BranchId, Hour)) -> Option<Current> {
        let current = self.frames.get(key)?.clone();
        if let Some(at) = self.order.iter().position(|cached| cached == key)
            && let Some(entry) = self.order.remove(at)
        {
            self.order.push_back(entry);
        }
        Some(current)
    }

    fn contains_key(&self, key: &(BranchId, Hour)) -> bool {
        self.frames.contains_key(key)
    }

    fn insert(&mut self, key: (BranchId, Hour), current: Current) {
        self.hold(&current);
        if let Some(old) = self.frames.insert(key, current) {
            self.release(&old);
            self.order.retain(|at| *at != key);
        }
        self.order.push_back(key);
    }

    fn hold(&mut self, current: &Current) {
        let Current::Frame(frame) = current else {
            return;
        };
        for table in &frame.tables {
            let entry = self
                .tables
                .entry(table.as_ptr() as usize)
                .or_insert_with(|| (0, table_bytes(table)));
            if entry.0 == 0 {
                self.bytes = self.bytes.saturating_add(entry.1);
            }
            entry.0 += 1;
        }
    }

    fn release(&mut self, current: &Current) {
        let Current::Frame(frame) = current else {
            return;
        };
        for table in &frame.tables {
            let key = table.as_ptr() as usize;
            if let Some(entry) = self.tables.get_mut(&key) {
                entry.0 -= 1;
                if entry.0 == 0 {
                    self.bytes = self.bytes.saturating_sub(entry.1);
                    self.tables.remove(&key);
                }
            }
        }
    }

    /// Drops the least recently used hours past [`FRAME_CACHE`] and past
    /// `budget` bytes, never an hour in `spare`: the hours the builders will
    /// still ask for.
    fn evict_except(&mut self, budget: u64, spare: &BTreeSet<(BranchId, Hour)>) {
        let mut at = 0;
        while at < self.order.len() && (self.order.len() > FRAME_CACHE || self.bytes > budget) {
            if spare.contains(&self.order[at]) {
                at += 1;
                continue;
            }
            let Some(old) = self.order.remove(at) else {
                break;
            };
            if let Some(current) = self.frames.remove(&old) {
                self.release(&current);
            }
        }
    }
}

/// The playback decode lane: a thread of its own that owns a [`Playback`] and
/// decodes the wanted hours, in the order the builders will take them, ahead
/// of the coordinator.
///
/// Decoding is serial (each hour applies its deltas to the one before), so it
/// was the coordinator's own work between dispatches: 1–7 ms an hour, a
/// keyframe 31–34 ms and a block boundary 0.5–1 s, during which no hour was
/// dispatched. Here the coordinator only takes decoded hours from
/// [`DecodeLane::frame`], and the lane's [`Playback`] reads the next block
/// ahead through its [`BlockReader`].
///
/// The lane runs below normal priority, as the builders do, so it never
/// competes with the window's own thread.
struct DecodeLane {
    shared: Arc<LaneShared>,
}

struct LaneShared {
    state: Mutex<LaneState>,
    changed: Condvar,
}

/// Wanted hours the lane decodes past the last one the coordinator took.
///
/// The builders take hours in want order, a pool's worth at a time, so a
/// window of this many keeps every dispatch waiting on nothing but the build.
/// A larger one only holds decoded hours in memory longer; a smaller one than
/// the pool would make dispatches wait for the decode.
const DECODE_AHEAD: usize = 32;

/// Hours a long walk through a block applies between checks that its target
/// is still wanted, so a seek away abandons it within a few milliseconds.
const DECODE_STEP: i32 = 24;

/// Why a lane that stopped answers with a gap rather than a wait.
const LANE_STOPPED: &str = "the hour decoder stopped";

#[derive(Default)]
struct LaneState {
    /// The hours to keep decoded, in the order the builders will ask for them.
    wanted: Vec<(BranchId, Hour)>,
    /// Earlier hours of a block to keep while passing them, for a step back.
    behind: i32,
    /// The last hour the coordinator took, which the window runs from.
    served: Option<(BranchId, Hour)>,
    /// The hour the coordinator is waiting for, decoded before anything else.
    urgent: Option<(BranchId, Hour)>,
    frames: FrameCache,
    /// Set by the lane's owner to stop the thread.
    stop: bool,
    /// Set when the thread has ended, however it ended.
    dead: bool,
}

impl LaneState {
    /// The wanted hours the lane keeps decoded now: from the want's start up to
    /// [`DECODE_AHEAD`] past the last one served.
    fn window(&self) -> &[(BranchId, Hour)] {
        let served = self
            .served
            .and_then(|served| self.wanted.iter().position(|key| *key == served))
            .map_or(0, |at| at + 1);
        &self.wanted[..(served + DECODE_AHEAD).min(self.wanted.len())]
    }

    /// The next hour to decode: an urgent one, else the first hour of the
    /// window not decoded yet.
    fn next(&self) -> Option<(BranchId, Hour)> {
        self.urgent
            .filter(|key| !self.frames.contains_key(key))
            .or_else(|| {
                self.window()
                    .iter()
                    .copied()
                    .find(|key| !self.frames.contains_key(key))
            })
    }

    /// Whether decoding `target` is still worth finishing.
    fn still_wanted(&self, target: (BranchId, Hour)) -> bool {
        !self.stop && (self.urgent == Some(target) || self.window().contains(&target))
    }

    /// Frees what the builders will not ask for again, keeping the window.
    fn evict(&mut self) {
        let spare: BTreeSet<_> = self.window().iter().copied().chain(self.urgent).collect();
        self.frames.evict_except(FRAME_CACHE_BYTES, &spare);
    }
}

/// Marks the lane dead when its thread ends, a panic included, so a waiting
/// coordinator gets a gap instead of waiting for ever.
struct LaneExit<'a>(&'a LaneShared);

impl Drop for LaneExit<'_> {
    fn drop(&mut self) {
        DecodeLane::lock(self.0).dead = true;
        self.0.changed.notify_all();
    }
}

impl DecodeLane {
    fn start(playback: Playback) -> io::Result<Self> {
        let shared = Arc::new(LaneShared {
            state: Mutex::new(LaneState::default()),
            changed: Condvar::new(),
        });
        let lane = Arc::clone(&shared);
        thread::Builder::new()
            .name("hour decoder".into())
            .spawn(move || {
                let _exit = LaneExit(&lane);
                background_priority();
                Self::run(&lane, playback);
            })?;
        Ok(Self { shared })
    }

    fn lock(shared: &LaneShared) -> MutexGuard<'_, LaneState> {
        shared
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn run(shared: &LaneShared, mut playback: Playback) {
        'next: loop {
            let (target, behind) = {
                let mut state = Self::lock(shared);
                loop {
                    if state.stop {
                        return;
                    }
                    if let Some(target) = state.next() {
                        break (target, state.behind);
                    }
                    state = shared
                        .changed
                        .wait(state)
                        .unwrap_or_else(|poisoned| poisoned.into_inner());
                }
            };
            let (branch, hour) = target;
            if !Self::walk(shared, &mut playback, target) {
                continue 'next;
            }
            let mut passed = Vec::new();
            let current = match playback.seek_with(branch, hour, |decoded| {
                if hour - decoded.hour <= behind {
                    passed.push((decoded.hour, Current::Frame(Arc::new(decoded.clone()))));
                }
            }) {
                Ok(()) => playback.current().clone(),
                Err(error) => Current::Gap {
                    branch,
                    hour,
                    reason: error.to_string(),
                },
            };
            let mut state = Self::lock(shared);
            for (at, frame) in passed {
                if !state.frames.contains_key(&(branch, at)) {
                    state.frames.insert((branch, at), frame);
                }
            }
            state.frames.insert(target, current);
            state.evict();
            drop(state);
            shared.changed.notify_all();
        }
    }

    /// Walks `playback` towards `target` in [`DECODE_STEP`]-hour steps, each
    /// ending with a check that the target is still wanted. False when the
    /// walk was abandoned; the last step is left to the caller.
    fn walk(shared: &LaneShared, playback: &mut Playback, target: (BranchId, Hour)) -> bool {
        let (branch, hour) = target;
        let mut from = playback.walk_start(branch, hour);
        while let Some(at) = from.filter(|at| hour - at > DECODE_STEP) {
            let step = at + DECODE_STEP;
            if playback.advance_without_output(branch, step).is_err() {
                break;
            }
            from = Some(step);
            if !Self::lock(shared).still_wanted(target) {
                return false;
            }
        }
        true
    }

    /// Replaces the hours the lane keeps decoded.
    fn want(&self, wanted: Vec<(BranchId, Hour)>, behind: i32) {
        let mut state = Self::lock(&self.shared);
        state.wanted = wanted;
        state.behind = behind;
        drop(state);
        self.shared.changed.notify_all();
    }

    /// The decoded hour, waiting for the lane to reach it. The hour asked for
    /// is decoded first, wanted or not, so the coordinator never waits on an
    /// hour outside the lane's window; a stopped lane answers with a gap.
    fn frame(&self, branch: BranchId, hour: Hour) -> Current {
        let key = (branch, hour);
        let mut state = Self::lock(&self.shared);
        loop {
            if let Some(current) = state.frames.get(&key) {
                if state.urgent == Some(key) {
                    state.urgent = None;
                }
                state.served = Some(key);
                drop(state);
                // The window moved on: decode further ahead.
                self.shared.changed.notify_all();
                return current;
            }
            if state.dead {
                return Current::Gap {
                    branch,
                    hour,
                    reason: LANE_STOPPED.to_owned(),
                };
            }
            if state.urgent != Some(key) {
                state.urgent = Some(key);
                self.shared.changed.notify_all();
            }
            state = self
                .shared
                .changed
                .wait(state)
                .unwrap_or_else(|poisoned| poisoned.into_inner());
        }
    }
}

impl Drop for DecodeLane {
    fn drop(&mut self) {
        Self::lock(&self.shared).stop = true;
        self.shared.changed.notify_all();
    }
}

type Requested = (u64, Want);

struct Coordinator {
    content: Arc<Content>,
    playback: Playback,
    shared_decoder: Option<Arc<SharedDecoder>>,
    /// The playback decode lane, unless a shared decoder serves this view.
    lane: Option<DecodeLane>,
    requested: Arc<Mutex<Option<Requested>>>,
    /// The window's engaged-flag index, which the overlay part reads when it
    /// is for the wanted perspective's country; it never starts one.
    engaged: Engaged,
    want: Option<Want>,
    epoch: u64,
    pending: BTreeMap<(BranchId, Hour), Pending>,
    /// Hours of the current epoch sent to the window, with the order they were
    /// sent in, until the window says it no longer holds them.
    sent: BTreeMap<(BranchId, Hour), u64>,
    /// How many built hours have been sent, of any epoch.
    sends: u64,
    /// The shared parts of the hours last dispatched, which the neighbouring
    /// hours' parts compare with.
    slots: BTreeMap<(BranchId, Hour), Slots>,
    memos: Arc<Memos>,
    reusable: BTreeMap<(BranchId, Hour), Arc<Reusable>>,
    pool: Arc<Pool>,
    parallel: usize,
    events: Sender<Event>,
    outbox: mpsc::SyncSender<Built>,
}

impl Coordinator {
    fn run(mut self, inbox: Receiver<Event>) {
        let run_started = Instant::now();
        loop {
            let event = if self.next_hour().is_some() && self.pending.len() < self.parallel {
                match inbox.try_recv() {
                    Ok(event) => Some(event),
                    Err(mpsc::TryRecvError::Empty) => None,
                    Err(mpsc::TryRecvError::Disconnected) => return,
                }
            } else {
                match inbox.recv() {
                    Ok(event) => Some(event),
                    Err(_) => return,
                }
            };
            match event {
                Some(Event::Shutdown) => {
                    self.invalidate();
                    return;
                }
                Some(Event::Want) => {
                    let requested = self
                        .requested
                        .lock()
                        .unwrap_or_else(|error| error.into_inner())
                        .take();
                    if let Some((epoch, want)) = requested {
                        self.set_want(epoch, want);
                    }
                }
                Some(Event::Part {
                    ticket,
                    epoch,
                    branch,
                    hour,
                    name,
                    ms,
                    part,
                }) => {
                    if epoch == self.epoch
                        && let Some(pending) = self.pending.get_mut(&(branch, hour))
                        && pending.take(ticket, name, ms, *part)
                    {
                        let pending = self.pending.remove(&(branch, hour)).expect("present");
                        pending.alive.store(false, Ordering::Relaxed);
                        // Laps arrive in pool order; report them in fixed
                        // part order so the profile reads the same.
                        let mut laps = pending.laps.clone();
                        laps.sort_by_key(|(name, _)| lap_rank(name));
                        if let Some(parts) = pending.reusable()
                            && self
                                .want
                                .as_ref()
                                .is_some_and(|want| want.keeps_reusable((branch, hour)))
                        {
                            self.reusable.insert((branch, hour), parts);
                        }
                        let view = pending.view;
                        let current = pending.current.clone();
                        let started = pending.started;
                        let layers = pending.layers();
                        let built = Built {
                            epoch,
                            branch,
                            hour,
                            view,
                            current,
                            wall: started.elapsed().as_secs_f64() * 1000.0,
                            laps,
                            layers,
                        };
                        if viewer_foundation::profile_writer::profiling() {
                            viewer_foundation::profile_writer::profile_print(format!(
                                "PROFILE hour_build hour {hour} wall {:.3} cpu {:.3} elapsed {:.6}",
                                built.wall,
                                built.laps.iter().map(|(_, ms)| ms).sum::<f64>(),
                                run_started.elapsed().as_secs_f64(),
                            ));
                        }
                        if !self.send(built) {
                            return;
                        }
                    }
                }
                None => {}
            }
            while self.pending.len() < self.parallel
                && let Some(hour) = self.next_hour()
            {
                if !self.dispatch(hour) {
                    return;
                }
            }
        }
    }

    /// Starts a new epoch; mode-independent results may still serve its hours.
    fn invalidate(&mut self) {
        self.epoch += 1;
        for pending in self.pending.values() {
            pending.alive.store(false, Ordering::Relaxed);
        }
        self.pending.clear();
        self.sent.clear();
        self.slots.clear();
    }

    fn set_want(&mut self, epoch: u64, mut want: Want) {
        let sought = self.epoch != epoch;
        if self
            .want
            .as_ref()
            .is_some_and(|previous| previous.invalidates_parts(&want))
        {
            self.reusable.clear();
        }
        want.discard_stale_have(self.want.as_ref(), self.epoch != epoch);
        if self.epoch != epoch {
            self.invalidate();
            self.epoch = epoch;
        }
        let range = self
            .playback
            .range(want.branch)
            .unwrap_or((want.hour, want.hour));
        let keep: BTreeSet<(BranchId, Hour)> = want
            .hours(range)
            .into_iter()
            .map(|hour| (want.branch, hour))
            .collect();
        self.reusable.retain(|key, _| want.keeps_reusable(*key));
        self.pending.retain(|key, pending| {
            let wanted = want.keeps_pending(*key, &keep);
            if !wanted {
                pending.alive.store(false, Ordering::Relaxed);
            }
            wanted
        });
        // Keep an hour the window holds, or one still on its way to it.
        self.sent.retain(|key, order| {
            want.keeps_reusable(*key)
                && (*order >= want.received || (key.0 == want.branch && want.have.contains(&key.1)))
        });
        let predecessor = self
            .slots
            .range((want.branch, i32::MIN)..(want.branch, want.hour))
            .next_back()
            .map(|(key, _)| *key);
        self.slots.retain(|key, _| {
            want.keeps_reusable(*key) || self.pending.contains_key(key) || Some(*key) == predecessor
        });
        if let Some(lane) = &self.lane {
            lane.want(
                want.hours(range)
                    .into_iter()
                    .map(|hour| (want.branch, hour))
                    .collect(),
                if want.playing && !sought {
                    0
                } else {
                    want.behind.max(2)
                },
            );
        }
        self.want = Some(want);
    }

    /// Sends a built hour to the window. Returns false when the window has gone.
    fn send(&mut self, built: Built) -> bool {
        self.sent.insert((built.branch, built.hour), self.sends);
        self.sends += 1;
        self.outbox.send(built).is_ok()
    }

    /// The first wanted hour neither sent nor pending.
    fn next_hour(&self) -> Option<Hour> {
        let want = self.want.as_ref()?;
        let range = self.playback.range(want.branch)?;
        want.hours(range).into_iter().find(|hour| {
            let key = (want.branch, *hour);
            !self.sent.contains_key(&key)
                && !self.pending.contains_key(&key)
                && (want.branch != key.0 || !want.have.contains(hour))
        })
    }

    /// The decoded hour: from the shared decoder of a dashboard view, or from
    /// the decode lane, which keeps the hours the want reaches decoded ahead.
    fn frame(&mut self, branch: BranchId, hour: Hour) -> io::Result<Current> {
        if let Some(decoder) = &self.shared_decoder {
            return decoder.frame(branch, hour);
        }
        let lane = self.lane.as_ref().ok_or_else(stopped)?;
        Ok(lane.frame(branch, hour))
    }

    /// Decodes an hour and hands its parts to the pool. Returns false when the
    /// window has gone.
    fn dispatch(&mut self, hour: Hour) -> bool {
        let want = self.want.clone().expect("next_hour found a want");
        let branch = want.branch;
        let epoch = self.epoch;
        let started = Instant::now();
        let nearby = neighbour_slots(&self.slots, branch, hour);
        let previous = nearby
            .iter()
            .find(|(key, _)| key.1 < hour)
            .map(|(key, slots)| (key.1, Arc::clone(&slots.political)));
        let neighbours: Vec<Slots> = nearby.into_iter().map(|(_, slots)| slots.clone()).collect();
        // The playhead may jump past unbuilt hours between wants. Scripted
        // colours still need their intervening recorded refreshes; other
        // layers are built only for the current request window.
        let mut mode_frames = Vec::new();
        let mode_from = previous.as_ref().map(|(from, _)| from.saturating_add(1));
        if matches!(
            &want.map_mode,
            viewer_world::map::map_mode::MapMode::Scripted(_)
        ) && let Some(from) = mode_from
        {
            for at in from..hour {
                mode_frames.push(self.frame(branch, at).unwrap_or_else(|error| Current::Gap {
                    branch,
                    hour: at,
                    reason: error.to_string(),
                }));
            }
        }
        let current = match self.frame(branch, hour) {
            Ok(current) => current,
            Err(error) => Current::Gap {
                branch,
                hour,
                reason: error.to_string(),
            },
        };
        let frame = match &current {
            Current::Frame(frame) => Arc::clone(frame),
            Current::Gap { reason, .. } => {
                let reason = reason.clone();
                return self.send(Built {
                    epoch,
                    branch,
                    hour,
                    view: want.view,
                    current,
                    layers: Err(reason),
                    laps: Vec::new(),
                    wall: 0.0,
                });
            }
        };
        let decode_ms = started.elapsed().as_secs_f64() * 1000.0;
        let alive = Arc::new(AtomicBool::new(true));
        let ticket = next_id();
        let reusable = self.reusable.get(&(branch, hour)).cloned();
        let mut pending = Pending {
            ticket,
            current: current.clone(),
            alive: Arc::clone(&alive),
            started,
            political: None,
            names: None,
            overlay: None,
            rails: None,
            buildings: None,
            movers: None,
            fog: None,
            snow: None,
            received: 0,
            error: None,
            error_rank: None,
            laps: vec![("decode", decode_ms)],
            view: want.view,
        };
        if let Some(parts) = &reusable {
            pending.reuse(parts);
        }
        self.pending.insert((branch, hour), pending);
        let slots = Slots::new();
        if let Some(parts) = &reusable {
            Fill(Arc::clone(&slots.rails)).set(Some(Arc::clone(&parts.rails)));
            Fill(Arc::clone(&slots.buildings)).set(Some(Arc::clone(&parts.buildings.0)));
        }
        // A part waits only for the neighbours' slots present now. Each of
        // those belongs to an hour dispatched before this one, whose jobs the
        // pool therefore took first; a cancelled or panicked part still fills
        // its slots, so no wait is for ever.
        self.slots.insert((branch, hour), slots.clone());
        let submit = |name: &'static str, work: Box<dyn FnOnce(&HourView<'_>) -> Part + Send>| {
            let content = Arc::clone(&self.content);
            let frame = Arc::clone(&frame);
            let alive = Arc::clone(&alive);
            let events = self.events.clone();
            self.pool.submit(Box::new(move || {
                if !alive.load(Ordering::Relaxed) {
                    // Dropping `work` fills its slots with nothing.
                    return;
                }
                let started = Instant::now();
                let view = HourView::new(&content, &frame, 0.0);
                // A panicking builder must not take its pool thread with it:
                // enough dead threads would stall every later hour with the
                // window waiting at the hour boundary. The panic becomes the
                // hour's error instead, which the window shows.
                let part =
                    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| work(&view))) {
                        Ok(part) => part,
                        Err(_) => Part::failed(name),
                    };
                let _ = events.send(Event::Part {
                    ticket,
                    epoch,
                    branch,
                    hour,
                    name,
                    ms: started.elapsed().as_secs_f64() * 1000.0,
                    part: Box::new(part),
                });
            }));
        };
        {
            let fill = Fill(Arc::clone(&slots.political));
            let fill_borders = Fill(Arc::clone(&slots.borders));
            let near_regions = slots_of(&neighbours, |slots| &slots.political);
            let political_perspective = want.perspective.clone();
            let map_mode = want.map_mode.clone();
            let near_borders = slots_of(&neighbours, |slots| &slots.borders);
            // Carry from the preceding built hour, including the mode-only
            // refreshes of skipped recorded hours. With no predecessor the
            // first shown hour starts cold, as a seek does.
            let previous = previous.map(|(_, slot)| slot);
            let content = Arc::clone(&self.content);
            submit(
                "political",
                Box::new(move |view| {
                    Part::Political((|| {
                        let naval = view.naval_fill(&political_perspective)?;
                        let previous = previous.and_then(|slot| slot.wait());
                        let colour = previous
                            .as_ref()
                            .and_then(|political| political.key.mode())
                            .and_then(|mode| mode.colour.clone());
                        let colour = carry_colour(colour, mode_frames, |frame, colour| {
                            Ok(HourView::new(&content, frame, 0.0)
                                .map_mode_fill(&map_mode, &political_perspective, colour)?
                                .colour)
                        })?;
                        let mode = view.map_mode_fill(
                            &map_mode,
                            &political_perspective,
                            colour.as_deref(),
                        )?;
                        let key = FrameKey::new(view.frame)?
                            .with_naval(&naval)
                            .with_mode(&mode);
                        let near = wait_all(near_regions);
                        let political = match near.iter().find(|near| near.key == key) {
                            Some(same) => Arc::clone(same),
                            None => {
                                let layers = political::build_with_naval(
                                    &view.map, view.frame, naval, mode,
                                )?;
                                Arc::new(Political {
                                    id: next_id(),
                                    key,
                                    layers,
                                })
                            }
                        };
                        fill.set(Some(Arc::clone(&political)));
                        let zone =
                            view.demilitarized_zone(&political.layers.regions, &BTreeSet::new())?;
                        let near = wait_all(near_borders);
                        let borders = match near
                            .iter()
                            .find(|near| near.political == political.id && near.zone == zone)
                        {
                            Some(same) => Arc::clone(same),
                            None => Arc::new(Borders {
                                id: next_id(),
                                political: political.id,
                                classes: {
                                    let mut classes = ClassPlan::new(
                                        &view.border_meshes,
                                        &political.layers.regions,
                                        &zone,
                                    );
                                    for near in &near {
                                        classes.share(&near.classes);
                                    }
                                    classes
                                },
                                highlighted: ZonePlan::new(
                                    &view.border_meshes,
                                    &political.layers.regions,
                                    &zone,
                                ),
                                zone,
                            }),
                        };
                        fill_borders.set(Some(Arc::clone(&borders)));
                        Ok((political, borders))
                    })())
                }),
            );
        }
        let memos = Arc::clone(&self.memos);
        let names_mode = want.map_mode.clone();
        submit(
            "names",
            Box::new(move |view| {
                Part::Names(view.name_areas(&names_mode).and_then(|areas| {
                    memos.names.get_or_insert_with(areas.clone(), || {
                        Ok(Arc::new(view.place_name_areas(&areas, &memos.placed)))
                    })
                }))
            }),
        );
        let (camera, viewport, ui_scale) =
            (want.view.camera, want.view.viewport, want.view.ui_scale);
        let overlay_perspective = want.perspective.clone();
        let overlay_counters = want.counters.clone();
        // The window's index when it is for this perspective's country, so a
        // prebuilt hour carries the exact flags once the index covers it.
        let overlay_engaged = self
            .engaged
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .as_ref()
            .filter(|index| index.country() == &want.perspective.country)
            .cloned();
        submit(
            "overlay",
            Box::new(move |view| {
                Part::Overlay((|| {
                    // Raid icons come from the window's raid lane
                    // ([`crate::raid_lane`]), which chains hours.
                    let tables = view.hour_tables_with_raids(
                        &overlay_perspective,
                        Arc::default(),
                        overlay_engaged.as_deref(),
                    )?;
                    tables.warm_counter_flags(&view.icons.counters);
                    let overlay = view.prepare_overlay_cached(
                        camera,
                        viewport,
                        ui_scale,
                        &tables,
                        &overlay_counters,
                        &Default::default(),
                    )?;
                    Ok(Box::new((overlay, tables)))
                })())
            }),
        );
        if reusable.is_some() {
            return true;
        }
        {
            let fill = Fill(Arc::clone(&slots.rails));
            let near = slots_of(&neighbours, |slots| &slots.rails);
            let memos = Arc::clone(&self.memos);
            submit(
                "rails",
                Box::new(move |view| {
                    Part::Rails(RailNetwork::read(view.frame).and_then(|network| {
                        let rails = memos.rails.get_or_insert_with(network.clone(), || {
                            let mesh = view.rail_mesh_of(&network);
                            let bridges = view.bridges.field(&mesh);
                            Ok::<_, io::Error>(Shared::share(wait_all(near), (mesh, bridges)))
                        })?;
                        fill.set(Some(Arc::clone(&rails)));
                        Ok(rails)
                    }))
                }),
            );
        }
        {
            let fill = Fill(Arc::clone(&slots.buildings));
            let near = slots_of(&neighbours, |slots| &slots.buildings);
            let building_perspective = want.perspective.clone();
            submit(
                "buildings",
                Box::new(move |view| {
                    Part::Buildings(view.map_building_field(&building_perspective).map(
                        |(field, summary)| {
                            let buildings = Shared::share(wait_all(near), field);
                            fill.set(Some(Arc::clone(&buildings)));
                            (buildings, summary.warnings())
                        },
                    ))
                }),
            );
        }
        let movers_perspective = want.perspective.clone();
        let show_allied_plans = want.show_allied_plans;
        submit(
            "movers",
            Box::new(move |view| {
                Part::Movers((|| {
                    // The raid arrows' units and the trade-route convoys join the unit models.
                    let (orders, convoys) =
                        view.orders_and_convoys(&movers_perspective, show_allied_plans)?;
                    let mut units = view.prepare_unit_models(&movers_perspective)?;
                    units.add_raid_units(&view.unit_models, &orders.raid_units);
                    if let Ok(defines) = &view.trade_routes {
                        units.add_convoys(convoys, defines.convoys);
                    }
                    units.stage_field(&view.unit_models);
                    Ok(Arc::new(Movers {
                        trains: view.train_plan(&movers_perspective)?,
                        aircraft: view.aircraft_plan(&movers_perspective)?,
                        units: Arc::new(units),
                        orders,
                        effects: weather_effects::prepare(
                            &view.weather_effects,
                            weather_effects::field(view.frame)?,
                            camera,
                            viewport,
                        ),
                        impacts: Arc::new(view.nuclear_impacts()?),
                    }))
                })())
            }),
        );
        let fog_perspective = want.perspective.clone();
        let memos = Arc::clone(&self.memos);
        submit(
            "fog",
            Box::new(move |view| {
                Part::Fog((|| {
                    let vision = view.vision(&fog_perspective)?;
                    let fog = vision.province_fog();
                    let intel = memos.intel.get_or_insert_with(fog.cloned(), || {
                        textures::intel_map(&view.map, fog).map(Arc::new)
                    })?;
                    let limitations = view.limitations_with(&fog_perspective, Some(&intel))?;
                    let music = view.music_chances(vision.ours()).map(Arc::new);
                    let battles = view.battle_sounds().map(Arc::new);
                    Ok(Arc::new(FogPart {
                        intel,
                        limitations,
                        header: Arc::new(Header {
                            viewing: view
                                .vision(&fog_perspective)
                                .and_then(|vision| view.viewing_tag(&vision))
                                .ok(),
                        }),
                        map_text: Arc::new(view.map_text(&vision)),
                        music,
                        battles,
                    }))
                })())
            }),
        );
        let memos = Arc::clone(&self.memos);
        submit(
            "snow",
            Box::new(move |view| {
                Part::Snow(view.snow_cells().and_then(|cells| {
                    memos.snow.get_or_build(view.frame, (), |projected| {
                        weather::grid_from_cells(cells, projected).map(Arc::new)
                    })
                }))
            }),
        );
        true
    }
}

/// The client owns shutdown; internal result senders cannot keep an idle
/// coordinator alive after the last window handle has gone.
struct Requests(Sender<Event>);

impl Drop for Requests {
    fn drop(&mut self) {
        let _ = self.0.send(Event::Shutdown);
    }
}

/// The window's handle on its hour builders.
pub struct Pipeline {
    content: Arc<Content>,
    replay: Arc<Replay>,
    events: Requests,
    results: Receiver<Built>,
    want: Option<Want>,
    epoch: u64,
    received: Cell<u64>,
    seek_pending: bool,
    requested: Arc<Mutex<Option<Requested>>>,
    /// The engaged-flag index for the last perspective country asked for,
    /// shared with the coordinator's overlay builds.
    engaged: Engaged,
    /// The builders' turns, which other work may borrow.
    workers: Workers,
    pool: Arc<Pool>,
}

/// The window's engaged-flag index, shared by the window and its builders.
type Engaged = Arc<Mutex<Option<Arc<EngagedIndex>>>>;

impl Pipeline {
    /// Starts the coordinator and a builder pool of all the hardware threads
    /// the process may use (its affinity mask) less eight, at least two, leaving the window's own frame, the GPU
    /// driver and the game's own threads theirs: on the sixteen-thread
    /// machine this is eight builders, and more finished hours no sooner
    /// while spiking the frames they arrived on.
    #[cfg(test)]
    pub fn new(content: Arc<Content>, replay: Arc<Replay>) -> io::Result<Self> {
        Self::with_checkpoint_source(content, replay, None)
    }

    pub fn with_checkpoint_source(
        content: Arc<Content>,
        replay: Arc<Replay>,
        checkpoints: Option<Arc<crate::map_checkpoints::CheckpointSource>>,
    ) -> io::Result<Self> {
        let parallel = builder_count();
        Self::start(
            content,
            replay,
            Arc::new(Pool::new(parallel)?),
            None,
            parallel,
            checkpoints,
        )
    }

    /// The Campaign map borrows the root's worker budget, but keeps the same
    /// decode-ahead lane and in-flight window as a standalone window.
    pub fn campaign(
        content: Arc<Content>,
        replay: Arc<Replay>,
        resources: &Resources,
        checkpoints: Option<Arc<crate::map_checkpoints::CheckpointSource>>,
    ) -> io::Result<Self> {
        let parallel = viewer_foundation::parallel::Spawn::threads(&*resources.pool);
        Self::start(
            content,
            replay,
            Arc::clone(&resources.pool),
            None,
            parallel,
            checkpoints,
        )
    }

    pub fn shared(
        content: Arc<Content>,
        replay: Arc<Replay>,
        resources: &Resources,
    ) -> io::Result<Self> {
        let decoder = resources.decoder(Arc::clone(&replay))?;
        // Dashboard maps ask for one absolute target, without per-map lookahead.
        Self::start(
            content,
            replay,
            Arc::clone(&resources.pool),
            Some(decoder),
            1,
            None,
        )
    }

    fn start(
        content: Arc<Content>,
        replay: Arc<Replay>,
        pool: Arc<Pool>,
        shared_decoder: Option<Arc<SharedDecoder>>,
        parallel: usize,
        checkpoints: Option<Arc<crate::map_checkpoints::CheckpointSource>>,
    ) -> io::Result<Self> {
        let (events, inbox) = mpsc::channel();
        // At most one finished hour waits per hour in flight; a window that
        // stops taking them holds the coordinator instead of a backlog. The
        // window dropping `results` releases a coordinator blocked on it.
        let (outbox, results) = mpsc::sync_channel(parallel);
        let requested = Arc::new(Mutex::new(None));
        let workers = Workers::new(Arc::clone(&pool) as _);
        let lane = match shared_decoder {
            Some(_) => None,
            None => {
                let reader = BlockReader::new(Arc::clone(&replay), Arc::new(Dropper::new()?));
                let mut playback = Playback::unloaded(Arc::clone(&replay))?.with_reader(reader);
                if let Some(checkpoints) = checkpoints {
                    playback = playback.with_checkpoint_source(checkpoints);
                }
                Some(DecodeLane::start(playback)?)
            }
        };
        let engaged: Engaged = Arc::new(Mutex::new(None));
        let coordinator = Coordinator {
            content: Arc::clone(&content),
            playback: Playback::unloaded(Arc::clone(&replay))?,
            shared_decoder,
            lane,
            requested: Arc::clone(&requested),
            engaged: Arc::clone(&engaged),
            want: None,
            epoch: 0,
            pending: BTreeMap::new(),
            sent: BTreeMap::new(),
            sends: 0,
            slots: BTreeMap::new(),
            memos: Arc::new(Memos::new()),
            reusable: BTreeMap::new(),
            pool: Arc::clone(&pool),
            parallel,
            events: events.clone(),
            outbox,
        };
        thread::Builder::new()
            .name("hour coordinator".into())
            .spawn(move || {
                background_priority();
                coordinator.run(inbox)
            })?;
        Ok(Self {
            content,
            replay,
            events: Requests(events),
            results,
            want: None,
            epoch: 0,
            received: Cell::new(0),
            seek_pending: false,
            requested,
            engaged,
            workers,
            pool,
        })
    }

    /// The engaged-flag index for `country`, started on first use; another
    /// country starts a new pass and stops the old one.
    pub(crate) fn engaged(&self, country: &perspective::Country) -> io::Result<Arc<EngagedIndex>> {
        let mut held = self.engaged.lock().unwrap_or_else(|p| p.into_inner());
        if let Some(index) = held.as_ref().filter(|index| index.country() == country) {
            return Ok(Arc::clone(index));
        }
        let index = Arc::new(EngagedIndex::start(
            Arc::clone(&self.replay),
            country.clone(),
        )?);
        *held = Some(Arc::clone(&index));
        Ok(index)
    }

    /// Hands this pipeline's index on to the next snapshot's pipeline.
    pub(crate) fn take_engaged(&self) -> Option<Arc<EngagedIndex>> {
        self.engaged
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .take()
    }

    /// Keeps a previous snapshot's index when this snapshot extends it.
    pub(crate) fn adopt_engaged(&self, index: Arc<EngagedIndex>) {
        if index.extend(&self.replay) {
            *self.engaged.lock().unwrap_or_else(|p| p.into_inner()) = Some(index);
        }
    }

    /// The builders' threads for work split across threads, such as a raid
    /// batch ([`crate::parallel::Workers`]).
    pub fn workers(&self) -> Workers {
        self.workers.clone()
    }

    /// Queues a view's CPU preparation without blocking its frame.
    pub(crate) fn try_submit(&self, work: Box<dyn FnOnce() + Send>) -> bool {
        self.pool.try_submit(work).is_ok()
    }

    /// Whether this pipeline builds for this content and replay.
    pub fn serves(&self, content: &Arc<Content>, replay: &Arc<Replay>) -> bool {
        Arc::ptr_eq(&self.content, content) && Arc::ptr_eq(&self.replay, replay)
    }

    /// An explicit seek discards ready and pending results at the next want,
    /// including a seek to the hour already shown. History starts cold there.
    pub fn seek(&mut self) {
        self.seek_pending = true;
    }

    /// The playhead plus its consecutive successors fill one live pool.
    pub fn playback_ahead(&self) -> i32 {
        playback_ahead(viewer_foundation::parallel::Spawn::threads(&*self.pool))
    }

    /// Tells the builders what the window wants now. Returns the epoch the
    /// window must keep built hours of: a new perspective begins one.
    pub fn want(&mut self, mut want: Want) -> io::Result<u64> {
        want.received = self.received.get();
        let invalidates = want.discard_stale_have(self.want.as_ref(), self.seek_pending);
        if self.want.as_ref() != Some(&want) || invalidates {
            if invalidates {
                self.epoch += 1;
            }
            let mut requested = self.requested.lock().map_err(|_| stopped())?;
            let wake = requested.is_none();
            *requested = Some((self.epoch, want.clone()));
            drop(requested);
            if wake {
                self.events.0.send(Event::Want).map_err(|_| stopped())?;
            }
            self.want = Some(want);
            self.seek_pending = false;
        }
        Ok(self.epoch)
    }

    /// Every hour finished since the last call, without waiting.
    pub fn take(&self) -> io::Result<Vec<Built>> {
        let mut built = Vec::new();
        loop {
            match self.results.try_recv() {
                Ok(hour) => {
                    self.received.set(self.received.get() + 1);
                    built.push(hour);
                }
                Err(mpsc::TryRecvError::Empty) => return Ok(built),
                Err(mpsc::TryRecvError::Disconnected) => return Err(stopped()),
            }
        }
    }

    /// The next hour to finish, waiting for it.
    pub fn wait(&self) -> io::Result<Built> {
        let built = self.results.recv().map_err(|_| stopped())?;
        self.received.set(self.received.get() + 1);
        Ok(built)
    }
}

fn stopped() -> io::Error {
    io::Error::other("the hour builders stopped")
}

// Removed when playback.rs lands.
pub use viewer_foundation::retirement::Dropper;

#[cfg(test)]
mod tests;

#[cfg(test)]
mod mode_tests {
    use super::*;

    fn want() -> Want {
        Want {
            branch: 1,
            hour: 10,
            ahead: 2,
            behind: 2,
            playing: false,
            perspective: Default::default(),
            map_mode: Default::default(),
            show_allied_plans: false,
            counters: Default::default(),
            view: View {
                camera: Camera::fit(
                    viewer_foundation::map::camera::MapSize {
                        width: 100,
                        height: 100,
                    },
                    viewer_foundation::map::camera::MapDefines::test(),
                    [100, 100],
                ),
                viewport: [100, 100],
                ui_scale: 1.0,
            },
            have: vec![10, 11],
            received: 2,
        }
    }

    #[test]
    fn a_mode_change_rebuilds_the_hour_but_keeps_its_independent_parts() {
        use viewer_world::map::map_mode::{MapMode, Stock};
        let old = want();
        for stock in [Stock::States, Stock::Terrain, Stock::StrategicAir] {
            let mut next = old.clone();
            next.map_mode = MapMode::Stock(stock);
            next.counters.set_mode(&next.map_mode);
            assert!(old.invalidates(&next));
            assert!(!old.invalidates_parts(&next));
            assert!(next.discard_stale_have(Some(&old), false));
            assert!(next.have.is_empty());
        }
        let mut next = old.clone();
        next.perspective.fog = !next.perspective.fog;
        assert!(old.invalidates_parts(&next));
        next = old.clone();
        next.perspective.country = perspective::Country::Tag("test".into());
        assert!(old.invalidates_parts(&next));
        next = old.clone();
        next.show_allied_plans = !next.show_allied_plans;
        assert!(old.invalidates_parts(&next));
        next = old.clone();
        next.counters.settings.nato = !next.counters.settings.nato;
        assert!(old.invalidates_parts(&next));
        next = old.clone();
        next.counters = CounterView::default();
        assert!(old.invalidates_parts(&next));
    }

    #[test]
    fn reusable_parts_follow_the_wants_window_and_branch_without_a_count_cap() {
        let mut want = want();
        for hour in 0..20 {
            assert_eq!(want.keeps_reusable((1, hour)), (8..=12).contains(&hour));
            assert!(!want.keeps_reusable((2, hour)));
        }
        want.ahead = 5_000;
        assert!((10..=5_010).all(|hour| want.keeps_reusable((1, hour))));
        let mut kept: BTreeMap<_, _> = (8..=5_010).map(|hour| ((1, hour), ())).collect();
        want.hour = 5_010;
        want.ahead = 0;
        kept.retain(|key, _| want.keeps_reusable(*key));
        assert_eq!(
            kept.keys().copied().collect::<Vec<_>>(),
            [(1, 5_008), (1, 5_009), (1, 5_010)]
        );
        want.branch = 2;
        kept.retain(|key, _| want.keeps_reusable(*key));
        assert!(kept.is_empty());
    }

    #[test]
    fn reused_part_deliveries_preserve_identity_and_wait_for_the_three_mode_parts() {
        let mut pending = Pending {
            ticket: 7,
            current: Current::Gap {
                branch: 1,
                hour: 10,
                reason: String::new(),
            },
            alive: Arc::new(AtomicBool::new(true)),
            started: Instant::now(),
            political: None,
            names: None,
            overlay: None,
            rails: None,
            buildings: None,
            movers: None,
            fog: None,
            snow: None,
            received: 0,
            error: None,
            error_rank: None,
            laps: Vec::new(),
            view: want().view,
        };
        let rails = Arc::new(Rails {
            id: next_id(),
            value: Default::default(),
        });
        assert!(!pending.take(7, "rails", 0.0, Part::Rails(Ok(Arc::clone(&rails)))));
        for name in ["buildings", "movers", "fog", "snow"] {
            assert!(!pending.take(7, name, 0.0, Part::failed(name)));
        }
        assert!(Arc::ptr_eq(pending.rails.as_ref().unwrap(), &rails));
        assert_eq!(pending.received, 5);
        for name in ["political", "names"] {
            assert!(!pending.take(7, name, 1.0, Part::failed(name)));
        }
        assert!(pending.take(7, "overlay", 1.0, Part::failed("overlay")));
        assert_eq!(pending.received, PARTS);
    }
}
