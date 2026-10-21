//! Viewer-owned lifetimes, popup scales and animation histories for unit entities.
//! All updates use the viewer's frame clock.
//!
//! Popup fades change uniform transform scale. Each visit subtracts frame time
//! times the content's scale-down speed, clamped at zero; destruction happens
//! when the old scale was already zero. Formation roots use
//! `SHIP_POPUP_SCALE_DOWN_SPEED`, carrying their hull offsets. Both land and
//! formation entities grow at `MESH_POPUP_SCALE_UP_SPEED`.
//!
//! A newly hidden kind is visited in both the gate-loss loop and inactive pass;
//! subsequent frames visit once. In-view provinces update every frame. Off-screen
//! provinces follow the shared modulo-100 schedule ([`viewer_foundation::map::province_schedule`])
//! and retain entities, transforms and advancing animation clocks until that
//! visit destroys them. Selected units bypass the phase using local selection.
//!
//! Divisions, transports and railway guns share the land path; naval formations
//! use the ship path. Land-gate loss deactivates naval combat graphics. Aircraft
//! and trains have their own managers. [`crate::map::mesh_budget`] owns the shared
//! creation allowance.
//!
//! The 38 visual buckets retain reconstructed land order with identity ties.
//! Naval representatives use recorded occupant order and viewer-owned animator
//! and selection presentation ([`naval`]). Deferred land selection runs in
//! parallel, so original completion and RNG order are unavailable; all queued
//! creations still become visible on one frame.

#[cfg(test)]
use std::time::Instant;
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
};
use viewer_foundation::map::province_admission::province_boxes;
use viewer_foundation::map::province_map::ProvinceMap;
use viewer_foundation::map::province_schedule::Shared;
use viewer_foundation::unit_handle::Unit;

use {
    crate::map::map_buildings::BuildingHistory, crate::map::mesh_budget::Budget,
    crate::map::models::ModelField, crate::map::models::ModelInstance,
    viewer_foundation::map::camera::Camera, viewer_foundation::map::model_admission::Gates,
    viewer_foundation::map::province_admission::Admission,
    viewer_foundation::map::province_schedule::Update,
};

pub mod naval;
pub mod naval_combat;
mod provinces;
use provinces::Provinces;
#[cfg(test)]
mod empty_tests;
pub use naval::Sources as NavalSources;

/// The model path an entity follows.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Kind {
    Land,
    Ship,
}

/// One figure or hull an hour places.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Target {
    pub province: u32,
    /// The stack's oldest division, or the formation.
    pub identity: u64,
    /// The hull within a formation; 0 for a land figure.
    pub part: u32,
    /// Land: the reconstructed native visual slot the game walks in
    /// order; stacks the reconstruction puts in one slot follow `identity`.
    pub order: u32,
    /// The ship a hull shows (the game keeps a hull while its ship and
    /// graphics stay); a land figure's representative.
    pub occupant: u64,
    pub kind: Kind,
    pub mesh: usize,
    pub instance: ModelInstance,
    /// Formation root; shrinking also moves hull attachments toward it.
    pub origin: [f32; 3],
}

/// What one native unit slot holds: a land stack or a naval formation.
type Slot = (Kind, u64);

impl Target {
    fn slot(self) -> Slot {
        (self.kind, self.identity)
    }
}

/// An hour's models: the field keeps their meshes resident, the targets
/// feed the [`History`].
#[derive(Clone, Default)]
pub struct Field {
    pub models: ModelField,
    pub targets: Targets,
    /// The raid arrows' units where the hour placed them ([`History::set_raids`]).
    pub raids: Vec<(usize, ModelInstance)>,
    pub naval: NavalSources,
}

/// One slot's targets, by part.
type Parts = BTreeMap<u32, Target>;

/// Immutable hourly targets, grouped on the builder before a view installs them.
/// Iteration retains placement order; duplicate slot parts still use the last target.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Targets(Arc<PreparedTargets>);

#[derive(Debug, Default, PartialEq)]
struct PreparedTargets {
    placed: Vec<Target>,
    provinces: BTreeMap<u32, Arc<ProvinceTargets>>,
}

#[derive(Debug, Default, PartialEq)]
struct ProvinceTargets {
    slots: BTreeMap<Slot, Parts>,
    lands: Arc<[(u32, Slot)]>,
}

impl From<Vec<Target>> for Targets {
    fn from(placed: Vec<Target>) -> Self {
        let mut provinces = BTreeMap::<u32, ProvinceTargets>::new();
        for &target in &placed {
            provinces
                .entry(target.province)
                .or_default()
                .slots
                .entry(target.slot())
                .or_default()
                .insert(target.part, target);
        }
        for province in provinces.values_mut() {
            let mut lands: Vec<_> = province
                .slots
                .iter()
                .filter(|(slot, _)| slot.0 == Kind::Land)
                .map(|(slot, parts)| (parts.first_key_value().unwrap().1.order, *slot))
                .collect();
            lands.sort_unstable();
            province.lands = lands.into();
        }
        Self(Arc::new(PreparedTargets {
            placed,
            provinces: provinces
                .into_iter()
                .map(|(id, targets)| (id, Arc::new(targets)))
                .collect(),
        }))
    }
}

impl std::ops::Deref for Targets {
    type Target = [Target];

    fn deref(&self) -> &Self::Target {
        &self.0.placed
    }
}

/// One figure, or one hull attached under a formation's root.
#[cfg_attr(test, derive(Debug, PartialEq))]
struct Part {
    target: Target,
    epoch: u32,
    identity: u32,
    /// Retained attachment's explicit graphics string.
    graphics: Vec<u8>,
}

impl Part {
    /// The game retains each unchanged hull's own entity. Dumps sample
    /// epoch zero, but same-model hulls still need distinct live players.
    fn new(target: Target, identity: &mut u32, steady: bool) -> Self {
        *identity = identity.wrapping_add(1);
        Self {
            target,
            epoch: if steady { 0 } else { *identity },
            identity: *identity,
            graphics: Vec::new(),
        }
    }
}

/// A slot's entity. Its popup scale belongs to the slot, so the
/// hulls of a formation share their root's.
#[cfg_attr(test, derive(Debug, PartialEq))]
struct Entity {
    kind: Kind,
    scale: f32,
    /// Shrinking through the inactive path, which drops the transport scale.
    shrinking: bool,
    parts: BTreeMap<u32, Part>,
}

impl Entity {
    /// One fade visit: it tests the OLD scale, then subtracts, so zero
    /// lasts one visit. False means the entity is destroyed.
    fn shrink(&mut self, delta: f32, gates: Gates) -> bool {
        if self.scale <= 0.0 {
            return false;
        }
        let speed = match self.kind {
            Kind::Land => gates.scale_down,
            Kind::Ship => gates.ship_scale_down,
        };
        self.scale = (self.scale - delta * speed).max(0.0);
        self.shrinking = true;
        true
    }

    fn placed(&self) -> impl Iterator<Item = (usize, ModelInstance)> + '_ {
        self.parts.values().map(|part| {
            let mut instance = part.target.instance;
            instance.animation_epoch = part.epoch;
            instance.animation_identity = part.identity;
            // Inactive land models use multiplier 1.0, including
            // transports; the admitted path uses the authored unit multiplier.
            instance.scale = if self.shrinking && self.kind == Kind::Land {
                self.scale
            } else {
                instance.scale * self.scale
            };
            if self.kind == Kind::Ship {
                instance.position = std::array::from_fn(|axis| {
                    let root = part.target.origin[axis];
                    root + (instance.position[axis] - root) * self.scale
                });
            }
            (part.target.mesh, instance)
        })
    }
}

/// What one viewer frame applies to each province it updates.
#[derive(Clone, Copy)]
struct Frame {
    delta: f32,
    gates: Gates,
    /// Whether the camera shows land models and ships.
    shown: [bool; 2],
    /// Whether it shows unit arrows.
    arrows: bool,
    steady: bool,
}

/// One province's unit graphics.
#[derive(Default)]
#[cfg_attr(test, derive(Debug, PartialEq))]
struct Province {
    /// The land and ship show bits of its last update.
    shown: [bool; 2],
    /// The arrow flag of its last update.
    arrows: bool,
    entities: BTreeMap<Slot, Entity>,
    /// Hourly land targets in visual-slot order, with identity ties.
    lands: Arc<[(u32, Slot)]>,
    /// The round-robin cursor: the visual slot
    /// ([`Target::order`]) it last visited, not its successor, whichever
    /// representative holds it now. Recreating units does not reset it.
    land_cursor: u32,
    /// Visits since each land slot was last queued.
    land_visits: BTreeMap<Slot, u16>,
}

impl Province {
    /// Queues land creations against what the counter leaves,
    /// then grows existing figures and attaches hulls, which spend it inline
    /// The queued land is created after every province's
    /// visit ([`Province::create_land`]).
    fn visit<'a>(
        &mut self,
        targets: Option<&BTreeMap<Slot, Parts>>,
        formations: impl Clone + Iterator<Item = &'a naval::Formation>,
        frame: Frame,
        epoch: &mut u32,
        budget: &mut Budget,
        mut queue: impl FnMut(Slot, bool),
    ) {
        let placed = |slot: &Slot| {
            targets.is_some_and(|targets| targets.contains_key(slot))
                || formations
                    .clone()
                    .any(|formation| *slot == (Kind::Ship, formation.identity))
        };
        let was = self.shown;
        let mut queued = 0;
        if frame.shown[Kind::Land as usize] {
            let lands = &self.lands;
            let start = lands.partition_point(|&(order, _)| order < self.land_cursor);
            for &(order, slot) in lands[start..].iter().chain(&lands[..start]) {
                if queued >= budget.remaining() {
                    break;
                }
                self.land_cursor = order;
                let visits = self.land_visits.entry(slot).or_default();
                *visits += 1;
                // A slot is queued when its representative changed, which
                // here means it has no entity, or on its hundredth visit.
                if frame.steady || *visits > 99 || !self.entities.contains_key(&slot) {
                    *visits = 0;
                    queued += 1;
                    queue(slot, was[Kind::Land as usize]);
                }
            }
        }
        self.land_visits.retain(|slot, _| placed(slot));
        self.entities.retain(|slot, entity| {
            let kind = slot.0 as usize;
            if frame.steady {
                return frame.shown[kind] && placed(slot);
            }
            if !frame.shown[kind] {
                // The gate-loss loop precedes the inactive fade pass, so the first frame
                // out of range visits twice.
                return (!was[kind] || entity.shrink(frame.delta, frame.gates))
                    && entity.shrink(frame.delta, frame.gates);
            }
            // A missing stack of a kind shown now and before is destroyed at
            // once when popup growth is requested; otherwise it shrinks.
            placed(slot) || (!was[kind] && entity.shrink(frame.delta, frame.gates))
        });
        for (&slot, parts) in targets.into_iter().flatten() {
            let kind = slot.0 as usize;
            if !frame.shown[kind] {
                continue;
            }
            if slot.0 == Kind::Land {
                if let Some(entity) = self.entities.get_mut(&slot) {
                    for (&index, &target) in parts {
                        if let Some(part) = entity.parts.get_mut(&index) {
                            // Transform/state follow this frame; the definition waits
                            // for the game's periodic selection or pointer change.
                            let mesh = part.target.mesh;
                            part.target = target;
                            part.target.mesh = mesh;
                        }
                    }
                    entity.shrinking = false;
                    entity.scale = if frame.steady {
                        1.0
                    } else {
                        (entity.scale + frame.delta * frame.gates.scale_up).min(1.0)
                    };
                }
                continue;
            }
            // The game acquires a formation root outside the counter, full
            // size where ships were already shown, and grows it even while
            // all its hulls wait.
            let entity = self.entities.entry(slot).or_insert_with(|| Entity {
                kind: slot.0,
                scale: if frame.steady || was[kind] { 1.0 } else { 0.0 },
                shrinking: false,
                parts: BTreeMap::new(),
            });
            // Hulls attach and detach under the root without touching its
            // scale, slot 0 first.
            entity.parts.retain(|part, _| parts.contains_key(part));
            for (&index, &target) in parts {
                let replace = entity.parts.get(&index).is_none_or(|part| {
                    part.target.mesh != target.mesh || part.target.occupant != target.occupant
                });
                if replace {
                    // The game clears a stale support slot before attaching;
                    // slot 0 attaches directly and keeps its old hull if held.
                    if index != 0 {
                        entity.parts.remove(&index);
                    }
                    if !budget.take() {
                        continue;
                    }
                    entity
                        .parts
                        .insert(index, Part::new(target, epoch, frame.steady));
                } else if let Some(part) = entity.parts.get_mut(&index) {
                    part.target = target;
                }
            }
            entity.shrinking = false;
            entity.scale = if frame.steady {
                1.0
            } else {
                (entity.scale + frame.delta * frame.gates.scale_up).min(1.0)
            };
        }
        // The game walks native visual slots, not task-force handles. A land
        // representative in the same slot admits no ordinary naval formation.
        for formation in formations {
            let entity = self
                .entities
                .entry((Kind::Ship, formation.identity))
                .or_insert_with(|| Entity {
                    kind: Kind::Ship,
                    scale: if frame.steady || was[Kind::Ship as usize] {
                        1.0
                    } else {
                        0.0
                    },
                    shrinking: false,
                    parts: BTreeMap::new(),
                });
            formation.update(entity, frame, epoch, budget);
            entity.shrinking = false;
            entity.scale = if frame.steady {
                1.0
            } else {
                (entity.scale + frame.delta * frame.gates.scale_up).min(1.0)
            };
        }
        self.arrows = frame.arrows;
    }

    fn finalize_shown(&mut self, frame: Frame, budget: &Budget) {
        // The game keeps a newly shown kind's bit clear while the counter is
        // spent, so what it creates next frame still grows from zero.
        for kind in 0..2 {
            if budget.remaining() != 0 || !frame.shown[kind] {
                self.shown[kind] = frame.shown[kind];
            }
        }
    }

    /// Resolve a queued land-creation request, recreating
    /// the figure only if its definition changed, at the scale of the show
    /// bit the request remembered.
    fn create_land(
        &mut self,
        slot: Slot,
        parts: &Parts,
        was_shown: bool,
        steady: bool,
        epoch: &mut u32,
        budget: &mut Budget,
    ) {
        let changed = self.entities.get(&slot).is_none_or(|entity| {
            entity.parts.len() != parts.len()
                || parts.iter().any(|(index, target)| {
                    entity
                        .parts
                        .get(index)
                        .is_none_or(|part| part.target.mesh != target.mesh)
                })
        });
        if !changed {
            return;
        }
        // The game has no budget guard; queued work can exceed the limit.
        budget.charge();
        self.entities.insert(
            slot,
            Entity {
                kind: Kind::Land,
                scale: if steady || was_shown { 1.0 } else { 0.0 },
                shrinking: false,
                parts: parts
                    .iter()
                    .map(|(&index, &target)| (index, Part::new(target, epoch, steady)))
                    .collect(),
            },
        );
    }

    /// The phase update of a province out of view. The gate-loss loops
    /// destroy each kind shown last update, and a changed arrow flag forces
    /// the inactive pass, which destroys the rest. Otherwise the update
    /// returns early and a kind already shrinking keeps its entity.
    fn leave(&mut self, steady: bool) {
        let (shown, arrows) = (self.shown, self.arrows);
        self.entities
            .retain(|slot, _| !steady && !shown[slot.0 as usize] && !arrows);
        self.shown = [false; 2];
        self.arrows = false;
    }
}

/// One frame's unit and (when changed) building instance streams.
pub struct FrameModels {
    pub units: Vec<(usize, ModelInstance)>,
    pub buildings: Option<Vec<(usize, ModelInstance)>>,
}

/// Every province's unit entities, advanced once per viewer frame.
#[derive(Default)]
pub struct History {
    /// The hour's targets by province and slot.
    targets: Targets,
    naval: naval::History,
    combats: naval_combat::History,
    provinces: Provinces,
    /// The shared schedule's reached updates in native province order.
    updates: Vec<(u32, Update)>,
    /// Deferred land requests, retaining capacity between frames and province visits.
    queued: Vec<(u32, Slot, bool)>,
    #[cfg(test)]
    queue_trace: Vec<(u32, Slot, bool)>,
    /// Admission operands in province order, refreshed only when the hour installs a roster.
    province_bounds: Vec<(u32, Option<[f32; 4]>)>,
    /// Province boxes for camera admission.
    bounds: Arc<BTreeMap<u32, [f32; 4]>>,
    /// The previous frame's animation clock.
    last: Option<f32>,
    /// The view's province phase and dirty bits, shared with its counters ([`Self::share_schedule`]).
    schedule: Shared,
    /// The last incarnation handed out.
    epoch: u32,
    /// Offline frames settle for their camera instead of animating.
    steady: bool,
    /// The raid arrows' units. No unit slot holds them:
    /// The game creates its entity at once
    /// without testing or charging the shared creation counter
    /// ([`crate::map::mesh_budget`]), and
    /// The game shows it at every camera height with no
    /// popup scale, so they stand on every frame whatever the gates.
    raids: Vec<(usize, ModelInstance)>,
    /// The raid units' incarnation: a load-equivalent reset recreates them
    /// too, so their animation starts again ([`History::reset`]).
    raid_epoch: u32,
    /// Select the independent full-walk oracle without changing process environment.
    #[cfg(test)]
    empty_fast_path: Option<bool>,
}

impl History {
    pub fn set_selection(&mut self, selected: &BTreeSet<Unit>) {
        self.naval.set_selection(selected);
    }

    /// Arrow admission is map geometry; a refused 3D entity library must not suppress it.
    pub fn prepare_arrow_bounds(
        &mut self,
        map: &installed_content_core::map::MapData,
        known: &Arc<BTreeMap<u32, [f32; 4]>>,
    ) {
        if !self.bounds.is_empty() {
            return;
        }
        self.bounds = if known.is_empty() {
            Arc::new(province_boxes(&ProvinceMap {
                width: map.width,
                height: map.height,
                ids: &map.province_ids,
            }))
        } else {
            Arc::clone(known)
        };
        self.provinces.ensure(self.bounds.keys().copied());
        self.provinces.prepare_inputs(&self.targets, &self.naval);
        self.prepare_province_bounds();
    }

    fn prepare_province_bounds(&mut self) {
        self.province_bounds.clear();
        self.province_bounds.extend(
            self.provinces
                .keys()
                .map(|&id| (id, self.bounds.get(&id).copied())),
        );
    }

    pub fn select_provinces(&mut self, provinces: BTreeSet<u32>) {
        self.schedule
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .set_selected(provinces);
    }

    /// This frame's native province arrow flag, shared rather than independently reculling
    /// counters, entities and their selected routes.
    pub fn arrows_admitted(&self, province: u32) -> bool {
        self.provinces
            .get(&province)
            .is_some_and(|province| province.arrows)
    }
    /// Recorded roster/route inputs; mutable animator and attachment state
    /// belongs to this viewer's frame history.
    pub fn set_naval(&mut self, sources: NavalSources) {
        let changed = self
            .provinces
            .ensure(sources.candidates.iter().map(|candidate| candidate.province));
        self.naval.install(sources);
        {
            let _join_span = viewer_foundation::profiling::span("units_join_naval");
            self.provinces.prepare_inputs(&self.targets, &self.naval);
        }
        if changed {
            self.prepare_province_bounds();
        }
    }
    /// Groups and installs targets from a direct caller. Hour builders prepare
    /// the shared index in advance and use [`Self::set_prepared`] instead.
    pub fn set(
        &mut self,
        targets: Vec<Target>,
        bounds: Arc<BTreeMap<u32, [f32; 4]>>,
        steady: bool,
    ) {
        self.set_prepared(targets.into(), bounds, steady);
    }

    /// Installs an hour's prepared targets. Entities carry over; the next frame
    /// admits, grows or retires them. Only provinces whose target lists occur in
    /// either hour need a new land list; empty provinces keep their show bits.
    pub fn set_prepared(
        &mut self,
        targets: Targets,
        bounds: Arc<BTreeMap<u32, [f32; 4]>>,
        steady: bool,
    ) {
        self.set_prepared_join(targets, bounds, steady, true);
    }

    /// Installs targets and naval inputs in their usual order, joining the final inputs once.
    /// Standalone setters still prepare immediately for callers that install only one source.
    pub fn set_prepared_with_naval(
        &mut self,
        targets: Targets,
        bounds: Arc<BTreeMap<u32, [f32; 4]>>,
        steady: bool,
        sources: NavalSources,
    ) {
        // Keep the setters' ensure and bounds-refresh order, but join only after both
        // sources are installed. Independent setters still join immediately.
        self.set_prepared_join(targets, bounds, steady, false);
        self.set_naval(sources);
    }

    fn set_prepared_join(
        &mut self,
        targets: Targets,
        bounds: Arc<BTreeMap<u32, [f32; 4]>>,
        steady: bool,
        join: bool,
    ) {
        let mut changed = false;
        if !bounds.is_empty() && !Arc::ptr_eq(&self.bounds, &bounds) {
            // Native updates the show bits of empty provinces too: a stack
            // arriving in a province already shown starts at full scale.
            self.provinces.ensure(bounds.keys().copied());
            self.bounds = bounds;
            changed = true;
        }
        for id in self.targets.0.provinces.keys() {
            if !targets.0.provinces.contains_key(id) {
                self.provinces.get_mut(id).unwrap().lands = Arc::default();
            }
        }
        changed |= self.provinces.ensure(targets.0.provinces.keys().copied());
        for (&id, prepared) in &targets.0.provinces {
            let province = self.provinces.get_mut(&id).unwrap();
            province.lands = Arc::clone(&prepared.lands);
        }
        self.targets = targets;
        if join {
            let _join_span = viewer_foundation::profiling::span("units_join_prepared");
            self.provinces.prepare_inputs(&self.targets, &self.naval);
        }
        self.steady = steady;
        if changed {
            self.prepare_province_bounds();
        }
    }

    /// The raid units to stand this frame: the hour's placement, or a
    /// playing window's placement at its playback fraction.
    pub fn set_raids(&mut self, raids: Vec<(usize, ModelInstance)>) {
        if self.raid_epoch == 0 {
            self.epoch += 1;
            self.raid_epoch = self.epoch;
        }
        self.raids = raids;
    }

    #[cfg(test)]
    pub fn steady_for_test(&mut self, steady: bool) {
        self.steady = steady;
    }

    pub fn settle(&mut self) {
        self.steady = true;
    }

    pub fn restart_live(&mut self) {
        self.reset(None);
        self.last = None;
        self.steady = false;
    }

    /// Clears the unit slots of `provinces`, or of every province, as a load
    /// or a tag switch does ([`viewer_foundation::map::presentation::UnitRestart`]): their entities go and their
    /// show bits drop, so what shows next grows in on a new clock.
    pub fn reset(&mut self, provinces: Option<&BTreeSet<u32>>) {
        self.naval.reset(provinces);
        self.combats.reset(provinces);
        self.schedule
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .reset(provinces);
        // A load destroys the raid arrows and their units with the rest of
        // the map (the raid manager builds them again);
        // a tag switch shows the new country's raids, other raid ids.
        if provinces.is_none() && self.raid_epoch != 0 {
            self.epoch += 1;
            self.raid_epoch = self.epoch;
        }
        self.provinces.reset(provinces);
    }

    /// Runs on `schedule`, the view's counters' own, from now on: a view keeps
    /// one schedule for its units and its map icons.
    pub fn share_schedule(&mut self, schedule: &Shared) {
        if !Arc::ptr_eq(&self.schedule, schedule) {
            self.schedule = Arc::clone(schedule);
        }
    }

    /// The frame's advance of the schedule at this camera, made before the
    /// view's map icons read the dirty bits (province updates run
    /// before map-icon updates); the frame's walk
    /// ([`Self::update_models`]) then takes it instead of advancing again. A
    /// frame whose walk has not yet taken the last advance advances nothing.
    pub fn advance_schedule(&mut self, camera: Camera, viewport: [u32; 2]) {
        let mut schedule = self
            .schedule
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        schedule.advance_frame_boxes(&self.province_bounds, camera, viewport, self.steady);
    }

    /// The scene's single frame walk: buildings then units per province,
    /// followed by deferred land creation. Offline exports settle both histories.
    #[allow(clippy::too_many_arguments)]
    pub fn update_models(
        &mut self,
        camera: Camera,
        viewport: [u32; 2],
        time: f32,
        gates: Gates,
        limit: i32,
        buildings: &mut BuildingHistory,
        buildings_enabled: bool,
    ) -> FrameModels {
        let _update_span = viewer_foundation::profiling::span("step_update_models");
        let admission = std::cell::LazyCell::new(|| Admission::new(camera, viewport));
        let bounds = Arc::clone(&self.bounds);
        let mut buildings = buildings.frame(camera, viewport, time, buildings_enabled, self.steady);
        let mut budget = Budget::new(limit, self.steady);
        let placed = self.step_with(
            time,
            gates,
            [gates.units(camera.height), gates.ships(camera.height)],
            camera.height < gates.arrows,
            |id| admission.admits_province(&bounds, id),
            &mut budget,
            |id, budget| buildings.through(id, budget),
        );
        FrameModels {
            units: placed,
            buildings: buildings.finish(&mut budget),
        }
    }

    #[cfg(test)]
    fn update(
        &mut self,
        camera: Camera,
        viewport: [u32; 2],
        time: f32,
        gates: Gates,
    ) -> Vec<(usize, ModelInstance)> {
        self.update_models(
            camera,
            viewport,
            time,
            gates,
            i32::MAX,
            &mut BuildingHistory::default(),
            false,
        )
        .units
    }

    #[cfg(test)]
    pub fn step(
        &mut self,
        time: f32,
        gates: Gates,
        shown: [bool; 2],
        arrows: bool,
        in_view: impl Fn(u32) -> bool,
    ) -> Vec<(usize, ModelInstance)> {
        self.step_with(
            time,
            gates,
            shown,
            arrows,
            in_view,
            &mut Budget::new(i32::MAX, self.steady),
            |_, _| {},
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn step_with(
        &mut self,
        time: f32,
        gates: Gates,
        shown: [bool; 2],
        arrows: bool,
        in_view: impl Fn(u32) -> bool,
        budget: &mut Budget,
        mut buildings: impl FnMut(u32, &mut Budget),
    ) -> Vec<(usize, ModelInstance)> {
        if self.last.is_some_and(|previous| time < previous) {
            self.reset(None);
        }
        let frame = Frame {
            delta: self.last.map_or(0.0, |previous| (time - previous).max(0.0)),
            gates,
            shown,
            arrows,
            steady: self.steady,
        };
        self.last = Some(time);
        self.combats.begin_frame(frame.delta);
        // The native literal, advanced before the province updates (unless
        // the frame already advanced it for its map icons); never indexed by
        // the replay hour.
        let schedule_span = viewer_foundation::profiling::span("units_schedule");
        let schedule = Arc::clone(&self.schedule);
        let mut schedule = schedule
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if !schedule.take_pending() {
            schedule.advance(self.provinces.keys().copied(), &in_view, self.steady);
            schedule.take_pending();
        }
        // The frame's update list, the one the map icons' owners read. Buildings consume
        // every group through each reached province, so omitted unit visits leave their
        // order and shared budget unchanged.
        self.queued.clear();
        self.updates.clear();
        self.updates.extend(schedule.updates());
        self.updates.sort_unstable_by_key(|&(id, _)| id);
        drop(schedule_span);
        let fast_empty = true;
        #[cfg(test)]
        let fast_empty = self.empty_fast_path.unwrap_or(fast_empty);
        let provinces_span = viewer_foundation::profiling::span("units_provinces");
        let dense = &mut self.provinces;
        let mut at = 0;
        for &(id, update) in &self.updates {
            while at < dense.records.len() && dense.records[at].id < id {
                at += 1;
            }
            let Some(record) = dense.records.get_mut(at).filter(|record| record.id == id) else {
                continue;
            };
            let province = &mut record.province;
            buildings(id, budget);
            match update {
                Update::InView => {
                    let land_was_shown = province.shown[Kind::Land as usize];
                    let targets = record.targets.as_ref().map(|p| &p.slots);
                    if fast_empty
                        && province.entities.is_empty()
                        && targets.is_none_or(BTreeMap::is_empty)
                        && !record.has_candidates
                    {
                        // No slot can queue, grow or retire. Empty provinces still
                        // remember admission, and stale land visits lose membership.
                        province.land_visits.clear();
                        province.arrows = frame.arrows;
                        self.naval.clear_combat_selection();
                    } else {
                        province.visit(
                            targets,
                            self.naval.representatives(id, frame),
                            frame,
                            &mut self.epoch,
                            budget,
                            |slot, shown| self.queued.push((id, slot, shown)),
                        );
                    }
                    self.combats.visit(
                        id,
                        self.naval.combat(id),
                        frame,
                        land_was_shown,
                        &mut self.epoch,
                        budget,
                    );
                    province.finalize_shown(frame, budget);
                }
                Update::OffScreen => {
                    self.combats
                        .leave(id, province.shown[Kind::Land as usize], self.steady);
                    province.leave(self.steady);
                }
            }
            if province.entities.is_empty() {
                dense.populated.remove(&at);
            } else {
                dense.populated.insert(at);
            }
        }
        drop(provinces_span);
        buildings(u32::MAX, budget);
        let deferred_span = viewer_foundation::profiling::span("units_deferred");
        #[cfg(test)]
        self.queue_trace.clone_from(&self.queued);
        for (id, slot, shown) in self.queued.drain(..) {
            let dense = &mut self.provinces;
            let at = dense.indices[&id];
            let record = &mut dense.records[at];
            record.province.create_land(
                slot,
                &record.targets.as_ref().unwrap().slots[&slot],
                shown,
                self.steady,
                &mut self.epoch,
                budget,
            );
            if !record.province.entities.is_empty() {
                dense.populated.insert(at);
            }
        }
        drop(deferred_span);
        // Graphical province setters and probes run before idle updates
        // dispatches the animation manager. Admit each private combat player
        // once here, after this frame's callbacks and before its rendered pose.
        self.combats.advance(frame.delta);
        let raid_epoch = self.raid_epoch;
        let _collect_span = viewer_foundation::profiling::span("units_collect");
        let mut placed = self.provinces.placed();
        placed.extend(self.combats.placed());
        placed.extend(self.raids.iter().map(|&(mesh, instance)| {
            (
                mesh,
                ModelInstance {
                    animation_epoch: raid_epoch,
                    ..instance
                },
            )
        }));
        placed
    }

    pub fn combat_warnings(&self) -> Vec<String> {
        self.combats.warnings()
    }

    pub fn combat_clocks(
        &self,
    ) -> BTreeMap<(usize, u32, u32), crate::map::entity_animation::Clock> {
        self.combats.clocks()
    }
}

#[cfg(any(test, feature = "test-support"))]
pub mod test_support {
    use super::*;
    pub const GATES: Gates = Gates {
        units: 100.0,
        ships: 200.0,
        arrows: 300.0,
        transport_scale: 0.3,
        scale_up: 2.0,
        scale_down: 1.0,
        ship_scale_down: 0.5,
        naval_movement_speed: 12.0,
        land_movement_speed: 12.0,
    };

    pub fn target(province: u32, kind: Kind) -> Target {
        Target {
            province,
            kind,
            identity: u64::from(province),
            part: 0,
            order: 0,
            occupant: u64::from(province),
            mesh: 0,
            instance: ModelInstance {
                scale: 1.0,
                animation_seed: province,
                ..Default::default()
            },
            origin: [0.0; 3],
        }
    }
}

#[cfg(test)]
pub mod tests {
    use super::test_support::{GATES, target};
    use super::*;

    fn history(targets: Vec<Target>, steady: bool) -> History {
        let mut h = History::default();
        h.set(targets, Arc::default(), steady);
        h
    }

    fn near(h: &mut History, time: f32) -> Vec<(usize, ModelInstance)> {
        h.step(time, GATES, [true; 2], true, |_| true)
    }

    fn limited(h: &mut History, time: f32, limit: i32) -> Vec<(usize, ModelInstance)> {
        h.step_with(
            time,
            GATES,
            [true; 2],
            true,
            |_| true,
            &mut Budget::new(limit, h.steady),
            |_, _| {},
        )
    }

    impl Province {
        // Original per-province allocation/collection is the queue-order oracle.
        fn visit_collected<'a>(
            &mut self,
            targets: Option<&BTreeMap<Slot, Parts>>,
            formations: impl Clone + Iterator<Item = &'a naval::Formation>,
            frame: Frame,
            epoch: &mut u32,
            budget: &mut Budget,
        ) -> Vec<(Slot, bool)> {
            let placed = |slot: &Slot| {
                targets.is_some_and(|targets| targets.contains_key(slot))
                    || formations
                        .clone()
                        .any(|formation| *slot == (Kind::Ship, formation.identity))
            };
            let was = self.shown;
            let mut queued = Vec::new();
            if frame.shown[Kind::Land as usize] {
                let lands = &self.lands;
                let start = lands.partition_point(|&(order, _)| order < self.land_cursor);
                for &(order, slot) in lands[start..].iter().chain(&lands[..start]) {
                    if queued.len() >= budget.remaining() {
                        break;
                    }
                    self.land_cursor = order;
                    let visits = self.land_visits.entry(slot).or_default();
                    *visits += 1;
                    // A slot is queued when its representative changed, which
                    // here means it has no entity, or on its hundredth visit.
                    if frame.steady || *visits > 99 || !self.entities.contains_key(&slot) {
                        *visits = 0;
                        queued.push((slot, was[Kind::Land as usize]));
                    }
                }
            }
            self.land_visits.retain(|slot, _| placed(slot));
            self.entities.retain(|slot, entity| {
                let kind = slot.0 as usize;
                if frame.steady {
                    return frame.shown[kind] && placed(slot);
                }
                if !frame.shown[kind] {
                    // The gate-loss loop precedes the inactive fade pass, so the first frame
                    // out of range visits twice.
                    return (!was[kind] || entity.shrink(frame.delta, frame.gates))
                        && entity.shrink(frame.delta, frame.gates);
                }
                // A missing stack of a kind shown now and before is destroyed at
                // once when popup growth is requested; otherwise it shrinks.
                placed(slot) || (!was[kind] && entity.shrink(frame.delta, frame.gates))
            });
            for (&slot, parts) in targets.into_iter().flatten() {
                let kind = slot.0 as usize;
                if !frame.shown[kind] {
                    continue;
                }
                if slot.0 == Kind::Land {
                    if let Some(entity) = self.entities.get_mut(&slot) {
                        for (&index, &target) in parts {
                            if let Some(part) = entity.parts.get_mut(&index) {
                                // Transform/state follow this frame; the definition waits
                                // for the game's periodic selection or pointer change.
                                let mesh = part.target.mesh;
                                part.target = target;
                                part.target.mesh = mesh;
                            }
                        }
                        entity.shrinking = false;
                        entity.scale = if frame.steady {
                            1.0
                        } else {
                            (entity.scale + frame.delta * frame.gates.scale_up).min(1.0)
                        };
                    }
                    continue;
                }
                // The game acquires a formation root outside the counter, full
                // size where ships were already shown, and grows it even while
                // all its hulls wait.
                let entity = self.entities.entry(slot).or_insert_with(|| Entity {
                    kind: slot.0,
                    scale: if frame.steady || was[kind] { 1.0 } else { 0.0 },
                    shrinking: false,
                    parts: BTreeMap::new(),
                });
                // Hulls attach and detach under the root without touching its
                // scale, slot 0 first.
                entity.parts.retain(|part, _| parts.contains_key(part));
                for (&index, &target) in parts {
                    let replace = entity.parts.get(&index).is_none_or(|part| {
                        part.target.mesh != target.mesh || part.target.occupant != target.occupant
                    });
                    if replace {
                        // The game clears a stale support slot before attaching;
                        // slot 0 attaches directly and keeps its old hull if held.
                        if index != 0 {
                            entity.parts.remove(&index);
                        }
                        if !budget.take() {
                            continue;
                        }
                        entity
                            .parts
                            .insert(index, Part::new(target, epoch, frame.steady));
                    } else if let Some(part) = entity.parts.get_mut(&index) {
                        part.target = target;
                    }
                }
                entity.shrinking = false;
                entity.scale = if frame.steady {
                    1.0
                } else {
                    (entity.scale + frame.delta * frame.gates.scale_up).min(1.0)
                };
            }
            // The game walks native visual slots, not task-force handles. A land
            // representative in the same slot admits no ordinary naval formation.
            for formation in formations {
                let entity = self
                    .entities
                    .entry((Kind::Ship, formation.identity))
                    .or_insert_with(|| Entity {
                        kind: Kind::Ship,
                        scale: if frame.steady || was[Kind::Ship as usize] {
                            1.0
                        } else {
                            0.0
                        },
                        shrinking: false,
                        parts: BTreeMap::new(),
                    });
                formation.update(entity, frame, epoch, budget);
                entity.shrinking = false;
                entity.scale = if frame.steady {
                    1.0
                } else {
                    (entity.scale + frame.delta * frame.gates.scale_up).min(1.0)
                };
            }
            self.arrows = frame.arrows;
            queued
        }
    }

    #[test]
    fn retained_land_queue_matches_collected_queue_frame_by_frame() {
        let mut streamed: Vec<Province> = (0..32).map(|_| Province::default()).collect();
        let mut collected: Vec<Province> = (0..32).map(|_| Province::default()).collect();
        let mut streamed_epoch = 0;
        let mut collected_epoch = 0;
        let mut streamed_queue = Vec::new();
        for number in 0..420u32 {
            let hour = number / 11;
            let targets = Targets::from(
                (0..32u32)
                    .flat_map(|province| {
                        (0..7u64).filter_map(move |slot| {
                            if (u64::from(province) + slot + u64::from(hour)) % 5 == 0 {
                                return None;
                            }
                            Some(Target {
                                identity: u64::from(province) * 100 + slot,
                                order: ((slot + u64::from(hour)) % 4) as u32,
                                kind: if slot < 5 { Kind::Land } else { Kind::Ship },
                                mesh: (hour as usize + slot as usize) % 3,
                                instance: ModelInstance {
                                    scale: if slot == 1 { 0.3 } else { 1.0 },
                                    position: [province as f32, 10.0, slot as f32],
                                    animation_seed: province * 100 + slot as u32,
                                    ..Default::default()
                                },
                                ..target(province, Kind::Land)
                            })
                        })
                    })
                    .collect::<Vec<_>>(),
            );
            let frame = Frame {
                delta: if number % 29 == 0 { 0.0 } else { 0.125 },
                gates: GATES,
                shown: [number % 37 < 25, number % 31 < 21],
                arrows: number % 19 < 10,
                steady: number % 43 == 0,
            };
            let mut streamed_budget = Budget::new((number % 9) as i32, frame.steady);
            let mut collected_budget = Budget::new((number % 9) as i32, frame.steady);
            streamed_queue.clear();
            let mut collected_queue = Vec::new();
            for id in 0..32u32 {
                let prepared = targets.0.provinces.get(&id);
                let lands =
                    prepared.map_or_else(Arc::default, |prepared| Arc::clone(&prepared.lands));
                streamed[id as usize].lands = Arc::clone(&lands);
                collected[id as usize].lands = lands;
                if (number + id) % 17 == 0 {
                    streamed[id as usize].leave(frame.steady);
                    collected[id as usize].leave(frame.steady);
                    continue;
                }
                // Buildings charge the same shared allowance before this province.
                if id % 3 == 0 {
                    assert_eq!(streamed_budget.take(), collected_budget.take());
                }
                streamed[id as usize].visit(
                    prepared.map(|prepared| &prepared.slots),
                    std::iter::empty(),
                    frame,
                    &mut streamed_epoch,
                    &mut streamed_budget,
                    |slot, shown| streamed_queue.push((id, slot, shown)),
                );
                collected_queue.extend(
                    collected[id as usize]
                        .visit_collected(
                            prepared.map(|prepared| &prepared.slots),
                            std::iter::empty(),
                            frame,
                            &mut collected_epoch,
                            &mut collected_budget,
                        )
                        .into_iter()
                        .map(|(slot, shown)| (id, slot, shown)),
                );
                streamed[id as usize].finalize_shown(frame, &streamed_budget);
                collected[id as usize].finalize_shown(frame, &collected_budget);
            }
            assert_eq!(streamed_queue, collected_queue, "frame {number} queue");
            for &(id, slot, shown) in &streamed_queue {
                streamed[id as usize].create_land(
                    slot,
                    &targets.0.provinces[&id].slots[&slot],
                    shown,
                    frame.steady,
                    &mut streamed_epoch,
                    &mut streamed_budget,
                );
            }
            for (id, slot, shown) in collected_queue {
                collected[id as usize].create_land(
                    slot,
                    &targets.0.provinces[&id].slots[&slot],
                    shown,
                    frame.steady,
                    &mut collected_epoch,
                    &mut collected_budget,
                );
            }
            assert_eq!(streamed, collected, "frame {number} state");
            assert_eq!(
                streamed_epoch, collected_epoch,
                "frame {number} incarnations"
            );
            assert_eq!(
                streamed_budget.remaining(),
                collected_budget.remaining(),
                "frame {number} budget"
            );
            let placed = |provinces: &[Province]| {
                provinces
                    .iter()
                    .flat_map(|province| province.entities.values().flat_map(Entity::placed))
                    .collect::<Vec<_>>()
            };
            assert_eq!(
                placed(&streamed),
                placed(&collected),
                "frame {number} instances"
            );
        }
    }

    #[test]
    fn prepared_targets_keep_last_parts_and_native_land_order() {
        let first = Target {
            identity: 8,
            part: 1,
            order: 20,
            ..target(9, Kind::Land)
        };
        let lower_part = Target {
            part: 0,
            order: 40,
            ..first
        };
        let tied = Target {
            identity: 7,
            order: 30,
            ..lower_part
        };
        let replacement = Target {
            order: 30,
            mesh: 2,
            ..lower_part
        };
        let ship = target(9, Kind::Ship);
        let placed = vec![first, lower_part, tied, replacement, ship];
        let targets = Targets::from(placed.clone());
        assert_eq!(&*targets, &placed);
        let province = &targets.0.provinces[&9];
        assert_eq!(province.slots[&first.slot()][&0], replacement);
        assert_eq!(province.slots[&first.slot()][&1], first);
        assert_eq!(&*province.lands, &[(30, tied.slot()), (30, first.slot())]);
        let mut h = History::default();
        h.set_prepared(targets.clone(), Arc::default(), false);
        assert!(Arc::ptr_eq(&h.targets.0, &targets.0));
        assert!(Arc::ptr_eq(&h.provinces[&9].lands, &province.lands));
        h.provinces.get_mut(&9).unwrap().shown = [true; 2];
        h.set(vec![target(10, Kind::Land)], Arc::default(), false);
        assert!(h.provinces[&9].lands.is_empty());
        assert_eq!(h.provinces[&9].shown, [true; 2]);
        assert_eq!(h.province_bounds, [(9, None), (10, None)]);
    }

    #[test]
    fn skipped_unit_visits_keep_buildings_before_hulls_and_deferred_land() {
        for limit in 1..=4 {
            let mut h = history(
                vec![
                    Target {
                        mesh: 2,
                        ..target(2, Kind::Land)
                    },
                    Target {
                        mesh: 4,
                        ..target(4, Kind::Ship)
                    },
                ],
                false,
            );
            for id in [1, 3, 5] {
                h.provinces.ensure([id]);
                h.provinces.prepare_inputs(&h.targets, &h.naval);
            }
            let mut budget = Budget::new(limit, false);
            let mut remaining = [1, 3, 5].into_iter().peekable();
            let mut buildings = Vec::new();
            let placed = h.step_with(
                0.0,
                GATES,
                [true; 2],
                true,
                |id| id == 2 || id == 4,
                &mut budget,
                |id, budget| {
                    while remaining.peek().is_some_and(|next| *next <= id) {
                        let building = remaining.next().unwrap();
                        if budget.take() {
                            buildings.push(building);
                        }
                    }
                },
            );
            let expected_buildings = match limit {
                1 => vec![1],
                2 | 3 => vec![1, 3],
                _ => vec![1, 3, 5],
            };
            assert_eq!(buildings, expected_buildings);
            assert_eq!(placed.iter().any(|(mesh, _)| *mesh == 2), limit >= 2);
            assert_eq!(placed.iter().any(|(mesh, _)| *mesh == 4), limit >= 3);
            assert_eq!(budget.remaining(), 0);
        }
    }

    #[test]
    fn reached_updates_and_populated_provinces_keep_the_full_placement_walk() {
        let mut h = history(Vec::new(), false);
        for frame in 0..350u32 {
            if frame % 23 == 0 {
                h.set(
                    (1..=300)
                        .filter(|id| (id + frame / 23) % 17 == 0)
                        .map(|id| target(id, if id % 2 == 0 { Kind::Land } else { Kind::Ship }))
                        .collect(),
                    Arc::default(),
                    false,
                );
            }
            if frame % 41 == 0 {
                h.reset(Some(&BTreeSet::from([17, 34, 51])));
            }
            h.select_provinces(if frame % 13 < 7 {
                BTreeSet::from([17, 34])
            } else {
                BTreeSet::new()
            });
            let placed = h.step(
                frame as f32 / 120.0,
                GATES,
                [frame % 37 < 25, frame % 31 < 20],
                frame % 19 < 10,
                |id| (id + frame / 11) % 3 == 0,
            );
            let full: Vec<_> = h
                .provinces
                .iter()
                .flat_map(|(_, province)| province.entities.values().flat_map(Entity::placed))
                .collect();
            assert_eq!(placed, full, "frame {frame}");
            assert_eq!(
                h.provinces.populated_ids(),
                h.provinces
                    .iter()
                    .filter_map(|(&id, province)| (!province.entities.is_empty()).then_some(id))
                    .collect(),
                "frame {frame}"
            );
        }
        h.reset(None);
        assert!(h.provinces.populated_ids().is_empty());
    }

    #[test]
    fn retained_admission_operands_follow_roster_and_box_replacement() {
        let mut h = history(vec![target(88, Kind::Land)], false);
        assert_eq!(h.province_bounds, [(88, None)]);
        let bounds = Arc::new(BTreeMap::from([(1, [0., 0., 2., 2.])]));
        h.set(vec![target(99, Kind::Land)], bounds, false);
        assert_eq!(
            h.province_bounds,
            [(1, Some([0., 0., 2., 2.])), (88, None), (99, None)]
        );
        let bounds = Arc::new(BTreeMap::from([(88, [4., 4., 6., 6.])]));
        h.set(Vec::new(), bounds, false);
        assert_eq!(
            h.province_bounds,
            [(1, None), (88, Some([4., 4., 6., 6.])), (99, None)]
        );
        h.set_naval(NavalSources::default());
        assert_eq!(h.province_bounds.len(), 3);
    }

    #[test]
    fn raid_units_stand_at_every_height_outside_the_budget_and_restart_on_load() {
        let raid = ModelInstance {
            scale: 2.0,
            animation_seed: 541,
            ..Default::default()
        };
        let mut h = history(vec![target(40, Kind::Land)], false);
        h.set_raids(vec![(9, raid)]);
        let raids = |placed: &[(usize, ModelInstance)]| {
            placed
                .iter()
                .filter(|(mesh, _)| *mesh == 9)
                .map(|(_, i)| *i)
                .collect::<Vec<_>>()
        };
        // Above both cutoffs, out of view and with no creation allowance
        // left, the raid unit stands at full scale; the figure does not.
        let placed = limited(&mut h, 0.0, 0);
        assert_eq!(placed.len(), 1);
        let first = raids(&placed)[0];
        assert_eq!(first.scale, 2.0);
        assert_ne!(first.animation_epoch, 0, "its clock starts at creation");
        let hidden = h.step(0.5, GATES, [false; 2], false, |_| false);
        assert_eq!(raids(&hidden), [first]);
        // A tag switch keeps its history, a load-equivalent reset renews it.
        h.reset(Some(&[40].into()));
        assert_eq!(
            raids(&near(&mut h, 1.0))[0].animation_epoch,
            first.animation_epoch
        );
        h.reset(None);
        assert_ne!(
            raids(&near(&mut h, 1.5))[0].animation_epoch,
            first.animation_epoch
        );
        // A playing window moves it by replacing the frame's placement.
        let moved = ModelInstance {
            position: [1.0, 2.0, 3.0],
            ..raid
        };
        h.set_raids(vec![(9, moved)]);
        assert_eq!(raids(&near(&mut h, 2.0))[0].position, [1.0, 2.0, 3.0]);
        h.set_raids(Vec::new());
        assert!(raids(&near(&mut h, 2.5)).is_empty());
    }

    #[test]
    fn deferred_land_allowance_is_per_province_and_can_exceed_limit() {
        let targets = [1, 2, 3]
            .into_iter()
            .flat_map(|province| {
                (0..3).map(move |order| Target {
                    order,
                    identity: u64::from(province * 10 + order),
                    mesh: (province * 10 + order) as usize,
                    ..target(province, Kind::Land)
                })
            })
            .collect();
        let mut h = history(targets, false);
        assert_eq!(
            limited(&mut h, 0.0, 2)
                .iter()
                .map(|p| p.0)
                .collect::<Vec<_>>(),
            [10, 11, 20, 21, 30, 31]
        );
        assert_eq!(
            limited(&mut h, 0.0, 2).len(),
            9,
            "same timestamp is still a new frame"
        );
    }

    #[test]
    fn the_land_cursor_is_a_visual_slot_kept_across_restarts() {
        let land = |order, identity| Target {
            order,
            identity,
            mesh: identity as usize,
            ..target(1, Kind::Land)
        };
        let meshes = |placed: Vec<(usize, ModelInstance)>| {
            placed.into_iter().map(|(mesh, _)| mesh).collect::<Vec<_>>()
        };
        let mut h = history(vec![land(0, 20), land(1, 30), land(2, 40)], false);
        assert_eq!(meshes(limited(&mut h, 0.0, 1)), [20]);
        assert_eq!(meshes(limited(&mut h, 0.1, 1)), [20, 30]);
        h.reset(None);
        assert_eq!(
            meshes(limited(&mut h, 0.2, 1)),
            [30],
            "Recreating units leaves the cursor on the last slot visited"
        );
        // A new representative in the cursor's slot is visited first.
        h.set(vec![land(1, 10), land(2, 40)], Arc::default(), false);
        assert_eq!(meshes(limited(&mut h, 0.3, 1)), [10]);
    }

    #[test]
    fn spent_inline_budget_holds_show_bits_and_next_land_starts_at_zero() {
        let mut h = history(vec![target(1, Kind::Ship), target(2, Kind::Land)], false);
        assert_eq!(limited(&mut h, 0.0, 1).len(), 1);
        assert_eq!(h.provinces[&2].shown, [false; 2]);
        let next = limited(&mut h, 0.25, 1);
        assert_eq!(next.len(), 2);
        assert_eq!(next[1].1.scale, 0.0);
        assert_eq!(next[0].1.scale, 0.5);
    }

    #[test]
    fn capital_replacement_holds_but_support_replacement_clears_before_budget_check() {
        let hull = |part, occupant| Target {
            part,
            occupant,
            ..target(1, Kind::Ship)
        };
        let mut h = history(vec![hull(0, 10), hull(1, 11)], false);
        let old = limited(&mut h, 0.0, 2);
        h.set(vec![hull(0, 20), hull(1, 21)], Arc::default(), false);
        let held = limited(&mut h, 0.1, 0);
        assert_eq!(held.len(), 1);
        assert_eq!(held[0].1.animation_epoch, old[0].1.animation_epoch);
        let next = limited(&mut h, 0.2, 2);
        assert_eq!(next.len(), 2);
        assert_ne!(next[0].1.animation_epoch, old[0].1.animation_epoch);
    }

    #[test]
    fn zoom_out_shrinks_twice_on_gate_loss_then_once_and_destroys_after_zero() {
        let mut h = history(vec![target(40, Kind::Land), target(40, Kind::Ship)], false);
        assert_eq!(near(&mut h, 0.0)[0].1.scale, 0.0);
        let full = near(&mut h, 1.0);
        assert!(full.iter().all(|(_, i)| i.scale == 1.0));
        let faded = h.step(1.25, GATES, [false; 2], true, |_| true);
        assert_eq!(faded[0].1.scale, 0.5);
        assert_eq!(faded[1].1.scale, 0.75);
        let zero = h.step(1.75, GATES, [false; 2], true, |_| true);
        assert_eq!(zero[0].1.scale, 0.0);
        assert_eq!(zero[1].1.scale, 0.5);
        let gone = h.step(2.0, GATES, [false; 2], true, |_| true);
        assert_eq!(gone.len(), 1);
        assert_eq!(gone[0].1.scale, 0.375);
        let back = near(&mut h, 2.125);
        assert_eq!(
            back[0].1.scale, 0.0,
            "new land entity is created after the growth pass"
        );
        assert_ne!(back[0].1.animation_epoch, full[0].1.animation_epoch);
        assert_eq!(back[1].1.animation_epoch, full[1].1.animation_epoch);
        assert_eq!(back[1].1.scale, 0.625, "existing ship reverses its shrink");
    }

    #[test]
    fn pan_out_keeps_scale_and_entity_until_the_province_id_slot() {
        // Order and sparse IDs are immaterial; these are native ID array indexes.
        let mut h = history(vec![target(104, Kind::Land), target(99, Kind::Ship)], false);
        near(&mut h, 0.0); // phase 1
        let full = near(&mut h, 1.0); // phase 2
        let hidden = h.step(1.1, GATES, [true; 2], true, |_| false); // phase 3
        assert_eq!(hidden, full, "pan-out does not fade");
        let hidden = h.step(1.2, GATES, [true; 2], true, |_| false); // phase 4
        assert_eq!(hidden.len(), 1);
        assert_eq!(hidden[0].1.animation_seed, 99);
        let back = near(&mut h, 1.3);
        assert_eq!(back[0].1.animation_epoch, full[0].1.animation_epoch);
        assert_ne!(back[1].1.animation_epoch, full[1].1.animation_epoch);
        assert_eq!(back[1].1.scale, 0.0);
    }

    #[test]
    fn quick_pan_back_reuses_but_after_a_scheduled_visit_recreates() {
        for kind in [Kind::Land, Kind::Ship] {
            let mut h = history(vec![target(50, kind)], false);
            near(&mut h, 0.0);
            let epoch = near(&mut h, 1.0)[0].1.animation_epoch;
            for _ in 0..20 {
                h.step(1.0, GATES, [true; 2], true, |_| false);
            }
            assert_eq!(near(&mut h, 1.0)[0].1.animation_epoch, epoch);
            for _ in 0..100 {
                h.step(1.0, GATES, [true; 2], true, |_| false);
            }
            assert!(h.provinces[&50].entities.is_empty());
            assert_ne!(near(&mut h, 1.0)[0].1.animation_epoch, epoch);
        }
    }

    #[test]
    fn hour_changes_wait_offscreen_and_resets_invalidate_even_the_same_target() {
        let old = target(70, Kind::Land);
        let mut h = history(vec![old], false);
        near(&mut h, 0.0);
        let full = near(&mut h, 1.0);
        let mut replacement = old;
        replacement.mesh = 2;
        h.set(vec![replacement], Arc::default(), false);
        assert_eq!(h.step(1.1, GATES, [true; 2], true, |_| false), full);
        let held = near(&mut h, 1.2);
        assert_eq!(
            held[0].0, 0,
            "same representative waits for the periodic selector"
        );
        for _ in 0..97 {
            near(&mut h, 1.2);
        }
        let new = near(&mut h, 1.2);
        assert_eq!(new[0].0, 2);
        assert_eq!(
            new[0].1.scale, 1.0,
            "already shown province replaces at full scale"
        );
        assert_ne!(new[0].1.animation_epoch, full[0].1.animation_epoch);
        h.reset(None);
        let reset = near(&mut h, 1.2);
        assert_ne!(reset[0].1.animation_epoch, new[0].1.animation_epoch);
        assert_eq!(reset[0].1.scale, 0.0);
        assert_ne!(
            near(&mut h, 0.0)[0].1.animation_epoch,
            reset[0].1.animation_epoch
        );
        h.set(Vec::new(), Arc::default(), false);
        assert!(
            near(&mut h, 1.0).is_empty(),
            "removed visible stack destroys immediately"
        );
    }

    #[test]
    fn a_tag_switch_recreates_only_the_listed_provinces() {
        let mut h = history(vec![target(1, Kind::Land), target(2, Kind::Ship)], false);
        near(&mut h, 0.0);
        let full = near(&mut h, 1.0);
        h.reset(Some(&BTreeSet::from([1])));
        let after = near(&mut h, 1.1);
        assert_ne!(after[0].1.animation_epoch, full[0].1.animation_epoch);
        assert_eq!(after[0].1.scale, 0.0, "cleared show bit: grows in again");
        assert_eq!(after[1], full[1], "other provinces keep their entities");
    }

    #[test]
    fn ship_hull_offsets_shrink_about_the_formation_root() {
        let mut ship = target(30, Kind::Ship);
        ship.origin = [10.0, 20.0, 30.0];
        ship.instance.position = [14.0, 22.0, 38.0];
        ship.instance.scale = 0.6;
        let mut h = history(vec![ship], false);
        near(&mut h, 0.0);
        let half = near(&mut h, 0.25);
        assert_eq!(half[0].1.position, [12.0, 21.0, 34.0]);
        assert_eq!(half[0].1.scale, 0.3);
    }

    #[test]
    fn hulls_joining_a_growing_formation_take_its_root_scale() {
        let hull = |part, mesh| Target {
            part,
            mesh,
            ..target(30, Kind::Ship)
        };
        let mut h = history(vec![hull(0, 0)], false);
        near(&mut h, 0.0);
        let growing = near(&mut h, 0.25);
        assert_eq!(growing[0].1.scale, 0.5);
        h.set(vec![hull(0, 3), hull(1, 0)], Arc::default(), false);
        let joined = near(&mut h, 0.375);
        assert_eq!(joined.len(), 2);
        assert!(joined.iter().all(|(_, instance)| instance.scale == 0.75));
        assert_ne!(
            joined[0].1.animation_epoch, growing[0].1.animation_epoch,
            "a replaced hull is a new attachment"
        );
        assert_ne!(joined[1].1.animation_epoch, joined[0].1.animation_epoch);
    }

    #[test]
    fn an_empty_visible_province_keeps_its_show_bit_for_arriving_stacks() {
        let bounds = Arc::new(BTreeMap::from([(9, [0.0; 4])]));
        let mut h = History::default();
        h.set(Vec::new(), bounds.clone(), false);
        assert!(near(&mut h, 0.0).is_empty());
        h.set(vec![target(9, Kind::Land)], bounds, false);
        assert_eq!(near(&mut h, 0.1)[0].1.scale, 1.0);
    }

    #[test]
    fn already_shrinking_offscreen_slot_follows_the_native_early_return() {
        let mut h = history(vec![target(5, Kind::Land)], false);
        near(&mut h, 0.0);
        let epoch = near(&mut h, 1.0)[0].1.animation_epoch;
        h.step(1.1, GATES, [false; 2], false, |_| true); // also beyond arrow cutoff
        h.step(1.2, GATES, [false; 2], false, |_| false);
        let held = h.step(1.3, GATES, [false; 2], false, |_| false); // phase 5
        assert_eq!(held[0].1.animation_epoch, epoch);
        assert_eq!(near(&mut h, 1.4)[0].1.animation_epoch, epoch);
    }

    /// The game's raids mode clears the same per-province land-model bit its units cutoff
    /// clears, so a political â†’ raids â†’ political round trip retires and restores land models
    /// exactly as a camera rising above the cutoff and coming back down does, frame for frame;
    /// ships are untouched.
    #[test]
    fn a_map_mode_without_land_models_acts_as_a_camera_above_their_cutoff() {
        use crate::map::map_mode::{MapMode, Stock, visibility};
        use viewer_foundation::map::camera::{MapDefines, MapSize, Placement};
        let targets = vec![target(7, Kind::Land), target(8, Kind::Ship)];
        let bounds = Arc::new(BTreeMap::from([
            (7, [0.0, 0.0, 2.0, 2.0]),
            (8, [1.0, 1.0, 3.0, 3.0]),
        ]));
        let start = |h: &mut History| h.bounds = Arc::clone(&bounds);
        let mut by_camera = history(targets.clone(), false);
        let mut by_mode = history(targets, false);
        start(&mut by_camera);
        start(&mut by_mode);
        let camera =
            |height| Placement::map([1.0, 1.0], height).on(MapSize::TEST, MapDefines::test());
        let political = visibility::of(&MapMode::Stock(Stock::Default));
        let raids = visibility::of(&MapMode::Stock(Stock::Raids));
        let mut shrinking = false;
        for (step, in_raids) in [false, false, true, true, true, false, false, false]
            .into_iter()
            .enumerate()
        {
            let time = step as f32 * 0.25;
            // Above the land cutoff (100) and below the ship cutoff (200).
            let high = if in_raids { 150.0 } else { 50.0 };
            let a = by_camera.update(camera(high), [1024, 768], time, GATES);
            let mode = if in_raids { &raids } else { &political };
            let b = by_mode.update(camera(50.0), [1024, 768], time, GATES.for_mode(mode));
            assert_eq!(a, b, "step {step}");
            shrinking |= in_raids && a.iter().any(|(_, i)| i.scale < 1.0);
        }
        assert!(shrinking, "the round trip passes through the shrink");
    }

    #[test]
    fn dumps_settle_for_their_camera_even_with_an_explicit_animation_time() {
        use viewer_foundation::map::camera::{MapDefines, MapSize, Placement};
        let mut h = history(vec![target(7, Kind::Land), target(8, Kind::Ship)], true);
        h.bounds = Arc::new(BTreeMap::from([
            (7, [0.0, 0.0, 2.0, 2.0]),
            (8, [3000.0, 1000.0, 3002.0, 1002.0]),
        ]));
        let camera = Placement {
            center: [0.0, 0.0],
            height: 50.0,
            pose: viewer_foundation::map::camera::Pose::Map,
        }
        .on(MapSize::TEST, MapDefines::test());
        let full = h.update(camera, [1024, 768], 120.0, GATES);
        assert_eq!(full.len(), 1);
        assert_eq!(full[0].1.scale, 1.0);
        assert!(
            h.update(
                Camera {
                    height: 250.0,
                    ..camera
                },
                [1024, 768],
                120.0,
                GATES
            )
            .is_empty()
        );
    }

    #[test]
    fn popup_rates_come_from_evaluated_content_defines() {
        let defines = installed_content_core::defines::Defines::new().unwrap();
        defines.load("synthetic.lua", br#"NDefines = { NGraphics = {
            UNITS_DISTANCE_CUTOFF=11, SHIPS_DISTANCE_CUTOFF=22, UNIT_ARROW_DISTANCE_CUTOFF=33, SHIP_IN_PORT_SCALE=.125,
            MESH_POPUP_SCALE_UP_SPEED=7, MESH_POPUP_SCALE_DOWN_SPEED=9, SHIP_POPUP_SCALE_DOWN_SPEED=13,
            NAVAL_UNIT_MOVEMENT_SPEED=12, LAND_UNIT_MOVEMENT_SPEED=12
        }}"#).unwrap();
        let gates = Gates::read(&defines).unwrap();
        assert_eq!(
            (gates.scale_up, gates.scale_down, gates.ship_scale_down),
            (7.0, 9.0, 13.0)
        );
        defines
            .load(
                "override.lua",
                b"NDefines.NGraphics.MESH_POPUP_SCALE_DOWN_SPEED = 3 * 5",
            )
            .unwrap();
        assert_eq!(Gates::read(&defines).unwrap().scale_down, 15.0);
        defines
            .load(
                "missing.lua",
                b"NDefines.NGraphics.SHIP_POPUP_SCALE_DOWN_SPEED = nil",
            )
            .unwrap();
        assert!(
            Gates::read(&defines)
                .unwrap_err()
                .to_string()
                .contains("SHIP_POPUP_SCALE_DOWN_SPEED")
        );
    }

    #[test]
    #[ignore = "release viewer-frame lifecycle measurement"]
    fn unit_lifetime_cost() {
        for count in [3_000u32, 10_000, 50_000] {
            let targets: Vec<_> = (1..=count).map(|id| target(id, Kind::Land)).collect();
            let mut h = history(targets.clone(), false);
            near(&mut h, 0.0);
            near(&mut h, 1.0);
            let mut frames = Vec::new();
            let mut builders = Vec::new();
            for frame in 0..300 {
                if frame % 30 == 0 {
                    let start = Instant::now();
                    h.set(targets.clone(), Arc::default(), false);
                    builders.push(start.elapsed().as_secs_f64() * 1e3);
                }
                let start = Instant::now();
                std::hint::black_box(h.step(
                    2.0 + frame as f32 / 120.0,
                    GATES,
                    [frame % 120 < 60; 2],
                    true,
                    |id| (id + frame / 30) % 3 != 0,
                ));
                frames.push(start.elapsed().as_secs_f64() * 1e3);
            }
            frames.sort_by(f64::total_cmp);
            builders.sort_by(f64::total_cmp);
            println!(
                "UNIT_LIFETIME count={count} frame_p50_ms={:.4} frame_p95_ms={:.4} frame_max_ms={:.4} target_replace_p50_ms={:.4} target_replace_max_ms={:.4}",
                frames[150], frames[285], frames[299], builders[5], builders[9]
            );
        }
    }

    /// The view's units and map icons run on one schedule: a frame whose map icons already
    /// advanced it (province updates before map-icon updates) walks its
    /// provinces at that phase instead of advancing again, and a reset sets the dirty bits
    #[test]
    fn the_units_walk_the_phase_their_map_icons_advanced() {
        let mut h = history(vec![target(1, Kind::Land)], false);
        let schedule = Shared::default();
        h.share_schedule(&schedule);
        schedule.lock().unwrap().advance([1], |_| false, false);
        h.step(0.0, GATES, [true, true], false, |_| false);
        // One advance: phase 1, which updates province 1 out of view (the walk read that list).
        assert_eq!(schedule.lock().unwrap().update(1), Some(Update::OffScreen));
        assert!(!schedule.lock().unwrap().dirty(1));
        // A frame the map icons did not advance: the walk advances it, and the clean province
        // out of view is no longer in the list.
        h.step(0.1, GATES, [true, true], false, |_| false);
        assert_eq!(schedule.lock().unwrap().update(1), None);
        h.reset(None);
        assert!(schedule.lock().unwrap().dirty(1));
    }
}
