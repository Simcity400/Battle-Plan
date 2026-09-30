//! Viewer-owned unit entity lifetimes: which land figures and ship hulls
//! exist, at what popup scale, and since when their animation runs. The
//! viewer is the game's client here, so this runs on the viewer's frame clock.
//!
//! **Popup scale, not alpha.** `FadeOrDestroyUnitSlotEntity@141279F50`
//! shrinks the uniform transform scale, `max(0, scale - dt *
//! MESH_POPUP_SCALE_DOWN_SPEED)` into all three axes (`14127A047..14127A062`
//! -> `1424E7450` -> `14227BA20`), and destroys an entity whose scale was
//! already zero when visited. Formations shrink at
//! `SHIP_POPUP_SCALE_DOWN_SPEED`, their root scale carrying the hull offsets
//! (`141652B90`). Both grow at `MESH_POPUP_SCALE_UP_SPEED` (`14127F000`).
//!
//! **Visits.** `UpdateUnitStackGraphics@14127C910` visits a kind that just
//! lost its show bit in its gate-loss loop and again in
//! `FadeInactiveUnitStackModels@14127A090`; later frames visit once.
//! Provinces in view update every frame. `CGraphicalMap::update@140B49340`
//! advances a phase `(+0x130 + 1) % 100` per frame before `140B4C120`
//! updates an off-screen province, only when its index modulo 100 equals the
//! phase: the view's one schedule, which also drives the provinces'
//! presentation-dirty bits ([`super::province_schedule`]). That update destroys at once (`141277FC0` clears the show bits,
//! `14127D260` passes camera admission false); until then the entity, its
//! transform and its animation clock survive, so a quick pan back resumes
//! them. Hidden entities' clocks advance (`14227CE00` -> `14226EF30`). A
//! selected unit bypasses the phase (`province+0x1764`); the viewer has no
//! unit selection.
//!
//! Divisions (transports included) and railway guns share the land path,
//! naval formations the ship path. Naval combat graphics are destroyed on
//! land-gate loss (`14127CAA9`); aircraft and trains have their own
//! managers. The shared creation counter is in [`super::mesh_budget`].
//!
//! Native slots are the 38 visual buckets of `14127F000`; this layer receives
//! the viewer's existing stack/formation reconstruction, not their recorded
//! ownership. Land uses its reconstructed visual order with identity ties;
//! ships keep the stationary reconstruction. Deferred native selection runs
//! in parallel (`140B3E400`, `1412785B0`), so worker completion/RNG order is
//! not recoverable; all queued creations still become visible on one frame.

use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
};

use super::{
    camera::Camera,
    map_buildings::{BuildingHistory, province_view},
    mesh_budget::Budget,
    models::{ModelField, ModelInstance},
    province_schedule::Update,
    unit_models::Gates,
};

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
    /// Land: the reconstructed native visual slot `141277810` walks in
    /// order; stacks the reconstruction puts in one slot follow `identity`.
    pub order: u32,
    /// The ship a hull shows (`1416542A0` keeps a hull while its ship and
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
    pub targets: Vec<Target>,
    /// The raid arrows' units where the hour placed them ([`History::set_raids`]).
    pub raids: Vec<(usize, ModelInstance)>,
}

/// One slot's targets, by part.
type Parts = BTreeMap<u32, Target>;

/// One figure, or one hull attached under a formation's root.
struct Part {
    target: Target,
    epoch: u32,
}

/// A slot's entity. Its popup scale belongs to the slot (`+0xD38`), so the
/// hulls of a formation share their root's.
struct Entity {
    kind: Kind,
    scale: f32,
    /// Shrinking through the inactive path, which drops the transport scale.
    shrinking: bool,
    parts: BTreeMap<u32, Part>,
}

impl Entity {
    /// One `141279F50` visit: it tests the OLD scale, then subtracts, so zero
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
            // FadeInactiveUnitStackModels passes land multiplier 1.0, including
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

/// A new incarnation's animation epoch. Explicit dump clocks keep their
/// sampling convention (epoch 0); only live incarnations start now.
fn incarnation(epoch: &mut u32, steady: bool) -> u32 {
    *epoch = epoch.wrapping_add(1);
    if steady { 0 } else { *epoch }
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
struct Province {
    /// The land and ship show bits of its last update (`+0x175D`).
    shown: [bool; 2],
    /// The arrow flag of its last update (`+0x175E`).
    arrows: bool,
    entities: BTreeMap<Slot, Entity>,
    /// `141277810`'s round-robin cursor (`+0x18`): the visual slot
    /// ([`Target::order`]) it last visited, not its successor, whichever
    /// representative holds it now. `1412771B0` does not reset it.
    land_cursor: u32,
    /// Visits since each land slot was last queued (`+0x30 + slot * 0x50`).
    land_visits: BTreeMap<Slot, u16>,
}

impl Province {
    /// Queues land creations against what the counter leaves (`141277810`),
    /// then grows existing figures and attaches hulls, which spend it inline
    /// (`14127F000`). The queued land is created after every province's
    /// visit ([`Province::create_land`]).
    fn visit(
        &mut self,
        targets: Option<&BTreeMap<Slot, Parts>>,
        frame: Frame,
        epoch: &mut u32,
        budget: &mut Budget,
    ) -> Vec<(Slot, bool)> {
        let placed = |slot: &Slot| targets.is_some_and(|targets| targets.contains_key(slot));
        let was = self.shown;
        let mut queued = Vec::new();
        if frame.shown[Kind::Land as usize] {
            let mut lands: Vec<_> = targets
                .into_iter()
                .flatten()
                .filter(|(slot, _)| slot.0 == Kind::Land)
                .map(|(slot, parts)| (parts.first_key_value().unwrap().1.order, *slot))
                .collect();
            lands.sort_unstable();
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
                // The gate-loss loop precedes 14127A090, so the first frame
                // out of range visits twice.
                return (!was[kind] || entity.shrink(frame.delta, frame.gates))
                    && entity.shrink(frame.delta, frame.gates);
            }
            // A missing stack of a kind shown now and before is destroyed at
            // once (14127A090 param 7); otherwise it shrinks.
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
                            // for 141277810's periodic selection or pointer change.
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
            // 14127F000 acquires a formation root outside the counter, full
            // size where ships were already shown, and grows it even while
            // all its hulls wait.
            let entity = self.entities.entry(slot).or_insert_with(|| Entity {
                kind: slot.0,
                scale: if frame.steady || was[kind] { 1.0 } else { 0.0 },
                shrinking: false,
                parts: BTreeMap::new(),
            });
            // Hulls attach and detach under the root without touching its
            // scale (141653F00 -> 1416542A0), slot 0 first.
            entity.parts.retain(|part, _| parts.contains_key(part));
            for (&index, &target) in parts {
                let replace = entity.parts.get(&index).is_none_or(|part| {
                    part.target.mesh != target.mesh || part.target.occupant != target.occupant
                });
                if replace {
                    // 141653F00 clears a stale support slot before attaching;
                    // slot 0 attaches directly and keeps its old hull if held.
                    if index != 0 {
                        entity.parts.remove(&index);
                    }
                    if !budget.take() {
                        continue;
                    }
                    entity.parts.insert(
                        index,
                        Part {
                            target,
                            epoch: incarnation(epoch, frame.steady),
                        },
                    );
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
        // 14127C910 keeps a newly shown kind's bit clear while the counter is
        // spent, so what it creates next frame still grows from zero.
        for kind in 0..2 {
            if budget.remaining() != 0 || !frame.shown[kind] {
                self.shown[kind] = frame.shown[kind];
            }
        }
        self.arrows = frame.arrows;
        queued
    }

    /// `141276C20` for a request `141277810` queued: `1412785B0` recreates
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
        // 141276C20 has no budget guard; queued work can exceed the limit.
        budget.charge();
        self.entities.insert(
            slot,
            Entity {
                kind: Kind::Land,
                scale: if steady || was_shown { 1.0 } else { 0.0 },
                shrinking: false,
                parts: parts
                    .iter()
                    .map(|(&index, &target)| {
                        (
                            index,
                            Part {
                                target,
                                epoch: incarnation(epoch, steady),
                            },
                        )
                    })
                    .collect(),
            },
        );
    }

    /// The phase update of a province out of view. The gate-loss loops
    /// destroy each kind shown last update, and a changed arrow flag forces
    /// the inactive pass, which destroys the rest. Otherwise `14127C910`
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
    targets: BTreeMap<u32, BTreeMap<Slot, Parts>>,
    provinces: BTreeMap<u32, Province>,
    /// Province boxes for camera admission.
    bounds: Arc<BTreeMap<u32, [f32; 4]>>,
    /// The previous frame's animation clock.
    last: Option<f32>,
    /// The view's province schedule (`CGraphicalMap+0x130` and the dirty
    /// bits), shared with its counters ([`Self::share_schedule`]).
    schedule: super::province_schedule::Shared,
    /// The last incarnation handed out.
    epoch: u32,
    /// Offline frames settle for their camera instead of animating.
    steady: bool,
    /// The raid arrows' units. No unit slot holds them:
    /// `CRaidArrow::CreateUnitEntity@141CD4900` creates its entity at once
    /// through `entity_factory@142277DF0`, never testing or charging the
    /// shared creation counter (`1433232A4`, [`super::mesh_budget`]), and
    /// `UpdateForRaid@141CD75A0` shows it at every camera height with no
    /// popup scale, so they stand on every frame whatever the gates.
    raids: Vec<(usize, ModelInstance)>,
    /// The raid units' incarnation: a load-equivalent reset recreates them
    /// too, so their animation starts again ([`History::reset`]).
    raid_epoch: u32,
}

impl History {
    /// Installs an hour's targets. Entities carry over; the next frame
    /// admits, grows or retires them.
    pub fn set(
        &mut self,
        targets: Vec<Target>,
        bounds: Arc<BTreeMap<u32, [f32; 4]>>,
        steady: bool,
    ) {
        if !Arc::ptr_eq(&self.bounds, &bounds) {
            // Native updates the show bits of empty provinces too: a stack
            // arriving in a province already shown starts at full scale.
            for &id in bounds.keys() {
                self.provinces.entry(id).or_default();
            }
            self.bounds = bounds;
        }
        self.targets.clear();
        for target in targets {
            self.provinces.entry(target.province).or_default();
            self.targets
                .entry(target.province)
                .or_default()
                .entry(target.slot())
                .or_default()
                .insert(target.part, target);
        }
        self.steady = steady;
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
    pub(super) fn steady_for_test(&mut self, steady: bool) {
        self.steady = steady;
    }

    pub(super) fn settle(&mut self) {
        self.steady = true;
    }

    pub(super) fn restart_live(&mut self) {
        self.reset(None);
        self.last = None;
        self.steady = false;
    }

    /// Clears the unit slots of `provinces`, or of every province, as a load
    /// [`super::presentation::UnitRestart`]): their entities go and their
    /// show bits drop, so what shows next grows in on a new clock.
    pub fn reset(&mut self, provinces: Option<&BTreeSet<u32>>) {
        self.schedule
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .reset(provinces);
        // A load destroys the raid arrows and their units with the rest of
        // the map (`CRaidArrowManager::Update@14166B540` builds them again);
        // a tag switch shows the new country's raids, other raid ids.
        if provinces.is_none() && self.raid_epoch != 0 {
            self.epoch += 1;
            self.raid_epoch = self.epoch;
        }
        for (id, province) in &mut self.provinces {
            if provinces.is_none_or(|provinces| provinces.contains(id)) {
                *province = Province {
                    land_cursor: province.land_cursor,
                    ..Province::default()
                };
            }
        }
    }

    /// Runs on `schedule`, the view's counters' own, from now on: a view keeps
    /// one schedule for its units and its map icons.
    pub fn share_schedule(&mut self, schedule: &super::province_schedule::Shared) {
        if !Arc::ptr_eq(&self.schedule, schedule) {
            self.schedule = Arc::clone(schedule);
        }
    }

    /// The frame's advance of the schedule at this camera, made before the
    /// view's map icons read the dirty bits (`CGraphicalMap::update` runs
    /// before `CMapIconManagerImpl::update`); the frame's walk
    /// ([`Self::update_models`]) then takes it instead of advancing again. A
    /// frame whose walk has not yet taken the last advance advances nothing.
    pub fn advance_schedule(&mut self, camera: Camera, viewport: [u32; 2]) {
        let mut schedule = self
            .schedule
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if schedule.pending() {
            return;
        }
        schedule.advance_view(
            &self.bounds,
            self.provinces.keys().copied(),
            camera,
            viewport,
            self.steady,
        );
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
        let in_view = province_view(camera, viewport);
        let bounds = Arc::clone(&self.bounds);
        let mut buildings = buildings.frame(camera, viewport, time, buildings_enabled, self.steady);
        let mut budget = Budget::new(limit, self.steady);
        let placed = self.step_with(
            time,
            gates,
            [gates.units(camera.height), gates.ships(camera.height)],
            camera.height < gates.arrows,
            |id| bounds.get(&id).is_some_and(|bounds| in_view(*bounds)),
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
    pub(super) fn step(
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
        // The native literal, advanced before the province updates (unless
        // the frame already advanced it for its map icons); never indexed by
        // the replay hour.
        let schedule = Arc::clone(&self.schedule);
        let mut schedule = schedule
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if !schedule.take_pending() {
            schedule.advance(self.provinces.keys().copied(), &in_view, self.steady);
            schedule.take_pending();
        }
        // The frame's update list, the one the map icons' owners read.
        let mut queued = Vec::new();
        for (&id, province) in &mut self.provinces {
            buildings(id, budget);
            match schedule.update(id) {
                Some(Update::InView) => queued.extend(
                    province
                        .visit(self.targets.get(&id), frame, &mut self.epoch, budget)
                        .into_iter()
                        .map(|(slot, shown)| (id, slot, shown)),
                ),
                Some(Update::OffScreen) => province.leave(self.steady),
                None => {}
            }
        }
        buildings(u32::MAX, budget);
        for (id, slot, shown) in queued {
            self.provinces.get_mut(&id).unwrap().create_land(
                slot,
                &self.targets[&id][&slot],
                shown,
                self.steady,
                &mut self.epoch,
                budget,
            );
        }
        let raid_epoch = self.raid_epoch;
        self.provinces
            .values()
            .flat_map(|province| province.entities.values().flat_map(Entity::placed))
            .chain(self.raids.iter().map(|&(mesh, instance)| {
                (
                    mesh,
                    ModelInstance {
                        animation_epoch: raid_epoch,
                        ..instance
                    },
                )
            }))
            .collect()
    }
}

#[cfg(test)]
pub(super) mod tests {
    use super::*;

    pub const GATES: Gates = Gates {
        units: 100.0,
        ships: 200.0,
        arrows: 300.0,
        transport_scale: 0.3,
        scale_up: 2.0,
        scale_down: 1.0,
        ship_scale_down: 0.5,
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
            "1412771B0 leaves the cursor on the last slot visited"
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

    #[test]
    fn dumps_settle_for_their_camera_even_with_an_explicit_animation_time() {
        use super::super::camera::{MapDefines, MapSize, Placement};
        let mut h = history(vec![target(7, Kind::Land), target(8, Kind::Ship)], true);
        h.bounds = Arc::new(BTreeMap::from([
            (7, [0.0, 0.0, 2.0, 2.0]),
            (8, [3000.0, 1000.0, 3002.0, 1002.0]),
        ]));
        let camera = Placement {
            center: [0.0, 0.0],
            height: 50.0,
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
        let defines = installed_content::defines::Defines::new().unwrap();
        defines.load("synthetic.lua", br#"NDefines = { NGraphics = {
            UNITS_DISTANCE_CUTOFF=11, SHIPS_DISTANCE_CUTOFF=22, UNIT_ARROW_DISTANCE_CUTOFF=33, SHIP_IN_PORT_SCALE=.125,
            MESH_POPUP_SCALE_UP_SPEED=7, MESH_POPUP_SCALE_DOWN_SPEED=9, SHIP_POPUP_SCALE_DOWN_SPEED=13
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
                    let start = std::time::Instant::now();
                    h.set(targets.clone(), Arc::default(), false);
                    builders.push(start.elapsed().as_secs_f64() * 1e3);
                }
                let start = std::time::Instant::now();
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
    /// advanced it (`CGraphicalMap::update` before `CMapIconManagerImpl::update`) walks its
    /// provinces at that phase instead of advancing again, and a reset sets the dirty bits
    /// (`ResetUnitSlots@1412771B0`).
    #[test]
    fn the_units_walk_the_phase_their_map_icons_advanced() {
        let mut h = history(vec![target(1, Kind::Land)], false);
        let schedule = super::super::province_schedule::Shared::default();
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
