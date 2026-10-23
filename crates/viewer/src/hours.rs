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
//! backlog of hours nobody wants any more. Only wholly unaccepted, unneeded
//! targets can be superseded; a pool-accepted target runs all of its parts.
//! Decode, pool and output readiness arrive as events, so none can keep a
//! completed hour waiting on a later dispatch.
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

mod cadence;
mod coordinator;
mod dispatch_plan;
mod flow;
mod memory_pressure;
mod shared_decoder;
mod source_mailbox;
pub use shared_decoder::SharedDecoder;
mod trace;

use crate::map_views::CounterView;
use std::cell::Cell;
use std::sync::MutexGuard;
use std::{
    collections::{BTreeMap, BTreeSet, HashMap, VecDeque},
    io,
    sync::{
        Arc, Condvar, Mutex, OnceLock, Weak,
        atomic::{AtomicBool, AtomicU64, Ordering},
        mpsc::{self, Receiver, Sender},
    },
    thread,
    time::{Duration, Instant},
};
use viewer_foundation::builder_pool::Pool;
use viewer_foundation::parallel::Workers;
use viewer_foundation::profiling::{BuilderSpan, BuilderSpans};
use viewer_foundation::worker_priority::background_priority;

use replay::container::Replay;
use replay_data::{BranchId, Hour};
use viewer_world::map::map_buildings::BuildingsBase;

use {
    crate::document::Content, crate::document::Hour as HourView, crate::document::OverlayTables,
    crate::document::PreparedOverlay, crate::engaged_index::EngagedIndex, crate::playback::Current,
    crate::playback::Playback, viewer_foundation::blocks::BlockReader,
    viewer_foundation::map::camera::Camera, viewer_foundation::map::memo::Settled,
    viewer_foundation::map::memo::Recent, viewer_foundation::map::name_draw::Placements,
    viewer_foundation::map::perspective, viewer_foundation::map::ribbon_content::OrderField,
    viewer_foundation::map::tables::Limitations,
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
    pub grid: Option<Grid>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Grid {
    pub cadence: Cadence,
    pub installed: Option<Hour>,
    pub usable: Vec<Hour>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Cadence {
    pub sequence: u64,
    pub stride: u32,
    pub count: u32,
    pub anchor: Option<Hour>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct GridFacts {
    installed: Option<Hour>,
    newest_usable_due: Option<Hour>,
    newest_reserved_due: Option<Hour>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct Obligation {
    sequence: u64,
    epoch: u64,
    key: (BranchId, Hour),
    sent: Option<u64>,
    done: bool,
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

    fn keeps_sent(&self, key: (BranchId, Hour), order: u64) -> bool {
        let kept = order >= self.received || (key.0 == self.branch && self.have.contains(&key.1));
        kept && (self.grid.is_some() || self.keeps_reusable(key))
    }

    fn grid_facts(
        &self,
        flights: impl IntoIterator<Item = ((BranchId, Hour), bool)>,
        usable_sent: impl IntoIterator<Item = (BranchId, Hour)>,
    ) -> GridFacts {
        let Some(grid) = &self.grid else {
            return GridFacts::default();
        };
        let installed = grid.installed;
        let due = |(branch, hour): (BranchId, Hour)| branch == self.branch && hour <= self.hour;
        GridFacts {
            installed,
            newest_usable_due: grid
                .usable
                .iter()
                .map(|hour| (self.branch, *hour))
                .chain(usable_sent)
                .filter(|key| due(*key))
                .map(|(_, hour)| hour)
                .max(),
            newest_reserved_due: flights
                .into_iter()
                .filter(|&(key, started)| {
                    !started && due(key) && installed.is_none_or(|installed| key.1 > installed)
                })
                .map(|((_, hour), _)| hour)
                .max(),
        }
    }

    fn keeps_flight(
        &self,
        key: (BranchId, Hour),
        started: bool,
        wanted: &BTreeSet<(BranchId, Hour)>,
        facts: &GridFacts,
    ) -> bool {
        let (branch, hour) = key;
        if branch != self.branch || facts.installed.is_some_and(|installed| hour <= installed) {
            return false;
        }
        wanted.contains(&key)
            || started
            || (hour <= self.hour
                && facts.newest_reserved_due == Some(hour)
                && facts.newest_usable_due.is_none_or(|usable| usable <= hour))
    }

    /// Explicit seeks, including same-hour seeks, restart history just like
    /// a changed mode or perspective. No ready or pending result may cross it.
    fn discard_stale_have(&mut self, previous: Option<&Self>, sought: bool) -> bool {
        let invalidates = sought || previous.is_some_and(|old| old.invalidates(self));
        if invalidates {
            self.have.clear();
            if let Some(grid) = &mut self.grid {
                grid.installed = None;
                grid.usable.clear();
            }
        }
        invalidates
    }
}

// Experimental fast-play grid; absent by default.
pub fn page_grid() -> bool {
    static SWITCH: std::sync::LazyLock<bool> = std::sync::LazyLock::new(|| {
        std::env::var_os("BATTLE_PLAN_VIEWER_PAGE_GRID").or_else(|| std::env::var_os("HOI4_VIEWER_PAGE_GRID")).is_some_and(|value| value == "1")
    });
    *SWITCH
}

pub fn grid_stride(speed: f64) -> Option<u32> {
    cadence::stride(speed)
}

fn new_intent(
    previous: Option<(BranchId, u32)>,
    want: (BranchId, u32),
    sought: bool,
) -> Option<&'static str> {
    if sought {
        return Some("seek");
    }
    match previous {
        None => Some("start"),
        Some((branch, _)) if branch != want.0 => Some("branch"),
        Some((_, stride)) if stride != want.1 => Some("stride"),
        Some(_) => None,
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
    frames: impl IntoIterator<Item = Current>,
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

/// Resolve dependent slots before claiming a single-flight cell. FIFO dequeue
/// alone does not order workers' entry into the memo after preemption.
fn memo_after_neighbours<K: PartialEq, V, T, E>(
    memo: &viewer_foundation::map::memo::Recent<K, V>,
    key: K,
    neighbours: impl FnOnce() -> Vec<Arc<T>>,
    build: impl FnOnce(Vec<Arc<T>>) -> Result<Arc<V>, E>,
) -> Result<Arc<V>, E> {
    let neighbours = neighbours();
    memo.get_or_insert_with(key, || build(neighbours))
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
pub struct Rails {
    pub id: u64,
    pub value: (RailMesh, ModelField),
    pub geometry: Option<Arc<viewer_world::map::railways::PreparedRailGeometry>>,
}

impl Rails {
    fn share(
        neighbours: Vec<Arc<Self>>,
        value: (RailMesh, ModelField),
        device: Option<&eframe::wgpu::Device>,
    ) -> io::Result<Arc<Self>> {
        if let Some(same) = neighbours.into_iter().find(|near| near.value == value) {
            return Ok(same);
        }
        let geometry =
            viewer_world::map::railways::PreparedRailGeometry::for_hour(device, &value.0)?;
        Ok(Arc::new(Self {
            id: next_id(),
            value,
            geometry,
        }))
    }
}

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

/// Elapsed time of one part, with its own thread's CPU time when profiled.
#[derive(Clone, Copy)]
pub struct PartTiming {
    pub ms: f64,
    pub cpu_ms: Option<f64>,
}

impl From<f64> for PartTiming {
    fn from(ms: f64) -> Self {
        Self { ms, cpu_ms: None }
    }
}

// Existing elapsed-only reports keep their numeric formatting and precision.
impl std::fmt::Display for PartTiming {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        std::fmt::Display::fmt(&self.ms, formatter)
    }
}

/// Per-step timing of the movers job, reported on its builder thread, not
/// mixed into a later window frame's thread-local spans. CPU time has the
/// platform thread clock's resolution; short steps may report zero.
struct MoversLaps {
    epoch: u64,
    branch: BranchId,
    hour: Hour,
    last: Option<Instant>,
    cpu: Option<Duration>,
    laps: Vec<(&'static str, PartTiming)>,
}

impl MoversLaps {
    fn new(epoch: u64, branch: BranchId, hour: Hour) -> Self {
        let enabled = viewer_foundation::profile_writer::profiling();
        Self {
            epoch,
            branch,
            hour,
            last: enabled.then(Instant::now),
            cpu: enabled
                .then(viewer_foundation::profiling::current_thread_cpu_time)
                .flatten(),
            laps: Vec::new(),
        }
    }

    /// Nested builder parts share this report without advancing its outer lap.
    /// Their timings are included in, not additive to, the named outer part.
    fn split<T>(
        &mut self,
        name: &'static str,
        build: impl FnOnce(&mut dyn FnMut(&'static str)) -> T,
    ) -> T {
        let mut last = self.last;
        let mut previous_cpu = self.cpu;
        let mut parts = Vec::new();
        let value = build(&mut |name| {
            let Some(before) = last else {
                return;
            };
            let now = Instant::now();
            let cpu = viewer_foundation::profiling::current_thread_cpu_time();
            parts.push((
                name,
                PartTiming {
                    ms: now.duration_since(before).as_secs_f64() * 1000.0,
                    cpu_ms: previous_cpu
                        .zip(cpu)
                        .and_then(|(before, now)| now.checked_sub(before))
                        .map(|used| used.as_secs_f64() * 1000.0),
                },
            ));
            last = Some(now);
            previous_cpu = cpu;
        });
        self.lap(name);
        self.laps.extend(parts);
        value
    }

    fn lap(&mut self, name: &'static str) {
        let Some(last) = self.last else {
            return;
        };
        let now = Instant::now();
        let cpu = viewer_foundation::profiling::current_thread_cpu_time();
        self.laps.push((
            name,
            PartTiming {
                ms: now.duration_since(last).as_secs_f64() * 1000.0,
                cpu_ms: self
                    .cpu
                    .zip(cpu)
                    .and_then(|(last, now)| now.checked_sub(last))
                    .map(|used| used.as_secs_f64() * 1000.0),
            },
        ));
        self.last = Some(now);
        self.cpu = cpu;
    }
}

impl Drop for MoversLaps {
    fn drop(&mut self) {
        if self.last.is_none() {
            return;
        }
        viewer_foundation::profile_writer::profile_print(format!(
            "PROFILE movers epoch {} branch {} hour {} parts {} cpu={}",
            self.epoch,
            self.branch,
            self.hour,
            self.laps
                .iter()
                .map(|(name, timing)| format!("{name}={:.3}", timing.ms))
                .collect::<Vec<_>>()
                .join(" "),
            self.laps
                .iter()
                .map(|(name, timing)| match timing.cpu_ms {
                    Some(ms) => format!("{name}:{ms:.3}"),
                    None => format!("{name}:unavailable"),
                })
                .collect::<Vec<_>>()
                .join(","),
        ));
    }
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
    /// Elapsed and optional thread CPU milliseconds of each part.
    pub laps: Vec<(&'static str, PartTiming)>,
    /// Completed job-local child scopes in fixed part order; nested scopes are inclusive.
    pub child_laps: Vec<BuilderSpan>,
    /// Wall milliseconds from dispatch.
    pub wall: f64,
    pub(crate) trace: trace::TargetTrace,
}

type PartWork = Box<dyn FnOnce(&HourView<'_>, &mut BuilderSpans) -> Part + Send>;

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
    buildings_state: Arc<Slot<BuildingsBase>>,
}

impl Slots {
    fn new() -> Self {
        Self {
            political: Slot::new(),
            borders: Slot::new(),
            rails: Slot::new(),
            buildings: Slot::new(),
            buildings_state: Slot::new(),
        }
    }

    // Reused hours submit no building job, so explicitly release successor waits.
    fn without_buildings_state(&self) {
        Fill(Arc::clone(&self.buildings_state)).set(None);
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
    rails: Recent<Arc<RailNetwork>, Rails>,
    /// Equal snow grids share an allocation even when the weather table changes identity.
    snow_shared: Settled<SnowMud>,
    /// The intel map, by the viewing country's rebuilt province bytes.
    intel: Recent<Option<BTreeMap<u32, u8>>, IntelMap>,
    music: crate::document::music_inputs::Memo,
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
            snow_shared: Settled::new(weather::TABLES, MEMO_ENTRIES),
            intel: Recent::new(MEMO_ENTRIES),
            music: crate::document::music_inputs::Memo::new(MEMO_ENTRIES),
        }
    }
}

enum Event {
    Shutdown,
    PoolSpace,
    ResultSpace,
    Frame { ticket: u64, key: (BranchId, Hour), reply: source_mailbox::Reply<Current, trace::DecodeSample> },
    Colour { ticket: u64, key: (BranchId, Hour), folded: Box<dispatch_plan::ColourFolded> },
    Want,
    Part {
        ticket: u64,
        epoch: u64,
        branch: BranchId,
        hour: Hour,
        name: &'static str,
        timing: PartTiming,
        children: Vec<BuilderSpan>,
        part: Box<Part>,
    },
}

/// One worker budget for every map in a native root. Per-view coordinators
/// retain independent cancellation and layer state, but do not multiply pools.
pub struct Resources {
    pool: Arc<Pool>,
    frame_workers: Workers,
    decoders: Mutex<BTreeMap<usize, Weak<SharedDecoder>>>,
    pub dropper: Arc<Dropper>,
}

impl Resources {
    pub fn new() -> io::Result<Arc<Self>> {
        let pool = Arc::new(Pool::new(builder_count())?);
        let frame_workers = viewer_foundation::frame_assist::workers(pool.clone());
        Ok(Arc::new(Self {
            pool,
            frame_workers,
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
        let decoder = Arc::new(SharedDecoder::new(replay)?);
        decoders.insert(key, Arc::downgrade(&decoder));
        Ok(decoder)
    }
}

/// An hour dispatched and not yet assembled.
struct Pending {
    ticket: u64,
    trace: trace::TargetTrace,
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
    laps: Vec<(&'static str, PartTiming)>,
    child_laps: Vec<(&'static str, Vec<BuilderSpan>)>,
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

/// Flatten completed child scopes in the same fixed part order as parent laps.
fn ordered_child_laps(mut parts: Vec<(&'static str, Vec<BuilderSpan>)>) -> Vec<BuilderSpan> {
    parts.sort_by_key(|(name, _)| lap_rank(name));
    parts
        .into_iter()
        .flat_map(|(_, children)| children)
        .collect()
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

    fn take(
        &mut self,
        ticket: u64,
        name: &'static str,
        timing: impl Into<PartTiming>,
        children: Vec<BuilderSpan>,
        part: Part,
    ) -> bool {
        // A seek may cancel and then request the same hour within one epoch.
        // Results from the cancelled dispatch must not fill its replacement.
        if ticket != self.ticket {
            return false;
        }
        self.received += 1;
        self.laps.push((name, timing.into()));
        if !children.is_empty() {
            self.child_laps.push((name, children));
        }
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
            self.trace.finish(name, "reused");
            self.take(
                self.ticket,
                name,
                PartTiming {
                    ms: 0.0,
                    cpu_ms: Some(0.0),
                },
                Vec::new(),
                part,
            );
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

fn true_table_bytes(table: &replay_frame::frame::SharedView) -> u64 {
    dashboard_base::prepared::prepass::HeldBytes::default()
        .view(table)
        .total()
}

#[derive(Default)]
struct FrameCache {
    frames: BTreeMap<(BranchId, Hour), Current>,
    tables: HashMap<usize, (usize, u64)>,
    bytes: u64,
    // Bytes newly held by the last insert, used to size the next decode's headroom.
    added_bytes: u64,
}

impl FrameCache {
    fn get(&self, key: &(BranchId, Hour)) -> Option<Current> {
        self.frames.get(key).cloned()
    }

    fn contains_key(&self, key: &(BranchId, Hour)) -> bool {
        self.frames.contains_key(key)
    }

    fn insert(&mut self, key: (BranchId, Hour), current: Current) {
        self.added_bytes = self.hold(&current);
        if let Some(old) = self.frames.insert(key, current) {
            self.release(&old);
        }
    }

    fn hold(&mut self, current: &Current) -> u64 {
        let Current::Frame(frame) = current else {
            return 0;
        };
        let mut added = 0u64;
        for table in &frame.tables {
            let entry = self
                .tables
                .entry(table.as_ptr() as usize)
                .or_insert_with(|| (0, true_table_bytes(table)));
            if entry.0 == 0 {
                self.bytes = self.bytes.saturating_add(entry.1);
                added = added.saturating_add(entry.1);
            }
            entry.0 += 1;
        }
        added
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

    fn retain_only(&mut self, required: &BTreeSet<(BranchId, Hour)>) {
        let gone: Vec<_> = self
            .frames
            .keys()
            .filter(|key| !required.contains(key))
            .copied()
            .collect();
        for key in gone {
            if let Some(current) = self.frames.remove(&key) {
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
/// dispatched. The coordinator posts requests to the lane mailbox and receives
/// readiness events without waiting on the lane.
/// The lane's [`Playback`] reads the next block ahead through its [`BlockReader`].
///
// The serial decode lane runs above the builders that wait for it.
struct DecodeLane {
    shared: Arc<LaneShared>,
}

struct LaneShared {
    mailbox: OnceLock<Arc<SourceMailbox>>,
    state: Mutex<LaneState>,
    changed: Condvar,
}

// Decode ahead only as far as recent demand and service time require.
fn window_len(take_rate: f64, service: f64, pool: usize, open: bool) -> usize {
    let pool = pool.max(1);
    if !open {
        return 1;
    }
    let ahead = (take_rate * service).ceil();
    let ahead = if ahead.is_finite() && ahead > 0.0 {
        ahead as usize
    } else {
        0
    };
    ahead.saturating_add(1).min(pool)
}

#[derive(Default)]
struct Throughput {
    takes: VecDeque<Instant>,
    services: VecDeque<f64>,
}

impl Throughput {
    fn took(&mut self, at: Instant, pool: usize) {
        self.takes.push_back(at);
        while self.takes.len() > pool.max(1) + 1 {
            self.takes.pop_front();
        }
    }

    fn served(&mut self, seconds: f64, pool: usize) {
        self.services.push_back(seconds);
        while self.services.len() > pool.max(1) {
            self.services.pop_front();
        }
    }

    fn take_rate(&self, now: Instant) -> f64 {
        if self.takes.len() < 2 {
            return 0.0;
        }
        let span = now.saturating_duration_since(self.takes[0]).as_secs_f64();
        if span > 0.0 {
            (self.takes.len() - 1) as f64 / span
        } else {
            0.0
        }
    }

    fn service_p90(&self) -> f64 {
        let mut services: Vec<f64> = self.services.iter().copied().collect();
        services.sort_by(f64::total_cmp);
        let at = (services.len() * 9 / 10).min(services.len().saturating_sub(1));
        services.get(at).copied().unwrap_or(0.0)
    }
}

fn memory_gate(available: Option<(u64, u64)>, newest: u64, low: bool) -> Option<&'static str> {
    if low {
        return Some("os");
    }
    let (commit, physical) = available?;
    if commit < newest {
        Some("commit")
    } else if physical < newest {
        Some("physical")
    } else {
        None
    }
}

/// Hours a long walk through a block applies between checks that its target
/// is still wanted, so a seek away abandons it within a few milliseconds.
const DECODE_STEP: i32 = 24;

/// Why a lane that stopped answers with a gap rather than a wait.
const LANE_STOPPED: &str = "the hour decoder stopped";

#[derive(Default)]
struct LaneState {
    wanted: Vec<(BranchId, Hour)>,
    behind: i32,
    served: Option<(BranchId, Hour)>,
    urgent: Option<(BranchId, Hour)>,
    frames: FrameCache,
    decode_profile: Option<BTreeMap<(BranchId, Hour), trace::DecodeSample>>,
    stop: bool,
    dead: bool,
    pool: usize,
    ahead: usize,
    throughput: Throughput,
    gate: Option<&'static str>,
    rewant: bool,
}

impl LaneState {
    fn window(&self) -> &[(BranchId, Hour)] {
        let served = self
            .served
            .and_then(|served| self.wanted.iter().position(|key| *key == served));
        let (start, ahead) = if self.gate.is_some() {
            (served.unwrap_or(0), self.ahead.max(1))
        } else {
            (0, self.ahead.max(1))
        };
        let end = (served.map_or(0, |at| at + 1) + ahead).min(self.wanted.len());
        &self.wanted[start.min(end)..end]
    }

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

    fn still_wanted(&self, target: (BranchId, Hour)) -> bool {
        !self.stop && (self.urgent == Some(target) || self.window().contains(&target))
    }

    fn set_want(&mut self, wanted: Vec<(BranchId, Hour)>, behind: i32, pool: usize) {
        self.wanted = wanted;
        self.behind = behind;
        self.pool = pool.max(1);
        self.rewant = true;
        self.resize(Instant::now());
    }

    fn resize(&mut self, now: Instant) {
        self.ahead = window_len(
            self.throughput.take_rate(now),
            self.throughput.service_p90(),
            self.pool,
            self.gate.is_none(),
        );
    }

    fn check_memory(&mut self, now: Instant) {
        self.gate = memory_gate(
            memory_pressure::available(),
            self.frames.added_bytes,
            memory_pressure::system_low(),
        );
        self.resize(now);
    }

    fn evict(&mut self, pending: Option<(BranchId, Hour)>) {
        let mut required: BTreeSet<_> = self.window().iter().copied().chain(self.urgent).collect();
        required.extend(self.served);
        required.extend(pending);
        self.frames.retain_only(&required);
        self.rewant = false;
        if let Some(profile) = &mut self.decode_profile {
            profile.retain(|key, _| self.frames.contains_key(key));
        }
    }
}

/// Marks the lane dead when its thread ends, a panic included, so a waiting
/// coordinator gets a gap instead of waiting for ever.
struct LaneExit<'a>(&'a LaneShared);

impl Drop for LaneExit<'_> {
    fn drop(&mut self) {
        DecodeLane::lock(self.0).dead = true;
        self.0.changed.notify_all();
        if let Some(mailbox) = self.0.mailbox.get() { mailbox.stop(); }
    }
}

impl DecodeLane {
    fn start(playback: Playback) -> io::Result<Self> {
        let shared = Arc::new(LaneShared {
            mailbox: OnceLock::new(),
            state: Mutex::new(LaneState::default()),
            changed: Condvar::new(),
        });
        let weak = Arc::downgrade(&shared);
        let mailbox = SourceMailbox::new(move |_| {
            let Some(shared) = weak.upgrade() else {
                return false;
            };
            // Same lock as the empty-check/Condvar sleep: no missed request wake.
            let state = Self::lock(&shared);
            if state.stop || state.dead {
                return false;
            }
            shared.changed.notify_all();
            true
        });
        let _ = shared.mailbox.set(mailbox);
        let lane = Arc::clone(&shared);
        thread::Builder::new()
            .name("hour decoder".into())
            .spawn(move || {
                let _exit = LaneExit(&lane);
                viewer_foundation::worker_priority::normal_priority();
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

    fn client(&self) -> Arc<SourceMailbox> {
        self.shared.mailbox.get().expect("decode mailbox").clone()
    }

    fn run(shared: &LaneShared, mut playback: Playback) {
        'next: loop {
            let (target, behind, read) = {
                let mut state = Self::lock(shared);
                loop {
                    if state.stop {
                        return;
                    }
                    if state.rewant {
                        state.evict(Self::pending(shared));
                    }
                    if let Some(read) = shared.mailbox.get().and_then(|mailbox| mailbox.take()) {
                        let target = read.key;
                        let now = Instant::now();
                        let pool = state.pool;
                        state.throughput.took(now, pool);
                        state.resize(now);
                        if let Some(current) = state.frames.get(&target) {
                            state.served = Some(target);
                            state.evict(Self::pending(shared));
                            let sample = state
                                .decode_profile
                                .as_ref()
                                .and_then(|p| p.get(&target))
                                .copied();
                            drop(state);
                            read.ready(current, sample);
                            shared.changed.notify_all();
                            continue 'next;
                        }
                        state.urgent = Some(target);
                        break (target, state.behind, Some(read));
                    }
                    if let Some(target) = state.next() {
                        break (target, state.behind, None);
                    }
                    state = shared
                        .changed
                        .wait(state)
                        .unwrap_or_else(|p| p.into_inner());
                }
            };
            let (branch, hour) = target;
            let decode_clock =
                trace::DecodeClock::start(viewer_foundation::profile_writer::profiling());
            let started = Instant::now();
            let walked = if let Some(read) = &read {
                Self::walk_for(shared, &mut playback, target, Some(read))
            } else {
                Self::walk(shared, &mut playback, target)
            };
            if !walked {
                let mut state = Self::lock(shared);
                if read.is_some() && state.urgent == Some(target) {
                    state.urgent = None;
                }
                state.evict(Self::pending(shared));
                drop(state);
                drop(read); // terminal cancellation acknowledgement retains the credit until here
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
            let mut sample = decode_clock.finish(branch, hour);
            let mut state = Self::lock(shared);
            let mut profile_keys = sample.map(|_| Vec::new());
            for (at, frame) in passed {
                if !state.frames.contains_key(&(branch, at)) {
                    state.frames.insert((branch, at), frame);
                    if let Some(keys) = &mut profile_keys {
                        keys.push((branch, at));
                    }
                }
            }
            state.frames.insert(target, current.clone());
            if let Some(sample) = &mut sample {
                sample.ready = trace::now(true).expect("enabled");
                let profile = state.decode_profile.get_or_insert_with(BTreeMap::new);
                for key in profile_keys.into_iter().flatten() {
                    profile.insert(key, *sample);
                }
                profile.insert(target, *sample);
            }
            if read.is_some() {
                state.served = Some(target);
                if state.urgent == Some(target) {
                    state.urgent = None;
                }
            }
            let now = Instant::now();
            let pool = state.pool;
            state
                .throughput
                .served(now.duration_since(started).as_secs_f64(), pool);
            state.check_memory(now);
            state.evict(Self::pending(shared));
            drop(state);
            if let Some(read) = read {
                read.ready(current, sample);
            }
            shared.changed.notify_all();
        }
    }

    /// Walks `playback` towards `target` in [`DECODE_STEP`]-hour steps, each
    /// ending with a check that the target is still wanted. False when the
    /// walk was abandoned; the last step is left to the caller.
    fn walk(shared: &LaneShared, playback: &mut Playback, target: (BranchId, Hour)) -> bool {
        Self::walk_for(shared, playback, target, None)
    }

    fn walk_for(
        shared: &LaneShared,
        playback: &mut Playback,
        target: (BranchId, Hour),
        read: Option<&SourceRead>,
    ) -> bool {
        if read.is_some_and(|read| read.cancelled()) {
            return false;
        }
        let (branch, hour) = target;
        let wanted = || {
            #[cfg(test)]
            tests::before_walk_wanted();
            !read.is_some_and(|read| read.cancelled()) && Self::lock(shared).still_wanted(target)
        };
        if (playback.branch(), playback.hour()) == (branch, hour) {
            Self::note_page_route(target, playback, "held");
            return true;
        }
        let behind = Self::lock(shared).behind;
        let visit_from = hour.saturating_sub(behind);
        let limit = if visit_from >= hour {
            hour
        } else {
            visit_from.saturating_sub(1)
        };
        let mut paged = match playback.advance_pages(branch, hour, visit_from, &wanted) {
            std::ops::ControlFlow::Continue(reached) => reached,
            std::ops::ControlFlow::Break(()) => return false,
        };
        Self::note_page_route(
            target,
            playback,
            if paged.is_some() { "pages" } else { "native" },
        );
        if !wanted() {
            return false;
        }
        let mut from = match paged {
            Some(reached) if reached == hour => {
                return true;
            }
            _ => playback.walk_start(branch, hour),
        };
        if paged.is_none()
            && let Some(start) = from.and_then(|at| Hour::try_from(at).ok())
            && let Some(entry) = playback.page_entry(branch, start, limit)
            && entry > start
        {
            while let Some(at) = from.filter(|at| *at < i64::from(entry)) {
                let step = (at + i64::from(DECODE_STEP)).min(i64::from(entry));
                let Ok(step) = Hour::try_from(step) else {
                    break;
                };
                if playback.advance_without_output(branch, step).is_err() {
                    break;
                }
                from = Some(i64::from(step));
                if !wanted() {
                    return false;
                }
            }
            paged = match playback.advance_pages(branch, hour, visit_from, &wanted) {
                std::ops::ControlFlow::Continue(reached) => reached,
                std::ops::ControlFlow::Break(()) => return false,
            };
            Self::note_page_route(
                target,
                playback,
                if paged.is_some() {
                    "native+pages"
                } else {
                    "native"
                },
            );
            if !wanted() {
                return false;
            }
            match paged {
                Some(reached) if reached == hour => {
                    return true;
                }
                Some(reached) => from = Some(i64::from(reached)),
                None => from = playback.walk_start(branch, hour),
            }
        }
        // Native steps for the rest; the last is the caller's.
        while let Some(at) = from.filter(|at| i64::from(hour) - at > i64::from(DECODE_STEP)) {
            let Ok(step) = Hour::try_from(at + i64::from(DECODE_STEP)) else {
                break;
            };
            if playback.advance_without_output(branch, step).is_err() {
                break;
            }
            from = Some(i64::from(step));
            if read.is_some_and(|read| read.cancelled()) || !Self::lock(shared).still_wanted(target)
            {
                return false;
            }
        }
        true
    }

    fn note_page_route((branch, hour): (BranchId, Hour), playback: &Playback, route: &str) {
        if !page_grid() || !viewer_foundation::profile_writer::profiling() {
            return;
        }
        let (skip, held) = playback.page_skip();
        let held = held.map_or_else(|| "none".to_owned(), |held| held.to_string());
        viewer_foundation::profile_writer::profile_print(format!(
            "PROFILE grid lane target branch {branch} hour {hour} route {route} page_skip {} held {held} page_fallbacks {}",
            skip.unwrap_or("none"),
            playback.page_fallbacks(),
        ));
    }

    #[cfg(test)]
    fn want(&self, wanted: Vec<(BranchId, Hour)>, behind: i32) {
        let pool = wanted.len();
        self.want_sized(wanted, behind, pool);
    }

    fn want_sized(&self, wanted: Vec<(BranchId, Hour)>, behind: i32, pool: usize) {
        let mut state = Self::lock(&self.shared);
        state.set_want(wanted, behind, pool);
        drop(state);
        self.shared.changed.notify_all();
    }

    fn pending(shared: &LaneShared) -> Option<(BranchId, Hour)> {
        shared
            .mailbox
            .get()
            .and_then(|mailbox| mailbox.pending_key())
    }
}

impl Drop for DecodeLane {
    fn drop(&mut self) {
        Self::lock(&self.shared).stop = true;
        self.shared.changed.notify_all();
    }
}

type SourceMailbox = source_mailbox::Mailbox<(BranchId, Hour), Current, trace::DecodeSample>;
type SourceRead = source_mailbox::Read<(BranchId, Hour), Current, trace::DecodeSample>;
type SourceHandle = source_mailbox::ReadHandle<(BranchId, Hour), Current, trace::DecodeSample>;

type Requested = (u64, Want, Option<trace::Stamp>);

struct Coordinator {
    content: Arc<Content>,
    state: CoordinatorState,
}

// Content-independent transport state, moved unchanged so the real Driver can
// be exercised with controlled source replies and synthetic builder products.
struct CoordinatorState {
    device: Arc<OnceLock<eframe::wgpu::Device>>,
    playback: Playback,
    shared_decoder: Option<Arc<SharedDecoder>>,
    /// The playback decode lane, unless a shared decoder serves this view.
    lane: Option<DecodeLane>,
    requested: Arc<Mutex<Option<Requested>>>,
    profile: trace::CoordinatorProfile,
    profile_want: Option<trace::Stamp>,
    profile_received: Option<trace::Stamp>,
    /// The window's engaged-flag index, which the overlay part reads when it
    /// is for the wanted perspective's country; it never starts one.
    engaged: Engaged,
    want: Option<Want>,
    epoch: u64,
    /// Hours of the current epoch sent to the window, with the order they were
    /// sent in, until the window says it no longer holds them.
    sent: BTreeMap<(BranchId, Hour), u64>,
    /// How many built hours have been sent, of any epoch.
    sends: u64,
    usable_sent: BTreeSet<(BranchId, Hour)>,
    anchor: Option<Obligation>,
    /// The shared parts of the hours last dispatched, which the neighbouring
    /// hours' parts compare with.
    slots: BTreeMap<(BranchId, Hour), Slots>,
    memos: Arc<Memos>,
    reusable: BTreeMap<(BranchId, Hour), Arc<Reusable>>,
    pool: Arc<Pool>,
    parallel: usize,
    events: Sender<Event>,
    outbox: mpsc::SyncSender<Built>,
    result_space: Arc<AtomicBool>,
}

impl std::ops::Deref for Coordinator {
    type Target = CoordinatorState;
    fn deref(&self) -> &Self::Target { &self.state }
}
impl std::ops::DerefMut for Coordinator {
    fn deref_mut(&mut self) -> &mut Self::Target { &mut self.state }
}

impl CoordinatorState {
    /// Starts a new epoch; mode-independent results may still serve its hours.
    fn invalidate(&mut self) {
        self.epoch += 1;
        self.sent.clear();
        self.usable_sent.clear();
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
        self.reduce_anchor(&want);
        let range = self
            .playback
            .range(want.branch)
            .unwrap_or((want.hour, want.hour));
        self.reusable.retain(|key, _| want.keeps_reusable(*key));
        // Keep an hour the window holds, or one still on its way to it.
        self.sent.retain(|key, order| want.keeps_sent(*key, *order));
        self.usable_sent.retain(|key| self.sent.contains_key(key));
        let predecessor = self
            .slots
            .range((want.branch, i32::MIN)..(want.branch, want.hour))
            .next_back()
            .map(|(key, _)| *key);
        self.slots
            .retain(|key, _| want.keeps_reusable(*key) || Some(*key) == predecessor);
        if let Some(lane) = &self.lane {
            lane.want_sized(
                self.wanted_hours(&want, range)
                    .into_iter()
                    .map(|hour| (want.branch, hour))
                    .collect(),
                if want.playing && !sought {
                    0
                } else {
                    want.behind.max(2)
                },
                want.grid
                    .as_ref()
                    .map_or(self.parallel, |grid| grid.cadence.count as usize),
            );
        }
        self.want = Some(want);
    }

    fn wanted_hours(&self, want: &Want, range: (Hour, Hour)) -> Vec<Hour> {
        let Some(grid) = &want.grid else {
            return want.hours(range);
        };
        let anchor = self
            .anchor
            .filter(|anchor| {
                !anchor.done && anchor.epoch == self.epoch && anchor.key.0 == want.branch
            })
            .map(|anchor| anchor.key.1);
        let (stride, count) = (grid.cadence.stride, grid.cadence.count);
        cadence::targets(stride, count, want.hour, anchor, range)
    }

    fn note_sent(&mut self, epoch: u64, key: (BranchId, Hour), order: u64, usable: bool) {
        if usable {
            self.usable_sent.insert(key);
        } else {
            self.usable_sent.remove(&key);
        }
        if let Some(anchor) = &mut self.anchor
            && !anchor.done
            && (anchor.epoch, anchor.key) == (epoch, key)
            && anchor.sent.is_none()
        {
            anchor.sent = Some(order);
        }
    }

    fn reduce_anchor(&mut self, want: &Want) {
        let Some(grid) = &want.grid else {
            return;
        };
        let cadence = &grid.cadence;
        if self
            .anchor
            .is_none_or(|anchor| anchor.sequence != cadence.sequence)
        {
            self.anchor = cadence.anchor.map(|hour| Obligation {
                sequence: cadence.sequence,
                epoch: self.epoch,
                key: (want.branch, hour),
                sent: None,
                done: false,
            });
        }
        let epoch = self.epoch;
        if let Some(anchor) = &mut self.anchor {
            let (branch, hour) = anchor.key;
            anchor.done |= anchor.epoch != epoch
                || branch != want.branch
                || anchor.sent.is_some_and(|order| order < want.received)
                || want.have.contains(&hour)
                || grid.installed.is_some_and(|installed| installed >= hour);
        }
    }
}

impl Coordinator {
    fn run(self, inbox: Receiver<Event>) {
        coordinator::run(self, inbox);
    }
}

impl coordinator::BuildTarget for Coordinator {
    fn colour_work(&self, plan: &mut dispatch_plan::DispatchPlan, current: Current) -> Box<dyn FnOnce() + Send> {
        let content = self.content.clone();
        let mode = plan.want.map_mode.clone();
        let perspective = plan.want.perspective.clone();
        plan.colour_work(current, self.events.clone(), move |frame, colour| {
            Ok(HourView::new(&content, frame, 0.0).map_mode_fill(&mode, &perspective, colour)?.colour)
        })
    }

    /// Constructs the target's jobs. No decode, admission or slot wait here.
    fn prepare(&mut self, plan: dispatch_plan::DispatchPlan, current: Current) -> dispatch_plan::PreparedTarget {
        use dispatch_plan::{DispatchPlan, HeldJob, PreparedTarget};
        let DispatchPlan { ticket, want, epoch, hour, started, trace, previous,
            neighbours, colour, history_timing, .. } = plan;
        let branch = want.branch;
        let frame = match &current {
            Current::Frame(frame) => Arc::clone(frame),
            Current::Gap { reason, .. } => {
                let reason = reason.clone();
                trace.assembled();
                return PreparedTarget::Ready(Box::new(Built {
                    epoch,
                    branch,
                    hour,
                    view: want.view,
                    current,
                    layers: Err(reason),
                    laps: Vec::new(),
                    child_laps: Vec::new(),
                    wall: 0.0,
                    trace,
                }));
            }
        };
        let decode_ms = started.elapsed().as_secs_f64() * 1000.0;
        let alive = Arc::new(AtomicBool::new(true));
        let reusable = self.reusable.get(&(branch, hour)).cloned();
        let mut pending = Pending {
            ticket,
            trace: trace.clone(),
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
            laps: vec![("decode", decode_ms.into())],
            child_laps: Vec::new(),
            view: want.view,
        };
        if let Some(timing) = history_timing {
            pending.laps.push(("political_history", timing));
        }
        if let Some(parts) = &reusable {
            pending.reuse(parts);
        }
        let slots = Slots::new();
        if let Some(parts) = &reusable {
            slots.without_buildings_state();
            Fill(Arc::clone(&slots.rails)).set(Some(Arc::clone(&parts.rails)));
            Fill(Arc::clone(&slots.buildings)).set(Some(Arc::clone(&parts.buildings.0)));
        }
        // A part waits only for the neighbours' slots present now. Each of
        // those belongs to an hour dispatched before this one, whose jobs the
        // pool therefore took first; a cancelled or panicked part still fills
        // its slots, so no wait is for ever.
        self.slots.insert((branch, hour), slots.clone());
        // Only overlay and movers share these immutable inputs within this ticket.
        let worker_etas = Arc::new(viewer_foundation::movement_timing::WorkerEtas::default());
        let mut jobs = VecDeque::new();
        let mut submit = |name: &'static str, work: PartWork| {
            let content = Arc::clone(&self.content);
            let frame = Arc::clone(&frame);
            let alive = Arc::clone(&alive);
            let events = self.events.clone();
            trace.enqueue(name);
            let job_trace = trace.clone();
            let worker_etas =
                matches!(name, "overlay" | "movers").then(|| Arc::clone(&worker_etas));
            jobs.push_back(HeldJob { name, work: Box::new(move || {
                job_trace.start(name);
                if !alive.load(Ordering::Relaxed) {
                    // Dropping `work` fills its slots with nothing.
                    job_trace.finish(name, "cancelled");
                    return;
                }
                let started = Instant::now();
                let view = HourView::new(&content, &frame, 0.0).with_worker_etas(worker_etas);
                let profiling = viewer_foundation::profile_writer::profiling();
                let mut children = BuilderSpans::new(profiling);
                let cpu_started = profiling
                    .then(viewer_foundation::profiling::current_thread_cpu_time)
                    .flatten();
                // A panicking builder must not take its pool thread with it:
                // enough dead threads would stall every later hour with the
                // window waiting at the hour boundary. The panic becomes the
                // hour's error instead, which the window shows.
                let part = match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    work(&view, &mut children)
                })) {
                    Ok(part) => part,
                    Err(_) => Part::failed(name),
                };
                let cpu_ms = cpu_started.and_then(|start| {
                    viewer_foundation::profiling::current_thread_cpu_time()?
                        .checked_sub(start)
                        .map(|used| used.as_secs_f64() * 1000.0)
                });
                job_trace.finish(name, "done");
                let _ = events.send(Event::Part {
                    ticket,
                    epoch,
                    branch,
                    hour,
                    name,
                    timing: PartTiming {
                        ms: started.elapsed().as_secs_f64() * 1000.0,
                        cpu_ms,
                    },
                    children: children.into_records(),
                    part: Box::new(part),
                });
            }) });
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
            let previous = previous
                .as_ref()
                .map(|(_, slots)| Arc::clone(&slots.political));
            submit(
                "political",
                Box::new(move |view, _spans| {
                    Part::Political((|| {
                        let naval = view.naval_fill(&political_perspective)?;
                        let colour = match colour {
                            Some(colour) => colour?,
                            None => previous.and_then(|slot| slot.wait())
                                .and_then(|political| political.key.mode().and_then(|mode| mode.colour.clone())),
                        };
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
            Box::new(move |view, _spans| {
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
        let temperature_frame = Arc::clone(&frame);
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
            Box::new(move |view, spans| {
                spans.measure("overlay.temperature", |_| {
                    view.counter_content
                        .province_hover
                        .prepare_temperature(&temperature_frame);
                });
                Part::Overlay((|| {
                    // Raid icons come from the window's raid lane
                    // ([`crate::raid_lane`]), which chains hours.
                    let tables = spans.measure("overlay.tables", |spans| {
                        view.hour_tables_with_raids_profiled(
                            &overlay_perspective,
                            Arc::default(),
                            overlay_engaged.as_deref(),
                            spans,
                        )
                    })?;
                    spans.measure("overlay.flags_warm", |_| {
                        tables.warm_counter_flags(&view.icons.counters)
                    });
                    let overlay = spans.measure("overlay.projection", |spans| {
                        view.prepare_overlay_profiled(
                            camera,
                            viewport,
                            ui_scale,
                            &tables,
                            &overlay_counters,
                            spans,
                        )
                    })?;
                    Ok(Box::new((overlay, tables)))
                })())
            }),
        );
        if reusable.is_some() {
            return PreparedTarget::Parts(Box::new(pending), jobs);
        }
        {
            let fill = Fill(Arc::clone(&slots.rails));
            let near = slots_of(&neighbours, |slots| &slots.rails);
            let memos = Arc::clone(&self.memos);
            let device = Arc::clone(&self.device);
            submit(
                "rails",
                Box::new(move |view, _spans| {
                    Part::Rails(RailNetwork::read(view.frame).and_then(|network| {
                        let network = Arc::new(network);
                        let rails = memo_after_neighbours(
                            &memos.rails,
                            Arc::clone(&network),
                            || wait_all(near),
                            |near| {
                                let mesh = view.rail_mesh_of(&network)?;
                                let bridges = view.bridges.field(&mesh);
                                Rails::share(near, (mesh, bridges), device.get())
                            },
                        )?;
                        fill.set(Some(Arc::clone(&rails)));
                        Ok(rails)
                    }))
                }),
            );
        }
        {
            let fill = Fill(Arc::clone(&slots.buildings));
            let fill_state = Fill(Arc::clone(&slots.buildings_state));
            let near = slots_of(&neighbours, |slots| &slots.buildings);
            // The predecessor was dispatched first, so this wait cannot form a cycle.
            let base = previous
                .as_ref()
                .map(|(_, slots)| Arc::clone(&slots.buildings_state));
            let building_perspective = want.perspective.clone();
            submit(
                "buildings",
                Box::new(move |view, spans| {
                    let base =
                        spans.measure("buildings.base_wait", |_| base.and_then(|slot| slot.wait()));
                    let field = view
                        .map_building_field_patched(&building_perspective, base.as_deref(), spans)
                        .map(|(field, summary, state)| {
                            fill_state.set(Some(Arc::new(state)));
                            (field, summary)
                        });
                    Part::Buildings(field.map(|(field, summary)| {
                        let near = spans.measure("buildings.share_wait", |_| wait_all(near));
                        let buildings =
                            spans.measure("buildings.share", |_| Shared::share(near, field));
                        fill.set(Some(Arc::clone(&buildings)));
                        let warnings = spans.measure("buildings.warnings", |_| summary.warnings());
                        (buildings, warnings)
                    }))
                }),
            );
        }
        let movers_perspective = want.perspective.clone();
        let show_allied_plans = want.show_allied_plans;
        submit(
            "movers",
            Box::new(move |view, spans| {
                Part::Movers((|| {
                    let mut laps = MoversLaps::new(epoch, branch, hour);
                    // The raid arrows' units and the trade-route convoys join the unit models.
                    let (orders, convoys) =
                        view.orders_and_convoys(&movers_perspective, show_allied_plans)?;
                    laps.lap("orders_and_convoys");
                    let mut units = laps.split("prepare_unit_models", |lap| {
                        view.prepare_unit_models_with_spans(&movers_perspective, lap, spans)
                    })?;
                    units.add_raid_units(&view.unit_models, &orders.raid_units);
                    laps.lap("add_raid_units");
                    if let Ok(defines) = &view.trade_routes {
                        units.add_convoys(convoys, defines.convoys);
                    }
                    laps.lap("add_convoys");
                    units.stage_field(&view.unit_models);
                    laps.lap("stage_field");
                    let trains = view.train_plan(&movers_perspective)?;
                    laps.lap("train_plan");
                    let aircraft = view.aircraft_plan(&movers_perspective)?;
                    laps.lap("aircraft_plan");
                    let units = Arc::new(units);
                    let effects = weather_effects::prepare(
                        &view.weather_effects,
                        weather_effects::field(view.frame)?,
                        camera,
                        viewport,
                    );
                    laps.lap("weather_effects_prepare");
                    let impacts = Arc::new(view.nuclear_impacts()?);
                    laps.lap("nuclear_impacts");
                    Ok(Arc::new(Movers {
                        trains,
                        aircraft,
                        units,
                        orders,
                        effects,
                        impacts,
                    }))
                })())
            }),
        );
        let fog_perspective = want.perspective.clone();
        let memos = Arc::clone(&self.memos);
        submit(
            "fog",
            Box::new(move |view, spans| {
                Part::Fog((|| {
                    let vision = spans.measure("fog.vision", |_| view.vision(&fog_perspective))?;
                    let fog = vision.province_fog();
                    let key = spans.measure("fog.intel_key", |_| fog.cloned());
                    let intel = spans.measure("fog.intel_lookup", |spans| {
                        memos.intel.get_or_insert_with(key, || {
                            spans.measure("fog.intel_build", |_| {
                                view.intel_map_from_visibility(fog).map(Arc::new)
                            })
                        })
                    })?;
                    let limitations = spans.measure("fog.limitations", |_| {
                        view.limitations_with(&fog_perspective, Some(&intel))
                    })?;
                    let music = spans.measure("fog.music", |_| {
                        view.music_chances_cached(vision.ours(), &memos.music)
                            .map(Arc::new)
                    });
                    let battles =
                        spans.measure("fog.battles", |_| view.battle_sounds().map(Arc::new));
                    Ok(Arc::new(FogPart {
                        intel,
                        limitations,
                        header: spans.measure("fog.header", |_| {
                            Arc::new(Header {
                                viewing: view
                                    .vision(&fog_perspective)
                                    .and_then(|vision| view.viewing_tag(&vision))
                                    .ok(),
                            })
                        }),
                        map_text: spans
                            .measure("fog.map_text", |_| Arc::new(view.map_text(&vision))),
                        music,
                        battles,
                    }))
                })())
            }),
        );
        let memos = Arc::clone(&self.memos);
        let snow_replay = self.playback.replay_handle();
        submit(
            "snow",
            Box::new(move |view, _spans| {
                let live = || {
                    view.snow_cells().and_then(|cells| {
                        memos.snow_shared.get_or_build(view.frame, |projected| {
                            map_snow::build(cells, projected, &view.assets.post.volumes)
                        })
                    })
                };
                Part::Snow(match view.snow_export.reader(&snow_replay) {
                    Ok(exports) => {
                        exports.build_or_restore(view.frame.branch, view.frame.hour, live)
                    }
                    Err(reason) => {
                        println!(
                            "PROFILE map_export_snow branch={} hour={} result=miss restore_us=0 reason={reason:?}",
                            view.frame.branch, view.frame.hour
                        );
                        live()
                    }
                })
            }),
        );
        PreparedTarget::Parts(Box::new(pending), jobs)
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
    device: Arc<OnceLock<eframe::wgpu::Device>>,
    content: Arc<Content>,
    replay: Arc<Replay>,
    events: Requests,
    results: Receiver<Built>,
    result_space: Arc<AtomicBool>,
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
    frame_workers: Workers,
    pool: Arc<Pool>,
    grid_sequence: u64,
    grid_decision: Option<Result<u32, &'static str>>,
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
        let pool = Arc::new(Pool::new(parallel)?);
        let frame_workers = viewer_foundation::frame_assist::workers(pool.clone());
        Self::start(
            content,
            replay,
            pool,
            frame_workers,
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
            resources.frame_workers.clone(),
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
            resources.frame_workers.clone(),
            Some(decoder),
            1,
            None,
        )
    }

    fn start(
        content: Arc<Content>,
        replay: Arc<Replay>,
        pool: Arc<Pool>,
        frame_workers: Workers,
        shared_decoder: Option<Arc<SharedDecoder>>,
        parallel: usize,
        checkpoints: Option<Arc<crate::map_checkpoints::CheckpointSource>>,
    ) -> io::Result<Self> {
        let (events, inbox) = mpsc::channel();
        // Keep the original result-channel width. Complete unsent targets retain
        // live credits, so a window that stops taking them bounds work rather
        // than parking the event-driven coordinator on a channel send.
        let (outbox, results) = mpsc::sync_channel(parallel);
        let requested = Arc::new(Mutex::new(None));
        let result_space = Arc::new(AtomicBool::new(false));
        let workers = Workers::new(Arc::clone(&pool) as _);
        let lane = match shared_decoder {
            Some(_) => None,
            None => {
                let reader = BlockReader::new(Arc::clone(&replay), Arc::new(Dropper::new()?));
                let page_workers = viewer_foundation::parallel::usable_processors()
                    .saturating_sub(viewer_foundation::parallel::Spawn::threads(&*pool))
                    .max(1);
                let mut playback = Playback::unloaded(Arc::clone(&replay))?
                    .with_reader(reader)
                    .with_page_workers(page_workers);
                if let Some(checkpoints) = checkpoints {
                    playback = playback.with_checkpoint_source(checkpoints);
                }
                Some(DecodeLane::start(playback)?)
            }
        };
        let engaged: Engaged = Arc::new(Mutex::new(None));
        let device = Arc::new(OnceLock::new());
        let coordinator = Coordinator {
            content: Arc::clone(&content),
            state: CoordinatorState {
            device: Arc::clone(&device),
            playback: Playback::unloaded(Arc::clone(&replay))?,
            shared_decoder,
            lane,
            requested: Arc::clone(&requested),
            profile: trace::CoordinatorProfile::new(),
            profile_want: None,
            profile_received: None,
            engaged: Arc::clone(&engaged),
            want: None,
            epoch: 0,
            sent: BTreeMap::new(),
            sends: 0,
            usable_sent: BTreeSet::new(),
            anchor: None,
            slots: BTreeMap::new(),
            memos: Arc::new(Memos::new()),
            reusable: BTreeMap::new(),
            pool: Arc::clone(&pool),
            parallel,
            events: events.clone(),
            outbox,
            result_space: result_space.clone(),
            },
        };
        thread::Builder::new()
            .name("hour coordinator".into())
            .spawn(move || {
                background_priority();
                coordinator.run(inbox)
            })?;
        Ok(Self {
            device,
            content,
            replay,
            events: Requests(events),
            results,
            result_space,
            want: None,
            epoch: 0,
            received: Cell::new(0),
            seek_pending: false,
            requested,
            engaged,
            workers,
            frame_workers,
            pool,
            grid_sequence: 0,
            grid_decision: None,
        })
    }

    /// Supplies this view's device before its first request. A pipeline retains
    /// one device, so immutable shared rail geometry never crosses devices.
    pub fn set_device(&self, device: &eframe::wgpu::Device) {
        self.device.get_or_init(|| device.clone());
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

    /// Synchronous frame turns: protected helpers, then borrowed builders.
    pub fn frame_workers(&self) -> Workers {
        self.frame_workers.clone()
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
        self.grid_intent(&mut want);
        if self.want.as_ref() != Some(&want) || invalidates {
            if invalidates {
                self.epoch += 1;
            }
            let mut requested = self.requested.lock().map_err(|_| stopped())?;
            let wake = requested.is_none();
            *requested = Some((
                self.epoch,
                want.clone(),
                trace::now(viewer_foundation::profile_writer::profiling()),
            ));
            drop(requested);
            if wake {
                self.events.0.send(Event::Want).map_err(|_| stopped())?;
            }
            self.want = Some(want);
            self.seek_pending = false;
        }
        Ok(self.epoch)
    }

    fn grid_intent(&mut self, want: &mut Want) {
        let previous = self.want.as_ref().and_then(|previous| {
            previous
                .grid
                .as_ref()
                .map(|grid| (previous.branch, grid.cadence.clone()))
        });
        let (branch, hour) = (want.branch, want.hour);
        let Some(grid) = &mut want.grid else {
            return;
        };
        let reason = new_intent(
            previous
                .as_ref()
                .map(|(branch, cadence)| (*branch, cadence.stride)),
            (branch, grid.cadence.stride),
            self.seek_pending,
        );
        match (reason, previous) {
            (None, Some((_, last))) => {
                grid.cadence.sequence = last.sequence;
                grid.cadence.anchor = last.anchor;
            }
            (reason, _) => {
                self.grid_sequence += 1;
                grid.cadence.sequence = self.grid_sequence;
                grid.cadence.anchor = Some(hour);
                if viewer_foundation::profile_writer::profiling() {
                    viewer_foundation::profile_writer::profile_print(format!(
                        "PROFILE grid intent sequence {} reason {} branch {branch} hour {hour} stride {} count {}",
                        self.grid_sequence,
                        reason.unwrap_or("start"),
                        grid.cadence.stride,
                        grid.cadence.count,
                    ));
                }
            }
        }
    }

    pub fn note_grid(&mut self, decision: Result<u32, &'static str>, hour: Hour) {
        if !page_grid() || self.grid_decision == Some(decision) {
            return;
        }
        self.grid_decision = Some(decision);
        if viewer_foundation::profile_writer::profiling() {
            viewer_foundation::profile_writer::profile_print(match decision {
                Ok(stride) => format!("PROFILE grid want hour {hour} active stride {stride}"),
                Err(reason) => format!("PROFILE grid want hour {hour} inactive reason {reason}"),
            });
        }
    }

    /// Every hour finished since the last call, without waiting.
    pub fn take(&self) -> io::Result<Vec<Built>> {
        let built = poll_results(|| {
            let hour = self.results.try_recv()?;
            hour.trace.delivered();
            self.received.set(self.received.get() + 1);
            Ok(hour)
        })?;
        if !built.is_empty() {
            self.results_freed();
        }
        Ok(built)
    }

    /// The next hour to finish, waiting for it.
    pub fn wait(&self) -> io::Result<Built> {
        let built = self.results.recv().map_err(|_| stopped())?;
        built.trace.delivered();
        self.received.set(self.received.get() + 1);
        self.results_freed();
        Ok(built)
    }

    fn results_freed(&self) {
        if !self.result_space.swap(true, Ordering::AcqRel) {
            let _ = self.events.0.send(Event::ResultSpace);
        }
    }
}

pub(crate) fn poll_results<T>(
    mut take: impl FnMut() -> Result<T, mpsc::TryRecvError>,
) -> io::Result<Vec<T>> {
    let mut built = Vec::new();
    loop {
        match take() {
            Ok(hour) => built.push(hour),
            Err(mpsc::TryRecvError::Empty) => return Ok(built),
            Err(mpsc::TryRecvError::Disconnected) => return Err(stopped()),
        }
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
mod grid_tests;

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
            grid: None,
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
            trace: Default::default(),
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
            child_laps: Vec::new(),
            view: want().view,
        };
        let rails = Arc::new(Rails {
            id: next_id(),
            value: Default::default(),
            geometry: None,
        });
        assert!(!pending.take(
            7,
            "rails",
            0.0,
            Vec::new(),
            Part::Rails(Ok(Arc::clone(&rails)))
        ));
        for name in ["buildings", "movers", "fog", "snow"] {
            assert!(!pending.take(7, name, 0.0, Vec::new(), Part::failed(name)));
        }
        assert!(Arc::ptr_eq(pending.rails.as_ref().unwrap(), &rails));
        assert_eq!(pending.received, 5);
        for name in ["political", "names"] {
            assert!(!pending.take(7, name, 1.0, Vec::new(), Part::failed(name)));
        }
        assert!(pending.take(7, "overlay", 1.0, Vec::new(), Part::failed("overlay")));
        assert_eq!(pending.received, PARTS);
    }
}
