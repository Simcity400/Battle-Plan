//! Authored building slots and their selected model entities.
//!
//! A slot is drawn while its order/max < level/`NBuildings.MAX_BUILDING_LEVELS`;
//! multi-mesh types select min(floor(meshes*level/MAX), meshes-1). Health is
//! healthy_level*10_000_000 + partial_health (fixed5 percent), compared with
//! the `NGraphics.MAP_BUILDINGS_*` thresholds. [`BuildingHistory`] keeps each
//! slot's destroyed selector through the hysteresis band and across hiding,
//! and keeps every entity created below the update cutoff drawn above it, from
//! this viewer's own camera history; a cold start is healthy and uncreated, as
//! the native slot constructor is. Damaged entities play their authored
//! animation and smoke/fire events on the viewer's state-entry clock. Geometry
//! grows on the frame clock; offline renders use the settled scale.

use std::{
    collections::{BTreeMap, BTreeSet},
    io,
    sync::Arc,
};
use viewer_foundation::map::building_facts::{SlotAdmission, admit_slot, is_floating_harbor_slot};
use viewer_foundation::table_access::Refusals;
use viewer_foundation::table_access::refuse;

use installed_content_core::{
    ResolvedLayers, buildings::BuildingType, buildings::Buildings, buildings::Slot, map::MapData,
    mesh, positions::BuildingPosition,
};
use installed_content_image::native::Decode;
use replay_data::{
    rows::BuildingRow, rows::FloatingHarborDeploymentRow, rows::ProvinceSupplyRow, rows::TableId,
    rows::tables,
};
use replay_frame::frame::DecodedFrame;

mod incremental;
pub use incremental::{BuildPath, BuildingsBase};
use incremental::{Chunked, Tally};

use viewer_foundation::table_access::{TokenKeys, TokenNames};
use {
    super::{
        entity_animation::{BUILDING_DAMAGED, BUILDING_IDLE},
        models::{ModelInstance, ModelLibrary, WATER_HEIGHT},
    },
    viewer_foundation::map::{
        camera::Camera,
        memo::Keyed,
        perspective::Vision,
        province_admission::{Admission, province_boxes},
        tables::{core, rows},
    },
};

/// The game routes this reserved slot name to the floating-harbour path.
/// The game appends `_entity` to the reserved name.
const FLOATING_HARBOR_ENTITY: &str = "floating_harbor_entity";
/// the game's floating-harbour fog gate: hidden below 0x96 with fog on.
const FLOATING_HARBOR_VISIBILITY: u8 = 0x96;

/// Side of the map-building cull grid, in world units. A state's six factory
/// slots sit within a few tens of units of each other, so 128 keeps them in one
/// group without merging neighbouring states.
pub const BUILDING_TILE: f32 = 128.0;

fn unselected_meshless_states(descriptors: &mesh::Descriptors, name: &str) -> Vec<String> {
    let Some(entity) = descriptors
        .inherit(name)
        .filter(|entity| entity.mesh.is_none())
    else {
        return Vec::new();
    };
    // Building updates request idle/damaged after initial default-state selection.
    // Follow authored next_state links before calling any other state unreached.
    let mut pending = vec!["idle", "damaged", entity.default_state.unwrap_or("idle")];
    let mut reached = BTreeSet::new();
    while let Some(name) = pending.pop() {
        if reached.insert(name)
            && let Some(states) = entity.states.get(name)
        {
            pending.extend(
                states
                    .iter()
                    .filter_map(|state| state.next_state.as_deref()),
            );
        }
    }
    entity.states.iter().filter(|(state, _)| !reached.contains(**state))
        .filter_map(|(state, variants)| {
            let count: usize = variants.iter().map(|state| super::models::map_effects(state)).sum();
            (count != 0).then(|| format!("entity {name} state {state}: {count} map effects refused; state is not selected by the building producer"))
        }).collect()
}

/// The content's map-building operands. The game converts the f32
/// thresholds through the fixed5 scale when it compares health.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Defines {
    /// `NGraphics.MAP_BUILDINGS_SHRINK_DISTANCE`: the update cutoff.
    pub shrink_distance: f32,
    /// `MAP_BUILDINGS_DESTROYED_STATUS`: at or below, the destroyed mesh.
    pub destroyed: i64,
    /// `MAP_BUILDINGS_DESTROYED_DEAD_ZONE`: above, recovered.
    pub recovered: i64,
    /// `MAP_BUILDINGS_DAMAGED_THRESHOLD`: below, the damaged state.
    pub damaged: i64,
    /// `NBuildings.MAX_BUILDING_LEVELS`.
    pub max_levels: i32,
    /// `NGraphics.MESH_POPUP_SCALE_UP_SPEED`: scale per second of the popup
    /// and the shrink step when hiding a building entity.
    pub popup_speed: f32,
}

impl Defines {
    pub fn load(defines: &installed_content_core::defines::Defines) -> io::Result<Self> {
        let graphics = |name| defines.fixed5("NGraphics", name);
        let max_levels = defines.int32("NBuildings", "MAX_BUILDING_LEVELS")?;
        if max_levels <= 0 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("NDefines.NBuildings.MAX_BUILDING_LEVELS {max_levels} is not positive"),
            ));
        }
        Ok(Self {
            shrink_distance: defines.number("NGraphics", "MAP_BUILDINGS_SHRINK_DISTANCE")? as f32,
            popup_speed: defines.number("NGraphics", "MESH_POPUP_SCALE_UP_SPEED")? as f32,
            destroyed: graphics("MAP_BUILDINGS_DESTROYED_STATUS")?,
            recovered: graphics("MAP_BUILDINGS_DESTROYED_DEAD_ZONE")?,
            damaged: graphics("MAP_BUILDINGS_DAMAGED_THRESHOLD")?,
            max_levels,
        })
    }
}

use viewer_foundation::map::province_map::{ProvinceMap, pixel};

/// `map/terrain.bmp`'s city-spawning flag, read with the slot table's own
/// truncate-and-index rule.
pub struct CityTerrain<'a> {
    pub width: u32,
    pub height: u32,
    /// One flag per pixel, rows top-down: `viewer_foundation::map::textures::MapAssets::spawn_city`.
    pub spawn_city: &'a [bool],
}

impl CityTerrain<'_> {
    fn at(&self, x: f32, z: f32) -> bool {
        pixel(self.width, self.height, x, z)
            .and_then(|index| self.spawn_city.get(index).copied())
            .unwrap_or(false)
    }
}

/// Whether this camera creates or updates building entities: the game
/// updates a province's records while `height - WATER_HEIGHT` is below
/// `MAP_BUILDINGS_SHRINK_DISTANCE`. Above it nothing is created or updated,
/// but an entity created below stays drawn; see [`BuildingHistory`].
/// `DRAW_MAP_OBJECTS_CUTOFF` only toggles the province's own entity;
/// it does not update building records.
pub fn draws_map_buildings(height: f32, shrink_distance: f32) -> bool {
    height - WATER_HEIGHT < shrink_distance
}

/// Strictly compare the slot fraction with the level divided by MAX_BUILDING_LEVELS.
pub fn slots_for_level(building: &BuildingType, level: i16, max_levels: i32) -> u32 {
    let Some(shown) = building.show_on_map.filter(|shown| *shown > 0) else {
        return 0;
    };
    if level <= 0 {
        return 0;
    }
    let fraction = f32::from(level) / max_levels as f32;
    (0..shown)
        .filter(|order| *order as f32 / (shown as f32) < fraction || fraction >= 1.0)
        .count() as u32
}

/// (destroyed, damaged, in the hysteresis band) for one row.
pub fn health_state(
    level: i16,
    healthy: i16,
    partial: i64,
    defines: &Defines,
) -> (bool, bool, bool) {
    // Keep the total in signed 64 bits and separately
    // computes the average with wrapping fixed5 arithmetic and sentinel words.
    let total = (i64::from(healthy) * 10_000_000).wrapping_add(partial);
    let destroyed = total <= defines.destroyed;
    let uncertain = !destroyed && total <= defines.recovered;
    let damaged =
        level > 0 && super::sprawl::average_health(level, healthy, partial) < defines.damaged;
    (destroyed, damaged, uncertain)
}

/// An hour's slot targets, before the scene's camera history applies them.
/// The hour is part of equality so a seek always reaches the scene.
#[derive(Clone, Debug, PartialEq)]
pub struct PreparedBuildings {
    branch: replay_data::BranchId,
    hour: replay_data::Hour,
    shrink_distance: f32,
    popup_speed: f32,
    slots: Chunked<PreparedSlot>,
    /// The slots of each province, which the camera admits together.
    groups: Arc<[ProvinceSlots]>,
    pub city_damage: Arc<super::sprawl::Damage>,
}

/// One province's box and its slots' ids.
#[derive(Clone, Debug, PartialEq)]
struct ProvinceSlots {
    province: u32,
    /// The province's box ([`province_boxes`]); the camera admits it before its update.
    bounds: [f32; 4],
    slots: Vec<u32>,
}

#[derive(Clone, Debug, PartialEq)]
struct PreparedSlot {
    /// Stable authored-slot ordinal, its index in `slots`.
    id: u32,
    /// The selected type's `drawn_at_distance`; `None` leaves the retained
    /// byte alone, as no positive-level type does natively.
    drawn_at_distance: Option<bool>,
    entity: Option<PreparedEntity>,
}

/// The entity request for one damage state.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Selection {
    admitted: bool,
    /// The drawn mesh: the damaged pose's own copy when damaged.
    mesh: usize,
    /// The entity definition before the damaged-pose choice.
    definition: usize,
    /// The variant selector.
    variant: usize,
}

#[derive(Clone, Debug, PartialEq)]
struct PreparedEntity {
    healthy: Selection,
    /// `destroyed` also selects the slot's ruined entity.
    ruined: Selection,
    destroyed: bool,
    uncertain: bool,
    damaged: bool,
    disable_grow_animation: bool,
    instance: ModelInstance,
}

type Placed = (usize, ModelInstance);

#[derive(Clone, Debug, Default)]
struct SlotHistory {
    destroyed: bool,
    drawn_at_distance: bool,
    entity: Option<Placed>,
    generation: u32,
    /// The drawn entity's definition, `None` exactly when none is drawn.
    definition: Option<usize>,
    /// The latched variant selector, kept like `destroyed`.
    variant: usize,
}

/// The renderer state the engine keeps per slot and a recording cannot hold.
///
/// New slots start zeroed; creation latches the destroyed
/// selector only on admitted creation, and hiding does not clear it.
/// Above the update cutoff the game touches a visible province once per
/// near-to-far transition (province byte `0x10`): landmarks (`drawn_at_distance`)
/// are left alone and every other entity takes one hiding step,
/// scale minus frame time times `MESH_POPUP_SCALE_UP_SPEED` and the idle
/// state, which removes it only if the scale reaches zero. Created entities therefore stay drawn, frozen,
/// at far zoom. The step uses this viewer's frame time; a dump has none.
///
/// Playback keeps this history, and a forward skip advances it through the
/// skipped hours that were built; playing, it passes over those the builders
/// did not reach, unless the recording has a gap there. A paused seek, a
/// branch change or a backwards clock starts cold.
/// The drawn entities are kept by cull tile, and the instance stream is
/// rebuilt only when an entity in it changes or the view leaves the rectangle
/// last collected, the visible ground widened by half on each side.
#[derive(Default)]
pub struct BuildingHistory {
    prepared: Option<PreparedBuildings>,
    slots: BTreeMap<u32, SlotHistory>,
    /// Drawn entities by cull tile, then slot.
    tiles: BTreeMap<(i32, i32), BTreeMap<u32, Placed>>,
    /// Provinces (groups) updated near since their last far step: the
    /// province byte's `0x10`.
    near: BTreeSet<usize>,
    /// Near groups already applied at full scale for the installed hour.
    settled: Vec<bool>,
    settled_steady: Option<bool>,
    last_time: Option<f32>,
    last_view: Option<(Camera, [u32; 2])>,
    /// The rectangle last collected; `Some(None)` collected every tile.
    collected: Option<Option<[f32; 4]>>,
    /// The hour changed and the slots in view need updating.
    dirty: bool,
    /// An entity changed since the stream was last collected.
    changed: bool,
}

impl BuildingHistory {
    /// Installs an hour. `continued` is false for a seek. Returns true when
    /// entity clocks must start cold too.
    pub fn set(&mut self, prepared: PreparedBuildings, continued: bool) -> bool {
        let reset = !continued
            || self.prepared.as_ref().is_none_or(|previous| {
                previous.branch != prepared.branch || prepared.hour < previous.hour
            });
        if reset {
            self.clear();
            self.last_time = None;
            self.last_view = None;
        }
        self.settled.clear();
        self.settled.resize(prepared.groups.len(), false);
        self.prepared = Some(prepared);
        self.dirty = true;
        reset
    }

    /// Applies a skipped hour at the last camera, as the frames the engine
    /// drew during it would have. Ignored unless it continues the history.
    pub fn advance(&mut self, prepared: &PreparedBuildings) {
        let continues = self.prepared.as_ref().is_some_and(|previous| {
            previous.branch == prepared.branch && prepared.hour > previous.hour
        });
        if let (true, Some((camera, viewport))) = (continues, self.last_view) {
            self.apply(prepared, camera, viewport, 0.0, false);
            self.settled.fill(false);
        }
    }

    pub fn restart_live(&mut self) {
        self.clear();
        self.last_time = None;
        self.last_view = None;
        self.dirty = true;
    }

    fn clear(&mut self) {
        self.slots.clear();
        self.tiles.clear();
        self.near.clear();
        self.settled.fill(false);
        self.settled_steady = None;
        self.changed = true;
    }

    /// Starts one frame. The caller visits provinces in ascending ID order,
    /// interleaving each province's buildings with its units and hulls.
    pub fn frame(
        &mut self,
        camera: Camera,
        viewport: [u32; 2],
        time: f32,
        enabled: bool,
        steady: bool,
    ) -> BuildingFrame<'_> {
        if self.last_time.is_some_and(|previous| time < previous) {
            self.clear();
            self.dirty = true;
        }
        let delta = self.last_time.map_or(0.0, |previous| time - previous);
        self.last_time = Some(time);
        let active = enabled && (self.dirty || self.last_view != Some((camera, viewport)));
        if enabled {
            self.last_view = Some((camera, viewport));
            self.dirty = false;
        }
        if self.settled_steady != Some(steady) {
            self.settled.fill(false);
            self.settled_steady = Some(steady);
        }
        if let Some(prepared) = &self.prepared {
            self.settled.resize(prepared.groups.len(), false);
        }
        let prepared = self.prepared.take();
        BuildingFrame {
            history: self,
            prepared,
            camera,
            viewport,
            delta,
            enabled,
            steady,
            active,
            next: 0,
            admission: Admission::new(camera, viewport),
        }
    }

    #[cfg(test)]
    fn update(
        &mut self,
        camera: Camera,
        viewport: [u32; 2],
        time: f32,
        enabled: bool,
    ) -> Option<Vec<Placed>> {
        self.frame(camera, viewport, time, enabled, true)
            .finish(&mut super::mesh_budget::Budget::new(0, true))
    }

    fn stream(&mut self, camera: Camera, viewport: [u32; 2]) -> Option<Vec<Placed>> {
        // The renderer culls with the same margin, so nothing pops in at edges.
        let visible = camera.visible_ground(viewport, super::terrain::MARGIN);
        let inside = match (self.collected, visible) {
            (Some(None), _) => true,
            (Some(Some(rect)), Some(view)) => {
                rect[0] <= view[0] && rect[1] <= view[1] && view[2] <= rect[2] && view[3] <= rect[3]
            }
            _ => false,
        };
        if inside && !self.changed {
            return None;
        }
        self.changed = false;
        let rect = visible.map(|[x0, z0, x1, z1]| {
            let (dx, dz) = ((x1 - x0) / 2.0, (z1 - z0) / 2.0);
            [x0 - dx, z0 - dz, x1 + dx, z1 + dz]
        });
        let rect = rect.filter(|rect| rect[2] - rect[0] < camera.map.x());
        self.collected = Some(rect);
        Some(self.collect(rect, camera.map.x()))
    }

    /// Applies a view the camera passed before the next update, as the
    /// engine's frames there would have, at a zero frame time.
    pub fn visit(&mut self, camera: Camera, viewport: [u32; 2]) {
        self.view(camera, viewport, 0.0);
    }

    fn view(&mut self, camera: Camera, viewport: [u32; 2], frame: f32) {
        if self.dirty || self.last_view != Some((camera, viewport)) {
            self.last_view = Some((camera, viewport));
            self.dirty = false;
            if let Some(prepared) = self.prepared.take() {
                self.apply(&prepared, camera, viewport, frame, true);
                self.prepared = Some(prepared);
            }
        }
    }

    /// Skipped hours carry selection/hysteresis, but create only on a rendered
    /// frame. Explicit historical dump cameras settle all their models.
    fn apply(
        &mut self,
        prepared: &PreparedBuildings,
        camera: Camera,
        viewport: [u32; 2],
        delta: f32,
        steady: bool,
    ) {
        self.settled.clear();
        self.settled.resize(prepared.groups.len(), false);
        self.settled_steady = Some(steady);
        let admission = Admission::new(camera, viewport);
        let mut budget = super::mesh_budget::Budget::new(0, steady);
        for (index, group) in prepared.groups.iter().enumerate() {
            if admission.admits(group.bounds) {
                self.apply_group(prepared, index, camera, delta, steady, &mut budget);
            }
        }
    }

    fn apply_group(
        &mut self,
        prepared: &PreparedBuildings,
        index: usize,
        camera: Camera,
        delta: f32,
        steady: bool,
        budget: &mut super::mesh_budget::Budget,
    ) {
        let group = &prepared.groups[index];
        if !draws_map_buildings(camera.height, prepared.shrink_distance) {
            if !self.near.remove(&index) {
                return;
            }
            self.settled[index] = false;
            for &id in &group.slots {
                let Some(state) = self.slots.get_mut(&id) else {
                    continue;
                };
                if state.drawn_at_distance {
                    continue;
                }
                let shrunk = state.entity.and_then(|(mesh, mut instance)| {
                    instance.scale -= delta * prepared.popup_speed;
                    instance.animation = BUILDING_IDLE;
                    (instance.scale > 0.0).then_some((mesh, instance))
                });
                place(&mut self.tiles, &mut self.changed, id, state, shrunk);
            }
            return;
        }
        if self.settled[index] {
            return;
        }
        self.near.insert(index);
        let mut settled = true;
        for &id in &group.slots {
            let Some(slot) = prepared.slots.get(id as usize) else {
                continue;
            };
            let state = self.slots.entry(id).or_default();
            if let Some(at_distance) = slot.drawn_at_distance {
                state.drawn_at_distance = at_distance;
            }
            let entity = slot.entity.as_ref().and_then(|entity| {
                let destroyed = if entity.uncertain {
                    state.destroyed
                } else {
                    entity.destroyed
                };
                let selection = if destroyed {
                    entity.ruined
                } else {
                    entity.healthy
                };
                selection.admitted.then_some((destroyed, selection, entity))
            });
            let Some((destroyed, selection, entity)) = entity else {
                place(&mut self.tiles, &mut self.changed, id, state, None);
                continue;
            };
            // The game destroys an entity whose selectors or definition
            // changed and latches the selectors BEFORE testing the budget, so
            // a held replacement is empty until a later frame creates it.
            if state.destroyed != destroyed
                || state.variant != selection.variant
                || state.definition != Some(selection.definition)
            {
                place(&mut self.tiles, &mut self.changed, id, state, None);
                state.destroyed = destroyed;
                state.variant = selection.variant;
                if !budget.take() {
                    self.dirty = true;
                    settled = false;
                    continue;
                }
                state.generation = state.generation.wrapping_add(1);
                state.definition = Some(selection.definition);
            }
            let mesh = selection.mesh;
            let mut instance = entity.instance;
            instance.animation_epoch = state.generation;
            instance.animation = if entity.damaged {
                BUILDING_DAMAGED
            } else {
                BUILDING_IDLE
            };
            if !steady && !entity.disable_grow_animation {
                let previous = state.entity.map_or(0.0, |(_, old)| old.scale);
                instance.scale = (previous + delta * prepared.popup_speed).min(1.0);
                self.dirty |= instance.scale < 1.0;
                settled &= instance.scale >= 1.0 && prepared.popup_speed >= 0.0;
            }
            place(
                &mut self.tiles,
                &mut self.changed,
                id,
                state,
                Some((mesh, instance)),
            );
            settled &= state.entity == Some((mesh, instance));
        }
        self.settled[index] = settled;
    }

    /// The drawn entities in the tiles `rect` reaches in any map copy, or all.
    fn collect(&self, rect: Option<[f32; 4]>, width: f32) -> Vec<Placed> {
        let Some(rect) = rect else {
            return self
                .tiles
                .values()
                .flat_map(|slots| slots.values().copied())
                .collect();
        };
        let mut out = Vec::new();
        let tile = |value: f32| (value / BUILDING_TILE).floor() as i32;
        let (z0, z1) = (tile(rect[1]), tile(rect[3]));
        let mut columns = BTreeSet::new();
        for offset in [-width, 0.0, width] {
            let (x0, x1) = ((rect[0] - offset).max(0.0), (rect[2] - offset).min(width));
            if x0 <= x1 {
                columns.extend(tile(x0)..=tile(x1));
            }
        }
        for x in columns {
            for (_, slots) in self.tiles.range((x, z0)..=(x, z1)) {
                out.extend(slots.values().copied());
            }
        }
        out
    }
}

/// A borrowed building frame; `through` precedes the unit update of that
/// province. `finish` restores the hour and collects the changed GPU stream.
pub struct BuildingFrame<'a> {
    history: &'a mut BuildingHistory,
    prepared: Option<PreparedBuildings>,
    camera: Camera,
    viewport: [u32; 2],
    delta: f32,
    enabled: bool,
    steady: bool,
    active: bool,
    next: usize,
    admission: Admission,
}

impl BuildingFrame<'_> {
    pub fn through(&mut self, province: u32, budget: &mut super::mesh_budget::Budget) {
        let Some(prepared) = &self.prepared else {
            return;
        };
        if !self.active {
            return;
        }
        while let Some(group) = prepared.groups.get(self.next) {
            if group.province > province {
                break;
            }
            if self.admission.admits(group.bounds) {
                self.history.apply_group(
                    prepared,
                    self.next,
                    self.camera,
                    self.delta,
                    self.steady,
                    budget,
                );
            }
            self.next += 1;
        }
    }

    pub fn finish(mut self, budget: &mut super::mesh_budget::Budget) -> Option<Vec<Placed>> {
        self.through(u32::MAX, budget);
        self.history.prepared = self.prepared;
        self.enabled
            .then(|| self.history.stream(self.camera, self.viewport))
            .flatten()
    }
}

/// Replaces one slot's drawn entity and its tile entry.
fn place(
    tiles: &mut BTreeMap<(i32, i32), BTreeMap<u32, Placed>>,
    changed: &mut bool,
    id: u32,
    state: &mut SlotHistory,
    entity: Option<Placed>,
) {
    if state.entity == entity {
        return;
    }
    let key = |(_, instance): Placed| {
        (
            (instance.position[0] / BUILDING_TILE).floor() as i32,
            (instance.position[2] / BUILDING_TILE).floor() as i32,
        )
    };
    if let Some(old) = state.entity
        && let Some(slots) = tiles.get_mut(&key(old))
    {
        slots.remove(&id);
        if slots.is_empty() {
            tiles.remove(&key(old));
        }
    }
    if let Some(new) = entity {
        tiles.entry(key(new)).or_default().insert(id, new);
    }
    state.entity = entity;
    if entity.is_none() {
        state.definition = None;
    }
    *changed = true;
}

/// The slots of each province with a box, given each slot's province in the order `field`
/// numbers them.
fn slot_groups(
    provinces: impl Iterator<Item = Option<u32>>,
    bounds: &BTreeMap<u32, [f32; 4]>,
) -> Arc<[ProvinceSlots]> {
    let mut groups: BTreeMap<u32, ProvinceSlots> = BTreeMap::new();
    for (id, province) in provinces.enumerate() {
        if let Some(province) = province
            && let Some(bounds) = bounds.get(&province)
        {
            groups
                .entry(province)
                .or_insert_with(|| ProvinceSlots {
                    province,
                    bounds: *bounds,
                    slots: Vec::new(),
                })
                .slots
                .push(id as u32);
        }
    }
    groups.into_values().collect()
}

/// The authored slots of one location and one bucket, in file order.
type SlotKey = (u32, String);

/// Everything static the layer needs: the slot table, the building types and
/// the resolved entity meshes.
pub struct MapBuildingAssets {
    pub facts: Arc<viewer_foundation::map::building_facts::BuildingFacts>,
    pub library: ModelLibrary,
    pub defines: Defines,
    /// The slot ids of each province with a box, in authored file order.
    groups: Arc<[ProvinceSlots]>,
    /// Each admitted slot with the province under its own position, which is
    /// the province the game reads the fog byte of.
    slots: BTreeMap<SlotKey, Vec<(BuildingPosition, Option<u32>)>>,
    /// Each slot bucket's candidate types, as [`BuildingFacts::ordered`](viewer_foundation::map::building_facts::BuildingFacts::ordered) indices in
    /// declaration order (`Buildings::slot_candidates`, resolved once).
    candidates: BTreeMap<String, Vec<usize>>,
    /// Type index and variant index (`usize::MAX` for destroyed) to mesh slot.
    entities: BTreeMap<(usize, usize), usize>,
    damaged_entities: BTreeMap<(usize, usize), usize>,
    /// Which [`BuildingFacts::ordered`](viewer_foundation::map::building_facts::BuildingFacts::ordered) type each `LexicalTokens` token names, kept
    /// while the hours share the token table.
    tokens: Keyed<TokenTypes>,
    /// The reserved floating-harbour slots with the province under each; their ids follow
    /// the ordinary slots'.
    floating: Vec<(BuildingPosition, u32)>,
    floating_mesh: Option<usize>,
    id: u64,
    sites: std::sync::OnceLock<incremental::Sites>,
    last_tokens: std::sync::Mutex<Option<(DecodedFrame, Arc<TokenTypes>)>>,
}
impl std::fmt::Debug for MapBuildingAssets {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MapBuildingAssets")
            .field("library", &self.library)
            .field("defines", &self.defines)
            .field("groups", &self.groups)
            .field("slots", &self.slots)
            .field("ordered", &self.facts.ordered)
            .field("candidates", &self.candidates)
            .field("entities", &self.entities)
            .field("damaged_entities", &self.damaged_entities)
            .field("keys", &self.facts.keys)
            .field("index", &self.facts.index)
            .field("floating", &self.floating)
            .field("floating_mesh", &self.floating_mesh)
            .finish()
    }
}

/// The recorded tokens whose spelling names a building type: `Ok(index)` into
/// [`BuildingFacts::ordered`](viewer_foundation::map::building_facts::BuildingFacts::ordered) for the one type equal to it under the
/// lexer's fold, the reason when several are. Every other token is absent;
/// [`MapBuildingAssets::field`] words why for the ones its rows reference.
type TokenTypes = BTreeMap<u32, Result<usize, String>>;

/// The tables [`token_types`] reads.
const TOKEN_TABLES: &[TableId] = &[TableId::LexicalTokens];

/// Counts the viewer reports for the layer.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct MapBuildingSummary {
    pub unknown_city_damage: BTreeMap<String, usize>,
    /// Admitted authored slots.
    pub slots: usize,
    /// Recorded rows with a level above zero whose type draws.
    pub built: usize,
    /// Entities the hour selects before camera history.
    pub placed: usize,
    /// Rows whose average health selects the damaged state.
    pub damaged: usize,
    /// Rows selecting the destroyed mesh.
    pub destroyed: usize,
    /// Rows inside the hysteresis band, resolved from slot history.
    pub uncertain_damage: usize,
    /// Rows whose type token the hour does not spell (absent, among the
    /// missing rows, or not text); their reasons are in `refused_tokens`.
    pub unnamed_tokens: usize,
    /// Rows whose type token names no one building type, by reason: an
    /// unspelled token, a spelling no authored type folds to, or one several do.
    pub refused_tokens: Refusals,
    /// Rows whose location has no admitted slot of their bucket.
    pub missing_slots: usize,
    pub missing_by_type: BTreeMap<String, usize>,
    /// Rows whose type resolved to no entity, by building type.
    pub unresolved_by_type: BTreeMap<String, usize>,
    /// Hub slots the port or city graphics carry instead.
    pub hubs_left_to_another_layer: usize,
    /// Hub slots kept because `NavalBases` is incomplete this hour.
    pub unknown_hub_membership: usize,
    /// Built slots the viewing country's fog hides.
    pub fogged: usize,
    /// Foreign `need_detection` slots the viewer has not detected.
    pub undetected: usize,
    /// Foreign `need_detection` slots kept because detection is incomplete.
    pub unknown_detection: usize,
    /// Floating harbours omitted because their fog byte or deployment is unknown, by reason.
    pub unknown_floating_harbors: BTreeMap<&'static str, usize>,
}

impl MapBuildingSummary {
    /// The refused type tokens and the undecided floating harbours, one diagnostics line per
    /// reason. The hour builder hands these to the window's diagnostics, a capture and the
    /// dashboard's map details.
    pub fn warnings(&self) -> Vec<String> {
        self.refused_tokens
            .iter()
            .map(|(reason, count)| {
                format!("Map buildings: {count} rows not drawn, refused type token: {reason}.")
            })
            .chain(self.unknown_city_damage.iter().map(|(reason, count)| {
                format!(
                    "City sprawl: particle states refused ({count} recorded provinces): {reason}."
                )
            }))
            .chain(self.unknown_floating_harbors.iter().map(|(reason, count)| {
                format!("Map buildings: {count} floating harbours not drawn: {reason}.")
            }))
            .collect()
    }
}

impl MapBuildingAssets {
    /// Reads the slot table, the building types and every entity a drawable
    /// type names, from the descriptors anywhere below `gfx`.
    pub fn load(
        layers: &ResolvedLayers,
        decode: Decode,
        map: &MapData,
        defines: &installed_content_core::defines::Defines,
        warnings: &mut Vec<installed_content_core::Diagnostic>,
    ) -> io::Result<Self> {
        let provinces = ProvinceMap {
            width: map.width,
            height: map.height,
            ids: &map.province_ids,
        };
        // document.rs reports the state id conflicts once.
        let (states, _) = installed_content_states::states::read(layers, defines, &map.definitions)?;
        let defines = Defines::load(defines)?;
        let types = installed_content_core::buildings::read(layers)?;
        let roster = layers.descriptor_roster()?;
        if !roster.unreadable.is_empty() {
            warnings.push((installed_content_core::DiagnosticKind::ParseFailure, format!(
                "These gfx descriptors do not parse, so the entities they declare are missing: {}.",
                roster.unreadable.join(", ")
            )));
        }
        let mut slots: BTreeMap<SlotKey, Vec<(BuildingPosition, Option<u32>)>> = BTreeMap::new();
        let mut authored: BTreeMap<SlotKey, Vec<usize>> = BTreeMap::new();
        let mut ambiguous_slots = BTreeSet::new();
        let mut outside = 0usize;
        // Each floating harbour with the province under it and its file row.
        let mut floating = Vec::new();
        for (order, position) in installed_content_core::positions::read_buildings(layers)?
            .into_iter()
            .enumerate()
        {
            if is_floating_harbor_slot(&position.kind) {
                // The game keeps the reserved slot under its bitmap province without the
                // land and state-membership tests; its seventh field names the deployment.
                match provinces.at(position.x, position.z) {
                    Some(receiver) if states.definitions().contains_key(&position.state) => {
                        floating.push((position, receiver, order));
                    }
                    _ => outside += 1,
                }
                continue;
            }
            let spawn = match types.slot(&position.kind) {
                None => continue,
                Some(Slot::Ambiguous(_)) => {
                    ambiguous_slots.insert(position.kind.clone());
                    continue;
                }
                Some(Slot::Bucket(spawn)) => spawn,
            };
            let province = provinces.at(position.x, position.z);
            match admit_slot(
                spawn,
                &position,
                province,
                map,
                states.definitions(),
                &roster,
            ) {
                SlotAdmission::Admitted => {}
                SlotAdmission::Unregistered => continue,
                SlotAdmission::Outside => {
                    outside += 1;
                    continue;
                }
            }
            let location = match province {
                Some(province) if spawn.provincial => province,
                _ => position.state,
            };
            authored
                .entry((location, spawn.name.clone()))
                .or_default()
                .push(order);
            slots
                .entry((location, spawn.name.clone()))
                .or_default()
                .push((position, province));
        }
        // Rail and port anchors resolve through the same `Buildings::slot` and skip
        // the same slots; this warning names them for all three.
        if !ambiguous_slots.is_empty() {
            warnings.push((installed_content_core::DiagnosticKind::Refused, format!(
                "Map-building slots are not drawn or used as rail and port anchors where content declares their type or bucket more than once differently (names equal but for letter case are one declaration), because the engine's choice is unverified: {}.",
                ambiguous_slots.into_iter().collect::<Vec<_>>().join(", ")
            )));
        }
        if outside != 0 {
            warnings.push((installed_content_core::DiagnosticKind::Warning, format!(
                "{outside} authored map-building positions are ignored, as the engine ignores them: their province is not land or not in the declared state."
            )));
        }
        let mut wanted: Vec<(String, usize, String)> = Vec::new();
        for building in types
            .types
            .values()
            .filter(|building| building.show_on_map.is_some_and(|shown| shown > 0))
        {
            for (variant, entity) in building.entity_names().into_iter().enumerate() {
                wanted.push((building.name.clone(), variant, entity));
            }
            // The game probes the destroyed descriptor whatever
            // has_destroyed_mesh says; its absence is valid.
            let destroyed = building.destroyed_entity_name();
            if roster.descriptors.entities.contains_key(&destroyed) {
                wanted.push((building.name.clone(), usize::MAX, destroyed));
            }
        }
        let mut descriptors = roster.descriptors.clone();
        let mut names: Vec<String> = wanted.iter().map(|(_, _, entity)| entity.clone()).collect();
        if !floating.is_empty() {
            names.push(FLOATING_HARBOR_ENTITY.to_owned());
        }
        // A damaged state with an animation needs its own posed geometry.
        for (_, _, entity) in &wanted {
            if descriptors
                .inherit(entity)
                .is_some_and(|value| value.states.contains_key("damaged"))
            {
                let name = format!("{entity}::damaged");
                // A viewer alias of the registered snapshot, not a new raw
                // clone declaration whose registration order is unknown.
                let mut damaged = descriptors.entities[entity].clone();
                damaged.name = name.clone();
                damaged.default_state = Some("damaged".into());
                descriptors.entities.insert(name.clone(), damaged);
                names.push(name);
            }
        }
        let library = ModelLibrary::load(layers, decode, &descriptors, &names, warnings)?;
        let unselected: BTreeSet<_> = wanted
            .iter()
            .flat_map(|(_, _, name)| unselected_meshless_states(&descriptors, name))
            .collect();
        warnings.extend(
            unselected
                .into_iter()
                .map(|message| (installed_content_core::DiagnosticKind::Warning, message)),
        );
        let mut entities = BTreeMap::new();
        let mut damaged_entities = BTreeMap::new();
        let mut unresolved = Vec::new();
        for (name, variant, entity) in wanted {
            if let Some(slot) = library.slot(&format!("{entity}::damaged")) {
                damaged_entities.insert((name.clone(), variant), slot);
            }
            match library.slot(&entity) {
                Some(slot) => {
                    entities.insert((name, variant), slot);
                }
                None => unresolved.push(entity),
            }
        }
        if !unresolved.is_empty() {
            warnings.push((installed_content_core::DiagnosticKind::Refused, format!(
                "{} map-building entities named by common/buildings resolve to no mesh ({}); buildings of those types are not drawn.",
                unresolved.len(),
                unresolved.join(", ")
            )));
        }
        let floating_mesh = library.slot(FLOATING_HARBOR_ENTITY);
        if !floating.is_empty() && floating_mesh.is_none() {
            warnings.push((
                installed_content_core::DiagnosticKind::Refused,
                format!(
                    "{FLOATING_HARBOR_ENTITY} resolves to no mesh; floating harbours are not drawn."
                ),
            ));
        }
        // The floating harbours' ids follow the ordinary slots'.
        let mut groups = slot_groups(
            slots
                .values()
                .flatten()
                .map(|(_, province)| *province)
                .chain(floating.iter().map(|(_, receiver, _)| Some(*receiver))),
            &province_boxes(&provinces),
        );
        let order: Vec<_> = authored
            .values()
            .flatten()
            .copied()
            .chain(floating.iter().map(|(_, _, order)| *order))
            .collect();
        // Read authored rows and append each to its province; a floating
        // harbour's too.
        for group in Arc::make_mut(&mut groups) {
            group.slots.sort_by_key(|id| order[*id as usize]);
        }
        let mut assets = Self::assemble(
            library,
            &types,
            defines,
            groups,
            slots,
            entities,
            damaged_entities,
        );
        assets.floating = floating
            .into_iter()
            .map(|(position, receiver, _)| (position, receiver))
            .collect();
        assets.floating_mesh = floating_mesh;
        Ok(assets)
    }

    /// The layer from its loaded parts, with every name-keyed lookup the hour
    /// needs resolved to [`BuildingFacts::ordered`](viewer_foundation::map::building_facts::BuildingFacts::ordered) indices once.
    fn assemble(
        library: ModelLibrary,
        types: &Buildings,
        defines: Defines,
        groups: Arc<[ProvinceSlots]>,
        slots: BTreeMap<SlotKey, Vec<(BuildingPosition, Option<u32>)>>,
        entities: BTreeMap<(String, usize), usize>,
        damaged_entities: BTreeMap<(String, usize), usize>,
    ) -> Self {
        let ordered: Vec<BuildingType> = types
            .declaration_order
            .iter()
            .filter_map(|name| types.types.get(name))
            .cloned()
            .collect();
        let index: BTreeMap<String, usize> = ordered
            .iter()
            .enumerate()
            .map(|(at, building)| (building.name.clone(), at))
            .collect();
        let keys = TokenKeys::new(index.keys());
        let mut candidates: BTreeMap<String, Vec<usize>> = BTreeMap::new();
        for (_, bucket) in slots.keys() {
            candidates.entry(bucket.clone()).or_insert_with(|| {
                types
                    .slot_candidates(bucket)
                    .filter_map(|building| index.get(building.name.as_str()).copied())
                    .collect()
            });
        }
        let by_index = |meshes: BTreeMap<(String, usize), usize>| {
            meshes
                .into_iter()
                .filter_map(|((name, variant), slot)| {
                    Some(((*index.get(name.as_str())?, variant), slot))
                })
                .collect::<BTreeMap<_, _>>()
        };
        let entities = by_index(entities);
        let damaged_entities = by_index(damaged_entities);
        let mut province_sites = BTreeMap::<_, [bool; 3]>::new();
        for ((_, bucket), positions) in &slots {
            let mut flags = [false; 3];
            for &index in candidates.get(bucket).into_iter().flatten() {
                let kind = &ordered[index];
                flags[0] |= kind.air_base;
                flags[1] |= kind.rocket_launch_capacity > 0;
                flags[2] |= kind.gun_emplacement;
            }
            for (position, province) in positions {
                if let Some(province) = province {
                    let entry = province_sites
                        .entry((position.state, *province))
                        .or_default();
                    for (value, flag) in entry.iter_mut().zip(flags) {
                        *value |= flag;
                    }
                }
            }
        }
        Self {
            library,
            defines,
            groups,
            slots,
            candidates,
            entities,
            damaged_entities,
            facts: Arc::new(viewer_foundation::map::building_facts::BuildingFacts {
                ordered,
                province_sites,
                keys,
                index,
            }),
            tokens: Keyed::new(TOKEN_TABLES, 4),
            floating: Vec::new(),
            floating_mesh: None,
            id: incremental::next_assets(),
            sites: std::sync::OnceLock::new(),
            last_tokens: std::sync::Mutex::new(None),
        }
    }

    /// Admitted slots, for reporting.
    pub fn slot_count(&self) -> usize {
        self.ordinary_slot_count() + self.floating.len()
    }

    /// This hour's slot targets.
    ///
    /// `StateBuildings` rows are keyed by state and `ProvinceBuildings` rows by
    /// province; a provincial bucket's slots were keyed by their bitmap
    /// province at load, so both join on `location`.
    pub fn field(
        &self,
        frame: &DecodedFrame,
        vision: &Vision,
        city: &CityTerrain<'_>,
    ) -> io::Result<(PreparedBuildings, MapBuildingSummary)> {
        self.field_profiled(
            frame,
            vision,
            city,
            &mut viewer_foundation::profiling::BuilderSpans::new(false),
        )
    }

    /// The same hour targets with explicit builder child scopes.
    pub fn field_profiled(
        &self,
        frame: &DecodedFrame,
        vision: &Vision,
        city: &CityTerrain<'_>,
        spans: &mut viewer_foundation::profiling::BuilderSpans,
    ) -> io::Result<(PreparedBuildings, MapBuildingSummary)> {
        let built = self.build(frame, vision, city, Err("no base was offered"), spans)?;
        Ok((built.prepared, built.summary))
    }

    pub fn field_patched(
        &self,
        frame: &DecodedFrame,
        vision: &Arc<Vision>,
        city: &CityTerrain<'_>,
        base: Option<&BuildingsBase>,
        spans: &mut viewer_foundation::profiling::BuilderSpans,
    ) -> io::Result<(PreparedBuildings, MapBuildingSummary, BuildingsBase)> {
        let base = match base {
            Some(base) => Ok(base),
            None => Err("no earlier built hour"),
        };
        let mut built = self.build(frame, vision, city, base, spans)?;
        let city_bits = built.city.take().unwrap_or_else(|| self.city_bits(city));
        let next = built.base(self, frame, vision, city_bits);
        Ok((built.prepared, built.summary, next))
    }

    fn build(
        &self,
        frame: &DecodedFrame,
        vision: &Vision,
        city: &CityTerrain<'_>,
        base: Result<&BuildingsBase, &'static str>,
        spans: &mut viewer_foundation::profiling::BuilderSpans,
    ) -> io::Result<incremental::Built> {
        let tokens = spans.measure("buildings.tokens", |_| {
            self.tokens
                .get_or_build(frame, (), |projected| self.token_map(projected))
        })?;
        // Read only when a row's token names no building type, to word why.
        let mut names = None;
        let mut summary = MapBuildingSummary {
            slots: self.slot_count(),
            ..MapBuildingSummary::default()
        };
        let state_rows = rows::<tables::StateBuildings>(frame)?;
        let province_rows = rows::<tables::ProvinceBuildings>(frame)?;
        let links: BTreeMap<u32, u32> = spans.measure("buildings.links", |_| {
            Ok::<_, io::Error>(
                core::<tables::CountryLinks>(frame)?
                    .iter()
                    .map(|link| (link.id, link.canonical_slot))
                    .collect(),
            )
        })?;
        let detection = spans.measure("buildings.detection", |_| {
            Detection::read(frame, &links, vision.ours())
        })?;
        // The game tests the strategic-navy base hash whatever the
        // base's level; `NavalBases` walks that hash.
        let (naval_bases, naval_base_provinces) = spans.measure("buildings.naval_bases", |_| {
            let naval_bases = rows::<tables::NavalBases>(frame)?;
            let naval_base_provinces: BTreeSet<u32> =
                naval_bases.rows().iter().map(|row| row.province).collect();
            Ok::<_, io::Error>((naval_bases, naval_base_provinces))
        })?;
        // Keyed by the table a row came from, its location and its type index.
        let available = spans.measure("buildings.rows", |_| {
            let mut available: BTreeMap<(bool, u32, usize), &BuildingRow> = BTreeMap::new();
            for (provincial, table) in [(false, state_rows.rows()), (true, province_rows.rows())] {
                for row in table {
                    // Building type 0 is none: token registration gives token 0 to
                    // every key it rejects, so it names no building type and a row
                    // holding it has nothing to draw or report.
                    if row.building_type == 0 {
                        continue;
                    }
                    let index = match tokens.get(&row.building_type) {
                        Some(Ok(index)) => *index,
                        Some(Err(reason)) => {
                            refuse(&mut summary.refused_tokens, reason.clone());
                            continue;
                        }
                        None => {
                            let names = match names {
                                Some(names) => names,
                                None => *names.insert(TokenNames::new(frame)?),
                            };
                            let reason = match names.name(row.building_type) {
                                Ok(spelling) => format!(
                                    "token {} ({spelling}) names no authored building type",
                                    row.building_type
                                ),
                                Err(error) => {
                                    summary.unnamed_tokens += 1;
                                    error.to_string()
                                }
                            };
                            refuse(&mut summary.refused_tokens, reason);
                            continue;
                        }
                    };
                    let building = &self.facts.ordered[index];
                    available.insert((provincial, row.location, index), row);
                    if slots_for_level(building, row.level, self.defines.max_levels) == 0 {
                        continue;
                    }
                    summary.built += 1;
                    if !self
                        .slots
                        .contains_key(&(row.location, building.slot_name().to_owned()))
                    {
                        summary.missing_slots += 1;
                        *summary
                            .missing_by_type
                            .entry(building.name.clone())
                            .or_default() += 1;
                    }
                    let (destroyed, damaged, uncertain) = health_state(
                        row.level,
                        row.healthy_level,
                        row.partial_health,
                        &self.defines,
                    );
                    summary.destroyed += usize::from(destroyed);
                    summary.damaged += usize::from(damaged);
                    summary.uncertain_damage += usize::from(uncertain);
                }
            }
            Ok::<_, io::Error>(available)
        })?;
        let city_damage = spans.measure("buildings.city_damage", |spans| {
            super::sprawl::Damage::read_profiled(frame, &self.facts.ordered, &available, spans)
        });
        let damage_refused = city_damage.refused.is_some();
        let context = SlotContext {
            vision,
            city,
            detection: &detection,
            naval_bases: &naval_base_provinces,
            naval_bases_complete: naval_bases.is_complete(),
            available: &available,
        };
        let mut hour_city = None;
        let plan = base.and_then(|base| {
            spans.measure("buildings.patch.plan", |_| {
                let city_bits = self.city_bits(city);
                let hour = incremental::Inputs {
                    frame,
                    vision,
                    detection: &detection,
                    naval_bases: &naval_base_provinces,
                    tokens: &tokens,
                    city: &city_bits,
                    damage_refused,
                };
                let plan = self
                    .affected(base, &hour)
                    .map(|affected| (base, affected.slots));
                hour_city = Some(city_bits);
                plan
            })
        });
        let ordinary_slots = self.ordinary_slot_count();
        let ordinary = match plan {
            Ok((base, affected)) => spans.measure("buildings.patch.slots", |_| {
                let (slots, tallies, totals) =
                    incremental::patch(base, &affected, &self.facts.ordered, |id| {
                        let (key, ordinal, position, province) = self.site(id);
                        let candidates = self.bucket_candidates(&key.1);
                        self.slot(id, ordinal, position, province, candidates, &context)
                    });
                incremental::Ordinary::Patched {
                    base,
                    slots,
                    tallies,
                    totals,
                }
            }),
            Err(reason) => spans.measure("buildings.slots", |_| {
                let mut slots = Vec::with_capacity(self.slot_count());
                let mut tallies = Vec::with_capacity(ordinary_slots);
                let mut totals = incremental::SlotSummary::default();
                for ((_, bucket), positions) in &self.slots {
                    let candidates = self.bucket_candidates(bucket);
                    for (ordinal, (position, province)) in positions.iter().enumerate() {
                        let id = slots.len() as u32;
                        let (slot, tally) =
                            self.slot(id, ordinal, position, *province, candidates, &context);
                        totals.add(tally, &self.facts.ordered);
                        tallies.push(tally);
                        slots.push(slot);
                    }
                }
                incremental::Ordinary::Full {
                    slots,
                    tallies,
                    totals,
                    reason,
                }
            }),
        };
        let mut harbors = Vec::with_capacity(self.floating.len());
        let (supply, deployments, supply_by_province) =
            spans.measure("buildings.supply", |_| {
                let supply = rows::<tables::ProvinceSupply>(frame)?;
                let deployments = rows::<tables::FloatingHarborDeployments>(frame)?;
                let supply_by_province: BTreeMap<u32, &ProvinceSupplyRow> = supply
                    .rows()
                    .iter()
                    .map(|row| (row.province, row))
                    .collect();
                Ok::<_, io::Error>((supply, deployments, supply_by_province))
            })?;
        spans.measure("buildings.floating", |_| {
            for (position, _) in &self.floating {
                let id = (ordinary_slots + harbors.len()) as u32;
                let entity = (|| {
                    // The seventh field must be a positive province: the fog byte and the
                    // deployment are that province's.
                    let province = position.adjacent_sea;
                    if (province as i32) <= 0 {
                        return None;
                    }
                    match vision
                        .province_byte(province)
                        .map(|byte| byte >= FLOATING_HARBOR_VISIBILITY)
                    {
                        Some(true) => {}
                        Some(false) => {
                            summary.fogged += 1;
                            return None;
                        }
                        None => {
                            *summary
                                .unknown_floating_harbors
                                .entry("the fog byte of the harbour's sea province is unknown")
                                .or_default() += 1;
                            return None;
                        }
                    }
                    let deployed = floating_harbor_deployed(
                        province,
                        supply_by_province.get(&province).copied(),
                        supply.is_complete(),
                        deployments.rows(),
                        deployments.is_complete(),
                    );
                    if deployed.is_none() {
                        *summary
                            .unknown_floating_harbors
                            .entry("the controller's floating-harbour deployment is unknown")
                            .or_default() += 1;
                    }
                    let mesh = self.floating_mesh.filter(|_| deployed == Some(true))?;
                    summary.placed += 1;
                    // The reserved slot has no damage states or variants: one entity definition,
                    // variant selector 0 from the default request.
                    let selection = Selection {
                        admitted: true,
                        mesh,
                        definition: mesh,
                        variant: 0,
                    };
                    Some(PreparedEntity {
                        healthy: selection,
                        ruined: selection,
                        destroyed: false,
                        uncertain: false,
                        damaged: false,
                        // The reserved slot receives a null template,
                        // which grows unless its template explicitly suppresses popup growth.
                        disable_grow_animation: false,
                        instance: ModelInstance {
                            position: [position.x, position.y, position.z],
                            rotation: position.rotation,
                            scale: 1.0,
                            animation_seed: id,
                            ..ModelInstance::default()
                        },
                    })
                })();
                harbors.push(PreparedSlot {
                    id,
                    // The reserved slot copies no template's distance flag.
                    drawn_at_distance: Some(false),
                    entity,
                });
            }
        });
        if let Some(reason) = &city_damage.refused {
            summary.unknown_city_damage.insert(
                reason.clone(),
                rows::<tables::Provinces>(frame)?.rows().len(),
            );
        }
        let (slots, tallies, totals, path) = ordinary.finish(harbors, ordinary_slots);
        totals.apply(&mut summary);
        Ok(incremental::Built {
            prepared: PreparedBuildings {
                branch: frame.branch,
                hour: frame.hour,
                shrink_distance: self.defines.shrink_distance,
                popup_speed: self.defines.popup_speed,
                slots,
                groups: Arc::clone(&self.groups),
                city_damage: Arc::new(city_damage),
            },
            summary,
            tallies,
            totals,
            path,
            damage_refused,
            tokens,
            gates: Arc::new(incremental::GateInputs {
                ports: naval_base_provinces,
                provinces: detection.provinces,
                states: detection.states,
            }),
            city: hour_city,
        })
    }

    fn bucket_candidates(&self, bucket: &str) -> &[usize] {
        self.candidates.get(bucket).map_or(&[][..], Vec::as_slice)
    }

    fn slot(
        &self,
        id: u32,
        ordinal: usize,
        position: &BuildingPosition,
        province: Option<u32>,
        candidates: &[usize],
        context: &SlotContext<'_>,
    ) -> (PreparedSlot, Tally) {
        let selected = candidates.iter().find_map(|&index| {
            let building = &self.facts.ordered[index];
            let location = if building.provincial() {
                province?
            } else {
                position.state
            };
            let row = context
                .available
                .get(&(building.provincial(), location, index))?;
            (row.level > 0).then_some((index, *row))
        });
        let mut prepared = PreparedSlot {
            id,
            drawn_at_distance: selected
                .map(|(index, _)| self.facts.ordered[index].drawn_at_distance),
            entity: None,
        };
        let mut tally = Tally::default();
        if let Some((index, row)) = selected {
            let gates = Gates {
                vision: context.vision,
                city: context.city,
                detection: context.detection,
                naval_bases: context.naval_bases,
                naval_bases_complete: context.naval_bases_complete,
                ordinal,
                seed: id,
            };
            prepared.entity = self.entity(index, row, position, province, &mut tally, gates);
        }
        (prepared, tally)
    }

    /// One selected slot's entity, or `None` where a gate hides it.
    fn entity(
        &self,
        index: usize,
        row: &BuildingRow,
        position: &BuildingPosition,
        province: Option<u32>,
        tally: &mut Tally,
        gates: Gates<'_>,
    ) -> Option<PreparedEntity> {
        let building = &self.facts.ordered[index];
        let in_order =
            gates.ordinal < slots_for_level(building, row.level, self.defines.max_levels) as usize;
        // Multi-mesh types admit every slot; a destroyed mesh only its ordered ones.
        let healthy_admitted = building.show_on_map_meshes >= 2 || in_order;
        let destroyed_admitted =
            (!building.has_destroyed_mesh && building.show_on_map_meshes >= 2) || in_order;
        if !healthy_admitted && !destroyed_admitted {
            return None;
        }
        // A province with no recorded byte, or a slot over no province, cannot
        // be decided and keeps its building.
        if province.is_some() && !building.always_shown {
            tally.mark(Tally::READ_FOG);
        }
        if province.is_some_and(|province| {
            gates
                .vision
                .sees_province_object(province, building.always_shown)
                == Some(false)
        }) {
            tally.mark(Tally::FOGGED);
            return None;
        }
        if building.supply_node {
            tally.mark(Tally::READ_HUB);
            let on_city = gates.city.at(position.x, position.z);
            match hub_left_to_another_layer(
                province,
                gates.naval_bases,
                gates.naval_bases_complete,
                on_city,
            ) {
                Some(true) => {
                    tally.mark(Tally::HUB_LEFT);
                    return None;
                }
                Some(false) => {}
                None => tally.mark(Tally::HUB_UNKNOWN),
            }
        }
        if !building.always_shown && building.need_detection {
            tally.mark(Tally::READ_DETECTION);
            match gates.detection.detected(row) {
                Some(false) => {
                    tally.mark(Tally::UNDETECTED);
                    return None;
                }
                None => tally.mark(Tally::DETECTION_UNKNOWN),
                Some(true) => {}
            }
        }
        let (destroyed, damaged, uncertain) = health_state(
            row.level,
            row.healthy_level,
            row.partial_health,
            &self.defines,
        );
        let definition = |variant| self.entities.get(&(index, variant)).copied();
        let resolve = |variant| {
            damaged
                .then(|| self.damaged_entities.get(&(index, variant)))
                .flatten()
                .copied()
                .or_else(|| definition(variant))
        };
        let variant = building.variant_for_level(i32::from(row.level), self.defines.max_levels);
        // The game leaves the variant selector 0 for a destroyed building of
        // a type with its own destroyed mesh.
        let ruined_variant = if building.has_destroyed_mesh {
            0
        } else {
            variant
        };
        let normal = resolve(variant);
        let destroyed_mesh = resolve(usize::MAX).or_else(|| resolve(ruined_variant));
        let Some(any) = normal.or(destroyed_mesh) else {
            tally.unresolved(index);
            return None;
        };
        let entity = PreparedEntity {
            // A missing mesh hides only the state that needs it.
            healthy: Selection {
                admitted: healthy_admitted && normal.is_some(),
                mesh: normal.unwrap_or(any),
                definition: definition(variant).unwrap_or(any),
                variant,
            },
            ruined: Selection {
                admitted: destroyed_admitted && destroyed_mesh.is_some(),
                mesh: destroyed_mesh.unwrap_or(any),
                definition: definition(usize::MAX)
                    .or_else(|| definition(ruined_variant))
                    .unwrap_or(any),
                variant: ruined_variant,
            },
            destroyed,
            uncertain,
            damaged,
            disable_grow_animation: building.disable_grow_animation,
            instance: ModelInstance {
                position: [position.x, position.y, position.z],
                rotation: position.rotation,
                scale: 1.0,
                animation_seed: gates.seed,
                ..ModelInstance::default()
            },
        };
        let admitted = if destroyed {
            entity.ruined.admitted
        } else {
            entity.healthy.admitted
        };
        if admitted {
            tally.mark(Tally::PLACED);
        }
        Some(entity)
    }
}

/// The deployment province needs supply flag bit 0 set and a nonzero
/// controller index. The first deployment record for that controller and
/// province, ordered by recorded ordinal, decides by its nonzero status.
fn floating_harbor_deployed(
    province: u32,
    supply: Option<&ProvinceSupplyRow>,
    supply_complete: bool,
    deployments: &[FloatingHarborDeploymentRow],
    deployments_complete: bool,
) -> Option<bool> {
    let Some(supply) = supply else {
        return supply_complete.then_some(false);
    };
    if supply.flags & 1 == 0 || supply.controller_index == 0 {
        return Some(false);
    }
    if !deployments_complete {
        return None;
    }
    Some(
        deployments
            .iter()
            .filter(|row| {
                row.country == u32::from(supply.controller_index) && row.province == province as i32
            })
            .min_by_key(|row| row.ordinal)
            .is_some_and(|row| row.status != 0),
    )
}

struct SlotContext<'a> {
    vision: &'a Vision,
    city: &'a CityTerrain<'a>,
    detection: &'a Detection<'a>,
    naval_bases: &'a BTreeSet<u32>,
    naval_bases_complete: bool,
    available: &'a BTreeMap<(bool, u32, usize), &'a BuildingRow>,
}

/// The per-slot inputs of [`MapBuildingAssets::entity`].
struct Gates<'a> {
    vision: &'a Vision,
    city: &'a CityTerrain<'a>,
    detection: &'a Detection<'a>,
    naval_bases: &'a BTreeSet<u32>,
    naval_bases_complete: bool,
    ordinal: usize,
    seed: u32,
}

/// Whether the game leaves a hub slot to another layer: the province is
/// in the naval-base hash or its terrain spawns a city. A province missing from
/// an incomplete hash is undecided (`None`).
fn hub_left_to_another_layer(
    province: Option<u32>,
    naval_bases: &BTreeSet<u32>,
    naval_bases_complete: bool,
    on_city_terrain: bool,
) -> Option<bool> {
    if on_city_terrain || province.is_some_and(|id| naval_bases.contains(&id)) {
        Some(true)
    } else if province.is_none() || naval_bases_complete {
        Some(false)
    } else {
        None
    }
}

/// A building's status scope: type token, location, status flags, owner override.
type Scope = (u32, u32, u32, u32);

fn raid_targets(frame: &DecodedFrame) -> Option<BTreeMap<Scope, &[u8]>> {
    frame
        .complete::<tables::RaidTargetStatuses>()
        .ok()
        .map(|rows| {
            let mut ordered: Vec<_> = rows
                .iter()
                .filter(|row| row.scope.building_type != 0)
                .collect();
            ordered.sort_by_key(|row| row.ordinal);
            // A later duplicate scope replaces an earlier one, as the loader does.
            ordered
                .into_iter()
                .map(|row| {
                    let scope = &row.scope;
                    (
                        (
                            scope.building_type,
                            scope.building_location,
                            scope.building_flags,
                            scope.building_country as u32,
                        ),
                        row.detected.as_slice(),
                    )
                })
                .collect()
        })
}

/// A foreign `need_detection` building is drawn only once the
/// viewer's bit is set in its raid target status. Fog does not change it.
struct Detection<'a> {
    links: &'a BTreeMap<u32, u32>,
    ours: u32,
    provinces: BTreeMap<u32, u32>,
    states: BTreeMap<u32, u32>,
    /// Detection bits by building scope; `None` unless the table is complete.
    targets: Option<BTreeMap<Scope, &'a [u8]>>,
}

impl<'a> Detection<'a> {
    fn read(frame: &'a DecodedFrame, links: &'a BTreeMap<u32, u32>, ours: u32) -> io::Result<Self> {
        let targets = raid_targets(frame);
        Ok(Self {
            links,
            ours,
            provinces: rows::<tables::Provinces>(frame)?
                .rows()
                .iter()
                .map(|row| (row.id, row.controller))
                .collect(),
            states: rows::<tables::States>(frame)?
                .rows()
                .iter()
                .map(|row| (row.id, row.controller))
                .collect(),
            targets,
        })
    }

    /// `None` when the hour cannot decide it.
    fn detected(&self, row: &BuildingRow) -> Option<bool> {
        // A positive override,
        // else the province (flag bit 0) or state controller.
        let owner = if (row.country as i32) > 0 {
            Some(row.country)
        } else if row.status_flags & 1 != 0 {
            self.provinces.get(&row.location).copied()
        } else {
            self.states.get(&row.location).copied()
        };
        let owner = owner.and_then(|raw| self.links.get(&raw).copied());
        if owner == Some(self.ours) {
            return Some(true);
        }
        let bits = self.targets.as_ref()?.get(&(
            row.building_type,
            row.location,
            row.status_flags,
            row.country,
        ));
        let slot = self.ours as usize;
        let detected = bits.is_some_and(|bits| {
            bits.get(slot / 8)
                .is_some_and(|byte| byte & (1 << (slot % 8)) != 0)
        });
        // An unresolved owner could be an alias of ours.
        if detected {
            Some(true)
        } else {
            owner.map(|_| false)
        }
    }
}

/// The `LexicalTokens` tokens that name a building type, with the type.
///
/// `Buildings.building_type` is a native token id; the recorder stores the
/// literal once and every row refers to it. A token names the building type
/// whose key equals its spelling under the lexer's fold (`keys`, with each
/// key's index in `index`). An hour may hold the table truncated when its
/// capture capacity stays short. Token 0 and a literal that is not NUL-free
/// UTF-8 name nothing (`TokenNames`); the caller words why for the rows that
/// reference a token this leaves out.
fn token_types(
    frame: &DecodedFrame,
    keys: &TokenKeys,
    index: &BTreeMap<String, usize>,
) -> io::Result<TokenTypes> {
    Ok(named_types(&TokenNames::new(frame)?, keys, index))
}

fn named_types(
    names: &TokenNames<'_>,
    keys: &TokenKeys,
    index: &BTreeMap<String, usize>,
) -> TokenTypes {
    names
        .iter()
        .filter_map(|(token, spelling)| {
            let named = match keys.resolve(spelling) {
                Ok(key) => Ok(*index.get(key?)?),
                Err(reason) => Err(reason),
            };
            Some((token, named))
        })
        .collect()
}

#[cfg(test)]
mod tests;
