//! The layer contract: what every map layer implements to draw from the playhead's change sets.
//!
//! A layer declares the tables and columns it reads at a view ([`Layer::inputs`]), makes its own
//! arrays whole from the frame side's mirrors at a reset, off the frame thread ([`Layer::compose`]),
//! marks the entities a step touched ([`Layer::apply`]), recomputes them from the mirrors within
//! the frame's budget, carrying the rest ([`Layer::settle`]), and writes changed ranges to the GPU
//! within the same budget ([`Layer::upload`]). The host ([`crate::playhead_host`]) owns the mirrors
//! and calls these in that order; only it writes the mirrors.

pub(crate) mod air_mission_counters;
pub(crate) mod base_counters;
pub(crate) mod buildings;
pub(crate) mod counters;
pub(crate) mod counter_kinds;
pub(crate) mod engagements;
pub(crate) mod gun_counters;
pub(crate) mod intel;
pub(crate) mod intel_icons;
pub(crate) mod labels;
pub(crate) mod map_text;
pub(crate) mod land_markers;
pub(crate) mod names;
pub(crate) mod owned;
pub(crate) mod naval_counters;
pub(crate) mod naval_markers;
pub(crate) mod naval;
#[cfg(test)]
mod p6_tests;
pub(crate) mod occupation_marks;
pub(crate) mod political;
pub(crate) mod relations;
pub(crate) mod radar;
pub(crate) mod rails;
pub(crate) mod resource_icons;
pub(crate) mod scripts;
pub(crate) mod snow;
pub(crate) mod sounds;
pub(crate) mod strikes;
pub(crate) mod touched;
pub(crate) mod trains;
pub(crate) mod units;
pub(crate) mod convoys;
pub(crate) mod orders;
pub(crate) mod aircraft;
pub(crate) mod mode_fills;
pub(crate) mod rebuilt;
pub(crate) mod ships;
#[cfg(test)]
pub(crate) mod recorded;
pub(crate) mod vision;
pub(crate) mod weather;

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
    time::{Duration, Instant},
};
use viewer_foundation::map::{camera::MapSize, perspective, province_admission::Admission};
use viewer_world::map::map_mode::MapMode;

/// What a layer is drawn at: the inputs that change what it reads.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct View {
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
    pub graphics: dashboard::protocol::MapGraphics,
    /// Whether this map may sound (only the focused one does), and its volumes: battle
    /// ambience is `audio.ambient` (and `audio.combat`) above zero.
    pub sound: bool,
    pub audio: dashboard::protocol::MapAudio,
    /// The overlay's parts drawn (counters, labels, raids, combats, naval markers, pins, bases).
    pub overlay: crate::document::OverlayLayers,
    /// Whose counters are drawn, and the player-only counters toggle.
    pub counters: dashboard::protocol::CounterFilter,
    pub player_counters: bool,
    /// The country selected on the map, which the diplomacy mode colours relations from.
    pub selected: Option<u32>,
}

impl View {
    /// Equal to `other` but for where the camera stands: camera, target, viewport, the camera's height.
    pub(crate) fn same_declaration(&self, other: &Self) -> bool {
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
pub(crate) struct Gpu<'a> {
    pub device: &'a wgpu::Device,
    pub queue: &'a wgpu::Queue,
    pub scene: &'a mut viewer_world::map::scene::Scene,
    pub content: &'a crate::document::Content,
}

pub(crate) trait Layer: Send {
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

    /// Its arrays made whole from the mirrors, everything marked to upload. Runs on the host's
    /// worker at a reset, never on the frame thread.
    fn compose(&mut self, mirrors: &Mirrors) -> io::Result<()>;

    /// Marks the entities `step` touched in its columns (the mirrors already hold the step); no
    /// derivation beyond finding the entity.
    fn apply(&mut self, step: &Step, mirrors: &Mirrors);

    /// Recomputes dirty entities from the mirrors in the layer's order ([`Dirty`]: oldest first)
    /// until done or out of budget; the rest stay dirty and show their newest values when settled.
    fn settle(&mut self, mirrors: &Mirrors, budget: &mut Budget) -> io::Result<()>;

    /// Writes changed ranges to the GPU until done or out of budget; the rest stay for the next
    /// frame.
    fn upload(&mut self, gpu: &mut Gpu<'_>, budget: &mut Budget) -> io::Result<()>;

    /// The frame thread's work it has left (entities dirty and not settled yet), and the frames
    /// since the oldest was marked. Work handed to the layer's own worker is not carried: say what
    /// is published instead ([`Self::published`]).
    fn carried(&self) -> Carried;

    /// The hour the product it has published is current for. A layer whose worker holds newer work
    /// says that hour (or that it has published nothing yet); one without a worker, or whose worker
    /// holds nothing, is current with what the host applied once it carries nothing and is uploaded.
    fn published(&self) -> Published {
        Published::Current
    }

    /// It takes the steps made ahead of the head ([`Self::ahead`]): a layer whose worker folds them
    /// before the frame applies them.
    fn wants_ahead(&self) -> bool {
        false
    }

    /// After this frame's applies, in order, each once: the steps applied this frame it was not
    /// handed before, then those the playhead made past the head (same epoch, up to a reset or a
    /// table of its own reset or renumbered). On the frame thread: hand them to the worker; read
    /// no row or mirror. A step handed here is applied later as any other.
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

    /// Everything it settled is on the GPU: nothing is left for `upload`.
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

    fn settle_with(&mut self, mirrors: &Mirrors, upstream: &Upstream<'_>, budget: &mut Budget) -> io::Result<()> {
        let _ = upstream;
        self.settle(mirrors, budget)
    }
}

/// What a layer reads of the layers registered before it.
pub(crate) struct Upstream<'a> {
    layers: &'a [Box<dyn Layer>],
}

impl<'a> Upstream<'a> {
    pub(crate) fn new(layers: &'a [Box<dyn Layer>]) -> Self {
        Self { layers }
    }

    /// The upstream layer of type `T`.
    pub(crate) fn get<T: 'static>(&self) -> Option<&'a T> {
        self.layers.iter().find_map(|layer| layer.any().downcast_ref::<T>())
    }

    /// What the upstream layer of type `T` changed since this dependent last settled.
    pub(crate) fn changed<T: 'static>(&self) -> &'a Changed {
        static NONE: Changed = Changed { kinds: Vec::new(), everything: false };
        self.layers.iter().find(|layer| layer.any().is::<T>()).and_then(|layer| layer.changed()).unwrap_or(&NONE)
    }
}

/// What a layer changed, for the layers that read it: ids up to 64 bits by kind ("province", "unit",
/// a layer's own names), each kind in the order marked, or everything. An id may repeat.
#[derive(Clone, Debug, Default)]
pub(crate) struct Changed {
    kinds: Vec<(&'static str, Vec<u64>)>,
    everything: bool,
}

impl Changed {
    pub(crate) fn mark(&mut self, kind: &'static str, id: u64) {
        match self.kinds.iter_mut().find(|(held, _)| *held == kind) {
            Some((_, ids)) => ids.push(id),
            None => self.kinds.push((kind, vec![id])),
        }
    }

    /// Everything changed: a dependent looks again at all it reads of this layer.
    pub(crate) fn everything(&mut self) {
        self.everything = true;
    }

    pub(crate) fn all(&self) -> bool {
        self.everything
    }

    /// The ids of `kind` marked.
    pub(crate) fn of(&self, kind: &str) -> &[u64] {
        self.kinds.iter().find(|(held, _)| *held == kind).map_or(&[], |(_, ids)| ids.as_slice())
    }

    pub(crate) fn clear(&mut self) {
        self.kinds.iter_mut().for_each(|(_, ids)| ids.clear());
        self.everything = false;
    }
}

/// What a layer's published product is current for ([`Layer::published`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Published {
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
pub(crate) struct Fed<P> {
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
    pub(crate) fn due(&self) -> Due {
        self.due.clone()
    }

    /// Composed at `hour`: nothing handed or made before counts.
    pub(crate) fn compose(&mut self, hour: Option<Hour>) {
        (self.handed, self.applied, self.composed, self.taken, self.stamp) = (None, None, hour, false, None);
        self.early.clear();
        self.due.set(None);
    }

    /// Of `steps`, those not handed yet, in order: a step of hours past the newest handed, or a step
    /// of none (a join's, handed once, when applied).
    pub(crate) fn hand<'a>(&mut self, steps: &'a [Arc<Step>]) -> Vec<&'a Arc<Step>> {
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

    pub(crate) fn applied(&mut self, step: &Step) {
        self.applied_through(step.last());
    }

    /// The last hour applied since the compose.
    pub(crate) fn applied_hour(&self) -> Option<Hour> {
        self.applied
    }

    /// The hour the frame's mirrors stand at: the last applied, or the compose's.
    pub(crate) fn through(&self) -> Option<Hour> {
        self.applied.or(self.composed)
    }

    /// The worker starts over from the mirrors at the hour applied: what it made before is dropped.
    pub(crate) fn restart(&mut self) {
        self.early.clear();
    }

    /// The steps handed past the hour applied are void ([`Layer::void_ahead`]): what was made from
    /// them is dropped and the steps from that hour are handed again. The product taken, of an hour
    /// applied, stays shown at its hour.
    pub(crate) fn void(&mut self) {
        let through = self.through();
        self.early.retain(|(stamp, _)| *stamp <= through);
        self.handed = through;
    }

    /// The frame applied `step`; the worker is told only at [`Self::publish`], once every step
    /// through it has been handed, so the hour it reads is never one whose steps it lacks.
    pub(crate) fn applied_unpublished(&mut self, step: &Step) {
        self.applied = Some(step.last());
    }

    /// The worker told the last hour applied.
    pub(crate) fn publish(&self) {
        self.due.set(self.applied);
    }

    /// The frame applied the steps through `hour`.
    pub(crate) fn applied_through(&mut self, hour: Hour) {
        self.applied = Some(hour);
        self.due.set(self.applied);
    }

    /// A product the worker made, current at `stamp`.
    pub(crate) fn receive(&mut self, stamp: Option<Hour>, product: P) {
        self.early.push_back((stamp, product));
    }

    /// The products now due, oldest first.
    pub(crate) fn take(&mut self, mut each: impl FnMut(P)) {
        while self.early.front().is_some_and(|(stamp, _)| *stamp <= self.applied) {
            let (stamp, product) = self.early.pop_front().expect("held");
            (self.taken, self.stamp) = (true, self.stamp.max(stamp));
            each(product);
        }
    }

    /// The product current at the last hour applied has arrived, taken or not.
    pub(crate) fn arrived(&self) -> bool {
        let newest = self.early.back().map(|(stamp, _)| *stamp).or(self.taken.then_some(self.stamp));
        newest.is_some_and(|stamp| stamp >= self.applied)
    }

    /// The hour of the newest product taken: current with every step applied, or that hour.
    pub(crate) fn published(&self) -> Published {
        if !self.taken {
            return Published::Nothing;
        }
        if self.stamp == self.applied {
            return Published::Current;
        }
        self.stamp.or(self.composed).map_or(Published::Nothing, Published::At)
    }

    /// The newest hour of a product received, taken or not.
    pub(crate) fn made_through(&self) -> Option<Hour> {
        self.early.back().map(|(stamp, _)| *stamp).or(self.taken.then_some(self.stamp)).flatten()
    }

    /// The hour shown: the newest product's, or the compose's.
    pub(crate) fn shown(&self) -> Option<Hour> {
        if self.taken { self.stamp.or(self.composed) } else { None }
    }
}

/// The last hour the frame applied, shared with a worker fed ahead: of several steps queued, one
/// already applied needs no product of its own when the next is applied too.
#[derive(Clone)]
pub(crate) struct Due(Arc<AtomicI64>);

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
    pub(crate) fn reached(&self, last: Hour) -> bool {
        self.0.load(Ordering::Acquire) >= i64::from(last)
    }

    /// The last hour applied; none before the first.
    pub(crate) fn hour(&self) -> Option<Hour> {
        let hour = self.0.load(Ordering::Acquire);
        (hour != i64::MIN).then_some(hour as Hour)
    }
}

/// A layer's carry-over: entities dirty after a settle, and how late the oldest is.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct Carried {
    pub entities: usize,
    pub frames: u64,
}

/// Whether the frames count each layer's settled entities against those the view shows
/// ([`Layer::scope`]): the paced run's measure. Off, a layer counts what it settled alone.
static IN_VIEW: AtomicBool = AtomicBool::new(false);

pub(crate) fn count_in_view(on: bool) {
    IN_VIEW.store(on, Ordering::Relaxed);
}

fn counting_in_view() -> bool {
    IN_VIEW.load(Ordering::Relaxed)
}

/// What the camera shows this frame: provinces and states by its admission, and its eye for what
/// is within hearing. Empty without a camera, or while the count is off.
pub(crate) struct Sight<'a> {
    admission: Option<Admission>,
    content: Option<&'a crate::document::Content>,
    eye: Option<[f32; 3]>,
}

impl<'a> Sight<'a> {
    pub(crate) fn none() -> Self {
        Self { admission: None, content: None, eye: None }
    }

    pub(crate) fn of(content: Option<&'a crate::document::Content>, view: &View) -> Self {
        let (Some(content), Some(placement)) = (content, view.camera) else { return Self::none() };
        if view.viewport == [0, 0] || !counting_in_view() {
            return Self::none();
        }
        let camera = placement.on(MapSize::of(&content.map), content.map_defines);
        let eye = camera.eye();
        Self { admission: Some(Admission::new(camera, view.viewport)), content: Some(content), eye: Some(eye) }
    }

    /// Whether the view admits `province`.
    pub(crate) fn province(&self, province: u32) -> Option<bool> {
        Some(self.admission.as_ref()?.admits_province(&self.content?.counters.bounds, province))
    }

    /// Whether the view admits a province of `state`.
    pub(crate) fn state(&self, state: u32) -> Option<bool> {
        let provinces = self.content?.icons.raids.trigger_content.state_provinces.get(&state);
        self.admission.as_ref()?;
        Some(provinces.is_some_and(|provinces| provinces.iter().any(|&province| self.province(province) == Some(true))))
    }

    /// Whether ground within `reach` of `at` is within `hearing` of the eye.
    pub(crate) fn heard(&self, at: [f32; 2], reach: f32, hearing: Option<f32>) -> Option<bool> {
        let (eye, hearing) = (self.eye?, hearing?);
        let [dx, dz] = [eye[0] - at[0], eye[2] - at[1]];
        Some(dx * dx + dz * dz <= (hearing + reach) * (hearing + reach))
    }

    /// Whether any ground of `province`'s box is within `hearing` of the eye.
    pub(crate) fn province_heard(&self, province: u32, hearing: Option<f32>) -> Option<bool> {
        let (eye, hearing) = (self.eye?, hearing?);
        let Some(&[min_x, min_z, max_x, max_z]) = self.content?.counters.bounds.get(&province) else { return Some(false) };
        let [dx, dz] = [eye[0] - eye[0].clamp(min_x, max_x), eye[2] - eye[2].clamp(min_z, max_z)];
        Some(dx * dx + dz * dz <= hearing * hearing)
    }
}

/// Entities a layer's settles made since last asked, and of them those the view shows (for sounds,
/// those within hearing); none in view when the frame does not count it.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct Scope {
    pub settled: u64,
    pub in_view: Option<u64>,
}

/// A layer's settled entities by where they stand, counted against the view at [`Layer::scope`].
#[derive(Debug, Default)]
pub(crate) struct Scoped {
    settled: u64,
    provinces: Vec<u32>,
    states: Vec<u32>,
    heard: Vec<([f32; 2], f32)>,
    heard_provinces: Vec<u32>,
}

impl Scoped {
    pub(crate) fn province(&mut self, province: u32) {
        self.settled += 1;
        if counting_in_view() {
            self.provinces.push(province);
        }
    }

    pub(crate) fn state(&mut self, state: u32) {
        self.settled += 1;
        if counting_in_view() {
            self.states.push(state);
        }
    }

    /// One whose sound reaches `reach` from `at`.
    pub(crate) fn heard(&mut self, at: [f32; 2], reach: f32) {
        self.settled += 1;
        if counting_in_view() {
            self.heard.push((at, reach));
        }
    }

    /// One heard when the eye is within hearing of `province`'s ground.
    pub(crate) fn heard_province(&mut self, province: u32) {
        self.settled += 1;
        if counting_in_view() {
            self.heard_provinces.push(province);
        }
    }

    /// One with no place on the map: settled, never in view.
    pub(crate) fn unplaced(&mut self) {
        self.settled += 1;
    }

    /// The count since the last, against `sight` (`hearing`: the distance sounds carry), emptied.
    pub(crate) fn take(&mut self, sight: &Sight<'_>, hearing: Option<f32>) -> Scope {
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

    pub(crate) fn bytes(&self) -> usize {
        (self.provinces.capacity() + self.states.capacity() + self.heard_provinces.capacity()) * 4 + self.heard.capacity() * 12
    }

    /// The count `other` made (on a worker) added to this one's.
    pub(crate) fn extend(&mut self, other: Self) {
        self.settled += other.settled;
        self.provinces.extend(other.provinces);
        self.states.extend(other.states);
        self.heard.extend(other.heard);
        self.heard_provinces.extend(other.heard_provinces);
    }
}

/// The clock the frame side's budget runs on: the real one, or in a test one that stands still
/// until the test moves it, so a budget test counts work instead of timing it.
pub(crate) mod clock {
    use std::time::Instant;

    #[cfg(test)]
    thread_local! {
        static HELD: std::cell::Cell<Option<Instant>> = const { std::cell::Cell::new(None) };
    }

    pub(crate) fn now() -> Instant {
        #[cfg(test)]
        if let Some(at) = HELD.with(std::cell::Cell::get) {
            return at;
        }
        Instant::now()
    }

    /// From now on this thread's clock stands still but for [`advance`].
    #[cfg(test)]
    pub(crate) fn hold() {
        HELD.with(|held| held.set(Some(Instant::now())));
    }

    #[cfg(test)]
    pub(crate) fn advance(by: std::time::Duration) {
        HELD.with(|held| held.set(held.get().map(|at| at + by)));
    }
}

/// The slice of a frame settle and upload may take, shared by every layer.
#[derive(Debug)]
pub(crate) struct Budget {
    until: Option<Instant>,
    spent: bool,
    /// The next `left()` answers yes whatever the time ([`Budget::first_free`]).
    free: bool,
    /// The times `left()` answered yes: the units of work given, as a layer asks before each.
    given: u32,
}

impl Budget {
    pub(crate) fn new(slice: Duration) -> Self {
        Self { until: Some(clock::now() + slice), spent: false, free: false, given: 0 }
    }

    /// No limit: compose and the proof tool's settled comparisons.
    pub(crate) fn unlimited() -> Self {
        Self { until: None, spent: false, free: false, given: 0 }
    }

    /// Whether it has no limit: a layer with work on its own worker waits for it then.
    pub(crate) fn unlimited_now(&self) -> bool {
        self.until.is_none()
    }

    /// This budget ending at most `slice` from now: a layer's own cap. Unlimited stays unlimited.
    pub(crate) fn capped(&self, slice: Duration) -> Self {
        Self { until: self.until.map(|until| until.min(Instant::now() + slice)), spent: self.spent, free: self.free, given: 0 }
    }

    /// This budget, but its first `left()` answers yes even when out of time: one unit of work
    /// always gets done (one entity, one whole upload, one phase of a phased one).
    pub(crate) fn first_free(&self) -> Self {
        Self { until: self.until, spent: self.spent, free: true, given: 0 }
    }

    /// The units of work this budget gave (`left()` answered yes).
    pub(crate) fn given(&self) -> u32 {
        self.given
    }

    /// Whether `left()` would answer yes, without taking the free unit: a gate before the work's own
    /// questions, which leaves that unit to the first of them.
    pub(crate) fn peek(&mut self) -> bool {
        if !self.spent && self.until.is_some_and(|until| clock::now() >= until) {
            self.spent = true;
        }
        self.free || !self.spent
    }

    /// What a budget made from this one ([`Self::capped`]) gave, as this one's: its units counted
    /// here, and the free unit gone once it was taken there.
    pub(crate) fn absorb(&mut self, part: &Self) {
        self.given += part.given;
        if part.given > 0 {
            self.free = false;
        }
    }

    /// Whether there is time left; once out, it stays out.
    pub(crate) fn left(&mut self) -> bool {
        if !self.spent && self.until.is_some_and(|until| clock::now() >= until) {
            self.spent = true;
        }
        let left = std::mem::take(&mut self.free) || !self.spent;
        self.given += u32::from(left);
        left
    }
}

/// A layer's dirty entities by index, each once, in the order they were first marked, each with
/// the frame it was marked at: the oldest still owed is the first.
#[derive(Debug, Default)]
pub(crate) struct Dirty {
    marked: Vec<u64>,
    list: Vec<(u32, u64)>,
}

impl Dirty {
    /// Marks `entity` at `frame`; whether it was not marked already.
    pub(crate) fn mark(&mut self, entity: u32, frame: u64) -> bool {
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

    pub(crate) fn len(&self) -> usize {
        self.list.len()
    }

    /// The frame the oldest entity still dirty was marked at.
    pub(crate) fn since(&self) -> Option<u64> {
        self.list.first().map(|&(_, frame)| frame)
    }

    /// Takes entities while `budget` lasts, `each` recomputing one; checks the clock every 32.
    pub(crate) fn settle(&mut self, budget: &mut Budget, mut each: impl FnMut(u32) -> io::Result<()>) -> io::Result<()> {
        let mut done = 0;
        while done < self.list.len() {
            if done % 32 == 0 && !budget.left() {
                break;
            }
            let (entity, _) = self.list[done];
            self.marked[entity as usize / 64] &= !(1 << (entity % 64));
            done += 1;
            each(entity)?;
        }
        self.list.drain(..done);
        Ok(())
    }

    pub(crate) fn clear(&mut self) {
        self.list.iter().for_each(|&(entity, _)| self.marked[entity as usize / 64] = 0);
        self.list.clear();
    }

    pub(crate) fn bytes(&self) -> usize {
        self.marked.capacity() * 8 + self.list.capacity() * 12
    }
}

/// A number column a layer reads, typed: `T` is the column's kind.
#[derive(Debug)]
pub(crate) struct Num<T> {
    pub(crate) table: TableId,
    pub(crate) column: usize,
    kind: PhantomData<fn() -> T>,
}

#[cfg(test)]
impl<T> Num<T> {
    pub(crate) fn of(table: TableId, column: usize) -> Self {
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
pub(crate) struct Text {
    pub(crate) table: TableId,
    pub(crate) column: usize,
}

/// A list or opaque column: each cell's bytes.
#[derive(Clone, Copy, Debug)]
pub(crate) struct List {
    pub(crate) table: TableId,
    pub(crate) column: usize,
}

/// A number type a column is read as, and the kinds that hold it.
pub(crate) trait Number: Copy {
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

/// What the drawn layers read of one table: columns (ascending), and whether rows' keys are
/// wanted.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct Read {
    pub columns: Vec<usize>,
    pub keyed: bool,
}

/// What a layer reads at a view, named by table and field and resolved against the recording's
/// schema once, when the layer declares.
pub(crate) struct Inputs<'a> {
    kinds: &'a [Vec<Kind>],
    layer: &'static str,
    /// By table index: its columns, and whether its rows' keys are wanted.
    tables: BTreeMap<usize, Read>,
    /// The layers it reads, by name: registered before it.
    after: Vec<&'static str>,
}

impl<'a> Inputs<'a> {
    pub(crate) fn new(kinds: &'a [Vec<Kind>], layer: &'static str) -> Self {
        Self { kinds, layer, tables: BTreeMap::new(), after: Vec::new() }
    }

    /// Reads `table`'s rows, with their keys when `keyed`, and no column yet.
    pub(crate) fn table(&mut self, table: TableId, keyed: bool) -> io::Result<()> {
        self.kinds_of(table)?;
        let entry = self.tables.entry(table.index()).or_default();
        entry.keyed |= keyed;
        Ok(())
    }

    /// Reads the product of layer `name`, registered before it: its `compose`, `apply` and
    /// `settle` come after that layer's in each frame, and it composes whenever that layer does.
    pub(crate) fn after(&mut self, name: &'static str) {
        if !self.after.contains(&name) {
            self.after.push(name);
        }
    }

    /// The layers it reads, by name.
    pub(crate) fn upstream(&self) -> &[&'static str] {
        &self.after
    }

    pub(crate) fn number<T: Number>(&mut self, table: TableId, field: &str) -> io::Result<Num<T>> {
        let (column, kind) = self.column(table, field)?;
        if !T::KINDS.contains(&kind) {
            return Err(self.refuse(format!("{table:?}.{field} is a {kind:?} column, not a {}", std::any::type_name::<T>())));
        }
        Ok(Num { table, column, kind: PhantomData })
    }

    pub(crate) fn text(&mut self, table: TableId, field: &str) -> io::Result<Text> {
        let (column, kind) = self.column(table, field)?;
        if kind != Kind::Bytes {
            return Err(self.refuse(format!("{table:?}.{field} is a {kind:?} column, not text")));
        }
        Ok(Text { table, column })
    }

    pub(crate) fn list(&mut self, table: TableId, field: &str) -> io::Result<List> {
        let (column, kind) = self.column(table, field)?;
        if !matches!(kind, Kind::U32List | Kind::U64List | Kind::Opaque) {
            return Err(self.refuse(format!("{table:?}.{field} is a {kind:?} column, not a list")));
        }
        Ok(List { table, column })
    }

    /// A column by its index in the table, whatever its kind.
    pub(crate) fn index(&mut self, table: TableId, column: usize) -> io::Result<()> {
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
    pub(crate) fn into_tables(self) -> BTreeMap<usize, Read> {
        self.tables
    }
}
