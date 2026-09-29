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
//! waits for the previous hour's value, which was dispatched first and so is
//! already being built. For the political part the wait is not about the cheap
//! region derivation: an equal hour must keep the same political id, because
//! the border classes are shared, and their upload skipped, only by that id.

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

use replay::{BranchId, Hour, container::Replay};

use crate::{
    blocks::BlockReader,
    document::{Content, Hour as HourView, OverlayTables, PreparedOverlay},
    map::{
        aircraft::AircraftPlan,
        border_pass::ClassPlan,
        camera::Camera,
        demilitarized::{Zone, ZonePlan},
        map_buildings::PreparedBuildings,
        memo::{Keyed, Recent},
        models::ModelField,
        name_pass::{NameAreas, PlacedAreas, Placements},
        orders::OrderField,
        perspective,
        political::{self, FrameKey, PoliticalLayers},
        railways::{RailMesh, RailNetwork},
        tables::Limitations,
        textures::{self, IntelMap},
        trains::TrainPlan,
        unit_models::PreparedModels,
        weather::{self, SnowMud},
        weather_effects,
    },
    playback::{Current, Playback},
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
        hours.extend(
            (1..=self.ahead)
                .map(|step| self.hour + step)
                .filter(|hour| *hour <= last),
        );
        hours.extend(
            (1..=self.behind)
                .map(|step| self.hour - step)
                .filter(|hour| *hour >= first),
        );
        hours.retain(|hour| (first..=last).contains(hour));
        hours
    }

    /// Whether a change from `self` to `next` invalidates what was built.
    fn invalidates(&self, next: &Want) -> bool {
        self.perspective != next.perspective
    }

    /// Whether an hour in flight is still worth finishing: one of `keep`, or,
    /// playing, one of this branch at or before the playhead.
    fn keeps_pending(&self, key: (BranchId, Hour), keep: &BTreeSet<(BranchId, Hour)>) -> bool {
        keep.contains(&key) || (self.playing && key.0 == self.branch && key.1 <= self.hour)
    }

    /// Old-generation hours cannot satisfy a changed perspective.
    fn discard_stale_have(&mut self, previous: Option<&Self>) -> bool {
        let invalidates = previous.is_some_and(|old| old.invalidates(self));
        if invalidates {
            self.have.clear();
        }
        invalidates
    }
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
    zone: Zone,
    pub classes: ClassPlan,
    pub highlighted: ZonePlan,
}

/// What the window's header shows of an hour: every country tag for the
/// viewing-country chooser and the viewing country's own tag. Both walk the
/// hour's country tables, so they are read here and not on a frame.
#[derive(Debug, Default)]
pub struct Header {
    pub countries: Vec<String>,
    pub viewing: Option<String>,
}

/// Everything the map draws for one recorded hour, built off the frame path.
pub struct Layers {
    pub political: Arc<Political>,
    pub borders: Arc<Borders>,
    pub rails: Arc<Rails>,
    pub buildings: Arc<Shared<PreparedBuildings>>,
    /// The building field's refused type tokens
    /// ([`crate::map::map_buildings::MapBuildingSummary::warnings`]), worded
    /// with the hour: an hour sharing its neighbour's field still reports its own.
    pub building_warnings: Vec<String>,
    pub trains: TrainPlan,
    pub aircraft: AircraftPlan,
    pub units: PreparedModels,
    /// Shared by the hours whose plan tables are the same
    /// ([`crate::map::orders::TABLES`]), so the window uploads it once.
    pub orders: Arc<OrderField>,
    pub effects: weather_effects::Prepared,
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
    Movers(io::Result<Box<Movers>>),
    Fog(io::Result<(Arc<IntelMap>, Limitations, Arc<Header>)>),
    Snow(io::Result<Arc<SnowMud>>),
}

struct Movers {
    trains: TrainPlan,
    aircraft: AircraftPlan,
    units: PreparedModels,
    orders: Arc<OrderField>,
    effects: weather_effects::Prepared,
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
/// were built from (`map::memo`). A hit hands back the same allocation, so the
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

type Job = Box<dyn FnOnce() + Send>;

/// Worker threads taking jobs in the order they were given.
///
/// The order matters: a part waits only for the previous hour's part, which
/// was submitted before it and so has already been taken by a thread.
struct Pool {
    jobs: Option<mpsc::SyncSender<Job>>,
    threads: Vec<thread::JoinHandle<()>>,
}

impl Pool {
    fn new(count: usize) -> io::Result<Self> {
        let (jobs, inbox) = mpsc::sync_channel::<Job>(count.saturating_mul(8).max(16));
        let inbox = Arc::new(Mutex::new(inbox));
        let threads = (0..count)
            .map(|index| {
                let inbox = Arc::clone(&inbox);
                thread::Builder::new()
                    .name(format!("hour builder {index}"))
                    .spawn(move || {
                        background_priority();
                        loop {
                            let job = match inbox.lock() {
                                Ok(inbox) => inbox.recv(),
                                Err(_) => return,
                            };
                            match job {
                                Ok(job) => job(),
                                Err(_) => return,
                            }
                        }
                    })
            })
            .collect::<io::Result<Vec<_>>>()?;
        Ok(Self {
            jobs: Some(jobs),
            threads,
        })
    }

    fn submit(&self, job: Job) {
        if let Some(jobs) = &self.jobs {
            let _ = jobs.send(job);
        }
    }

    fn try_submit(&self, job: Job) -> Result<(), Job> {
        match &self.jobs {
            Some(jobs) => jobs.try_send(job).map_err(|error| match error {
                mpsc::TrySendError::Full(job) | mpsc::TrySendError::Disconnected(job) => job,
            }),
            None => Err(job),
        }
    }
}

/// The builders lend their turns to a batch split across threads
/// ([`crate::parallel::Workers`]): a turn taken while the builders are busy
/// waits in their queue, and the batch does not wait for it.
impl crate::parallel::Spawn for Pool {
    fn try_spawn(&self, job: Job) -> Result<(), Job> {
        self.try_submit(job)
    }

    fn threads(&self) -> usize {
        self.threads.len()
    }
}

impl Drop for Pool {
    fn drop(&mut self) {
        self.jobs = None;
        for thread in self.threads.drain(..) {
            let _ = thread.join();
        }
    }
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
    frames: BTreeMap<(BranchId, Hour), Weak<replay::frame::DecodedFrame>>,
}

impl SharedDecoder {
    pub fn seed(&self, playback: Playback) -> io::Result<()> {
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

/// Builders for the processors this process may use
/// ([`crate::parallel::usable_processors`]), not the machine's.
fn builder_count() -> usize {
    crate::parallel::usable_processors()
        .saturating_sub(8)
        .max(2)
}

/// A builder pool as the viewer starts one, lent as [`crate::parallel::Workers`].
#[cfg(test)]
pub(crate) fn builder_workers() -> io::Result<crate::parallel::Workers> {
    Ok(crate::parallel::Workers::new(Arc::new(Pool::new(
        builder_count(),
    )?)))
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
    movers: Option<Movers>,
    fog: Option<(Arc<IntelMap>, Limitations, Arc<Header>)>,
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
                Ok(value) => self.movers = Some(*value),
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

    fn layers(self) -> Result<Layers, String> {
        if let Some(error) = self.error {
            return Err(error);
        }
        let missing = || "an hour part did not arrive".to_owned();
        let (political, borders) = self.political.ok_or_else(missing)?;
        let movers = self.movers.ok_or_else(missing)?;
        let (intel, limitations, header) = self.fog.ok_or_else(missing)?;
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
/// own row sizes ([`replay::rows::TableRows::allocated_bytes`]), without the
/// strings and lists a row owns on the heap. The cache keeps at most
/// [`FRAME_CACHE`] hours and, past this budget, only the hours the current
/// want still reaches ([`FrameCache::evict`]).
const FRAME_CACHE_BYTES: u64 = 2 << 30;

/// A decoded table's row allocation in bytes.
fn table_bytes(table: &replay::frame::TableView) -> u64 {
    match table {
        replay::frame::TableView::Complete(rows)
        | replay::frame::TableView::Truncated { rows, .. } => rows.allocated_bytes() as u64,
        replay::frame::TableView::Unavailable { .. } => 0,
    }
}

/// The coordinator's recently decoded hours, least recently used first,
/// bounded by count and by row bytes.
///
/// Hours share every table that did not change between them
/// ([`replay::frame::SharedView`]), so the byte count is of distinct tables:
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

    fn lock(shared: &LaneShared) -> std::sync::MutexGuard<'_, LaneState> {
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
            if playback.seek(branch, step).is_err() {
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

struct Coordinator {
    content: Arc<Content>,
    playback: Playback,
    shared_decoder: Option<Arc<SharedDecoder>>,
    /// The playback decode lane, unless a shared decoder serves this view.
    lane: Option<DecodeLane>,
    requested: Arc<Mutex<Option<(u64, Want)>>>,
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
    pool: Arc<Pool>,
    parallel: usize,
    events: Sender<Event>,
    outbox: mpsc::SyncSender<Built>,
}

impl Coordinator {
    fn run(mut self, inbox: Receiver<Event>) {
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
                        let built = Built {
                            epoch,
                            branch,
                            hour,
                            view: pending.view,
                            current: pending.current.clone(),
                            wall: pending.started.elapsed().as_secs_f64() * 1000.0,
                            laps,
                            layers: pending.layers(),
                        };
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

    /// Starts a new epoch: every hour built or in flight is forgotten.
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
        want.discard_stale_have(self.want.as_ref());
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
        self.pending.retain(|key, pending| {
            let wanted = want.keeps_pending(*key, &keep);
            if !wanted {
                pending.alive.store(false, Ordering::Relaxed);
            }
            wanted
        });
        // Keep an hour the window holds, or one still on its way to it.
        self.sent.retain(|key, order| {
            keep.contains(key)
                && (*order >= want.received || (key.0 == want.branch && want.have.contains(&key.1)))
        });
        self.slots
            .retain(|key, _| keep.contains(key) || self.pending.contains_key(key));
        if let Some(lane) = &self.lane {
            lane.want(
                want.hours(range)
                    .into_iter()
                    .map(|hour| (want.branch, hour))
                    .collect(),
                want.behind.max(2),
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
        let pending = Pending {
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
        self.pending.insert((branch, hour), pending);
        let slots = Slots::new();
        // A part waits only for the neighbours' slots present now. Each of
        // those belongs to an hour dispatched before this one, whose jobs the
        // pool therefore took first; a cancelled or panicked part still fills
        // its slots, so no wait is for ever.
        let neighbours: Vec<Slots> = [hour - 1, hour + 1]
            .iter()
            .filter_map(|neighbour| self.slots.get(&(branch, *neighbour)).cloned())
            .collect();
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
            let near_borders = slots_of(&neighbours, |slots| &slots.borders);
            submit(
                "political",
                Box::new(move |view| {
                    Part::Political((|| {
                        let key = FrameKey::new(view.frame)?;
                        let near = wait_all(near_regions);
                        let political = match near.iter().find(|near| near.key == key) {
                            Some(same) => Arc::clone(same),
                            None => {
                                let layers = political::build(&view.map, view.frame)?;
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
        submit(
            "names",
            Box::new(move |view| {
                Part::Names(view.name_areas().and_then(|areas| {
                    memos.names.get_or_insert_with(areas.clone(), || {
                        Ok(Arc::new(view.place_name_areas(&areas, &memos.placed)))
                    })
                }))
            }),
        );
        let (camera, viewport, ui_scale) =
            (want.view.camera, want.view.viewport, want.view.ui_scale);
        let overlay_perspective = want.perspective.clone();
        submit(
            "overlay",
            Box::new(move |view| {
                Part::Overlay((|| {
                    // Raid icons come from the window's raid lane
                    // ([`crate::raid_lane`]), which chains hours.
                    let tables =
                        view.hour_tables_with_raids(&overlay_perspective, Arc::default())?;
                    let overlay =
                        view.prepare_overlay_cached(camera, viewport, ui_scale, &tables)?;
                    Ok(Box::new((overlay, tables)))
                })())
            }),
        );
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
        submit(
            "movers",
            Box::new(move |view| {
                Part::Movers((|| {
                    // The raid arrows' units join the unit models.
                    let orders = view.order_field(&movers_perspective)?;
                    let mut units = view.prepare_unit_models(&movers_perspective)?;
                    units.add_raid_units(&view.unit_models, &orders.raid_units);
                    Ok(Box::new(Movers {
                        trains: view.train_plan(&movers_perspective)?,
                        aircraft: view.aircraft_plan(&movers_perspective)?,
                        units,
                        orders,
                        effects: weather_effects::prepare(
                            &view.weather_effects,
                            weather_effects::field(view.frame)?,
                            camera,
                            viewport,
                        ),
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
                    Ok((
                        intel,
                        limitations,
                        Arc::new(Header {
                            countries: view.country_tags()?,
                            viewing: view
                                .vision(&fog_perspective)
                                .and_then(|vision| view.viewing_tag(&vision))
                                .ok(),
                        }),
                    ))
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
    received: std::cell::Cell<u64>,
    requested: Arc<Mutex<Option<(u64, Want)>>>,
    /// The builders' turns, which other work may borrow.
    workers: crate::parallel::Workers,
}

impl Pipeline {
    /// Starts the coordinator and a builder pool of all the hardware threads
    /// the process may use (its affinity mask) less eight, at least two, leaving the window's own frame, the GPU
    /// driver and the game's own threads theirs: on the sixteen-thread
    /// machine this is eight builders, and more finished hours no sooner
    /// while spiking the frames they arrived on.
    pub fn new(content: Arc<Content>, replay: Arc<Replay>) -> io::Result<Self> {
        let parallel = builder_count();
        Self::start(
            content,
            replay,
            Arc::new(Pool::new(parallel)?),
            None,
            parallel,
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
        )
    }

    fn start(
        content: Arc<Content>,
        replay: Arc<Replay>,
        pool: Arc<Pool>,
        shared_decoder: Option<Arc<SharedDecoder>>,
        parallel: usize,
    ) -> io::Result<Self> {
        let (events, inbox) = mpsc::channel();
        // At most one finished hour waits per hour in flight; a window that
        // stops taking them holds the coordinator instead of a backlog. The
        // window dropping `results` releases a coordinator blocked on it.
        let (outbox, results) = mpsc::sync_channel(parallel);
        let requested = Arc::new(Mutex::new(None));
        let workers = crate::parallel::Workers::new(Arc::clone(&pool) as _);
        let lane = match shared_decoder {
            Some(_) => None,
            None => {
                let reader = BlockReader::new(Arc::clone(&replay), Arc::new(Dropper::new()?));
                Some(DecodeLane::start(
                    Playback::unloaded(Arc::clone(&replay))?.with_reader(reader),
                )?)
            }
        };
        let coordinator = Coordinator {
            content: Arc::clone(&content),
            playback: Playback::unloaded(Arc::clone(&replay))?,
            shared_decoder,
            lane,
            requested: Arc::clone(&requested),
            want: None,
            epoch: 0,
            pending: BTreeMap::new(),
            sent: BTreeMap::new(),
            sends: 0,
            slots: BTreeMap::new(),
            memos: Arc::new(Memos::new()),
            pool,
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
            received: std::cell::Cell::new(0),
            requested,
            workers,
        })
    }

    /// The builders' threads for work split across threads, such as a raid
    /// batch ([`crate::parallel::Workers`]).
    pub fn workers(&self) -> crate::parallel::Workers {
        self.workers.clone()
    }

    /// Whether this pipeline builds for this content and replay.
    pub fn serves(&self, content: &Arc<Content>, replay: &Arc<Replay>) -> bool {
        Arc::ptr_eq(&self.content, content) && Arc::ptr_eq(&self.replay, replay)
    }

    /// Tells the builders what the window wants now. Returns the epoch the
    /// window must keep built hours of: a new perspective begins one.
    pub fn want(&mut self, mut want: Want) -> io::Result<u64> {
        want.received = self.received.get();
        let invalidates = want.discard_stale_have(self.want.as_ref());
        if self.want.as_ref() != Some(&want) {
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

/// Lowers the calling thread below the window's, for threads that build ahead.
///
/// A frame that became ready behind builders at normal priority waited out
/// the rest of a scheduler quantum, 15 ms on Windows: steady playback frames
/// of 15 to 100 ms with nothing in them but the wait. Below normal priority
/// the window's thread runs as soon as it is ready and the builders take
/// the rest.
pub use crate::worker_priority::background_priority;

fn stopped() -> io::Error {
    io::Error::other("the hour builders stopped")
}

/// Frees what the window let go of on a thread of its own, so a frame that
/// swaps in a new hour does not pay for returning the last one's memory.
///
/// The destructor runs at the builders' priority, so saturated builders can
/// starve it. At most one retirement per builder waits behind the one being
/// freed; past that the caller frees its retirement itself, spending that
/// frame's time rather than holding an unbounded backlog of hours.
pub struct Dropper {
    bin: Option<mpsc::SyncSender<Box<dyn Send>>>,
}

/// Everything one frame lets go of, handed to the [`Dropper`] as a single
/// retirement so its bound counts frames, not the pieces of an hour.
#[derive(Default)]
pub struct Retired(Vec<Box<dyn Send>>);

impl Retired {
    pub fn push<T: Send + 'static>(&mut self, value: T) {
        self.0.push(Box::new(value));
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

impl Dropper {
    pub fn new() -> io::Result<Self> {
        Self::with_capacity(builder_count())
    }

    fn with_capacity(capacity: usize) -> io::Result<Self> {
        let (bin, inbox) = mpsc::sync_channel::<Box<dyn Send>>(capacity);
        thread::Builder::new()
            .name("hour dropper".into())
            .spawn(move || {
                background_priority();
                while inbox.recv().is_ok() {}
            })?;
        Ok(Self { bin: Some(bin) })
    }

    /// Frees `value` off the frame path, or here, without waiting, when the
    /// queue is full.
    pub fn drop_later<T: Send + 'static>(&self, value: T) {
        if let Some(bin) = &self.bin {
            // A full or closed queue hands the value back; it drops here.
            let _ = bin.try_send(Box::new(value));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn retirements_are_bounded_even_when_the_destructor_stalls() {
        /// Counts its drop; the first one stalls the destructor until released.
        struct Counted(
            Arc<std::sync::atomic::AtomicUsize>,
            Option<(Sender<()>, Receiver<()>)>,
        );
        impl Drop for Counted {
            fn drop(&mut self) {
                if let Some((started, release)) = self.1.take() {
                    started.send(()).unwrap();
                    release.recv().unwrap();
                }
                self.0.fetch_add(1, Ordering::SeqCst);
            }
        }
        for capacity in [1, builder_count()] {
            let dropped = Arc::new(std::sync::atomic::AtomicUsize::new(0));
            let frame = |pieces| {
                let mut retired = Retired::default();
                for _ in 0..pieces {
                    retired.push(Counted(Arc::clone(&dropped), None));
                }
                retired
            };
            let (started, stalled) = mpsc::channel();
            let (release, wait) = mpsc::channel();
            let dropper = Dropper::with_capacity(capacity).unwrap();
            dropper.drop_later(Counted(Arc::clone(&dropped), Some((started, wait))));
            stalled.recv().unwrap();
            // A frame's pieces take one place in the queue however many.
            for _ in 0..capacity {
                dropper.drop_later(frame(3));
            }
            assert_eq!(dropped.load(Ordering::SeqCst), 0);
            // Full: this frame's retirement is freed here, without waiting on
            // the stalled destructor; every queued one is still held.
            dropper.drop_later(frame(2));
            assert_eq!(dropped.load(Ordering::SeqCst), 2);
            release.send(()).unwrap();
            // The destructor frees the queue in order, then the thread ends.
            drop(dropper);
            let deadline = Instant::now() + std::time::Duration::from_secs(10);
            while dropped.load(Ordering::SeqCst) < 3 + 3 * capacity {
                assert!(
                    Instant::now() < deadline,
                    "the queued retirements were not freed"
                );
                thread::yield_now();
            }
        }
    }

    #[test]
    fn playing_finishes_in_flight_hours_at_or_before_the_playhead() {
        let old = Want {
            branch: 1,
            hour: 10,
            ahead: 0,
            behind: 0,
            playing: true,
            perspective: Default::default(),
            view: test_view(),
            have: vec![],
            received: 0,
        };
        let mut next = Want {
            hour: 100,
            ..old.clone()
        };
        let keep = BTreeSet::from([(1, 100)]);
        assert!(next.keeps_pending((1, 10), &keep));
        assert!(!next.keeps_pending((2, 10), &keep));
        assert!(!next.keeps_pending((1, 101), &keep));
        // Paused, only the wanted hours are finished.
        next.playing = false;
        assert!(!next.keeps_pending((1, 10), &keep));
        assert!(next.keeps_pending((1, 100), &keep));
        // Seeks and branch changes keep what was built for a step back; only
        // a perspective change starts a new epoch.
        for moved in [
            Want {
                hour: 9,
                ..old.clone()
            },
            Want {
                branch: 2,
                ..old.clone()
            },
        ] {
            assert!(!old.invalidates(&moved));
        }
        next = old.clone();
        next.perspective.fog = !next.perspective.fog;
        assert!(old.invalidates(&next));
    }

    /// The row size `sized_frame` fills its `Countries` table with.
    const ROW: u64 = std::mem::size_of::<replay::rows::CountryRow>() as u64;

    /// A decoded hour whose `Countries` table holds `rows` rows.
    fn sized_frame(hour: Hour, rows: usize) -> Current {
        use replay::{
            frame::{DecodedFrame, Metrics, TableView},
            rows::{CountryRow, TableId, TableRows},
        };
        let mut tables = std::array::from_fn(|_| TableView::Unavailable {
            reason: "not needed".into(),
        });
        tables[TableId::Countries.index()] = TableView::Complete(TableRows::Countries(
            (0..rows)
                .map(|slot| CountryRow {
                    canonical_slot: slot as u32,
                    ..Default::default()
                })
                .collect(),
        ));
        Current::Frame(Arc::new(DecodedFrame {
            branch: 1,
            hour,
            tables: tables.map(Into::into),
            metrics: Metrics::default(),
        }))
    }

    #[test]
    fn the_frame_cache_is_bounded_by_bytes_but_keeps_the_wanted_hours() {
        let mut cache = FrameCache::default();
        // Each hour holds 1,000 rows.
        let each = 1_000 * ROW;
        for hour in 0..10 {
            cache.insert((1, hour), sized_frame(hour, 1_000));
        }
        assert_eq!(cache.bytes, 10 * each);
        // A budget of four hours keeps the newest four.
        cache.evict_except(4 * each, &BTreeSet::new());
        assert_eq!(cache.frames.len(), 4);
        assert!(cache.contains_key(&(1, 9)) && !cache.contains_key(&(1, 5)));
        assert_eq!(cache.bytes, 4 * each);
        // Spared hours stay even over budget; the oldest others go first.
        cache.evict_except(0, &BTreeSet::from([(1, 6), (1, 8)]));
        assert_eq!(cache.frames.len(), 2);
        assert!(cache.contains_key(&(1, 6)) && cache.contains_key(&(1, 8)));
        // A hit makes an hour the most recently used: hour 6 outlives 8.
        assert!(cache.get(&(1, 6)).is_some());
        cache.evict_except(each, &BTreeSet::new());
        assert!(cache.contains_key(&(1, 6)) && !cache.contains_key(&(1, 8)));
        cache.insert((1, 7), sized_frame(7, 1_000));
        cache.insert((1, 8), sized_frame(8, 1_000));
        // Replacing an hour does not count it twice.
        cache.insert((1, 8), sized_frame(8, 500));
        assert_eq!(cache.frames.len(), 3);
        assert_eq!(cache.bytes, 2 * each + 500 * ROW);
        // The count bound still applies to hours that cost nothing.
        let mut gaps = FrameCache::default();
        for hour in 0..(FRAME_CACHE as i32 + 10) {
            gaps.insert(
                (1, hour),
                Current::Gap {
                    branch: 1,
                    hour,
                    reason: String::new(),
                },
            );
        }
        gaps.evict_except(FRAME_CACHE_BYTES, &BTreeSet::new());
        assert_eq!(gaps.frames.len(), FRAME_CACHE);
    }

    /// Hours share their unchanged tables, and the cache counts each shared
    /// table once, releasing it only with the last hour that holds it.
    #[test]
    fn the_frame_cache_counts_a_shared_table_once() {
        let each = 1_000 * ROW;
        let Current::Frame(first) = sized_frame(0, 1_000) else {
            unreachable!()
        };
        let mut second = (*first).clone();
        second.hour = 1;
        let mut cache = FrameCache::default();
        cache.insert((1, 0), Current::Frame(first));
        cache.insert((1, 1), Current::Frame(Arc::new(second)));
        assert_eq!(cache.bytes, each);
        cache.evict_except(0, &BTreeSet::from([(1, 1)]));
        assert_eq!(cache.frames.len(), 1);
        assert_eq!(cache.bytes, each, "the kept hour still holds the table");
        let mut third = FrameCache::default();
        third.insert((1, 0), sized_frame(0, 1_000));
        third.insert((1, 1), sized_frame(1, 1_000));
        assert_eq!(third.bytes, 2 * each, "equal rows in two allocations");
    }

    fn test_view() -> View {
        View {
            camera: Camera::fit(
                crate::map::camera::MapSize {
                    width: 100,
                    height: 100,
                },
                crate::map::camera::MapDefines::test(),
                [100, 100],
            ),
            viewport: [100, 100],
            ui_scale: 1.0,
        }
    }

    fn pending(ticket: u64) -> Pending {
        Pending {
            ticket,
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
            view: test_view(),
        }
    }

    #[test]
    fn concurrent_errors_wait_for_all_parts_and_choose_fixed_priority() {
        let names = [
            "political",
            "names",
            "overlay",
            "rails",
            "buildings",
            "movers",
            "fog",
            "snow",
        ];
        for shift in 0..PARTS {
            let mut pending = pending(7);
            for index in 0..PARTS {
                let name = names[(index + shift) % PARTS];
                let done = pending.take(7, name, 0.0, Part::failed(name));
                assert_eq!(
                    done,
                    index + 1 == PARTS,
                    "arrival {index}, rotation {shift}"
                );
            }
            assert_eq!(pending.error_rank, Some(0));
            assert!(pending.layers().err().unwrap().starts_with("political:"));
        }
    }

    #[test]
    fn a_cancelled_dispatch_cannot_fill_a_rerequested_hour() {
        let mut replacement = pending(8);
        assert!(!replacement.take(7, "snow", 0.0, Part::failed("snow")));
        assert_eq!(replacement.received, 0);
        assert!(replacement.error.is_none());
        assert!(replacement.laps.is_empty());
        assert!(!replacement.take(8, "snow", 0.0, Part::failed("snow")));
        assert_eq!(replacement.received, 1);
    }

    #[test]
    fn dropping_requests_stops_a_coordinator_that_keeps_an_internal_sender() {
        let (events, inbox) = mpsc::channel();
        let internal = events.clone();
        drop(Requests(events));
        assert!(matches!(inbox.try_recv(), Ok(Event::Shutdown)));
        drop(internal);
    }

    #[test]
    fn perspective_changes_discard_old_hour_acknowledgements() {
        let old = Want {
            branch: 1,
            hour: 10,
            ahead: 2,
            behind: 2,
            playing: false,
            perspective: Default::default(),
            view: test_view(),
            have: vec![10, 11],
            received: 2,
        };
        let mut next = old.clone();
        next.perspective.fog = !next.perspective.fog;
        assert!(next.discard_stale_have(Some(&old)));
        assert!(next.have.is_empty());
        let mut next = old.clone();
        next.view.camera.height += 1.0;
        assert!(!next.discard_stale_have(Some(&old)));
        assert_eq!(next.have, old.have);
    }

    #[test]
    fn a_filled_slot_answers_every_waiter() {
        let slot = Slot::<u32>::new();
        let fill = Fill(Arc::clone(&slot));
        let waiter = {
            let slot = Arc::clone(&slot);
            thread::spawn(move || slot.wait().map(|value| *value))
        };
        fill.set(Some(Arc::new(7)));
        assert_eq!(waiter.join().unwrap(), Some(7));
        assert_eq!(slot.wait().map(|value| *value), Some(7));
    }

    #[test]
    fn a_part_that_never_runs_leaves_its_slot_empty_not_pending() {
        let slot = Slot::<u32>::new();
        drop(Fill(Arc::clone(&slot)));
        assert_eq!(slot.wait(), None);
    }

    #[test]
    fn the_intel_map_follows_the_viewing_countrys_province_bytes() {
        let memos = Memos::new();
        let built = std::cell::Cell::new(0);
        let intel = |fog: Option<BTreeMap<u32, u8>>| {
            memos
                .intel
                .get_or_insert_with(fog.clone(), || {
                    built.set(built.get() + 1);
                    Ok::<_, ()>(Arc::new(IntelMap {
                        texels: fog.map_or_else(Vec::new, |fog| fog.into_values().collect()),
                        fog: true,
                    }))
                })
                .unwrap()
        };
        let german = intel(Some(BTreeMap::from([(1, 150), (2, 0)])));
        // Another country, or the same one after its sight changed: rebuilt.
        let soviet = intel(Some(BTreeMap::from([(1, 0), (2, 150)])));
        // Fog off draws no fog: a key of its own.
        let clear = intel(None);
        assert_eq!(built.get(), 3);
        assert!(!Arc::ptr_eq(&german, &soviet) && !Arc::ptr_eq(&german, &clear));
        // Back to the first view: the same grid, nothing uploaded again.
        assert!(Arc::ptr_eq(
            &german,
            &intel(Some(BTreeMap::from([(1, 150), (2, 0)])))
        ));
        assert_eq!(built.get(), 3);
    }

    fn lane_over(replay: &Arc<Replay>) -> DecodeLane {
        DecodeLane::start(Playback::unloaded(Arc::clone(replay)).unwrap().with_reader(
            BlockReader::new(Arc::clone(replay), Arc::new(Dropper::new().unwrap())),
        ))
        .unwrap()
    }

    fn long_replay(hours: &[Hour]) -> Arc<Replay> {
        Arc::new(
            Replay::from_records(
                b"campaign".to_vec(),
                &[crate::playback::tests::records(1, hours)],
            )
            .unwrap(),
        )
    }

    /// `lane.frame` on another thread, failing the test rather than hanging it.
    fn frame_within(lane: &Arc<DecodeLane>, branch: BranchId, hour: Hour) -> Current {
        let (done, result) = mpsc::channel();
        let asker = Arc::clone(lane);
        thread::spawn(move || {
            let _ = done.send(asker.frame(branch, hour));
        });
        result
            .recv_timeout(std::time::Duration::from_secs(10))
            .expect("the lane answers")
    }

    #[test]
    fn the_lane_serves_a_wanted_hour_past_its_window() {
        let hours: Vec<Hour> = (10..=110).collect();
        let replay = long_replay(&hours);
        let lane = Arc::new(lane_over(&replay));
        // A want of 101 hours, as playing at 720 h/s asks for.
        lane.want(hours.iter().map(|hour| (1, *hour)).collect(), 2);
        assert!(matches!(frame_within(&lane, 1, 10), Current::Frame(_)));
        // The window already holds 11..=59 built, so the next hour asked for
        // is 60: wanted, but past the lane's window.
        match frame_within(&lane, 1, 60) {
            Current::Frame(frame) => assert_eq!(frame.hour, 60),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn a_walk_whose_target_is_no_longer_wanted_stops_after_one_step() {
        let hours: Vec<Hour> = (10..=700).collect();
        let replay = long_replay(&hours);
        let mut playback = Playback::unloaded(Arc::clone(&replay)).unwrap();
        let shared = LaneShared {
            state: Mutex::new(LaneState::default()),
            changed: Condvar::new(),
        };
        // Nothing wants 690: the walk stops after its first step.
        assert!(!DecodeLane::walk(&shared, &mut playback, (1, 690)));
        assert_eq!(playback.walk_start(1, 690), Some(9 + DECODE_STEP));
        // Wanted, the walk goes all the way to the last step before it.
        Mutex::lock(&shared.state).unwrap().urgent = Some((1, 690));
        assert!(DecodeLane::walk(&shared, &mut playback, (1, 690)));
        assert!(
            playback
                .walk_start(1, 690)
                .is_some_and(|at| 690 - at <= DECODE_STEP)
        );
    }

    #[test]
    fn a_new_want_mid_walk_is_served() {
        let hours: Vec<Hour> = (10..=700).collect();
        let replay = long_replay(&hours);
        let lane = Arc::new(lane_over(&replay));
        lane.want(vec![(1, 690)], 2);
        // Seek away before or while the lane walks to 690.
        lane.want(vec![(1, 12), (1, 13)], 2);
        match frame_within(&lane, 1, 12) {
            Current::Frame(frame) => assert_eq!(frame.hour, 12),
            other => panic!("{other:?}"),
        }
        match frame_within(&lane, 1, 13) {
            Current::Frame(frame) => assert_eq!(frame.hour, 13),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn a_dropped_lane_ends_its_thread_and_a_dead_lane_answers_with_a_gap() {
        let replay = long_replay(&[10, 11, 12]);
        let lane = lane_over(&replay);
        let shared = Arc::downgrade(&lane.shared);
        drop(lane);
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while shared.strong_count() > 0 {
            assert!(
                std::time::Instant::now() < deadline,
                "the lane thread ended"
            );
            thread::sleep(std::time::Duration::from_millis(5));
        }
        // A lane whose thread is gone answers at once.
        let dead = DecodeLane {
            shared: Arc::new(LaneShared {
                state: Mutex::new(LaneState {
                    dead: true,
                    ..LaneState::default()
                }),
                changed: Condvar::new(),
            }),
        };
        assert!(matches!(
            dead.frame(1, 10),
            Current::Gap { reason, .. } if reason == LANE_STOPPED
        ));
    }

    #[test]
    fn the_decode_lane_decodes_wanted_hours_ahead_and_serves_any_other() {
        let replay = Arc::new(crate::playback::tests::fixture());
        let lane = lane_over(&replay);
        lane.want(vec![(1, 100), (1, 101), (1, 105)], 2);
        let decoded = |current: Current| match current {
            Current::Frame(frame) => Some(frame.hour),
            Current::Gap { .. } => None,
        };
        assert_eq!(decoded(lane.frame(1, 105)), Some(105));
        // Decoded ahead, in want order: already there.
        assert_eq!(decoded(lane.frame(1, 100)), Some(100));
        // An hour the want does not name is decoded on request; a hole is a gap.
        assert_eq!(decoded(lane.frame(1, 820)), Some(820));
        assert_eq!(decoded(lane.frame(1, 103)), None);
        let expected = match replay.seek(1, 101).unwrap() {
            replay::container::SeekResult::Frame(frame) => *frame,
            other => panic!("{other:?}"),
        };
        match lane.frame(1, 101) {
            Current::Frame(frame) => assert_eq!(*frame, expected),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn the_lane_window_runs_from_the_last_hour_served_and_is_never_evicted() {
        let mut state = LaneState {
            wanted: (0..100).map(|hour| (1, hour)).collect(),
            ..LaneState::default()
        };
        assert_eq!(state.window().len(), DECODE_AHEAD);
        state.served = Some((1, 10));
        assert_eq!(state.window().last(), Some(&(1, 10 + DECODE_AHEAD as i32)));
        state.served = Some((1, 99));
        assert_eq!(state.window().len(), 100);
        // A full cache frees hours outside the window first.
        state.served = Some((1, 0));
        for hour in 0..(FRAME_CACHE as i32 + 20) {
            state.frames.insert((1, hour), sized_frame(hour, 1));
        }
        state.evict();
        assert_eq!(state.frames.frames.len(), FRAME_CACHE);
        for key in state.window().to_vec() {
            assert!(state.frames.contains_key(&key), "{key:?}");
        }
    }

    #[test]
    fn an_equal_value_keeps_the_previous_id() {
        let first = Shared::share(Vec::new(), vec![1, 2]);
        let other = Shared::share(Vec::new(), vec![5]);
        let same = Shared::share(vec![Arc::clone(&other), Arc::clone(&first)], vec![1, 2]);
        let changed = Shared::share(vec![Arc::clone(&first)], vec![1, 3]);
        assert_eq!(first.id, same.id);
        assert!(Arc::ptr_eq(&first, &same));
        assert_ne!(first.id, changed.id);
    }

    #[test]
    fn a_failed_part_of_every_name_is_an_error_not_a_value() {
        for name in [
            "political",
            "names",
            "overlay",
            "rails",
            "buildings",
            "movers",
            "fog",
            "snow",
        ] {
            let failed = match Part::failed(name) {
                Part::Political(result) => result.is_err(),
                Part::Names(result) => result.is_err(),
                Part::Overlay(result) => result.is_err(),
                Part::Rails(result) => result.is_err(),
                Part::Buildings(result) => result.is_err(),
                Part::Movers(result) => result.is_err(),
                Part::Fog(result) => result.is_err(),
                Part::Snow(result) => result.is_err(),
            };
            assert!(failed, "{name} must fail its hour");
        }
    }
}
