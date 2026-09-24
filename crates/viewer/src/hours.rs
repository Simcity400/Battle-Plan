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
//! A coordinator uses its own [`Playback`] or a source-shared decoder. Maps
//! using the same [`Resources`], replay and content also share each branch's
//! snow and mud history, whose hours are walked in order. For each decoded
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
//! a political fill that did change carries the rows it differs in from the
//! previous hour's, so the window rewrites only those rows when the previous
//! hour's fill is the one on the GPU. Each such part waits for the previous
//! hour's value, which was dispatched first and so is already being built.

use std::{
    collections::{BTreeMap, BTreeSet, VecDeque},
    io,
    ops::Range,
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
    document::{Content, Hour as HourView, PreparedOverlay},
    map::{
        aircraft::AircraftPlan,
        border_pass::ClassPlan,
        camera::Camera,
        demilitarized::{Zone, ZonePlan},
        gradient::LayerPlane,
        models::ModelField,
        name_pass::Placements,
        orders::OrderField,
        perspective,
        political::{self, FrameKey, PoliticalLayers},
        railways::RailMesh,
        tables::Limitations,
        textures::IntelMap,
        trains::TrainPlan,
        unit_models::PreparedModels,
        weather::{self, SnowMud, WeatherHistory},
        weather_effects,
    },
    playback::{Current, Playback},
};

/// The camera the icon layer and the names are built for.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct View {
    pub camera: Camera,
    pub viewport: [u32; 2],
    /// Whether the state name layer is placed as well as the country one.
    pub states: bool,
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
    /// The neighbouring hours' political fills these textures were compared
    /// with, each with the rows they differ from it in. A neighbour most of
    /// whose rows differ is not listed.
    pub rows_from: Vec<(u64, Vec<Range<u32>>)>,
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
    pub buildings: Arc<Shared<ModelField>>,
    pub trains: TrainPlan,
    pub aircraft: AircraftPlan,
    pub units: PreparedModels,
    pub orders: OrderField,
    pub effects: weather_effects::Warmed,
    pub intel: IntelMap,
    /// Shared so the window can hold the grid it uploaded and compare.
    pub snow_mud: Arc<SnowMud>,
    /// Worded at fraction 0; the window sets its own fraction before reading.
    pub limitations: Limitations,
    pub header: Arc<Header>,
    pub names: Placements,
    pub overlay: PreparedOverlay,
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
    Names(io::Result<Placements>),
    Overlay(io::Result<Box<PreparedOverlay>>),
    Rails(io::Result<Arc<Rails>>),
    Buildings(io::Result<Arc<Shared<ModelField>>>),
    Movers(io::Result<Box<Movers>>),
    Fog(io::Result<(IntelMap, Limitations, Arc<Header>)>),
    Snow(io::Result<Arc<SnowMud>>),
}

struct Movers {
    trains: TrainPlan,
    aircraft: AircraftPlan,
    units: PreparedModels,
    orders: OrderField,
    effects: weather_effects::Warmed,
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
    buildings: Arc<Slot<Shared<ModelField>>>,
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
    weather: Mutex<BTreeMap<(usize, usize), Weak<SharedWeather>>>,
    pub dropper: Arc<Dropper>,
}

impl Resources {
    pub fn new() -> io::Result<Arc<Self>> {
        Ok(Arc::new(Self {
            pool: Arc::new(Pool::new(builder_count())?),
            decoders: Mutex::new(BTreeMap::new()),
            weather: Mutex::new(BTreeMap::new()),
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

    fn weather(
        &self,
        content: Arc<Content>,
        replay: Arc<Replay>,
    ) -> io::Result<Arc<SharedWeather>> {
        let key = (
            Arc::as_ptr(&replay) as usize,
            Arc::as_ptr(&content) as usize,
        );
        let mut histories = self.weather.lock().map_err(|_| stopped())?;
        histories.retain(|_, history| history.strong_count() > 0);
        if let Some(history) = histories.get(&key).and_then(Weak::upgrade) {
            return Ok(history);
        }
        // The owner retains both Arcs, so neither address can be reused while
        // this entry upgrades. Content identity also fixes the map and regions.
        let history = Arc::new(SharedWeather::new(content, replay));
        histories.insert(key, Arc::downgrade(&history));
        Ok(history)
    }
}

/// Ordered weather reconstruction for one immutable replay/content pair.
/// Its lock is used on coordinators, before any raster jobs reach the pool;
/// history reconstruction never waits for a pool job or a shared decoder.
struct SharedWeather {
    content: Arc<Content>,
    replay: Arc<Replay>,
    state: Mutex<WeatherState>,
}

struct WeatherState {
    history: WeatherHistory,
    // Share snapshots still held by coordinators or raster jobs. The registry
    // retains no additional snapshot payloads beyond WeatherHistory's cache.
    snapshots: BTreeMap<(BranchId, Hour), Weak<weather::Snapshot>>,
}

impl SharedWeather {
    fn new(content: Arc<Content>, replay: Arc<Replay>) -> Self {
        Self {
            content,
            replay,
            state: Mutex::new(WeatherState {
                history: WeatherHistory::new(),
                snapshots: BTreeMap::new(),
            }),
        }
    }

    fn snapshot(&self, frame: &replay::frame::DecodedFrame) -> io::Result<Arc<weather::Snapshot>> {
        let key = (frame.branch, frame.hour);
        let mut state = self.state.lock().map_err(|_| stopped())?;
        if let Some(snapshot) = state.snapshots.get(&key).and_then(Weak::upgrade) {
            return Ok(snapshot);
        }
        let snapshot = Arc::new(state.history.snapshot(
            &self.content.map,
            &self.content.strategic_regions,
            &self.replay,
            frame,
        )?);
        // Only successful results enter the cache. Existing history errors and
        // branch/load reconstruction rules stay with WeatherHistory.
        state
            .snapshots
            .retain(|_, snapshot| snapshot.strong_count() > 0);
        if state.snapshots.len() >= FRAME_CACHE {
            state.snapshots.pop_first();
        }
        state.snapshots.insert(key, Arc::downgrade(&snapshot));
        Ok(snapshot)
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

fn builder_count() -> usize {
    thread::available_parallelism()
        .map_or(4, |count| count.get())
        .saturating_sub(8)
        .max(2)
}

/// An hour dispatched and not yet assembled.
struct Pending {
    ticket: u64,
    current: Current,
    alive: Arc<AtomicBool>,
    started: Instant,
    political: Option<(Arc<Political>, Arc<Borders>)>,
    names: Option<Placements>,
    overlay: Option<PreparedOverlay>,
    rails: Option<Arc<Rails>>,
    buildings: Option<Arc<Shared<ModelField>>>,
    movers: Option<Movers>,
    fog: Option<(IntelMap, Limitations, Arc<Header>)>,
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

/// Lap order: the serial decode and snow-history walks first, then parts in
/// fixed order.
fn lap_rank(name: &str) -> u8 {
    match name {
        "decode" => 0,
        "snow_history" => 1,
        "political" => 2,
        "names" => 3,
        "overlay" => 4,
        "rails" => 5,
        "buildings" => 6,
        "movers" => 7,
        "fog" => 8,
        "snow" => 9,
        _ => 10,
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
        Ok(Layers {
            political,
            borders,
            rails: self.rails.ok_or_else(missing)?,
            buildings: self.buildings.ok_or_else(missing)?,
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
            overlay: self.overlay.ok_or_else(missing)?,
        })
    }
}

/// Recently decoded frames the coordinator keeps, so a step back or a
/// rebuild for a new perspective decodes nothing.
const FRAME_CACHE: usize = 96;

struct Coordinator {
    content: Arc<Content>,
    playback: Playback,
    shared_decoder: Option<Arc<SharedDecoder>>,
    requested: Arc<Mutex<Option<(u64, Want)>>>,
    frames: BTreeMap<(BranchId, Hour), Current>,
    order: VecDeque<(BranchId, Hour)>,
    weather: Arc<SharedWeather>,
    strategic: Option<Arc<LayerPlane>>,
    want: Option<Want>,
    epoch: u64,
    pending: BTreeMap<(BranchId, Hour), Pending>,
    /// Hours of the current epoch sent to the window, with the order they were
    /// sent in, until the window says it no longer holds them.
    sent: BTreeMap<(BranchId, Hour), u64>,
    /// How many built hours have been sent, of any epoch.
    sends: u64,
    /// Snow and mud caches of recently built hours, so a step back walks nothing.
    snapshots: BTreeMap<(BranchId, Hour), Arc<weather::Snapshot>>,
    /// The shared parts of the hours last dispatched, which the neighbouring
    /// hours' parts compare with.
    slots: BTreeMap<(BranchId, Hour), Slots>,
    pool: Arc<Pool>,
    parallel: usize,
    events: Sender<Event>,
    outbox: Sender<Built>,
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
            let wanted = keep.contains(key);
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
        self.snapshots.retain(|key, _| keep.contains(key));
        self.slots.retain(|key, _| keep.contains(key));
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

    /// The decoded hour, from the cache or the replay. `behind` earlier hours
    /// the decoder passes on its way are kept too, so the step back from a
    /// seek does not decode the block a second time.
    fn frame(&mut self, branch: BranchId, hour: Hour, behind: i32) -> io::Result<Current> {
        if let Some(decoder) = &self.shared_decoder {
            return decoder.frame(branch, hour);
        }
        if let Some(current) = self.frames.get(&(branch, hour)) {
            return Ok(current.clone());
        }
        let mut passed = Vec::new();
        let frames = &self.frames;
        self.playback.seek_with(branch, hour, |decoded| {
            if hour - decoded.hour <= behind && !frames.contains_key(&(branch, decoded.hour)) {
                passed.push(Current::Frame(Arc::new(decoded.clone())));
            }
        })?;
        let current = self.playback.current().clone();
        for (at, current) in passed
            .into_iter()
            .filter_map(|passed| match &passed {
                Current::Frame(frame) => Some((frame.hour, passed.clone())),
                Current::Gap { .. } => None,
            })
            .chain([(hour, current.clone())])
        {
            self.frames.insert((branch, at), current);
            self.order.push_back((branch, at));
        }
        while self.order.len() > FRAME_CACHE {
            if let Some(old) = self.order.pop_front() {
                self.frames.remove(&old);
            }
        }
        Ok(current)
    }

    /// Decodes an hour and hands its parts to the pool. Returns false when the
    /// window has gone.
    fn dispatch(&mut self, hour: Hour) -> bool {
        let want = self.want.clone().expect("next_hour found a want");
        let branch = want.branch;
        let epoch = self.epoch;
        let started = Instant::now();
        let current = match self.frame(branch, hour, want.behind.max(2)) {
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
        // The source/content history is walked in order before submitting any
        // pool work. Other maps can reuse this snapshot; rasterisation stays
        // with each view's parts.
        let weather_started = Instant::now();
        let snapshot = match self.snapshots.get(&(branch, hour)) {
            Some(snapshot) => Ok(Arc::clone(snapshot)),
            None => self.weather.snapshot(&frame).inspect(|snapshot| {
                self.snapshots.insert((branch, hour), Arc::clone(snapshot));
            }),
        };
        pending.laps.push((
            "snow_history",
            weather_started.elapsed().as_secs_f64() * 1000.0,
        ));
        if self.strategic.is_none() {
            match political::strategic_layer(&self.content.map, &self.content.strategic_regions) {
                Ok(plane) => self.strategic = Some(Arc::new(plane)),
                Err(error) => pending.error = Some(format!("strategic regions: {error}")),
            }
        }
        if let Some(error) = pending.error.take() {
            return self.send(Built {
                epoch,
                branch,
                hour,
                view: want.view,
                current,
                layers: Err(error),
                laps: pending.laps,
                wall: 0.0,
            });
        }
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
        let strategic = Arc::clone(self.strategic.as_ref().expect("built above"));
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
            let near_political = slots_of(&neighbours, |slots| &slots.political);
            let near_borders = slots_of(&neighbours, |slots| &slots.borders);
            submit(
                "political",
                Box::new(move |view| {
                    Part::Political((|| {
                        let key = FrameKey::new(view.frame)?;
                        let near = wait_all(near_political);
                        let political = match near.iter().find(|near| near.key == key) {
                            Some(same) => Arc::clone(same),
                            None => {
                                let layers = political::build(&view.map, view.frame, &strategic)?;
                                let rows_from = near
                                    .iter()
                                    .filter_map(|near| {
                                        layers
                                            .textures
                                            .changed_rows(&near.layers.textures)
                                            .map(|rows| (near.id, rows))
                                    })
                                    .collect();
                                Arc::new(Political {
                                    id: next_id(),
                                    key,
                                    layers,
                                    rows_from,
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
        let states = want.view.states;
        submit(
            "names",
            Box::new(move |view| Part::Names(view.place_names(states))),
        );
        let (camera, viewport) = (want.view.camera, want.view.viewport);
        let overlay_perspective = want.perspective.clone();
        submit(
            "overlay",
            Box::new(move |view| {
                Part::Overlay(
                    view.prepare_overlay(camera, viewport, &overlay_perspective)
                        .map(Box::new),
                )
            }),
        );
        {
            let fill = Fill(Arc::clone(&slots.rails));
            let near = slots_of(&neighbours, |slots| &slots.rails);
            submit(
                "rails",
                Box::new(move |view| {
                    Part::Rails(view.rail_mesh().map(|mesh| {
                        let bridges = view.bridges.field(&mesh);
                        let rails = Shared::share(wait_all(near), (mesh, bridges));
                        fill.set(Some(Arc::clone(&rails)));
                        rails
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
                        |(field, _)| {
                            let buildings = Shared::share(wait_all(near), field);
                            fill.set(Some(Arc::clone(&buildings)));
                            buildings
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
                    Ok(Box::new(Movers {
                        trains: view.train_plan(&movers_perspective)?,
                        aircraft: view.aircraft_plan(&movers_perspective)?,
                        units: view.prepare_unit_models(&movers_perspective)?,
                        orders: view.order_field(&movers_perspective)?,
                        effects: weather_effects::warm(
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
        submit(
            "fog",
            Box::new(move |view| {
                Part::Fog((|| {
                    Ok((
                        view.intel_map(&fog_perspective)?,
                        view.limitations(&fog_perspective)?,
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
        submit(
            "snow",
            Box::new(move |view| {
                Part::Snow(
                    snapshot
                        .and_then(|snapshot| {
                            weather::grid(&view.map, &view.assets.perm_snow, &snapshot, view.frame)
                        })
                        .map(Arc::new),
                )
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
}

impl Pipeline {
    /// Starts the coordinator and a builder pool of all hardware threads
    /// less eight, at least two, leaving the window's own frame, the GPU
    /// driver and the game's own threads theirs: on the sixteen-thread
    /// machine this is eight builders, and more finished hours no sooner
    /// while spiking the frames they arrived on.
    pub fn new(content: Arc<Content>, replay: Arc<Replay>) -> io::Result<Self> {
        let parallel = builder_count();
        let weather = Arc::new(SharedWeather::new(
            Arc::clone(&content),
            Arc::clone(&replay),
        ));
        Self::start(
            content,
            replay,
            Arc::new(Pool::new(parallel)?),
            None,
            weather,
            parallel,
        )
    }

    pub fn shared(
        content: Arc<Content>,
        replay: Arc<Replay>,
        resources: &Resources,
    ) -> io::Result<Self> {
        let decoder = resources.decoder(Arc::clone(&replay))?;
        let weather = resources.weather(Arc::clone(&content), Arc::clone(&replay))?;
        // Dashboard maps ask for one absolute target, without per-map lookahead.
        Self::start(
            content,
            replay,
            Arc::clone(&resources.pool),
            Some(decoder),
            weather,
            1,
        )
    }

    fn start(
        content: Arc<Content>,
        replay: Arc<Replay>,
        pool: Arc<Pool>,
        shared_decoder: Option<Arc<SharedDecoder>>,
        weather: Arc<SharedWeather>,
        parallel: usize,
    ) -> io::Result<Self> {
        let (events, inbox) = mpsc::channel();
        let (outbox, results) = mpsc::channel();
        let requested = Arc::new(Mutex::new(None));
        let coordinator = Coordinator {
            content: Arc::clone(&content),
            playback: Playback::unloaded(Arc::clone(&replay))?,
            shared_decoder,
            requested: Arc::clone(&requested),
            frames: BTreeMap::new(),
            order: VecDeque::new(),
            weather,
            strategic: None,
            want: None,
            epoch: 0,
            pending: BTreeMap::new(),
            sent: BTreeMap::new(),
            sends: 0,
            snapshots: BTreeMap::new(),
            slots: BTreeMap::new(),
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
        })
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
pub struct Dropper {
    bin: Option<Sender<Box<dyn Send>>>,
}

impl Dropper {
    pub fn new() -> io::Result<Self> {
        let (bin, inbox) = mpsc::channel::<Box<dyn Send>>();
        thread::Builder::new()
            .name("hour dropper".into())
            .spawn(move || {
                background_priority();
                while inbox.recv().is_ok() {}
            })?;
        Ok(Self { bin: Some(bin) })
    }

    /// Frees `value` off the frame path.
    pub fn drop_later<T: Send + 'static>(&self, value: T) {
        match &self.bin {
            Some(bin) => {
                if let Err(mpsc::SendError(value)) = bin.send(Box::new(value)) {
                    drop(value);
                }
            }
            None => drop(value),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
            states: false,
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
