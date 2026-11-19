//! The layer contract: what every map layer implements to draw from the playhead's change sets.
//!
//! A layer declares the tables and columns it reads at a view ([`Layer::inputs`]), makes its own
//! arrays whole from the mirrors at a reset ([`Layer::compose`]), marks the entities a step touched
//! ([`Layer::apply`]), recomputes every one of them from the mirrors ([`Layer::settle`]), and
//! leaves what it changed for the GPU as owned commands ([`Layer::stage`]). The map's side thread
//! ([`crate::playhead_host`]) owns the mirrors and calls these in that order, one step at a time;
//! only it writes the mirrors, and only the frame thread writes the commands it staged.

pub mod air_mission_counters;
pub mod base_counters;
pub mod buildings;
pub mod counters;
pub mod counter_kinds;
pub mod engagements;
pub mod gun_counters;
pub mod intel;
pub mod intel_icons;
pub mod labels;
pub mod map_text;
pub mod land_markers;
pub mod names;
pub mod owned;
pub mod naval_counters;
pub mod naval_markers;
pub mod naval;
#[cfg(test)]
mod p6_tests;
pub mod occupation_marks;
pub mod political;
pub mod relations;
pub mod radar;
pub mod rails;
pub mod resource_icons;
pub mod scripts;
pub mod snow;
pub mod sounds;
pub mod strikes;
pub mod touched;
pub mod trains;
pub mod units;
pub mod convoys;
pub mod orders;
pub mod aircraft;
pub mod mode_fills;
pub mod rebuilt;
pub mod ships;
#[cfg(any(test, feature = "test-support"))]
pub mod recorded;
pub mod vision;
pub mod weather;

use crate::playhead_host::mirror::Mirrors;
use eframe::wgpu;
use playhead::Step;
use replay_columnar::state::Kind;
use replay_data::{Hour, rows::TableId};
use std::{
    collections::{BTreeMap, VecDeque},
    io,
    marker::PhantomData,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicI64, Ordering},
    },
};
use viewer_foundation::{
    engagement_history::History,
    map::{camera::MapSize, perspective, province_admission::Admission},
};
use viewer_world::map::map_mode::MapMode;

/// What a layer is drawn at: the inputs that change what it reads.
#[derive(Clone, Debug, PartialEq)]
pub struct View {
    pub map_mode: MapMode,
    pub perspective: perspective::Choice,
    /// The camera's height over the map, which picks the far or near view.
    pub camera_height: f32,
    /// Where the camera looks from; none before a frame has placed it.
    pub camera: Option<viewer_foundation::map::camera::Placement>,
    /// Where the wheel or a zoom key has sent the camera, which the shown one eases towards; none
    /// when it stands where it was sent.
    pub target: Option<viewer_foundation::map::camera::Placement>,
    pub fog: bool,
    /// The map's size in pixels on screen, which the camera's admission tests against; zero
    /// before a frame has given it.
    pub viewport: [u32; 2],
    /// Every show/hide switch of the map (borders, railways, buildings, trains, strikes, map
    /// text) and its quality choices, as the window resolved them.
    pub graphics: dashboard_core::protocol::MapGraphics,
    /// Whether this map may sound (only the focused one does), and its volumes: battle
    /// ambience is `audio.ambient` (and `audio.combat`) above zero.
    pub sound: bool,
    pub audio: dashboard_core::protocol::MapAudio,
    /// The overlay's parts drawn (counters, labels, raids, combats, naval markers, pins, bases).
    pub overlay: crate::document::OverlayLayers,
    /// Whose counters are drawn, and the player-only counters toggle.
    pub counters: dashboard_core::protocol::CounterFilter,
    pub player_counters: bool,
    /// The country selected on the map, which the diplomacy mode colours relations from.
    pub selected: Option<u32>,
}

impl View {
    /// Equal to `other` but for where the camera stands: camera, target, viewport, the camera's height.
    pub fn same_declaration(&self, other: &Self) -> bool {
        let unplaced = |view: &Self| Self { camera_height: 0.0, camera: None, target: None, viewport: [0; 2], ..view.clone() };
        unplaced(self) == unplaced(other)
    }
}

impl Default for View {
    fn default() -> Self {
        Self {
            map_mode: MapMode::default(),
            perspective: perspective::Choice::default(),
            camera_height: 0.0,
            camera: None,
            target: None,
            viewport: [0, 0],
            fog: true,
            graphics: Default::default(),
            sound: true,
            audio: Default::default(),
            overlay: Default::default(),
            counters: Default::default(),
            player_counters: false,
            selected: None,
        }
    }
}

/// What a layer uploads to: the device, the scene whose passes it feeds, and the recording's
/// static content (meshes, map definitions) its uploads are shaped by.
pub struct Gpu<'a> {
    pub device: &'a wgpu::Device,
    pub queue: &'a wgpu::Queue,
    pub scene: &'a mut viewer_world::map::scene::Scene,
    pub content: &'a crate::document::Content,
    /// This picture must be complete and exact at the hour it stands at (a saved frame, a pause, a
    /// first picture): a layer whose product is made on a worker waits for the product of that hour
    /// here. A playing frame installs the newest product that has arrived and goes on, so no display
    /// frame waits for a maker of the simulation's side.
    pub whole: bool,
}

/// What a timed layer ([`Layer::timed`]) hands the display frame, taken once as the map's side
/// goes to its thread ([`Layer::installs`]): the change sets of the publications' hours and the
/// writer of them. The frame owns it from then on, so it writes the hours its own head has reached
/// whether or not the side's thread is busy preparing the next publication, and no display frame
/// repeats an hour because the side was busy.
pub trait Installs: Send {
    fn name(&self) -> &'static str;

    /// Takes every change set handed over since the last frame and writes those whose hour
    /// `through` has reached, newest value per entity; `None` writes every set taken, for a
    /// picture that must be whole at the hour it stands at (a pause, a saved frame, a first
    /// picture). A set of another generation than `composed` is void: the side has composed a new
    /// picture since it was dated (a seek, a piece's first hour), which replaces those hours.
    fn install(&mut self, gpu: &mut Gpu<'_>, through: Option<Hour>, composed: u64) -> io::Result<()>;

    /// The hour the picture it has written stands at; none before its first write.
    fn shown(&self) -> Option<Hour>;

    /// A set whose hour the last frame's head had reached is still unwritten: what the publication
    /// judge refuses by name.
    fn owes(&self) -> bool;

    /// What its last install wrote, for the run's record.
    fn wrote(&self) -> Wrote;

    /// Something outside it replaced what it wrote (a faction highlight's restore of the border
    /// plans): it writes its own again, now.
    fn reassert(&mut self, gpu: &mut Gpu<'_>) -> io::Result<()> {
        let _ = gpu;
        Ok(())
    }

    /// The hour of the set whose border plans it wrote last; none for an installer of none.
    fn plans_shown(&self) -> Option<Hour> {
        None
    }
}

/// What one install wrote: its wall nanoseconds, the bytes it handed the queue, the writes it
/// made by kind, and the layer's own phases of it (the layer names them in the run's report).
#[derive(Clone, Copy, Default)]
pub struct Wrote {
    pub ns: u64,
    pub bytes: u64,
    pub writes: [u64; 3],
    pub phases: [u64; 4],
}

/// The hour a timed layer's installer has written, which the side's thread reads to say what the
/// picture shows ([`Layer::shown_through`]): written by the display frame, read by the side.
pub struct Shown(std::sync::atomic::AtomicI64);

impl Shown {
    const NONE: i64 = i64::MIN;

    pub fn set(&self, hour: Hour) {
        self.0.store(i64::from(hour), std::sync::atomic::Ordering::Release);
    }

    pub fn get(&self) -> Option<Hour> {
        match self.0.load(std::sync::atomic::Ordering::Acquire) {
            Self::NONE => None,
            hour => Some(hour as Hour),
        }
    }
}

impl Default for Shown {
    fn default() -> Self {
        Self(std::sync::atomic::AtomicI64::new(Self::NONE))
    }
}

/// The publications a product may stand behind the publication's hour before it is waited for. A
/// publication is one of the map side's steps, which applies one of the playhead's and carries
/// that step's own hourly change sets: everything this bound, a layer's own [`Layer::behind_bound`] and
/// [`crate::playhead_host::standing::Standings`] count is counted in them. A worker whose cycle is
/// longer than one publication lands every second one, which is pacing; one that has landed
/// nothing for this many is starving, and the publication waits for it by name. At the rate the
/// simulation holds this is 67 ms, and the products measured there stand one publication behind at
/// worst.
pub const BOUND: u32 = 2;

/// How often, at most, a layer's product is made while the hours run: the ceiling a product
/// nobody can perceive changing faster is held to ([`Layer::rate`]). A product made at a rate
/// stands behind the publication's hour by the publications its interval covers, so it is waited
/// for at that many instead of at [`BOUND`]; a publication the head stands still for and a whole
/// settle make it at the hour shown, whatever the rate.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rate(u32);

impl Rate {
    /// A rate of `makings` a second.
    pub const fn a_second(makings: u32) -> Self {
        Self(if makings == 0 { 1 } else { makings })
    }

    /// The rate of a product made once at every publication, which is the fastest any product is
    /// made: the simulation's own rate.
    pub fn a_publication() -> Self {
        Self::a_second(viewer_foundation::map::simulation_rate::rate().round().max(1.0) as u32)
    }

    pub fn makings_a_second(self) -> u32 {
        self.0
    }

    /// The least time between two makings while the hours run.
    pub fn interval(self) -> std::time::Duration {
        std::time::Duration::from_secs(1) / self.0
    }

    /// The publications its product may stand behind the publication's hour: those its own
    /// interval covers, and [`BOUND`] more for the making itself.
    pub fn behind(self) -> u32 {
        let publications = viewer_foundation::map::simulation_rate::rate();
        (publications / f64::from(self.0)).ceil() as u32 + BOUND
    }
}

/// Whether a settle waits for the products a layer's own workers make. While the hours run the
/// map side's publication waits for nothing that may be one late: it settles what is ready, takes
/// what is published and ends, and a product that is not ready joins the publication after. A
/// product that has stood [`BOUND`] publications behind, a publication the head stands still for
/// ([`Settling::still`]) and a whole settle are waited for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Settling {
    /// One publication of the map's side thread. `late`: this one waits for the layer's product
    /// rather than letting it starve, because the product has stood [`BOUND`] publications behind
    /// or because the head stands still and no later hour is coming.
    Step { late: bool },
    /// Everything, waited for however long each product takes and refused by name if one never
    /// comes: a saved frame, a settled plan and the proof tool.
    Whole,
}

impl Settling {
    /// A step of the map's side thread, which waits for nothing.
    pub fn step() -> Self {
        Self::Step { late: false }
    }

    /// A publication the head stands still for: paused, held at a seek, or at the recording's last
    /// hour. No later hour is coming, so it waits for every product its layers owe and the picture
    /// a watcher is left looking at is whole at the first publication after the hours stop.
    pub fn still() -> Self {
        Self::Step { late: true }
    }

    /// This settling for a layer whose product has stood `behind` steps behind the step's hour,
    /// waited for at `bound` behind ([`BOUND`], or its own rate's: [`Rate::behind`]): the one rule
    /// every settle and every join of the step waits by.
    pub fn behind_bound(self, behind: u32, bound: u32) -> Self {
        match self {
            Self::Whole => Self::Whole,
            Self::Step { late } => Self::Step { late: late || behind >= bound },
        }
    }

    /// Whether a product not published yet is waited for at all.
    pub fn waits(self) -> bool {
        !matches!(self, Self::Step { late: false })
    }

    /// How long to wait for a product made from rows at the head: nothing, or `whole` for a settle
    /// that waits however long it takes.
    pub fn waiting(self, whole: std::time::Duration) -> std::time::Duration {
        if self.waits() { whole } else { std::time::Duration::ZERO }
    }

    /// Whether a product that does not come refuses the settle by name, rather than leaving the
    /// layer with the product it has.
    pub fn strict(self) -> bool {
        self == Self::Whole
    }
}

/// The layer of type `T` registered on `side` while the layer named `name` draws (declared,
/// composed and not refused) and the side's parts are home: how the products the frame reads are
/// found. Read in the window the side's thread hands the frame.
pub fn drawn_layer<'a, T: 'static>(side: &'a crate::playhead_host::MapSide, name: &str) -> Option<&'a T> {
    if !side.draws(name) {
        return None;
    }
    side.parts()?.1.iter().find_map(|layer| layer.any().downcast_ref::<T>())
}

pub trait Layer: Send {
    fn name(&self) -> &'static str;

    /// The camera moved and nothing else of the view changed: the layer takes the new placement
    /// (camera, target, viewport, the camera's height) here instead of declaring again, and answers
    /// what of its declaration the camera decides (a cutoff crossed, a reach left) as a number the
    /// host compares with the last: a change declares the view again. None: it reads the camera in
    /// `inputs` only, which the host then runs again for it alone.
    fn placed(&mut self, view: &View) -> Option<u64> {
        let _ = view;
        None
    }

    /// Declares the tables and columns it reads while drawn at `view`, keeping the handles it is
    /// given; a column the recording's schema lacks refuses the layer by name.
    fn inputs(&mut self, view: &View, inputs: &mut Inputs<'_>) -> io::Result<()>;

    /// The map side's view store, given once as the side is made: a layer with a maker opens its
    /// reader of it, and the tables it declares there are held by the side's mirrors beside the
    /// drawn layers' own columns.
    fn views(&mut self, _store: &crate::playhead_host::views::Store) -> io::Result<()> {
        Ok(())
    }

    /// Its arrays made whole from the mirrors, everything marked to upload. Runs on the host's
    /// worker at a reset, never on the frame thread.
    fn compose(&mut self, mirrors: &Mirrors) -> io::Result<()>;

    /// Marks the entities `step` touched in its columns (the mirrors already hold the step); no
    /// derivation beyond finding the entity.
    fn apply(&mut self, step: &Step, mirrors: &Mirrors);

    /// The hours of `step` (over the mirrors as they stand before it is applied) at which the
    /// recording moved the movers this layer's counters place,
    /// and the province each hour left each in, taken before the overlay of that step is asked
    /// for: its layout then gives every mover a place for each hour its own rows moved it instead
    /// of one for the interval's end. A layer that places no mover gives none, and so does a step
    /// whose hours were not read ([`Inputs::hourly`]: a seek's reset, a piece's crossing or a
    /// catch-up carries none), which leaves its movers on their publication's placements.
    fn moved_at(&self, step: &Step, mirrors: &Mirrors, into: &mut viewer_foundation::map::counter_move::MovedHours) {
        let _ = (step, mirrors, into);
    }

    /// Recomputes every dirty entity from the mirrors in the layer's order ([`Dirty`]: oldest
    /// first), and joins what its own workers made for the step.
    fn settle(&mut self, mirrors: &Mirrors, settling: Settling) -> io::Result<()>;

    /// Prepares, on the thread that settled the step, everything its install needs that is not a
    /// call to the device or the queue: the bytes [`Self::upload`] then hands over. Asked of every
    /// drawn layer after its settle, so nothing of an install is prepared on the frame thread.
    fn stage(&mut self) -> io::Result<()> {
        Ok(())
    }

    /// Writes what it settled to the GPU, whole: on the frame thread, in the window the map's side
    /// thread hands it the layers for. Every drawn layer that owes the GPU anything writes it in
    /// that window, whatever the frame's clock says, and the publication judge
    /// (`BATTLE_PLAN_VIEWER_CHECK_PUBLICATION`) refuses by name a layer that left any of it
    /// ([`Self::uploaded`]).
    fn upload(&mut self, gpu: &mut Gpu<'_>) -> io::Result<()> {
        let _ = gpu;
        Ok(())
    }

    /// Whether a publication of it is the hours' own change sets rather than one picture of the
    /// interval's end: it is applied and settled once for the publication, as every layer is, and
    /// asked to date what it changed at the game hours it changed at ([`Self::timed_at`]), which
    /// it reads from the rows of those hours ([`Inputs::hourly`]); the display frame then writes
    /// the sets its own head has reached ([`Self::installs`]). A timed layer reads no other layer
    /// and no other layer reads it: what it hands the frame stands at hours of its own, where an
    /// upstream's product stands at the publication's.
    fn timed(&self) -> bool {
        false
    }

    /// A timed layer: the publication just applied and settled, whose last hour is `hour`, over
    /// the picture the side composed `composed` ago. What it changed becomes the change sets of
    /// the hours it changed at, handed to the frame's installer at once ([`Self::installs`]) and
    /// written by the frames whose heads reach those hours; a compose since makes them void.
    fn timed_at(&mut self, hour: Hour, composed: u64) -> io::Result<()> {
        let _ = (hour, composed);
        Ok(())
    }

    /// A timed layer's installer, taken once as the side goes to its thread: the display frame
    /// owns it, and the layer hands it every change set it dates. None (the usual) leaves the
    /// layer writing what it settled, whole, in the frame's window ([`Self::upload`]); a timed
    /// layer whose installer was never taken dates nothing and keeps no set.
    fn installs(&mut self) -> Option<Box<dyn Installs>> {
        None
    }

    /// A timed layer: the hour the picture its installer has written stands at. None for a layer
    /// whose install is one publication's, whose hour is the host's own ([`Published`]).
    fn shown_through(&self) -> Option<Hour> {
        None
    }

    /// The work it has left (entities dirty and not settled yet), and the frames since the oldest
    /// was marked. Work handed to the layer's own worker is not carried: say what is published
    /// instead ([`Self::published`]).
    fn carried(&self) -> Carried;

    /// The hour the product it has published is current for. A layer whose worker holds newer work
    /// says that hour (or that it has published nothing yet); one without a worker, or whose worker
    /// holds nothing, is current with what the host applied once it carries nothing and is uploaded.
    fn published(&self) -> Published {
        Published::Current
    }

    /// Whether a picture that must be whole (a saved frame, the proof, a publication the head
    /// stands still for) may be shown at `head`: a layer that makes its product from rows at the
    /// head on a worker of its own answers false until it has made that hour's, and a stepped
    /// frame's settle is taken again until it has. A layer with nothing to make says true.
    fn made_at(&self, head: Hour) -> bool {
        let _ = head;
        true
    }

    /// How many publications its product may stand behind the publication's hour before the step
    /// waits for it: [`BOUND`] for a layer whose product is made within the publication, and a
    /// rated layer's own [`Rate::behind`]. A product the camera shows is never left further behind
    /// than this, and the judge (`BATTLE_PLAN_VIEWER_CHECK_RATES`) refuses a layer past its own by name.
    fn behind_bound(&self) -> u32 {
        BOUND
    }

    /// It takes the steps made ahead of the head ([`Self::ahead`]): a layer whose worker folds them
    /// before the frame applies them.
    fn wants_ahead(&self) -> bool {
        false
    }

    /// After this frame's applies, in order, each once: the steps applied this frame it was not
    /// handed before, then those the playhead made past the head (same epoch, up to a reset or a
    /// table of its own reset or renumbered). On the thread the publication is made on: hand
    /// them to the worker; read no row or mirror. A step handed here is applied later as any
    /// other.
    fn ahead(&mut self, steps: &[Arc<Step>]) {
        let _ = steps;
    }

    /// The steps handed ahead past the hour applied are void: the playhead rebuilt them for a new
    /// pace ([`playhead::Step::restep`]) and none will be applied. A worker that folded them starts
    /// over from the mirrors at the hour applied; what it made from them is never shown, and until
    /// its next product arrives the layer says the hour of the product it has. The steps it was
    /// handed through `kept`, the hour applied before this frame, stand; every step after it is
    /// handed again, this frame's applied first, since one it holds may be a void step that spans
    /// this frame's hour.
    fn void_ahead(&mut self, mirrors: &Mirrors, kept: Option<replay_data::Hour>) {
        let _ = (mirrors, kept);
    }

    /// Bytes its arrays and dirty lists hold.
    fn held(&self) -> usize;

    /// The layer itself, for the proof tool's comparison of its arrays and for the layers that
    /// read it ([`Upstream::get`]).
    fn any(&self) -> &dyn std::any::Any;

    /// The entities its settles made since last asked, against `sight`; asked every frame it is
    /// drawn. None: it does not count them, and the host counts what left its carry-over instead.
    fn scope(&mut self, _sight: &Sight<'_>) -> Option<Scope> {
        None
    }

    /// Everything it settled is on the GPU: nothing is left for [`Self::upload`].
    fn uploaded(&self) -> bool {
        true
    }

    /// The layer itself, mutably, for the host's calls only one layer answers.
    fn any_mut(&mut self) -> Option<&mut dyn std::any::Any> {
        None
    }

    /// The view changed (the camera moved, a switch flipped), after `inputs` were asked again: on
    /// the frame thread, with no redeclare and no compose, so a layer that draws only what the
    /// camera admits can mark the entities newly admitted and settle them within the budget. A
    /// layer whose columns changed also composes, once its tables arrive reset.
    fn viewed(&mut self, view: &View, mirrors: &Mirrors) {
        let _ = (view, mirrors);
    }

    /// For the layers that read it: what its settles changed since the host last cleared it, which
    /// it does only once every drawn dependent has settled after it (so a held dependent misses
    /// nothing). A dependent reads it in `settle_with` ([`Upstream::changed`]).
    fn changed(&self) -> Option<&Changed> {
        None
    }

    fn clear_changed(&mut self) {}

    /// `compose`, `apply` and `settle` of a layer that reads the layers registered before it
    /// (named in `inputs` with [`Inputs::after`]); by default the plain ones. Upstream layers have
    /// composed, applied and settled before it in the same frame, and a dependent's `settle` is
    /// held while any upstream carries dirty entities, so it never reads a half-settled upstream.
    fn compose_with(&mut self, mirrors: &Mirrors, upstream: &Upstream<'_>) -> io::Result<()> {
        let _ = upstream;
        self.compose(mirrors)
    }

    fn apply_with(&mut self, step: &Step, mirrors: &Mirrors, upstream: &Upstream<'_>) {
        let _ = upstream;
        self.apply(step, mirrors);
    }

    fn settle_with(&mut self, mirrors: &Mirrors, upstream: &Upstream<'_>, settling: Settling) -> io::Result<()> {
        let _ = upstream;
        self.settle(mirrors, settling)
    }

    /// The step's applies are done and the layers before it have settled: a layer whose settle
    /// joins a product one of its own workers makes asks that worker here, for everything it can
    /// ask without waiting, and answers true. The host then settles it with [`Self::join`] after
    /// every layer that joins no maker, so the build runs beside their settles instead of after
    /// them. A layer that answers false is settled here, as before.
    ///
    /// Its product is never waited for by the step itself: what the worker has published when the
    /// join comes is taken, and the rest joins the step after ([`Settling`]).
    fn ask(&mut self, mirrors: &Mirrors, upstream: &Upstream<'_>) -> io::Result<bool> {
        let _ = (mirrors, upstream);
        Ok(false)
    }

    /// The products [`Self::ask`] asked for, joined to the step: what is ready is taken, and what
    /// is not is waited for within the step's budget. Only for a layer whose `ask` answered true;
    /// a whole settle ([`Self::settle_with`]) asks and joins in one.
    fn join(&mut self, mirrors: &Mirrors, upstream: &Upstream<'_>, settling: Settling) -> io::Result<()> {
        self.settle_with(mirrors, upstream, settling)
    }
}

/// What a layer reads of the layers registered before it.
pub struct Upstream<'a> {
    layers: &'a [Box<dyn Layer>],
}

impl<'a> Upstream<'a> {
    pub fn new(layers: &'a [Box<dyn Layer>]) -> Self {
        Self { layers }
    }

    /// The upstream layer of type `T`.
    pub fn get<T: 'static>(&self) -> Option<&'a T> {
        self.layers.iter().find_map(|layer| layer.any().downcast_ref::<T>())
    }

    /// What the upstream layer of type `T` changed since this dependent last settled.
    pub fn changed<T: 'static>(&self) -> Change<'a> {
        static NONE: Changed = Changed { kinds: Vec::new(), everything: false };
        self.layers.iter().find(|layer| layer.any().is::<T>()).and_then(|layer| layer.changed()).unwrap_or(&NONE).read()
    }
}

/// What a layer changed, for the layers that read it: ids up to 64 bits by kind ("province", "unit",
/// a layer's own names), each kind in the order marked, or everything. An id may repeat.
#[derive(Clone, Debug, Default)]
pub struct Changed {
    kinds: Vec<(&'static str, Vec<u64>)>,
    everything: bool,
}

impl Changed {
    pub fn mark(&mut self, kind: &'static str, id: u64) {
        match self.kinds.iter_mut().find(|(held, _)| *held == kind) {
            Some((_, ids)) => ids.push(id),
            None => self.kinds.push((kind, vec![id])),
        }
    }

    /// Everything changed: a dependent looks again at all it reads of this layer.
    pub fn everything(&mut self) {
        self.everything = true;
    }

    /// What a reader must look at. The ids by kind are reachable only through [`Change::Marked`],
    /// so a reader cannot take one kind's ids and miss a whole change.
    pub fn read(&self) -> Change<'_> {
        if self.everything { Change::Whole } else { Change::Marked(Marked(self)) }
    }

    pub fn clear(&mut self) {
        self.kinds.iter_mut().for_each(|(_, ids)| ids.clear());
        self.everything = false;
    }

    /// The ids it holds, in bytes, for a layer's held count.
    pub fn bytes(&self) -> usize {
        self.kinds.iter().map(|(_, ids)| ids.len() * 8).sum()
    }
}

/// What a reader of a layer must look at of it: everything it reads, or the ids the layer marked.
#[derive(Clone, Copy)]
pub enum Change<'a> {
    /// Everything the reader reads of the layer may have moved.
    Whole,
    Marked(Marked<'a>),
}

/// The ids a layer marked, by kind.
#[derive(Clone, Copy)]
pub struct Marked<'a>(&'a Changed);

impl<'a> Marked<'a> {
    /// The ids of `kind` marked, in the order marked; an id may repeat.
    pub fn of(&self, kind: &str) -> &'a [u64] {
        self.0.kinds.iter().find(|(held, _)| *held == kind).map_or(&[], |(_, ids)| ids.as_slice())
    }
}

/// What a layer's published product is current for ([`Layer::published`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Published {
    /// Every step the host gave it, once it carries nothing and is uploaded.
    Current,
    /// This hour: its worker holds work newer than it.
    At(replay_data::Hour),
    /// Nothing yet: its worker is making its first product.
    Nothing,
    /// This hour: a product made from rows at the head on a worker (the unit models and meshes, the
    /// aircraft, the arrows between their jobs, the intel icons), the hour those rows stand at. It
    /// counts in the lag ([`crate::playhead_host::Frame::oldest`]) but does not hold the window
    /// clock's drawn hour.
    Cadence(replay_data::Hour),
}

/// The frame side of a worker fed the steps ahead ([`Layer::ahead`]): the newest hour handed to it,
/// the last hour applied, and the products it made, each stamped with the hour it is current at
/// (none: the compose's), taken in order once the frame has applied that hour, never before.
pub struct Fed<P> {
    handed: Option<Hour>,
    applied: Option<Hour>,
    composed: Option<Hour>,
    early: VecDeque<(Option<Hour>, P)>,
    /// A product was taken since the compose, and the stamp of the newest taken.
    taken: bool,
    stamp: Option<Hour>,
    due: Due,
}

impl<P> Default for Fed<P> {
    fn default() -> Self {
        Self { handed: None, applied: None, composed: None, early: VecDeque::new(), taken: false, stamp: None, due: Due::default() }
    }
}

impl<P> Fed<P> {
    /// The last hour applied, as the worker reads it.
    pub fn due(&self) -> Due {
        self.due.clone()
    }

    /// Composed at `hour`: nothing handed or made before counts.
    pub fn compose(&mut self, hour: Option<Hour>) {
        (self.handed, self.applied, self.composed, self.taken, self.stamp) = (None, None, hour, false, None);
        self.early.clear();
        self.due.set(None);
    }

    /// Of `steps`, those not handed yet, in order: a step of hours past the newest handed, or a step
    /// of none (a join's, handed once, when applied).
    pub fn hand<'a>(&mut self, steps: &'a [Arc<Step>]) -> Vec<&'a Arc<Step>> {
        let mut out = Vec::with_capacity(steps.len());
        for step in steps {
            if step.first() > step.last() {
                out.push(step);
            } else if self.handed.is_none_or(|handed| step.last() > handed) {
                self.handed = Some(step.last());
                out.push(step);
            }
        }
        out
    }

    pub fn applied(&mut self, step: &Step) {
        self.applied_through(step.last());
    }

    /// The last hour applied since the compose.
    pub fn applied_hour(&self) -> Option<Hour> {
        self.applied
    }

    /// The hour the frame's mirrors stand at: the last applied, or the compose's.
    pub fn through(&self) -> Option<Hour> {
        self.applied.or(self.composed)
    }

    /// The worker starts over from the mirrors at the hour applied: what it made before is dropped.
    pub fn restart(&mut self) {
        self.early.clear();
    }

    /// The steps handed past the hour applied are void ([`Layer::void_ahead`]): what was made from
    /// them is dropped and the steps from that hour are handed again. The product taken, of an hour
    /// applied, stays shown at its hour.
    pub fn void(&mut self) {
        let through = self.through();
        self.early.retain(|(stamp, _)| *stamp <= through);
        self.handed = through;
    }

    /// The frame applied `step`; the worker is told only at [`Self::publish`], once every step
    /// through it has been handed, so the hour it reads is never one whose steps it lacks.
    pub fn applied_unpublished(&mut self, step: &Step) {
        self.applied = Some(step.last());
    }

    /// The worker told the last hour applied.
    pub fn publish(&self) {
        self.due.set(self.applied);
    }

    /// The frame applied the steps through `hour`.
    pub fn applied_through(&mut self, hour: Hour) {
        self.applied = Some(hour);
        self.due.set(self.applied);
    }

    /// A product the worker made, current at `stamp`.
    pub fn receive(&mut self, stamp: Option<Hour>, product: P) {
        self.early.push_back((stamp, product));
    }

    /// The products now due, oldest first.
    pub fn take(&mut self, mut each: impl FnMut(P)) {
        while self.early.front().is_some_and(|(stamp, _)| *stamp <= self.applied) {
            let (stamp, product) = self.early.pop_front().expect("held");
            (self.taken, self.stamp) = (true, self.stamp.max(stamp));
            each(product);
        }
    }

    /// The product current at the last hour applied has arrived, taken or not.
    pub fn arrived(&self) -> bool {
        let newest = self.early.back().map(|(stamp, _)| *stamp).or(self.taken.then_some(self.stamp));
        newest.is_some_and(|stamp| stamp >= self.applied)
    }

    /// The hour of the newest product taken: current with every step applied, or that hour.
    pub fn published(&self) -> Published {
        if !self.taken {
            return Published::Nothing;
        }
        if self.stamp == self.applied {
            return Published::Current;
        }
        self.stamp.or(self.composed).map_or(Published::Nothing, Published::At)
    }

    /// The newest hour of a product received, taken or not.
    pub fn made_through(&self) -> Option<Hour> {
        self.early.back().map(|(stamp, _)| *stamp).or(self.taken.then_some(self.stamp)).flatten()
    }

    /// The hour shown: the newest product's, or the compose's.
    pub fn shown(&self) -> Option<Hour> {
        if self.taken { self.stamp.or(self.composed) } else { None }
    }
}

/// The last hour the frame applied, shared with a worker fed ahead: of several steps queued, one
/// already applied needs no product of its own when the next is applied too.
#[derive(Clone)]
pub struct Due(Arc<AtomicI64>);

impl Default for Due {
    fn default() -> Self {
        Self(Arc::new(AtomicI64::new(i64::MIN)))
    }
}

impl Due {
    fn set(&self, hour: Option<Hour>) {
        self.0.store(hour.map_or(i64::MIN, i64::from), Ordering::Release);
    }

    /// Whether a step ending at `last` has been applied.
    pub fn reached(&self, last: Hour) -> bool {
        self.0.load(Ordering::Acquire) >= i64::from(last)
    }

    /// The last hour applied; none before the first.
    pub fn hour(&self) -> Option<Hour> {
        let hour = self.0.load(Ordering::Acquire);
        (hour != i64::MIN).then_some(hour as Hour)
    }
}

/// A layer's carry-over: the entities its own worker still owes, none of its own.
#[derive(Clone, Copy, Debug, Default)]
pub struct Carried {
    pub entities: usize,
}

/// Whether the frames count each layer's settled entities against those the view shows
/// ([`Layer::scope`]): the paced run's measure. Off, a layer counts what it settled alone.
static IN_VIEW: AtomicBool = AtomicBool::new(false);

pub fn count_in_view(on: bool) {
    IN_VIEW.store(on, Ordering::Relaxed);
}

fn counting_in_view() -> bool {
    IN_VIEW.load(Ordering::Relaxed)
}

/// What the camera shows this frame: provinces and states by its admission, and its eye for what
/// is within hearing. Empty without a camera, or while the count is off.
pub struct Sight<'a> {
    admission: Option<Admission>,
    content: Option<&'a crate::document::Content>,
    eye: Option<[f32; 3]>,
}

impl<'a> Sight<'a> {
    pub fn none() -> Self {
        Self { admission: None, content: None, eye: None }
    }

    pub fn of(content: Option<&'a crate::document::Content>, view: &View) -> Self {
        let (Some(content), Some(placement)) = (content, view.camera) else { return Self::none() };
        if view.viewport == [0, 0] || !counting_in_view() {
            return Self::none();
        }
        let camera = placement.on(MapSize::of(&content.map), content.map_defines);
        let eye = camera.eye();
        Self { admission: Some(Admission::new(camera, view.viewport)), content: Some(content), eye: Some(eye) }
    }

    /// Whether the view admits `province`.
    pub fn province(&self, province: u32) -> Option<bool> {
        Some(self.admission.as_ref()?.admits_province(&self.content?.counters.bounds, province))
    }

    /// Whether the view admits a province of `state`.
    pub fn state(&self, state: u32) -> Option<bool> {
        let provinces = self.content?.icons.raids.trigger_content.state_provinces.get(&state);
        self.admission.as_ref()?;
        Some(provinces.is_some_and(|provinces| provinces.iter().any(|&province| self.province(province) == Some(true))))
    }

    /// Whether ground within `reach` of `at` is within `hearing` of the eye.
    pub fn heard(&self, at: [f32; 2], reach: f32, hearing: Option<f32>) -> Option<bool> {
        let (eye, hearing) = (self.eye?, hearing?);
        let [dx, dz] = [eye[0] - at[0], eye[2] - at[1]];
        Some(dx * dx + dz * dz <= (hearing + reach) * (hearing + reach))
    }

    /// Whether any ground of `province`'s box is within `hearing` of the eye.
    pub fn province_heard(&self, province: u32, hearing: Option<f32>) -> Option<bool> {
        let (eye, hearing) = (self.eye?, hearing?);
        let Some(&[min_x, min_z, max_x, max_z]) = self.content?.counters.bounds.get(&province) else { return Some(false) };
        let [dx, dz] = [eye[0] - eye[0].clamp(min_x, max_x), eye[2] - eye[2].clamp(min_z, max_z)];
        Some(dx * dx + dz * dz <= hearing * hearing)
    }
}

/// Entities a layer's settles made since last asked, and of them those the view shows (for sounds,
/// those within hearing); none in view when the frame does not count it.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Scope {
    pub settled: u64,
    pub in_view: Option<u64>,
}

/// A layer's settled entities by where they stand, counted against the view at [`Layer::scope`].
#[derive(Debug, Default)]
pub struct Scoped {
    settled: u64,
    provinces: Vec<u32>,
    states: Vec<u32>,
    heard: Vec<([f32; 2], f32)>,
    heard_provinces: Vec<u32>,
}

impl Scoped {
    pub fn province(&mut self, province: u32) {
        self.settled += 1;
        if counting_in_view() {
            self.provinces.push(province);
        }
    }

    pub fn state(&mut self, state: u32) {
        self.settled += 1;
        if counting_in_view() {
            self.states.push(state);
        }
    }

    /// One whose sound reaches `reach` from `at`.
    pub fn heard(&mut self, at: [f32; 2], reach: f32) {
        self.settled += 1;
        if counting_in_view() {
            self.heard.push((at, reach));
        }
    }

    /// One heard when the eye is within hearing of `province`'s ground.
    pub fn heard_province(&mut self, province: u32) {
        self.settled += 1;
        if counting_in_view() {
            self.heard_provinces.push(province);
        }
    }

    /// One with no place on the map: settled, never in view.
    pub fn unplaced(&mut self) {
        self.settled += 1;
    }

    /// The count since the last, against `sight` (`hearing`: the distance sounds carry), emptied.
    pub fn take(&mut self, sight: &Sight<'_>, hearing: Option<f32>) -> Scope {
        let counted = sight.admission.is_some();
        let seen = self
            .provinces
            .iter()
            .map(|&province| sight.province(province))
            .chain(self.states.iter().map(|&state| sight.state(state)))
            .chain(self.heard.iter().map(|&(at, reach)| sight.heard(at, reach, hearing)))
            .chain(self.heard_provinces.iter().map(|&province| sight.province_heard(province, hearing)))
            .try_fold(0u64, |count, seen| seen.map(|seen| count + u64::from(seen)));
        let scope = Scope { settled: self.settled, in_view: seen.filter(|_| counted) };
        self.settled = 0;
        self.provinces.clear();
        self.states.clear();
        self.heard.clear();
        self.heard_provinces.clear();
        scope
    }

    pub fn bytes(&self) -> usize {
        (self.provinces.capacity() + self.states.capacity() + self.heard_provinces.capacity()) * 4 + self.heard.capacity() * 12
    }

    /// The count `other` made (on a worker) added to this one's.
    pub fn extend(&mut self, other: Self) {
        self.settled += other.settled;
        self.provinces.extend(other.provinces);
        self.states.extend(other.states);
        self.heard.extend(other.heard);
        self.heard_provinces.extend(other.heard_provinces);
    }
}

/// A layer's dirty entities by index, each once, in the order they were first marked, each with
/// the frame it was marked at: the oldest still owed is the first.
#[derive(Debug, Default)]
pub struct Dirty {
    marked: Vec<u64>,
    list: Vec<(u32, u64)>,
}

impl Dirty {
    /// Marks `entity` at `frame`; whether it was not marked already.
    pub fn mark(&mut self, entity: u32, frame: u64) -> bool {
        let word = entity as usize / 64;
        if word >= self.marked.len() {
            self.marked.resize(word + 1, 0);
        }
        let bit = 1 << (entity % 64);
        if self.marked[word] & bit != 0 {
            return false;
        }
        self.marked[word] |= bit;
        self.list.push((entity, frame));
        true
    }

    pub fn len(&self) -> usize {
        self.list.len()
    }

    /// The frame the oldest entity still dirty was marked at.
    pub fn since(&self) -> Option<u64> {
        self.list.first().map(|&(_, frame)| frame)
    }

    /// Takes every entity marked, oldest first, `each` recomputing one.
    pub fn settle(&mut self, mut each: impl FnMut(u32) -> io::Result<()>) -> io::Result<()> {
        let mut done = 0;
        while done < self.list.len() {
            let (entity, _) = self.list[done];
            self.marked[entity as usize / 64] &= !(1 << (entity % 64));
            done += 1;
            each(entity)?;
        }
        self.list.drain(..done);
        Ok(())
    }

    pub fn clear(&mut self) {
        self.list.iter().for_each(|&(entity, _)| self.marked[entity as usize / 64] = 0);
        self.list.clear();
    }

    pub fn bytes(&self) -> usize {
        self.marked.capacity() * 8 + self.list.capacity() * 12
    }
}

/// Where a layer's kept combat-icon frames stand: what it last read them from, so the next read
/// takes only the icons the index changed since ([`History::icons_changed`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KeptAt {
    /// Never read.
    Unread,
    /// There is no index, or no head to read one at: every live combat is unindexed, whatever the
    /// hour.
    Unindexed,
    /// Read at this hour, which the index answered from here.
    At(Hour, viewer_foundation::engagement_history::Provenance),
}

impl KeptAt {
    /// Where `index` answers the kept frames of `viewer` from at `head`.
    pub fn of(index: Option<(&Arc<History>, replay_data::BranchId)>, head: Option<Hour>, viewer: u32) -> Self {
        match (index, head) {
            (Some((history, branch)), Some(head)) => Self::At(head, history.provenance(branch, head, viewer)),
            _ => Self::Unindexed,
        }
    }

    /// The combats whose kept frame of `kind` changed between where it stood and `now`; none when
    /// every live combat's must be read again.
    pub fn advance(self, now: Self, index: Option<(&Arc<History>, replay_data::BranchId)>, kind: usize, viewer: u32) -> Option<Vec<u64>> {
        match (self, now) {
            (Self::Unindexed, Self::Unindexed) => Some(Vec::new()),
            (Self::At(from, held), Self::At(head, found)) if held == found => match found {
                viewer_foundation::engagement_history::Provenance::Indexed => {
                    let (history, branch) = index?;
                    history.icons_changed(kind, branch, from, head, viewer)
                }
                // Not answered from the fold: every live combat is unindexed, whatever the hour.
                _ => Some(Vec::new()),
            },
            _ => None,
        }
    }
}

/// A layer's dirty entities by key, each once, in the order they were first marked, each with the
/// frame it was marked at: for entities whose identity outlives a slot (a combat, a country).
#[derive(Debug, Default)]
pub struct Keys {
    list: Vec<(u64, u64)>,
    held: std::collections::HashSet<u64>,
}

impl Keys {
    /// Marks `key` at `frame`; whether it was not marked already.
    pub fn mark(&mut self, key: u64, frame: u64) -> bool {
        if !self.held.insert(key) {
            return false;
        }
        self.list.push((key, frame));
        true
    }

    pub fn len(&self) -> usize {
        self.list.len()
    }

    /// Every key marked, oldest first, taken off the list.
    pub fn take(&mut self) -> Vec<u64> {
        let taken: Vec<u64> = self.list.drain(..).map(|(key, _)| key).collect();
        taken.iter().for_each(|key| {
            self.held.remove(key);
        });
        taken
    }

    pub fn clear(&mut self) {
        self.list.clear();
        self.held.clear();
    }

    pub fn bytes(&self) -> usize {
        self.list.capacity() * 16 + self.held.capacity() * 16
    }
}

/// A number column a layer reads, typed: `T` is the column's kind.
#[derive(Debug)]
pub struct Num<T> {
    pub table: TableId,
    pub column: usize,
    kind: PhantomData<fn() -> T>,
}

#[cfg(any(test, feature = "test-support"))]
impl<T> Num<T> {
    pub fn of(table: TableId, column: usize) -> Self {
        Self { table, column, kind: PhantomData }
    }
}

impl<T> Clone for Num<T> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<T> Copy for Num<T> {}

/// A text column: ids, with the table's strings.
#[derive(Clone, Copy, Debug)]
pub struct Text {
    pub table: TableId,
    pub column: usize,
}

/// The hours of `step` at which `table`'s `province` column moved a row, noted as that row's own
/// mover ([`Layer::moved_at`]). Each hour's change set names only the cells that hour wrote, and a
/// province cell is written when it changed, so each one is one move at the hour it happened.
///
/// `mirrors` stand as they were before `step` is applied, where its slots still name the rows its
/// hours wrote. A row born or gone inside the step is left out: the slot it took may be another
/// row's within it, so its mover stands on the publication's own placement.
pub fn moved_provinces(
    step: &Step,
    mirrors: &Mirrors,
    table: TableId,
    province: usize,
    unit: impl Fn(u64) -> viewer_foundation::unit_handle::Unit,
    into: &mut viewer_foundation::map::counter_move::MovedHours,
) {
    let hours = step.hours_of(table.index());
    if hours.is_empty() {
        return;
    }
    let Some(mirror) = mirrors.get(table) else { return };
    let lived: std::collections::BTreeSet<u128> = step
        .table(table.index())
        .into_iter()
        .flat_map(|changes| {
            changes
                .deleted()
                .keys()
                .unwrap_or_default()
                .iter()
                .chain(changes.inserted().keys().unwrap_or_default())
                .copied()
        })
        .collect();
    for set in hours {
        let Some(hour) = set.head() else { continue };
        for held in set.columns().iter().filter(|held| held.column() == province) {
            held.each(|slot, value| {
                // A slot no live row holds before the step is written by a row born inside it: the
                // key the slot kept is a row gone before, never this write's.
                if let (Some(key), replay_columnar::store::Value::Number(bits)) = (mirror.key(slot).filter(|_| mirror.live(slot)), value)
                    && !lived.contains(&key)
                {
                    into.note(unit(key as u64), hour, bits as u32);
                }
            });
        }
    }
}

/// A list or opaque column: each cell's bytes.
#[derive(Clone, Copy, Debug)]
pub struct List {
    pub table: TableId,
    pub column: usize,
}

/// A number type a column is read as, and the kinds that hold it.
pub trait Number: Copy {
    const KINDS: &'static [Kind];
    fn from_raw(raw: u64) -> Self;
}

macro_rules! number {
    ($($type:ty => [$($kind:ident),+], |$raw:ident| $from:expr;)+) => {
        $(impl Number for $type {
            const KINDS: &'static [Kind] = &[$(Kind::$kind),+];
            #[inline]
            fn from_raw($raw: u64) -> Self {
                $from
            }
        })+
    };
}

number! {
    bool => [Bool], |raw| raw != 0;
    u8 => [U8], |raw| raw as u8;
    i8 => [I8], |raw| raw as u8 as i8;
    u16 => [U16], |raw| raw as u16;
    i16 => [I16], |raw| raw as u16 as i16;
    u32 => [U32, OptionalU32], |raw| raw as u32;
    i32 => [I32, OptionalI32], |raw| raw as u32 as i32;
    u64 => [U64, OptionalU64], |raw| raw;
    i64 => [I64, OptionalI64], |raw| raw as i64;
}

/// What the drawn layers read of one table: columns (ascending), whether rows' keys are wanted,
/// and whether each hour of a step that wrote it carries its own change set.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Read {
    pub columns: Vec<usize>,
    pub keyed: bool,
    pub hourly: bool,
}

/// What a layer reads at a view, named by table and field and resolved against the recording's
/// schema once, when the layer declares.
pub struct Inputs<'a> {
    kinds: &'a [Vec<Kind>],
    layer: &'static str,
    /// By table index: its columns, and whether its rows' keys are wanted.
    tables: BTreeMap<usize, Read>,
    /// The layers it reads, by name: registered before it.
    after: Vec<&'static str>,
}

impl<'a> Inputs<'a> {
    pub fn new(kinds: &'a [Vec<Kind>], layer: &'static str) -> Self {
        Self { kinds, layer, tables: BTreeMap::new(), after: Vec::new() }
    }

    /// Reads `table`'s rows, with their keys when `keyed`, and no column yet.
    pub fn table(&mut self, table: TableId, keyed: bool) -> io::Result<()> {
        self.kinds_of(table)?;
        let entry = self.tables.entry(table.index()).or_default();
        entry.keyed |= keyed;
        Ok(())
    }

    /// Reads `table` hour by hour: beside the step's own change set, each hour of it that wrote
    /// the table carries its own ([`playhead::Step::hours_of`]), so the layer can date what
    /// changed at the hour it changed instead of at the interval's end. It costs the lane the
    /// hours' own pass over the table and nothing else: no second read and no second decode.
    pub fn hourly(&mut self, table: TableId) -> io::Result<()> {
        self.kinds_of(table)?;
        self.tables.entry(table.index()).or_default().hourly = true;
        Ok(())
    }

    /// Reads the product of layer `name`, registered before it: its `compose`, `apply` and
    /// `settle` come after that layer's in each frame, and it composes whenever that layer does.
    pub fn after(&mut self, name: &'static str) {
        if !self.after.contains(&name) {
            self.after.push(name);
        }
    }

    /// The layers it reads, by name.
    pub fn upstream(&self) -> &[&'static str] {
        &self.after
    }

    pub fn number<T: Number>(&mut self, table: TableId, field: &str) -> io::Result<Num<T>> {
        let (column, kind) = self.column(table, field)?;
        if !T::KINDS.contains(&kind) {
            return Err(self.refuse(format!("{table:?}.{field} is a {kind:?} column, not a {}", std::any::type_name::<T>())));
        }
        Ok(Num { table, column, kind: PhantomData })
    }

    pub fn text(&mut self, table: TableId, field: &str) -> io::Result<Text> {
        let (column, kind) = self.column(table, field)?;
        if kind != Kind::Bytes {
            return Err(self.refuse(format!("{table:?}.{field} is a {kind:?} column, not text")));
        }
        Ok(Text { table, column })
    }

    pub fn list(&mut self, table: TableId, field: &str) -> io::Result<List> {
        let (column, kind) = self.column(table, field)?;
        if !matches!(kind, Kind::U32List | Kind::U64List | Kind::Opaque) {
            return Err(self.refuse(format!("{table:?}.{field} is a {kind:?} column, not a list")));
        }
        Ok(List { table, column })
    }

    /// A column by its index in the table, whatever its kind.
    pub fn index(&mut self, table: TableId, column: usize) -> io::Result<()> {
        let kinds = self.kinds_of(table)?;
        if column >= kinds.len() {
            return Err(self.refuse(format!("{table:?} holds no column {column}")));
        }
        self.select(table, column);
        Ok(())
    }

    fn column(&mut self, table: TableId, field: &str) -> io::Result<(usize, Kind)> {
        let kinds = self.kinds_of(table)?;
        let column = (0..kinds.len())
            .find(|&column| table.field_name(column) == Some(field))
            .ok_or_else(|| self.refuse(format!("it reads {table:?}.{field}, which this recording's schema does not hold")))?;
        let kind = kinds[column];
        self.select(table, column);
        Ok((column, kind))
    }

    fn select(&mut self, table: TableId, column: usize) {
        let entry = self.tables.entry(table.index()).or_default();
        if let Err(at) = entry.columns.binary_search(&column) {
            entry.columns.insert(at, column);
        }
    }

    fn kinds_of(&self, table: TableId) -> io::Result<&'a [Kind]> {
        let kinds = self.kinds;
        kinds.get(table.index()).map(Vec::as_slice).ok_or_else(|| self.refuse(format!("it reads {table:?}, which this recording's schema does not hold")))
    }

    fn refuse(&self, why: String) -> io::Error {
        io::Error::new(io::ErrorKind::InvalidData, format!("The {} layer is refused: {why}", self.layer))
    }

    /// Per table read, its columns and whether keys are wanted.
    pub fn into_tables(self) -> BTreeMap<usize, Read> {
        self.tables
    }

    /// The columns declared so far, per table, ascending: what a layer keeps to tell a write it
    /// reads from one it does not.
    pub fn selected(&self) -> BTreeMap<usize, Vec<usize>> {
        self.tables.iter().map(|(&table, read)| (table, read.columns.clone())).collect()
    }
}
