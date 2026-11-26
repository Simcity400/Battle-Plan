
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
};
use viewer_unit_reads::counter_move::{AtHead, Position, SharedMovement};
use viewer_gpu::province_admission::province_boxes;
use viewer_gpu::province_map::ProvinceMap;
use viewer_gpu::province_schedule::{FrameRestore, ProvinceFrame, Shared};
use viewer_runtime::unit_handle::Unit;

use viewer_buildings::map_buildings::BuildingHistory;
use viewer_scene_base::mesh_budget::Budget;
use viewer_models::{ModelField, ModelInstance};
use viewer_gpu::{camera::Camera, province_admission::Admission, province_schedule::Update};
use viewer_unit_reads::model_admission::Gates;

pub mod naval;
pub mod naval_combat;
mod provinces;
use provinces::Provinces;
#[cfg(test)]
mod same_history;
pub use naval::Sources as NavalSources;

pub const MESH_FORWARD_OFFSET: f32 = std::f32::consts::FRAC_PI_2;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Kind {
    Land,
    Ship,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Target {
    pub province: u32,
    pub identity: u64,
    pub part: u32,
    pub order: u32,
    pub occupant: u64,
    pub kind: Kind,
    pub mesh: usize,
    pub instance: ModelInstance,
    pub origin: [f32; 3],
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LandFigure {
    pub province: u32,
    pub order: u32,
    pub mesh: usize,
    pub instance: ModelInstance,
}

type Slot = (Kind, u64);

impl Target {
    fn slot(self) -> Slot {
        (self.kind, self.identity)
    }
}

#[derive(Clone, Default)]
pub struct Field {
    pub models: ModelField,
    pub targets: Targets,
    pub raids: Vec<(usize, ModelInstance)>,
    pub naval: NavalSources,
}

type Parts = BTreeMap<u32, Target>;

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

#[derive(Clone, Debug, PartialEq)]
struct Part {
    target: Target,
    epoch: u32,
    identity: u32,
    graphics: Vec<u8>,
}

impl Part {
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

#[derive(Clone, Debug, PartialEq)]
struct Entity {
    kind: Kind,
    scale: f32,
    shrinking: bool,
    parts: BTreeMap<u32, Part>,
}

impl Entity {
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

const MOTION_SLACK: f32 = 1e-3;

#[derive(Clone, Copy)]
struct Frame {
    delta: f32,
    gates: Gates,
    shown: [bool; 2],
    arrows: bool,
    steady: bool,
}

#[derive(Clone, Debug, Default, PartialEq)]
struct Province {
    shown: [bool; 2],
    arrows: bool,
    entities: BTreeMap<Slot, Entity>,
    lands: Arc<[(u32, Slot)]>,
    land_cursor: u32,
    land_passes: u64,
    land_visits: BTreeMap<Slot, u64>,
    due_pass: u64,
    quiet: bool,
}

impl Province {
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
        let (mut whole, mut held) = (true, false);
        if frame.shown[Kind::Land as usize] {
            let allowance = budget.remaining();
            if allowance == 0 && budget.holding() {
                let owed = self.lands.iter().filter(|(_, slot)| !self.entities.contains_key(slot)).count();
                budget.hold(owed);
            }
            if allowance != 0 {
                self.land_passes += 1;
                let lands = Arc::clone(&self.lands);
                let mut queued = 0;
                let mut cut = lands.len();
                let start = lands.partition_point(|&(order, _)| order < self.land_cursor);
                for step in 0..lands.len() {
                    let (order, slot) = lands[(start + step) % lands.len()];
                    if queued >= allowance {
                        whole = false;
                        cut = step;
                        break;
                    }
                    self.land_cursor = order;
                    let visits = *self.land_visits.entry(slot).or_insert(self.land_passes - 1);
                    if frame.steady
                        || self.land_passes.saturating_sub(visits) > 99
                        || !self.entities.contains_key(&slot)
                    {
                        self.land_visits.insert(slot, self.land_passes);
                        queued += 1;
                        queue(slot, was[Kind::Land as usize]);
                    }
                }
                for step in cut..lands.len() {
                    let (_, slot) = lands[(start + step) % lands.len()];
                    if let Some(visits) = self.land_visits.get_mut(&slot) {
                        *visits += 1;
                    }
                }
            }
        }
        self.land_visits.retain(|slot, _| placed(slot));
        let before = self.entities.len();
        self.entities.retain(|slot, entity| {
            let kind = slot.0 as usize;
            if frame.steady {
                return frame.shown[kind] && placed(slot);
            }
            if !frame.shown[kind] {
                return (!was[kind] || entity.shrink(frame.delta, frame.gates))
                    && entity.shrink(frame.delta, frame.gates);
            }
            placed(slot) || (!was[kind] && entity.shrink(frame.delta, frame.gates))
        });
        viewer_runtime::view_record::add("unit models", "history_destroyed", (before - self.entities.len()) as u64);
        for (&slot, parts) in targets.into_iter().flatten() {
            let kind = slot.0 as usize;
            if !frame.shown[kind] {
                continue;
            }
            if slot.0 == Kind::Land {
                if let Some(entity) = self.entities.get_mut(&slot) {
                    for (&index, &target) in parts {
                        if let Some(part) = entity.parts.get_mut(&index) {
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
            let entity = self.entities.entry(slot).or_insert_with(|| Entity {
                kind: slot.0,
                scale: if frame.steady || was[kind] { 1.0 } else { 0.0 },
                shrinking: false,
                parts: BTreeMap::new(),
            });
            entity.parts.retain(|part, _| parts.contains_key(part));
            for (&index, &target) in parts {
                let replace = entity.parts.get(&index).is_none_or(|part| {
                    part.target.mesh != target.mesh || part.target.occupant != target.occupant
                });
                if replace {
                    if index != 0 {
                        entity.parts.remove(&index);
                    }
                    if !budget.take() {
                        held = true;
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
        self.settle(frame, targets, whole, held);
    }

    fn step_cursor(&mut self) {
        if self.lands.is_empty() {
            return;
        }
        let start = self
            .lands
            .partition_point(|&(order, _)| order < self.land_cursor);
        self.land_cursor = self.lands[(start + self.lands.len() - 1) % self.lands.len()].0;
    }

    fn settle(
        &mut self,
        frame: Frame,
        targets: Option<&BTreeMap<Slot, Parts>>,
        whole: bool,
        held: bool,
    ) {
        self.quiet = whole
            && !held
            && !frame.steady
            && self.entities.iter().all(|(slot, entity)| {
                targets.is_some_and(|targets| targets.contains_key(slot))
                    && frame.shown[slot.0 as usize]
                    && entity.scale >= 1.0
                    && !entity.shrinking
            })
            && (!frame.shown[Kind::Land as usize]
                || (self.land_visits.len() == self.lands.len()
                    && self
                        .lands
                        .iter()
                        .all(|(_, slot)| self.entities.contains_key(slot))));
        self.due_pass = self
            .land_visits
            .values()
            .min()
            .map_or(u64::MAX, |visits| visits + 100);
    }

    fn idle(&self, update: Update, frame: Frame, pass: bool) -> bool {
        if frame.steady || !self.quiet {
            return false;
        }
        match update {
            Update::InView => {
                self.shown == frame.shown
                    && self.arrows == frame.arrows
                    && self.land_passes + u64::from(pass) < self.due_pass
            }
            Update::OffScreen => {
                self.shown == [false; 2] && !self.arrows && self.entities.is_empty()
            }
        }
    }

    fn differences(&self, other: &Self) -> Option<String> {
        if self.shown != other.shown {
            return Some(format!("shown {:?} against {:?}", self.shown, other.shown));
        }
        if self.arrows != other.arrows {
            return Some(format!("arrows {} against {}", self.arrows, other.arrows));
        }
        if self.entities != other.entities {
            return Some(format!(
                "entities {:?} against {:?}",
                self.entities.keys().collect::<Vec<_>>(),
                other.entities.keys().collect::<Vec<_>>()
            ));
        }
        for (name, mine, theirs) in [
            (
                "land_cursor",
                u64::from(self.land_cursor),
                u64::from(other.land_cursor),
            ),
            ("land_passes", self.land_passes, other.land_passes),
            ("due_pass", self.due_pass, other.due_pass),
        ] {
            if mine != theirs {
                return Some(format!("{name} {mine} against {theirs}"));
            }
        }
        if self.land_visits != other.land_visits {
            return Some(format!(
                "land_visits {:?} against {:?}",
                self.land_visits, other.land_visits
            ));
        }
        if self.quiet != other.quiet {
            return Some(format!("quiet {} against {}", self.quiet, other.quiet));
        }
        if self.lands != other.lands {
            return Some("lands".to_owned());
        }
        None
    }

    fn finalize_shown(&mut self, frame: Frame, budget: &Budget) {
        for kind in 0..2 {
            if budget.remaining() != 0 || !frame.shown[kind] {
                self.shown[kind] = frame.shown[kind];
            }
        }
    }

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
        budget.charge();
        viewer_runtime::view_record::add("unit models", "history_created", 1);
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

    fn leave(&mut self, steady: bool) {
        let (shown, arrows) = (self.shown, self.arrows);
        let before = self.entities.len();
        self.entities
            .retain(|slot, _| !steady && !shown[slot.0 as usize] && !arrows);
        viewer_runtime::view_record::add("unit models", "history_left_view", (before - self.entities.len()) as u64);
        self.shown = [false; 2];
        self.arrows = false;
        self.quiet = !steady && self.entities.is_empty();
    }
}

fn judge_idle(
    record: &provinces::Record,
    update: Update,
    frame: Frame,
    epoch: u32,
    budget: &Budget,
    pass: bool,
) {
    let mut eager = record.province.clone();
    let (mut spent, mut incarnation, mut queued) = (budget.clone(), epoch, 0u64);
    match update {
        Update::InView => {
            eager.visit(
                record.targets.as_ref().map(|prepared| &prepared.slots),
                std::iter::empty(),
                frame,
                &mut incarnation,
                &mut spent,
                |_, _| queued += 1,
            );
            eager.finalize_shown(frame, &spent);
        }
        Update::OffScreen => eager.leave(frame.steady),
    }
    let mut left = record.province.clone();
    if pass {
        left.land_passes += 1;
        left.step_cursor();
    }
    if let Some(difference) = left.differences(&eager) {
        let targets: Vec<_> = record
            .targets
            .as_ref()
            .map(|prepared| prepared.slots.keys().collect())
            .unwrap_or_default();
        viewer_scene_base::actor_check::differs(
            "unit walk",
            &record.id,
            &format!("{difference}; the hour holds {targets:?}"),
        );
    }
    if incarnation != epoch || queued != 0 || spent.remaining() != budget.remaining() {
        viewer_scene_base::actor_check::differs(
            "unit walk",
            &record.id,
            &format!(
                "the visit would have spent something: incarnation {incarnation} against \
                 {epoch}, {queued} queued, allowance {} against {}",
                spent.remaining(),
                budget.remaining()
            ),
        );
    }
}

#[derive(Default)]
struct WalkCounts {
    updates: u64,
    in_view: u64,
    off_screen: u64,
    visited: u64,
    empty: u64,
    lands: u64,
    entities: u64,
    candidates: u64,
    idle: u64,
    unchanged: u64,
    counters_only: u64,
    changed: u64,
}

impl WalkCounts {
    fn record(&self) {
        use viewer_runtime::view_record::add;
        for (name, value) in [
            ("updates", self.updates),
            ("in_view", self.in_view),
            ("off_screen", self.off_screen),
            ("visited", self.visited),
            ("empty", self.empty),
            ("lands", self.lands),
            ("entities", self.entities),
            ("candidates", self.candidates),
            ("idle", self.idle),
            ("visits_unchanged", self.unchanged),
            ("visits_counters_only", self.counters_only),
            ("visits_changed", self.changed),
        ] {
            add("unit walk", name, value);
        }
    }

    fn judge(&mut self, before: &Province, after: &Province) {
        if before == after {
            self.unchanged += 1;
            return;
        }
        let mut counted = after.clone();
        counted.land_visits.clone_from(&before.land_visits);
        counted.land_cursor = before.land_cursor;
        if counted == *before {
            self.counters_only += 1;
        } else {
            self.changed += 1;
        }
    }
}

pub struct FrameModels {
    pub units: viewer_models::Moving,
    pub buildings: Option<Vec<(usize, ModelInstance)>>,
}

#[derive(Default)]
pub struct History {
    targets: Targets,
    movement: Vec<(Slot, MovementDelta)>,
    standing: Vec<(usize, ModelInstance)>,
    standing_next: Vec<(usize, ModelInstance)>,
    standing_owed: bool,
    moving: Vec<(usize, MovementDelta)>,
    was_moving: Vec<usize>,
    movement_delta: Option<f32>,
    naval: naval::History,
    combats: naval_combat::History,
    provinces: Provinces,
    updates: Vec<(u32, Update)>,
    queued: Vec<(u32, Slot, bool)>,
    #[cfg(test)]
    queue_trace: Vec<(u32, Slot, bool)>,
    #[cfg(test)]
    left_alone: Vec<u32>,
    province_bounds: Vec<(u32, Option<[f32; 4]>)>,
    province_boxes: u64,
    bounds: Arc<BTreeMap<u32, [f32; 4]>>,
    last: Option<f32>,
    drawn_at: BTreeMap<Slot, ([f32; 3], u32, Option<[f32; 3]>)>,
    schedule: Shared,
    epoch: u32,
    steady: bool,
    checks: u64,
    held_slots: usize,
    held_walks: u32,
    unbudgeted: bool,
    raids: Vec<(usize, ModelInstance)>,
    raid_epoch: u32,
    land_stamp: u64,
}

#[derive(Clone, Copy, Debug)]
struct MovementDelta {
    base: [f32; 3],
    at: [f32; 3],
    turn: f32,
    scale: f32,
}

impl MovementDelta {
    fn new(position: &Position, at: AtHead, rotation: f32, scale: f32) -> Self {
        Self {
            base: position.at(),
            at: at.at,
            turn: at.heading.map_or(0.0, |heading| {
                heading + MESH_FORWARD_OFFSET - rotation
            }),
            scale: at
                .scale
                .map_or(1.0, |drawn| if scale > 0.0 { drawn / scale } else { 1.0 }),
        }
    }

    fn apply(self, mut instance: ModelInstance) -> ModelInstance {
        let (sin, cos) = self.turn.sin_cos();
        let [x, y, z] =
            std::array::from_fn(|axis| (instance.position[axis] - self.base[axis]) * self.scale);
        instance.position = [
            self.at[0] + x * cos - z * sin,
            self.at[1] + y,
            self.at[2] + x * sin + z * cos,
        ];
        instance.rotation += self.turn;
        instance.scale *= self.scale;
        instance
    }
}

impl History {
    pub fn land_stamp(&self) -> u64 {
        self.land_stamp
    }

    fn land_changed(&mut self) {
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
        self.land_stamp = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    }

    pub fn set_selection(&mut self, selected: &BTreeSet<Unit>) {
        self.naval.set_selection(selected);
    }

    pub fn set_movement<'a>(
        &mut self,
        positions: impl Iterator<Item = (Unit, &'a Position, f32, f32)>,
        history: &SharedMovement,
    ) {
        self.land_changed();
        self.movement.clear();
        let mut history = history.lock().unwrap_or_else(|error| error.into_inner());
        self.movement_delta = history.delta();
        for (unit, position, rotation, scale) in positions {
            let slot = match unit {
                Unit::Division(id) => (Kind::Land, id),
                Unit::TaskForce(id) => (Kind::Ship, id),
                Unit::RailwayGun(_) => continue,
            };
            if let Some(at) = history.at_head(unit, position) {
                self.movement
                    .push((slot, MovementDelta::new(position, at, rotation, scale)));
            }
        }
        self.movement.sort_unstable_by_key(|entry| entry.0);
        self.movement.dedup_by_key(|entry| entry.0);
    }

    pub fn prepare_arrow_bounds(
        &mut self,
        map: &content_reader_map::map::MapData,
        known: &Arc<BTreeMap<u32, [f32; 4]>>,
    ) {
        if !self.bounds.is_empty() {
            return;
        }
        self.land_changed();
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
        self.province_boxes += 1;
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

    pub fn arrows_admitted(&self, province: u32) -> bool {
        self.provinces
            .get(&province)
            .is_some_and(|province| province.arrows)
    }
    pub fn set_naval(&mut self, sources: NavalSources) {
        self.land_changed();
        let changed = self
            .provinces
            .ensure(sources.candidates.iter().map(|candidate| candidate.province));
        self.naval.install(sources);
        {
            let _join_span = viewer_runtime::profiling::span("units_join_naval");
            self.provinces.prepare_inputs(&self.targets, &self.naval);
        }
        if changed {
            self.prepare_province_bounds();
        }
    }
    pub fn set(
        &mut self,
        targets: Vec<Target>,
        bounds: Arc<BTreeMap<u32, [f32; 4]>>,
        steady: bool,
    ) {
        self.set_prepared(targets.into(), bounds, steady);
    }

    pub fn set_prepared(
        &mut self,
        targets: Targets,
        bounds: Arc<BTreeMap<u32, [f32; 4]>>,
        steady: bool,
    ) {
        self.set_prepared_join(targets, bounds, steady, true);
    }

    pub fn set_prepared_with_naval(
        &mut self,
        targets: Targets,
        bounds: Arc<BTreeMap<u32, [f32; 4]>>,
        steady: bool,
        sources: NavalSources,
    ) {
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
        self.land_changed();
        let mut changed = false;
        if !bounds.is_empty() && !Arc::ptr_eq(&self.bounds, &bounds) {
            self.provinces.ensure(bounds.keys().copied());
            self.bounds = bounds;
            changed = true;
        }
        changed |= self.provinces.install_targets(&self.targets, &targets);
        self.targets = targets;
        if join {
            let _join_span = viewer_runtime::profiling::span("units_join_prepared");
            self.provinces.prepare_inputs(&self.targets, &self.naval);
        }
        self.steady = steady;
        if changed {
            self.prepare_province_bounds();
        }
    }

    pub fn set_raids(&mut self, raids: Vec<(usize, ModelInstance)>) {
        if self.raid_epoch == 0 {
            self.epoch += 1;
            self.raid_epoch = self.epoch;
        }
        self.raids = raids;
    }

    pub fn settle(&mut self) {
        self.steady = true;
    }

    pub fn field_replaced(&mut self) {
        self.standing_owed = true;
    }

    pub fn restart_live(&mut self) {
        self.reset(None);
        self.last = None;
        self.steady = false;
    }

    pub fn reset(&mut self, provinces: Option<&BTreeSet<u32>>) {
        self.land_changed();
        (self.held_slots, self.held_walks, self.unbudgeted) = (0, 0, true);
        self.naval.reset(provinces);
        self.combats.reset(provinces);
        self.schedule
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .reset(provinces);
        if provinces.is_none() && self.raid_epoch != 0 {
            self.epoch += 1;
            self.raid_epoch = self.epoch;
        }
        self.provinces.reset(provinces);
    }

    pub fn share_schedule(&mut self, schedule: &Shared) {
        if !Arc::ptr_eq(&self.schedule, schedule) {
            self.schedule = Arc::clone(schedule);
        }
    }

    pub fn advance_schedule(&mut self, camera: Camera, viewport: [u32; 2]) {
        let mut schedule = self
            .schedule
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        schedule.advance_frame_boxes(&self.province_bounds, camera, viewport, self.steady, self.province_boxes);
    }

    pub fn schedule(&self) -> &Shared {
        &self.schedule
    }

    pub fn carrying_figures(&self) -> bool {
        self.held_slots != 0
    }

    pub fn take_province_frame(&self, time: f32) -> Option<(ProvinceFrame, FrameRestore)> {
        if self.last.is_some_and(|previous| time < previous) {
            return None;
        }
        self.schedule
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .take_frame()
    }

    pub fn untake_province_frame(&self, restore: FrameRestore) {
        self.schedule
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .untake_frame(restore);
    }

    #[expect(clippy::too_many_arguments, reason = "the camera and viewport, the time and gates, the limit, and the building history")]
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
        self.update_models_with_frame(
            camera,
            viewport,
            time,
            gates,
            limit,
            buildings,
            buildings_enabled,
            None,
        )
    }

    fn floor(&self) -> usize {
        match viewer_runtime::frame_budget::BOUND.saturating_sub(self.held_walks) {
            0 => usize::MAX,
            left => self.held_slots.div_ceil(left as usize),
        }
    }

    fn carried(&mut self, held: usize) {
        self.held_slots = held;
        self.held_walks = if held == 0 { 0 } else { self.held_walks + 1 };
    }

    #[expect(clippy::too_many_arguments, reason = "the camera and viewport, the time and gates, the limit, the building history and the province frame")]
    pub fn update_models_with_frame(
        &mut self,
        camera: Camera,
        viewport: [u32; 2],
        time: f32,
        gates: Gates,
        limit: i32,
        buildings: &mut BuildingHistory,
        buildings_enabled: bool,
        frame: Option<&ProvinceFrame>,
    ) -> FrameModels {
        let _update_span = viewer_runtime::profiling::span("step_update_models");
        let admission = std::cell::LazyCell::new(|| Admission::new(camera, viewport));
        let bounds = Arc::clone(&self.bounds);
        let mut buildings = buildings.frame(camera, viewport, time, buildings_enabled, self.steady);
        use viewer_runtime::frame_budget;
        let deadline = frame_budget::deadline(frame_budget::Mark::Actors);
        if let (Some(spent), Some(room)) = (frame_budget::spent(), frame_budget::room(frame_budget::Mark::Actors)) {
            viewer_runtime::view_record::add("unit models", "budget_spent_us", spent.as_micros() as u64);
            viewer_runtime::view_record::add("unit models", "budget_room_us", room.as_micros() as u64);
        }
        let mut budget = Budget::new(limit, self.steady);
        let placed = self.step_with_frame(
            time,
            gates,
            [gates.units(camera.height), gates.ships(camera.height)],
            camera.height < gates.arrows,
            |id| admission.admits_province(&bounds, id),
            &mut budget,
            |id, budget| buildings.through(id, budget),
            frame,
            deadline,
        );
        let models = FrameModels {
            units: placed,
            buildings: buildings.finish(&mut budget),
        };
        self.carried(budget.held());
        viewer_runtime::view_record::add("unit models", "history_held", self.held_slots as u64);
        match deadline {
            None => frame_budget::check::quiet("unit figures", self.held_slots),
            Some(_) => frame_budget::check::carrying("unit figures", self.held_slots, self.held_walks),
        }
        models
    }

    #[cfg(test)]
    fn update(
        &mut self,
        camera: Camera,
        viewport: [u32; 2],
        time: f32,
        gates: Gates,
    ) -> Vec<(usize, ModelInstance)> {
        let models = self.update_models(
            camera,
            viewport,
            time,
            gates,
            i32::MAX,
            &mut BuildingHistory::default(),
            false,
        );
        self.drawn(&models.units)
    }

    #[cfg(any(test, feature = "test-support"))]
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

    #[cfg(any(test, feature = "test-support"))]
    #[expect(clippy::too_many_arguments, reason = "the time and gates, what is shown and in view, the budget, and the building step")]
    fn step_with(
        &mut self,
        time: f32,
        gates: Gates,
        shown: [bool; 2],
        arrows: bool,
        in_view: impl Fn(u32) -> bool,
        budget: &mut Budget,
        buildings: impl FnMut(u32, &mut Budget),
    ) -> Vec<(usize, ModelInstance)> {
        let moving =
            self.step_with_frame(time, gates, shown, arrows, in_view, budget, buildings, None, None);
        self.drawn(&moving)
    }

    #[cfg(test)]
    fn step_held(&mut self, time: f32, gates: Gates, shown: [bool; 2], budget: &mut Budget) -> Vec<(usize, ModelInstance)> {
        let moving = self.step_with_frame(time, gates, shown, true, |_| true, budget, |_, _| {}, None,
            Some(std::time::Instant::now() - std::time::Duration::from_micros(1)));
        self.carried(budget.held());
        self.drawn(&moving)
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn drawn(
        &self,
        moving: &viewer_models::Moving,
    ) -> Vec<(usize, ModelInstance)> {
        let mut placed = self.standing.clone();
        for &(ordinal, instance) in &moving.moved {
            placed[ordinal].1 = instance;
        }
        placed
    }

    #[expect(clippy::too_many_arguments, reason = "the time and gates, what is shown and in view, the budget, the building step, the province frame and deadline")]
    fn step_with_frame(
        &mut self,
        time: f32,
        gates: Gates,
        shown: [bool; 2],
        arrows: bool,
        in_view: impl Fn(u32) -> bool,
        budget: &mut Budget,
        mut buildings: impl FnMut(u32, &mut Budget),
        province_frame: Option<&ProvinceFrame>,
        deadline: Option<std::time::Instant>,
    ) -> viewer_models::Moving {
        self.land_changed();
        let deadline = deadline.filter(|_| !std::mem::take(&mut self.unbudgeted));
        if deadline.is_some() {
            budget.set_floor(self.floor());
        }
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
        let animators = Frame {
            delta: self.movement_delta.take().unwrap_or(frame.delta),
            ..frame
        };
        self.combats.begin_frame(frame.delta);
        let schedule_span = viewer_runtime::profiling::span("units_schedule");
        self.queued.clear();
        self.updates.clear();
        match province_frame {
            Some(taken) => self.updates.extend_from_slice(taken.updates()),
            None => {
                let schedule = Arc::clone(&self.schedule);
                let mut schedule = schedule
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner());
                if !schedule.take_pending() {
                    schedule.advance(self.provinces.keys().copied(), &in_view, self.steady);
                    schedule.take_pending();
                }
                self.updates.extend(schedule.updates());
                self.updates.sort_unstable_by_key(|&(id, _)| id);
            }
        }
        drop(schedule_span);
        let provinces_span = viewer_runtime::profiling::span("units_provinces");
        let mut counts = WalkCounts::default();
        let judged = viewer_scene_base::actor_check::due(&mut self.checks);
        let mut judged_before = Vec::new();
        let land_pass = frame.shown[Kind::Land as usize];
        let dense = &mut self.provinces;
        let mut at = 0;
        #[cfg(test)]
        self.left_alone.clear();
        for &(id, update) in &self.updates {
            while at < dense.records.len() && dense.records[at].id < id {
                at += 1;
            }
            let Some(record) = dense.records.get_mut(at).filter(|record| record.id == id) else {
                continue;
            };
            counts.updates += 1;
            let pass = land_pass && budget.remaining() != 0;
            if record.province.idle(update, frame, pass)
                && !record.has_candidates
                && record.applied_hour()
                && !self.combats.holds(id)
            {
                counts.idle += 1;
                #[cfg(test)]
                self.left_alone.push(id);
                if judged {
                    judge_idle(record, update, frame, self.epoch, budget, pass);
                }
                if pass {
                    record.province.land_passes += 1;
                    record.province.step_cursor();
                }
                continue;
            }
            if judged {
                judged_before.push((at, record.province.clone()));
            }
            let province = &mut record.province;
            buildings(id, budget);
            match update {
                Update::InView => {
                    counts.in_view += 1;
                    counts.lands += province.lands.len() as u64;
                    counts.entities += province.entities.len() as u64;
                    counts.candidates += u64::from(record.has_candidates);
                    let land_was_shown = province.shown[Kind::Land as usize];
                    let targets = record.targets.as_ref().map(|p| &p.slots);
                    counts.visited += 1;
                    counts.empty += u64::from(
                        province.entities.is_empty() && targets.is_none_or(BTreeMap::is_empty),
                    );
                    let (formations, combat) = self.naval.representatives(id, animators);
                    let queued_before = self.queued.len();
                    province.visit(
                        targets,
                        formations,
                        frame,
                        &mut self.epoch,
                        budget,
                        |slot, shown| self.queued.push((id, slot, shown)),
                    );
                    self.combats.visit(
                        id,
                        self.naval.combat(id, combat),
                        frame,
                        land_was_shown,
                        &mut self.epoch,
                        budget,
                    );
                    province.finalize_shown(frame, budget);
                    if let Some(deadline) = deadline
                        && !budget.is_cut()
                        && self.queued.len() > queued_before
                        && std::time::Instant::now() >= deadline
                    {
                        budget.cut();
                    }
                }
                Update::OffScreen => {
                    counts.off_screen += 1;
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
            let record = &mut dense.records[at];
            record.applied.clone_from(&record.targets);
        }
        drop(provinces_span);
        buildings(u32::MAX, budget);
        let deferred_span = viewer_runtime::profiling::span("units_deferred");
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
            record.province.quiet &= record
                .province
                .entities
                .values()
                .all(|entity| entity.scale >= 1.0 && !entity.shrinking);
        }
        drop(deferred_span);
        if judged {
            for (at, before) in &judged_before {
                counts.judge(before, &self.provinces.records[*at].province);
            }
            viewer_scene_base::actor_check::checked("unit walk", counts.updates);
        }
        counts.record();
        self.combats.advance(frame.delta);
        let raid_epoch = self.raid_epoch;
        let _collect_span = viewer_runtime::profiling::span("units_collect");
        let mut next = std::mem::take(&mut self.standing_next);
        let mut moving = std::mem::take(&mut self.moving);
        next.clear();
        moving.clear();
        self.provinces.standing(&mut next, &mut moving, &self.movement);
        next.extend(self.combats.placed());
        next.extend(self.raids.iter().map(|&(mesh, instance)| {
            (
                mesh,
                ModelInstance {
                    animation_epoch: raid_epoch,
                    ..instance
                },
            )
        }));
        let kept = !std::mem::take(&mut self.standing_owed)
            && next.len() == self.standing.len()
            && std::iter::zip(&next, &self.standing).all(|(now, held)| {
                now.0 == held.0 && bytemuck::bytes_of(&now.1) == bytemuck::bytes_of(&held.1)
            });
        std::mem::swap(&mut self.standing, &mut next);
        self.standing_next = next;
        let mut moved: Vec<(usize, ModelInstance)> = moving
            .iter()
            .map(|&(ordinal, delta)| (ordinal, delta.apply(self.standing[ordinal].1)))
            .collect();
        if kept {
            moved.extend(self.was_moving.iter().copied().filter_map(|ordinal| {
                moving
                    .binary_search_by_key(&ordinal, |entry| entry.0)
                    .is_err()
                    .then(|| (ordinal, self.standing[ordinal].1))
            }));
            moved.sort_unstable_by_key(|entry| entry.0);
        }
        self.was_moving.clear();
        self.was_moving
            .extend(moving.iter().map(|&(ordinal, _)| ordinal));
        self.moving = moving;
        if viewer_runtime::view_record::on() {
            use viewer_runtime::view_record::add;
            add("unit walk", "standing", self.standing.len() as u64);
            add("unit walk", "standing_kept", u64::from(kept));
            add("unit walk", "moved_parts", moved.len() as u64);
            self.note_motion();
        }
        viewer_models::Moving {
            standing: (!kept).then(|| self.standing.clone()),
            moved,
        }
    }

    fn note_motion(&mut self) {
        use viewer_runtime::view_record::{add, set};
        let distance = |a: [f32; 3], b: [f32; 3]| (a[0] - b[0]).hypot(a[2] - b[2]);
        let mut now = BTreeMap::new();
        for at in self.provinces.populated.iter() {
            for (slot, entity) in &self.provinces.records[at].province.entities {
                let Some(part) = entity.parts.values().next().filter(|_| slot.0 == Kind::Land) else { continue };
                let delta = self.movement.binary_search_by_key(slot, |entry| entry.0).ok().map(|index| self.movement[index].1);
                let root = delta.map_or(part.target.instance, |delta| delta.apply(part.target.instance)).position;
                now.insert(*slot, (root, part.identity, delta.map(|delta| delta.at)));
            }
        }
        let (mut jumps, mut restarts, mut appeared) = (0, 0, 0);
        for (slot, &(root, identity, drawn)) in &now {
            let Some(&(was, was_identity, was_drawn)) = self.drawn_at.get(slot) else {
                appeared += 1;
                continue;
            };
            let allowed = drawn.zip(was_drawn).map_or(0.0, |(drawn, was)| distance(drawn, was));
            if distance(root, was) > allowed + MOTION_SLACK {
                jumps += 1;
            }
            restarts += u64::from(identity != was_identity);
        }
        add("units", "jumps", jumps);
        add("units", "restarts", restarts);
        add("units", "appeared", appeared);
        set("units", "figures", now.len() as u64);
        self.drawn_at = now;
    }

    pub fn land_figures(&self) -> Vec<LandFigure> {
        let mut figures = Vec::new();
        for at in self.provinces.populated.iter() {
            let record = &self.provinces.records[at];
            for (slot, entity) in &record.province.entities {
                if slot.0 != Kind::Land || entity.shrinking {
                    continue;
                }
                let delta = self
                    .movement
                    .binary_search_by_key(slot, |entry| entry.0)
                    .ok()
                    .map(|index| self.movement[index].1);
                let order = entity.parts.values().next().map_or(0, |part| part.target.order);
                figures.extend(entity.placed().map(|(mesh, instance)| LandFigure {
                    province: record.id,
                    order,
                    mesh,
                    instance: delta.map_or(instance, |delta| delta.apply(instance)),
                }));
            }
        }
        figures
    }

    pub fn combat_warnings(&self) -> Vec<String> {
        self.combats.warnings()
    }

    pub fn combat_clocks(
        &self,
    ) -> BTreeMap<(usize, u32, u32), viewer_models::entity_animation::Clock> {
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

    fn walk(h: &mut History, time: f32) -> viewer_models::Moving {
        h.step_with_frame(time, GATES, [true; 2], true, |_| true, &mut Budget::new(i32::MAX, h.steady), |_, _| {}, None, None)
    }

    #[test]
    fn a_walk_after_an_upload_places_its_parts_though_none_moved() {
        let mut h = history(vec![target(1, Kind::Land)], true);
        walk(&mut h, 0.0);
        assert!(walk(&mut h, 0.1).standing.is_none(), "a walk that moved nothing keeps the field");
        h.field_replaced();
        assert!(walk(&mut h, 0.2).standing.is_some(), "the walk after an upload places its parts");
        assert!(walk(&mut h, 0.3).standing.is_none(), "and once placed, the field is kept again");
        let mut empty = history(Vec::new(), true);
        walk(&mut empty, 0.0);
        empty.field_replaced();
        assert!(walk(&mut empty, 0.1).standing.is_some_and(|standing| standing.is_empty()), "an hour with no unit clears the uploaded field");
    }

    #[test]
    fn the_land_stamp_holds_while_the_figures_stand_and_changes_with_any_walk_or_install() {
        let mut h = history(vec![target(1, Kind::Land)], true);
        walk(&mut h, 0.0);
        let (stamp, figures) = (h.land_stamp(), h.land_figures());
        assert_eq!(figures.len(), 1);
        assert_eq!((h.land_stamp(), h.land_figures()), (stamp, figures), "reading the figures changes nothing");
        walk(&mut h, 0.1);
        let walked = h.land_stamp();
        assert_ne!(walked, stamp, "a walk may move the figures");
        h.set(vec![target(2, Kind::Land)], Arc::default(), true);
        assert_ne!(h.land_stamp(), walked, "so may an hour's targets");
        let other = history(vec![target(1, Kind::Land)], true);
        assert_ne!(other.land_stamp(), h.land_stamp(), "no two histories share a stamp");
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
            let (mut whole, mut held) = (true, false);
            let mut queued = Vec::new();
            if frame.shown[Kind::Land as usize] {
                let allowance = budget.remaining();
                if allowance != 0 {
                    self.land_passes += 1;
                    let lands = Arc::clone(&self.lands);
                    let mut cut = lands.len();
                    let start = lands.partition_point(|&(order, _)| order < self.land_cursor);
                    for step in 0..lands.len() {
                        let (order, slot) = lands[(start + step) % lands.len()];
                        if queued.len() >= allowance {
                            whole = false;
                            cut = step;
                            break;
                        }
                        self.land_cursor = order;
                        let visits = *self.land_visits.entry(slot).or_insert(self.land_passes - 1);
                        if frame.steady
                            || self.land_passes.saturating_sub(visits) > 99
                            || !self.entities.contains_key(&slot)
                        {
                            self.land_visits.insert(slot, self.land_passes);
                            queued.push((slot, was[Kind::Land as usize]));
                        }
                    }
                    for step in cut..lands.len() {
                        let (_, slot) = lands[(start + step) % lands.len()];
                        if let Some(visits) = self.land_visits.get_mut(&slot) {
                            *visits += 1;
                        }
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
                    return (!was[kind] || entity.shrink(frame.delta, frame.gates))
                        && entity.shrink(frame.delta, frame.gates);
                }
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
                let entity = self.entities.entry(slot).or_insert_with(|| Entity {
                    kind: slot.0,
                    scale: if frame.steady || was[kind] { 1.0 } else { 0.0 },
                    shrinking: false,
                    parts: BTreeMap::new(),
                });
                entity.parts.retain(|part, _| parts.contains_key(part));
                for (&index, &target) in parts {
                    let replace = entity.parts.get(&index).is_none_or(|part| {
                        part.target.mesh != target.mesh || part.target.occupant != target.occupant
                    });
                    if replace {
                        if index != 0 {
                            entity.parts.remove(&index);
                        }
                        if !budget.take() {
                            held = true;
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
            self.settle(frame, targets, whole, held);
            queued
        }
    }

    #[test]
    fn moving_formations_rotate_hull_offsets_with_their_headings() {
        let mut target = target(1, Kind::Ship);
        target.identity = 7;
        target.instance.position = [2.0, 0.0, 0.0];
        let mut epoch = 0;
        let mut provinces = Provinces::default();
        provinces.ensure([1]);
        provinces.records[0].province.entities.insert(
            (Kind::Ship, 7),
            Entity {
                kind: Kind::Ship,
                scale: 1.0,
                shrinking: false,
                parts: BTreeMap::from([(0, Part::new(target, &mut epoch, true))]),
            },
        );
        provinces.populated.insert(0);
        let turn = std::f32::consts::FRAC_PI_2;
        let (mut standing, mut moving) = (Vec::new(), Vec::new());
        provinces.standing(
            &mut standing,
            &mut moving,
            &[(
                (Kind::Ship, 7),
                MovementDelta {
                    base: [0.0; 3],
                    at: [10.0, 0.0, 0.0],
                    turn,
                    scale: 1.5,
                },
            )],
        );
        let instance = moving[0].1.apply(standing[moving[0].0].1);
        assert!((instance.position[0] - 10.0).abs() < 1e-5);
        assert!((instance.position[2] - 3.0).abs() < 1e-5);
        assert_eq!(instance.rotation, turn);
        assert_eq!(instance.scale, 1.5);
    }

    #[test]
    fn a_glide_into_a_standing_province_takes_its_heading_whatever_the_standing_rotation() {
        let position = Position::still([3.0, 0.0, 4.0]);
        let motion = viewer_unit_reads::counter_move::Motion::new(
            1000.0,
            &[[0.0, 0.0, 0.0], [0.0, 0.0, 100.0]],
        )
        .unwrap();
        let (at, heading) = motion.position_heading();
        let drawn = AtHead { at, heading: Some(heading), scale: Some(1.0) };
        let expected = heading + MESH_FORWARD_OFFSET;
        for standing in [0.0, 1.25, -2.5] {
            let delta = MovementDelta::new(&position, drawn, standing, 1.0);
            let instance = delta.apply(ModelInstance {
                position: position.standing,
                rotation: standing,
                ..ModelInstance::default()
            });
            assert!((instance.rotation - expected).abs() < 1e-5, "{standing}");
            assert_eq!(instance.position, at);
        }
    }

    #[test]
    fn movement_deltas_reach_retained_figures_without_replacing_their_players() {
        let mut h = history(vec![target(1, Kind::Land)], true);
        let first = near(&mut h, 0.0)[0].1;
        for x in [0.25, 0.5, 0.75] {
            h.movement = vec![(
                (Kind::Land, 1),
                MovementDelta {
                    base: [0.0; 3],
                    at: [x, 0.0, 0.0],
                    turn: 0.0,
                    scale: 1.0,
                },
            )];
            let shown = near(&mut h, x)[0].1;
            assert_eq!(shown.position, [x, 0.0, 0.0]);
            assert_eq!(shown.animation_identity, first.animation_identity);
            assert_eq!(shown.animation_epoch, first.animation_epoch);
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
    fn a_province_left_alone_ends_where_every_visit_would_have() {
        let (mut eager, mut lazy) = (Province::default(), Province::default());
        let (mut eager_epoch, mut lazy_epoch) = (0, 0);
        let mut applied: Option<Arc<ProvinceTargets>> = None;
        let mut held: Option<Arc<ProvinceTargets>> = None;
        let (mut left, mut visited) = (0, 0);
        for number in 0..900u32 {
            let hour = number / 97;
            let targets = Targets::from(
                (0..5u64)
                    .map(|slot| Target {
                        identity: slot,
                        order: (slot % 3) as u32,
                        mesh: (hour as usize + slot as usize) % 3,
                        ..target(5, Kind::Land)
                    })
                    .collect::<Vec<_>>(),
            );
            let prepared = Arc::clone(&targets.0.provinces[&5]);
            let prepared = match &held {
                Some(table) if **table == *prepared => Arc::clone(table),
                _ => {
                    held = Some(Arc::clone(&prepared));
                    prepared
                }
            };
            for province in [&mut eager, &mut lazy] {
                province.lands = Arc::clone(&prepared.lands);
            }
            let frame = Frame {
                delta: if number % 7 == 0 { 0.0 } else { 1.0 / 60.0 },
                gates: GATES,
                shown: [number % 211 < 180, number % 307 < 280],
                arrows: number % 401 < 380,
                steady: number % 457 == 456,
            };
            let limit = if number % 199 == 0 { 0 } else { 64 };
            let (mut eager_budget, mut lazy_budget) =
                (Budget::new(limit, frame.steady), Budget::new(limit, frame.steady));
            let mut queue = Vec::new();
            eager.visit(
                Some(&prepared.slots),
                std::iter::empty(),
                frame,
                &mut eager_epoch,
                &mut eager_budget,
                |slot, shown| queue.push((slot, shown)),
            );
            eager.finalize_shown(frame, &eager_budget);
            for (slot, shown) in queue.drain(..) {
                eager.create_land(
                    slot,
                    &prepared.slots[&slot],
                    shown,
                    frame.steady,
                    &mut eager_epoch,
                    &mut eager_budget,
                );
            }
            eager.quiet &= eager
                .entities
                .values()
                .all(|entity| entity.scale >= 1.0 && !entity.shrinking);
            let hour_kept = applied
                .as_ref()
                .is_some_and(|applied| Arc::ptr_eq(applied, &prepared));
            let pass = frame.shown[Kind::Land as usize] && lazy_budget.remaining() != 0;
            if lazy.idle(Update::InView, frame, pass) && hour_kept {
                left += 1;
                if pass {
                    lazy.land_passes += 1;
                    lazy.step_cursor();
                }
            } else {
                visited += 1;
                lazy.visit(
                    Some(&prepared.slots),
                    std::iter::empty(),
                    frame,
                    &mut lazy_epoch,
                    &mut lazy_budget,
                    |slot, shown| queue.push((slot, shown)),
                );
                lazy.finalize_shown(frame, &lazy_budget);
                for (slot, shown) in queue.drain(..) {
                    lazy.create_land(
                        slot,
                        &prepared.slots[&slot],
                        shown,
                        frame.steady,
                        &mut lazy_epoch,
                        &mut lazy_budget,
                    );
                }
                lazy.quiet &= lazy
                    .entities
                    .values()
                    .all(|entity| entity.scale >= 1.0 && !entity.shrinking);
                applied = Some(Arc::clone(&prepared));
            }
            assert_eq!(lazy.differences(&eager), None, "frame {number}");
            let settled = &lazy;
            assert_eq!(eager_epoch, lazy_epoch, "frame {number} incarnations");
            assert_eq!(
                eager_budget.remaining(),
                lazy_budget.remaining(),
                "frame {number} allowance"
            );
            assert_eq!(
                eager.entities.values().flat_map(Entity::placed).collect::<Vec<_>>(),
                settled.entities.values().flat_map(Entity::placed).collect::<Vec<_>>(),
                "frame {number} instances"
            );
        }
        assert!(left > 600, "{left} frames left alone, {visited} visited");
        assert!(visited > 20, "{visited} visited");
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
        let placed = limited(&mut h, 0.0, 0);
        assert_eq!(placed.len(), 1);
        let first = raids(&placed)[0];
        assert_eq!(first.scale, 2.0);
        assert_ne!(first.animation_epoch, 0, "its clock starts at creation");
        let hidden = h.step(0.5, GATES, [false; 2], false, |_| false);
        assert_eq!(raids(&hidden), [first]);
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
        h.set(vec![land(1, 10), land(2, 40)], Arc::default(), false);
        assert_eq!(meshes(limited(&mut h, 0.3, 1)), [10]);
    }

    #[test]
    fn a_frame_that_has_spent_its_room_carries_its_figures_and_makes_them_all_within_the_bound() {
        use viewer_runtime::frame_budget::BOUND;
        let targets: Vec<Target> = (1..=8)
            .flat_map(|province| {
                (0..3).map(move |order| Target {
                    order,
                    identity: u64::from(province * 10 + order),
                    mesh: (province * 10 + order) as usize,
                    ..target(province, Kind::Land)
                })
            })
            .collect();
        let whole: BTreeSet<usize> = targets.iter().map(|target| target.mesh).collect();
        let mut h = history(targets.clone(), false);
        let mut made: Vec<usize> = Vec::new();
        let mut epochs: BTreeMap<usize, u32> = BTreeMap::new();
        let mut gates = GATES;
        for frame in 0..=BOUND {
            if frame == 1 {
                gates.arrows = GATES.arrows * 0.5;
            }
            let mut budget = Budget::new(i32::MAX, false);
            let placed = h.step_held(frame as f32 * 0.1, gates, [true; 2], &mut budget);
            for (mesh, instance) in &placed {
                let (seed, _) = instance.animation_key();
                match epochs.entry(*mesh) {
                    std::collections::btree_map::Entry::Vacant(slot) => {
                        slot.insert(seed);
                        made.push(*mesh);
                    }
                    std::collections::btree_map::Entry::Occupied(held) => {
                        assert_eq!(*held.get(), seed, "mesh {mesh} was made again on frame {frame}");
                    }
                }
            }
            if frame == 0 {
                assert!(h.carrying_figures());
                assert!(placed.len() < whole.len(), "a frame with no room left holds some of the hour's figures back");
                assert!(!placed.is_empty(), "and still advances what it can");
            }
        }
        assert_eq!(made.len(), whole.len(), "every figure is made");
        assert_eq!(made.iter().copied().collect::<BTreeSet<_>>(), whole);
        assert!(
            !h.carrying_figures(),
            "and nothing is left pending at the bound"
        );
        let mut budget = Budget::new(i32::MAX, false);
        let paused = h.step_held(0.4, gates, [true; 2], &mut budget);
        assert_eq!(paused.len(), whole.len(), "a paused frame shows every figure");
        assert_eq!(h.held_slots, 0);
        h.reset(None);
        h.set(targets, Arc::default(), false);
        let mut budget = Budget::new(i32::MAX, false);
        let sought = h.step_held(0.5, gates, [true; 2], &mut budget);
        assert_eq!(sought.len(), whole.len(), "a seek's first picture is whole");
        assert_eq!(h.held_slots, 0);
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
        let mut h = history(vec![target(104, Kind::Land), target(99, Kind::Ship)], false);
        near(&mut h, 0.0);
        let full = near(&mut h, 1.0);
        let hidden = h.step(1.1, GATES, [true; 2], true, |_| false);
        assert_eq!(hidden, full, "pan-out does not fade");
        let hidden = h.step(1.2, GATES, [true; 2], true, |_| false);
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
        h.step(1.1, GATES, [false; 2], false, |_| true);
        h.step(1.2, GATES, [false; 2], false, |_| false);
        let held = h.step(1.3, GATES, [false; 2], false, |_| false);
        assert_eq!(held[0].1.animation_epoch, epoch);
        assert_eq!(near(&mut h, 1.4)[0].1.animation_epoch, epoch);
    }

    #[test]
    fn a_map_mode_without_land_models_acts_as_a_camera_above_their_cutoff() {
        use viewer_map_modes::map_mode::{MapMode, Stock, visibility};
        use viewer_gpu::camera::{MapDefines, MapSize, Placement};
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
        use viewer_gpu::camera::{MapDefines, MapSize, Placement};
        let mut h = history(vec![target(7, Kind::Land), target(8, Kind::Ship)], true);
        h.bounds = Arc::new(BTreeMap::from([
            (7, [0.0, 0.0, 2.0, 2.0]),
            (8, [3000.0, 1000.0, 3002.0, 1002.0]),
        ]));
        let camera = Placement {
            center: [0.0, 0.0],
            height: 50.0,
            pose: viewer_gpu::camera::Pose::Map,
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
        let defines = content_reader_defines::defines::Defines::new().unwrap();
        defines.load("synthetic.lua", br#"NDefines = { NGraphics = {
            UNITS_DISTANCE_CUTOFF=11, SHIPS_DISTANCE_CUTOFF=22, UNIT_ARROW_DISTANCE_CUTOFF=33, SHIP_IN_PORT_SCALE=.125,
            MESH_POPUP_SCALE_UP_SPEED=7, MESH_POPUP_SCALE_DOWN_SPEED=9, SHIP_POPUP_SCALE_DOWN_SPEED=13,
            NAVAL_UNIT_MOVEMENT_SPEED=14, LAND_UNIT_MOVEMENT_SPEED=15
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
    fn the_units_walk_the_phase_their_map_icons_advanced() {
        let mut h = history(vec![target(1, Kind::Land)], false);
        let schedule = Shared::default();
        h.share_schedule(&schedule);
        schedule.lock().unwrap().advance([1], |_| false, false);
        h.step(0.0, GATES, [true, true], false, |_| false);
        assert_eq!(schedule.lock().unwrap().update(1), Some(Update::OffScreen));
        assert!(!schedule.lock().unwrap().dirty(1));
        h.step(0.1, GATES, [true, true], false, |_| false);
        assert_eq!(schedule.lock().unwrap().update(1), None);
        h.reset(None);
        assert!(schedule.lock().unwrap().dirty(1));
    }

    #[test]
    fn cut_frames_place_and_schedule_like_walks_that_take_their_own() {
        let targets: Vec<_> = (1..=40).map(|id| target(id, Kind::Land)).collect();
        let arm = || {
            let mut h = history(targets.clone(), false);
            let schedule = Shared::default();
            h.share_schedule(&schedule);
            (h, schedule)
        };
        let ((mut walk, walk_schedule), (mut cut, cut_schedule)) = (arm(), arm());
        let mut time = 0.0f32;
        let mut cut_walks = 0;
        for frame in 0..400u32 {
            time = if frame % 53 == 52 {
                time - 0.25
            } else {
                time + 1.0 / 60.0
            };
            let in_view = move |id: u32| !(id + frame / 7).is_multiple_of(4);
            for schedule in [&walk_schedule, &cut_schedule] {
                let mut schedule = schedule.lock().unwrap();
                if frame % 5 != 4 {
                    schedule.advance(1..=40, in_view, frame.is_multiple_of(41));
                }
                if frame % 9 == 3 {
                    schedule.set_selected(BTreeSet::from([frame % 40 + 1, 7]));
                }
                if frame % 17 == 8 {
                    schedule.set_selected(BTreeSet::new());
                }
                if frame % 31 == 30 {
                    schedule.cancel_pending();
                }
                if frame % 47 == 46 {
                    schedule.reset(Some(&BTreeSet::from([2, 3, 5])));
                }
            }
            let taken = cut.take_province_frame(time);
            if frame % 13 == 12 {
                if let Some((_, restore)) = taken {
                    cut.untake_province_frame(restore);
                }
            } else {
                cut_walks += usize::from(taken.is_some());
                let walked = walk.step(time, GATES, [true; 2], true, in_view);
                let steady = cut.steady;
                let placed = cut.step_with_frame(
                    time,
                    GATES,
                    [true; 2],
                    true,
                    in_view,
                    &mut Budget::new(i32::MAX, steady),
                    |_, _| {},
                    taken.as_ref().map(|(frame, _)| frame),
                    None,
                );
                assert_eq!(cut.drawn(&placed), walked, "frame {frame}");
                assert_eq!(cut.updates, walk.updates, "frame {frame}");
            }
            {
                let cut_guard = cut_schedule.lock().unwrap();
                let walk_guard = walk_schedule.lock().unwrap();
                assert_eq!(
                    cut_guard.presentation_state(),
                    walk_guard.presentation_state(),
                    "frame {frame}"
                );
            }
        }
        assert!(cut_walks > 200, "{cut_walks} cut walks");
    }
}

#[cfg(test)]
mod model_lifetime_tests;

#[cfg(test)]
mod building_budget_tests;
