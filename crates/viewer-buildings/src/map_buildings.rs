use content_reader_mesh::mesh::MeshProducts as _;

use std::{
    collections::{BTreeMap, BTreeSet},
    io,
    sync::Arc,
};
use viewer_foundation::building_facts::{SlotAdmission, admit_slot, is_floating_harbor_slot};
use dashboard_text::table_access::Refusals;

use content_reader_layers::ResolvedLayers;
use content_reader_key_names::buildings::{BuildingType, Buildings, Slot};
use content_reader_map::{map::MapData, positions::BuildingPosition};
use content_reader_mesh::mesh;
use content_reader_image::native::Decode;
use replay_data::rows::{BuildingRow, TableId, tables};
#[cfg(test)]
use replay_data::rows::{FloatingHarborDeploymentRow, ProvinceSupplyRow};
use replay_frame::frame::DecodedFrame;

fn invalid(why: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, why.into())
}

mod available;
mod display;
mod floating;
mod incremental;
pub use available::BuildingInput;
pub use display::{SlotPages, WireHour};
pub const WIRE_CHUNK: usize = incremental::CHUNK_SIZE;
pub use floating::HarbourCounts;
pub use incremental::{BuildPath, BuildingsBase};
use incremental::{Chunked, Tally};

use dashboard_text::table_access::{TokenKeys, TokenNames};
use viewer_models::{
    entity_animation::{BUILDING_DAMAGED, BUILDING_IDLE},
    ModelInstance,
    ModelLibrary,
    WATER_HEIGHT,
};
use viewer_gpu::{camera::Camera, province_admission::{Admission, province_boxes}};
use viewer_foundation::{perspective::Vision, tables::{core, rows}};

const FLOATING_HARBOR_ENTITY: &str = "floating_harbor_entity";
pub const FLOATING_HARBOR_VISIBILITY: u8 = 0x96;

pub const BUILDING_TILE: f32 = 128.0;

fn unselected_meshless_states(descriptors: &mesh::Descriptors, name: &str) -> Vec<String> {
    let Some(entity) = descriptors
        .inherit(name)
        .filter(|entity| entity.mesh.is_none())
    else {
        return Vec::new();
    };
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
            let count: usize = variants.iter().map(|state| viewer_models::map_effects(state)).sum();
            (count != 0).then(|| format!("entity {name} state {state}: {count} map effects refused; state is not selected by the building producer"))
        }).collect()
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Defines {
    pub shrink_distance: f32,
    pub destroyed: i64,
    pub recovered: i64,
    pub damaged: i64,
    pub max_levels: i32,
    pub popup_speed: f32,
}

impl Defines {
    pub fn load(defines: &content_reader_defines::defines::Defines) -> io::Result<Self> {
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

use viewer_gpu::province_map::{ProvinceMap, pixel};

pub struct CityTerrain<'a> {
    pub width: u32,
    pub height: u32,
    pub spawn_city: &'a [bool],
}

impl CityTerrain<'_> {
    pub fn at(&self, x: f32, z: f32) -> bool {
        pixel(self.width, self.height, x, z)
            .and_then(|index| self.spawn_city.get(index).copied())
            .unwrap_or(false)
    }
}

pub fn draws_map_buildings(height: f32, shrink_distance: f32) -> bool {
    height - WATER_HEIGHT < shrink_distance
}

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

pub fn health_state(
    level: i16,
    healthy: i16,
    partial: i64,
    defines: &Defines,
) -> (bool, bool, bool) {
    let total = (i64::from(healthy) * 10_000_000).wrapping_add(partial);
    let destroyed = total <= defines.destroyed;
    let uncertain = !destroyed && total <= defines.recovered;
    let damaged =
        level > 0 && super::sprawl::average_health(level, healthy, partial) < defines.damaged;
    (destroyed, damaged, uncertain)
}

#[derive(Clone, Debug, PartialEq)]
pub struct PreparedBuildings {
    branch: replay_data::BranchId,
    hour: replay_data::Hour,
    shrink_distance: f32,
    popup_speed: f32,
    slots: Chunked<PreparedSlot>,
    groups: Arc<[ProvinceSlots]>,
    pub city_damage: Arc<super::sprawl::Damage>,
}

impl PreparedBuildings {
    pub fn for_hour(&self, branch: u64, hour: i32) -> Self {
        Self {
            branch,
            hour,
            ..self.clone()
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
struct ProvinceSlots {
    province: u32,
    bounds: [f32; 4],
    slots: Vec<u32>,
}

#[derive(Clone, Debug)]
pub struct PreparedSlot {
    source: Option<(usize, u32)>,
    id: u32,
    drawn_at_distance: Option<bool>,
    entity: Option<map_display_data::retained::Retained<PreparedEntity>>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct Selection {
    admitted: bool,
    mesh: usize,
    definition: usize,
    variant: usize,
}

#[derive(Clone, Debug, PartialEq)]
struct PreparedEntity {
    healthy: Selection,
    ruined: Selection,
    destroyed: bool,
    uncertain: bool,
    damaged: bool,
    disable_grow_animation: bool,
    instance: ModelInstance,
}

impl PreparedEntity {
    fn retain(self) -> map_display_data::retained::Retained<Self> {
        map_display_data::retained::Retained::new(self)
    }
}

impl PartialEq for PreparedSlot {
    fn eq(&self, other: &Self) -> bool {
        self.same_bits(other)
    }
}
impl PreparedSlot {
    pub fn empty(id: u32) -> Self {
        Self { source: None, id, drawn_at_distance: None, entity: None }
    }

    pub fn has_entity(&self) -> bool {
        self.entity.is_some()
    }

    pub fn held(&self) -> usize {
        std::mem::size_of::<Self>()
            + self.entity.as_ref().map_or(0, |_| std::mem::size_of::<PreparedEntity>() + 16)
    }

    #[cfg(any(test, feature = "test-support"))]
    fn entity_mut(&mut self) -> &mut PreparedEntity {
        let entity = self.entity.as_mut().unwrap();
        if entity.get_mut().is_none() {
            *entity = (**entity).clone().retain();
        }
        entity.get_mut().unwrap()
    }

    fn same_bits(&self, other: &Self) -> bool {
        if (self.id, self.source, self.drawn_at_distance)
            != (other.id, other.source, other.drawn_at_distance)
        {
            return false;
        }
        match (&self.entity, &other.entity) {
            (None, None) => true,
            (Some(a), Some(b)) => {
                a.healthy == b.healthy
                    && a.ruined == b.ruined
                    && (
                        a.destroyed,
                        a.uncertain,
                        a.damaged,
                        a.disable_grow_animation,
                    ) == (
                        b.destroyed,
                        b.uncertain,
                        b.damaged,
                        b.disable_grow_animation,
                    )
                    && bytemuck::bytes_of(&a.instance) == bytemuck::bytes_of(&b.instance)
            }
            _ => false,
        }
    }
}

type Placed = (usize, ModelInstance);

#[derive(Clone, Debug, Default)]
struct SlotHistory {
    destroyed: bool,
    drawn_at_distance: bool,
    entity: Option<Placed>,
    generation: u32,
    definition: Option<usize>,
    variant: usize,
}

#[derive(Default)]
pub struct BuildingHistory {
    prepared: Option<PreparedBuildings>,
    display_install: Option<(
        u64,
        map_display_data::buildings::Buildings,
        PreparedBuildings,
    )>,
    slots: BTreeMap<u32, SlotHistory>,
    tiles: BTreeMap<(i32, i32), BTreeMap<u32, Placed>>,
    near: BTreeSet<usize>,
    settled: Vec<bool>,
    settled_steady: Option<bool>,
    last_time: Option<f32>,
    last_view: Option<(Camera, [u32; 2])>,
    collected: Option<Option<[f32; 4]>>,
    dirty: bool,
    growing: bool,
    settling: bool,
    changed: bool,
    candidates: Vec<u32>,
    admitted_against: viewer_gpu::camera::Hold,
    candidates_for: Option<(Camera, [u32; 2], viewer_gpu::camera::Slack, Arc<[ProvinceSlots]>)>,
    admitted: Vec<u32>,
    admitted_for: Option<(Camera, [u32; 2])>,
    counts: BuildingCounts,
}

#[derive(Default)]
struct BuildingCounts {
    groups: u64,
    applied: u64,
    slots: u64,
}

impl BuildingHistory {
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
        let kept = self.prepared.as_ref().filter(|_| !reset).and_then(|previous| {
            (Arc::ptr_eq(&previous.groups, &prepared.groups)
                && previous.popup_speed == prepared.popup_speed
                && previous.shrink_distance == prepared.shrink_distance)
                .then(|| prepared.slots.changed_chunks(&previous.slots).ok())
                .flatten()
        });
        match kept {
            Some(changed) => {
                self.settled.resize(prepared.groups.len(), false);
                for (index, group) in prepared.groups.iter().enumerate() {
                    if self.settled[index]
                        && group
                            .slots
                            .iter()
                            .any(|&id| changed.get(&(id as usize / WIRE_CHUNK)).is_some())
                    {
                        self.settled[index] = false;
                    }
                }
            }
            None => {
                self.settled.clear();
                self.settled.resize(prepared.groups.len(), false);
            }
        }
        self.prepared = Some(prepared);
        self.dirty = true;
        reset
    }

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
        self.growing = false;
        self.changed = true;
    }

    pub fn growing(&self) -> bool {
        self.growing
    }

    pub fn settle(&mut self, settled: bool) {
        self.dirty |= self.settling != settled;
        self.settling = settled;
    }

    pub fn frame(
        &mut self,
        camera: Camera,
        viewport: [u32; 2],
        time: f32,
        enabled: bool,
        steady: bool,
    ) -> BuildingFrame<'_> {
        let steady = steady || self.settling;
        if self.last_time.is_some_and(|previous| time < previous) {
            if !self.settling {
                self.clear();
                self.dirty = true;
            }
            self.last_time = None;
        }
        let delta = self.last_time.map_or(0.0, |previous| time - previous);
        self.last_time = Some(time);
        self.growing = false;
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
        let (against, slack) = self.admitted_against.at(camera, viewport);
        if let Some(groups) = self.prepared.as_ref().map(|prepared| Arc::clone(&prepared.groups))
            && enabled
        {
            if self.candidates_for.as_ref().is_none_or(|(was, port, with, tested)| {
                *was != against || *port != viewport || *with != slack || !Arc::ptr_eq(tested, &groups)
            }) {
                let padded = Admission::held(against, viewport, slack);
                self.candidates.clear();
                self.candidates.extend(
                    groups
                        .iter()
                        .enumerate()
                        .filter(|(_, group)| padded.admits(group.bounds))
                        .map(|(index, _)| index as u32),
                );
                self.candidates_for = Some((against, viewport, slack, Arc::clone(&groups)));
                self.admitted_for = None;
            }
            if self.admitted_for != Some((camera, viewport)) {
                let admission = Admission::new(camera, viewport);
                self.admitted.clear();
                self.admitted.extend(self.candidates.iter().copied().filter(|&index| {
                    groups.get(index as usize).is_some_and(|group| admission.admits(group.bounds))
                }));
                self.admitted_for = Some((camera, viewport));
            }
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
            .finish(&mut viewer_scene_base::mesh_budget::Budget::new(0, true))
    }

    fn stream(&mut self, camera: Camera, viewport: [u32; 2]) -> Option<Vec<Placed>> {
        let visible = camera.visible_ground(viewport, viewer_scene_base::line_lod::MARGIN);
        let inside = match (self.collected, visible) {
            (Some(None), _) => true,
            (Some(Some(rect)), Some(view)) => {
                rect[0] <= view[0] && rect[1] <= view[1] && view[2] <= rect[2] && view[3] <= rect[3]
            }
            _ => false,
        };
        use viewer_runtime::view_record::add;
        if inside && !self.changed {
            add("map buildings", "stream_kept", 1);
            return None;
        }
        add("map buildings", if inside { "stream_recollected_changed" } else { "stream_recollected_view" }, 1);
        self.changed = false;
        let rect = visible.map(|[x0, z0, x1, z1]| {
            let (dx, dz) = ((x1 - x0) / 2.0, (z1 - z0) / 2.0);
            [x0 - dx, z0 - dz, x1 + dx, z1 + dz]
        });
        let rect = rect.filter(|rect| rect[2] - rect[0] < camera.map.x());
        self.collected = Some(rect);
        Some(self.collect(rect, camera.map.x()))
    }

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
        let mut budget = viewer_scene_base::mesh_budget::Budget::new(0, steady);
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
        budget: &mut viewer_scene_base::mesh_budget::Budget,
    ) {
        let group = &prepared.groups[index];
        self.counts.groups += 1;
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
        self.counts.applied += 1;
        self.counts.slots += group.slots.len() as u64;
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
            if state.destroyed != destroyed
                || state.variant != selection.variant
                || state.definition != Some(selection.definition)
            {
                place(&mut self.tiles, &mut self.changed, id, state, None);
                state.destroyed = destroyed;
                state.variant = selection.variant;
                if !steady && !budget.take() {
                    viewer_runtime::view_record::add("map buildings", "history_deferred", 1);
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
                if viewer_runtime::view_record::on() && instance.scale < 1.0 {
                    viewer_runtime::view_record::add("map buildings", "growing", 1);
                }
                self.dirty |= instance.scale < 1.0;
                self.growing |= instance.scale < 1.0 && prepared.popup_speed > 0.0;
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

    pub fn collect(&self, rect: Option<[f32; 4]>, width: f32) -> Vec<Placed> {
        viewer_runtime::view_record::add("map buildings", "collects", 1);
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
        viewer_runtime::view_record::add("map buildings", "collected", out.len() as u64);
        out
    }
}

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
}

impl BuildingFrame<'_> {
    pub fn through(&mut self, province: u32, budget: &mut viewer_scene_base::mesh_budget::Budget) {
        let Some(prepared) = &self.prepared else {
            return;
        };
        if !self.active {
            return;
        }
        if !draws_map_buildings(self.camera.height, prepared.shrink_distance)
            && self.history.near.is_empty()
        {
            self.next = self.history.admitted.len();
            return;
        }
        while let Some(index) = self.history.admitted.get(self.next).copied() {
            let Some(group) = prepared.groups.get(index as usize) else {
                break;
            };
            if group.province > province {
                break;
            }
            self.history.apply_group(
                prepared,
                index as usize,
                self.camera,
                self.delta,
                self.steady,
                budget,
            );
            self.next += 1;
        }
    }

    pub fn finish(mut self, budget: &mut viewer_scene_base::mesh_budget::Budget) -> Option<Vec<Placed>> {
        self.through(u32::MAX, budget);
        self.history.prepared = self.prepared;
        {
            use viewer_runtime::view_record::add;
            let counts = std::mem::take(&mut self.history.counts);
            add("map buildings", "groups_admitted", counts.groups);
            add("map buildings", "groups_applied", counts.applied);
            add("map buildings", "slots_walked", counts.slots);
        }
        self.enabled
            .then(|| self.history.stream(self.camera, self.viewport))
            .flatten()
    }
}

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
    let what = match (state.entity, entity) {
        (None, Some(_)) => "history_appeared",
        (Some(_), None) => "history_removed",
        _ => "history_changed",
    };
    viewer_runtime::view_record::add("map buildings", what, 1);
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

type SlotKey = (u32, String);

pub struct MapBuildingAssets {
    pub facts: Arc<viewer_foundation::building_facts::BuildingFacts>,
    pub library: ModelLibrary,
    pub defines: Defines,
    groups: Arc<[ProvinceSlots]>,
    slots: BTreeMap<SlotKey, Vec<(BuildingPosition, Option<u32>)>>,
    candidates: BTreeMap<String, Vec<usize>>,
    entities: BTreeMap<(usize, usize), usize>,
    damaged_entities: BTreeMap<(usize, usize), usize>,
    tokens: std::sync::Mutex<
        Option<(
            replay_frame::frame::ViewIdentity,
            map_display_data::retained::Retained<TokenTypes>,
        )>,
    >,
    floating: Vec<(BuildingPosition, u32)>,
    floating_mesh: Option<usize>,
    id: u64,
    sites: std::sync::OnceLock<incremental::Sites>,
    display_keys: std::sync::OnceLock<Vec<Arc<map_display_data::buildings::AssetKey>>>,
    display_binding: std::sync::OnceLock<[u8; 32]>,
    display_sites: std::sync::OnceLock<Arc<Vec<map_display_data::buildings::AuthoredSlot>>>,
    display_groups: std::sync::OnceLock<Arc<Vec<map_display_data::buildings::Group>>>,
    display_bindings: std::sync::OnceLock<
        Result<BTreeMap<Arc<map_display_data::buildings::AssetKey>, usize>, String>,
    >,
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

#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct TokenTypes(
    map_display_data::value_index::ValueIndex<
        u32,
        map_display_data::retained::Retained<Result<usize, String>>,
    >,
);
impl TokenTypes {
    fn get(&self, token: &u32) -> Option<&Result<usize, String>> {
        self.0.get(token).map(|value| &**value)
    }
    fn remove(&mut self, token: &u32) -> io::Result<()> {
        self.0 = self.0.removed(token)?;
        Ok(())
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct MapBuildingSummary {
    pub unknown_city_damage: BTreeMap<String, usize>,
    pub slots: usize,
    pub built: usize,
    pub placed: usize,
    pub damaged: usize,
    pub destroyed: usize,
    pub uncertain_damage: usize,
    pub unnamed_tokens: usize,
    pub refused_tokens: Refusals,
    pub missing_slots: usize,
    pub missing_by_type: BTreeMap<String, usize>,
    pub unresolved_by_type: BTreeMap<String, usize>,
    pub hubs_left_to_another_layer: usize,
    pub unknown_hub_membership: usize,
    pub fogged: usize,
    pub undetected: usize,
    pub unknown_detection: usize,
    pub unknown_floating_harbors: BTreeMap<&'static str, usize>,
}

impl MapBuildingSummary {
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
    pub fn projection_id(&self) -> u64 {
        self.id
    }

    pub fn load(
        layers: &ResolvedLayers,
        decode: Decode,
        map: &MapData,
        defines: &content_reader_defines::defines::Defines,
        warnings: &mut Vec<content_reader_parser::Diagnostic>,
    ) -> io::Result<Self> {
        let provinces = ProvinceMap {
            width: map.width,
            height: map.height,
            ids: &map.province_ids,
        };
        let (states, _) = content_reader_states::states::read(layers, defines, &map.definitions)?;
        let defines = Defines::load(defines)?;
        let types = content_reader_key_names::buildings::read(layers)?;
        let roster = layers.descriptor_roster()?;
        if !roster.unreadable.is_empty() {
            warnings.push((content_reader_parser::DiagnosticKind::ParseFailure, format!(
                "These gfx descriptors do not parse, so the entities they declare are missing: {}.",
                roster.unreadable.join(", ")
            )));
        }
        let mut slots: BTreeMap<SlotKey, Vec<(BuildingPosition, Option<u32>)>> = BTreeMap::new();
        let mut authored: BTreeMap<SlotKey, Vec<usize>> = BTreeMap::new();
        let mut ambiguous_slots = BTreeSet::new();
        let mut outside = 0usize;
        let mut floating = Vec::new();
        for (order, position) in content_reader_map::positions::read_buildings(layers)?
            .into_iter()
            .enumerate()
        {
            if is_floating_harbor_slot(&position.kind) {
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
        if !ambiguous_slots.is_empty() {
            warnings.push((content_reader_parser::DiagnosticKind::Refused, format!(
                "Map-building slots are not drawn or used as rail and port anchors where content declares their type or bucket more than once differently (names equal but for letter case are one declaration), because the engine's choice is unverified: {}.",
                ambiguous_slots.into_iter().collect::<Vec<_>>().join(", ")
            )));
        }
        if outside != 0 {
            warnings.push((content_reader_parser::DiagnosticKind::Warning, format!(
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
        for (_, _, entity) in &wanted {
            if descriptors
                .inherit(entity)
                .is_some_and(|value| value.states.contains_key("damaged"))
            {
                let name = format!("{entity}::damaged");
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
                .map(|message| (content_reader_parser::DiagnosticKind::Warning, message)),
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
            warnings.push((content_reader_parser::DiagnosticKind::Refused, format!(
                "{} map-building entities named by common/buildings resolve to no mesh ({}); buildings of those types are not drawn.",
                unresolved.len(),
                unresolved.join(", ")
            )));
        }
        let floating_mesh = library.slot(FLOATING_HARBOR_ENTITY);
        if !floating.is_empty() && floating_mesh.is_none() {
            warnings.push((
                content_reader_parser::DiagnosticKind::Refused,
                format!(
                    "{FLOATING_HARBOR_ENTITY} resolves to no mesh; floating harbours are not drawn."
                ),
            ));
        }
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
            display_groups: std::sync::OnceLock::new(),
            slots,
            candidates,
            entities,
            damaged_entities,
            facts: Arc::new(viewer_foundation::building_facts::BuildingFacts {
                ordered,
                province_sites,
                keys,
                index,
            }),
            tokens: Default::default(),
            floating: Vec::new(),
            floating_mesh: None,
            id: incremental::next_assets(),
            sites: std::sync::OnceLock::new(),
            display_keys: std::sync::OnceLock::new(),
            display_binding: std::sync::OnceLock::new(),
            display_sites: std::sync::OnceLock::new(),
            display_bindings: std::sync::OnceLock::new(),
        }
    }

    pub fn slot_count(&self) -> usize {
        self.ordinary_slot_count() + self.floating.len()
    }

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
            &mut viewer_runtime::profiling::BuilderSpans::new(false),
        )
    }

    pub fn field_profiled(
        &self,
        frame: &DecodedFrame,
        vision: &Vision,
        city: &CityTerrain<'_>,
        spans: &mut viewer_runtime::profiling::BuilderSpans,
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
        spans: &mut viewer_runtime::profiling::BuilderSpans,
    ) -> io::Result<(PreparedBuildings, MapBuildingSummary, BuildingsBase)> {
        self.field_patched_bound(
            replay_frame::frame::HourInput::new(frame),
            vision,
            city,
            None,
            base,
            spans,
        )
    }

    pub fn field_patched_shared_city(
        &self,
        frame: &DecodedFrame,
        vision: &Arc<Vision>,
        city: &CityTerrain<'_>,
        bank: &Arc<Vec<bool>>,
        base: Option<&BuildingsBase>,
        spans: &mut viewer_runtime::profiling::BuilderSpans,
    ) -> io::Result<(PreparedBuildings, MapBuildingSummary, BuildingsBase)> {
        self.field_patched_input(
            replay_frame::frame::HourInput::new(frame),
            vision,
            city,
            bank,
            base,
            spans,
        )
    }

    pub fn field_patched_input(
        &self,
        input: replay_frame::frame::HourInput<'_>,
        vision: &Arc<Vision>,
        city: &CityTerrain<'_>,
        bank: &Arc<Vec<bool>>,
        base: Option<&BuildingsBase>,
        spans: &mut viewer_runtime::profiling::BuilderSpans,
    ) -> io::Result<(PreparedBuildings, MapBuildingSummary, BuildingsBase)> {
        self.field_patched_bound(input, vision, city, Some(bank), base, spans)
    }

    fn field_patched_bound(
        &self,
        input: replay_frame::frame::HourInput<'_>,
        vision: &Arc<Vision>,
        city: &CityTerrain<'_>,
        bank: Option<&Arc<Vec<bool>>>,
        base: Option<&BuildingsBase>,
        spans: &mut viewer_runtime::profiling::BuilderSpans,
    ) -> io::Result<(PreparedBuildings, MapBuildingSummary, BuildingsBase)> {
        let bank = bank
            .map(|bank| incremental::CityBank::new(city, bank))
            .transpose()?;
        let shared_city = bank
            .as_ref()
            .and_then(|bank| base.and_then(|base| base.city_for(bank, self.id)));
        let base = match base {
            Some(base) => Ok(base),
            None => Err("no earlier built hour"),
        };
        let mut built = self.build_with_input(input, vision, city, shared_city, base, spans)?;
        let city_bits = built.city.take().expect("city operands prepared once");
        let mut next = built.base(self, vision, city_bits);
        next.city_bank = bank;
        Ok((built.prepared, built.summary, next))
    }

    fn build(
        &self,
        frame: &DecodedFrame,
        vision: &Vision,
        city: &CityTerrain<'_>,
        base: Result<&BuildingsBase, &'static str>,
        spans: &mut viewer_runtime::profiling::BuilderSpans,
    ) -> io::Result<incremental::Built> {
        self.build_with_city(frame, vision, city, None, base, spans)
    }

    fn build_with_city(
        &self,
        frame: &DecodedFrame,
        vision: &Vision,
        city: &CityTerrain<'_>,
        shared_city: Option<&map_display_data::retained::Retained<Vec<bool>>>,
        base: Result<&BuildingsBase, &'static str>,
        spans: &mut viewer_runtime::profiling::BuilderSpans,
    ) -> io::Result<incremental::Built> {
        self.build_with_input(
            replay_frame::frame::HourInput::new(frame),
            vision,
            city,
            shared_city,
            base,
            spans,
        )
    }

    fn build_with_input(
        &self,
        input: replay_frame::frame::HourInput<'_>,
        vision: &Vision,
        city: &CityTerrain<'_>,
        shared_city: Option<&map_display_data::retained::Retained<Vec<bool>>>,
        base: Result<&BuildingsBase, &'static str>,
        spans: &mut viewer_runtime::profiling::BuilderSpans,
    ) -> io::Result<incremental::Built> {
        let frame = input.frame();
        let tokens = spans.measure("buildings.tokens", |_| {
            let _cost = replay_frame::cpu::scope("buildings.tokens", Some("buildings"));
            self.token_map_input(input)
        })?;
        let mut summary = MapBuildingSummary {
            slots: self.slot_count(),
            ..MapBuildingSummary::default()
        };
        let observation = {
            let _cost = replay_frame::cpu::scope("buildings.observation", Some("buildings"));
            incremental::Observation::read_input(
                input,
                &self.defines,
                vision.ours(),
                base.ok().map(|base| &base.observation),
            )?
        };
        let detection = spans.measure("buildings.detection", |_| {
            let _cost = replay_frame::cpu::scope("buildings.detection", Some("buildings"));
            Detection::observed(&observation)
        });
        let naval_bases = rows::<tables::NavalBases>(frame)?;
        let naval_base_provinces = observation.port_membership();
        let mut available = spans.measure("buildings.rows", |_| {
            let _cost = replay_frame::cpu::scope("buildings.rows", Some("buildings"));
            available::AvailableRows::read_input(
                self,
                input,
                &tokens,
                base.ok().and_then(|base| base.available_for(self.id)),
                &mut summary,
            )
        })?;
        let city_damage = spans.measure("buildings.city_damage", |_| {
            let _cost = replay_frame::cpu::scope("buildings.city_damage", Some("buildings"));
            available.city_damage(frame, self)
        })?;
        let damage_refused = city_damage.refused.is_some();
        let fog_changes = {
            let _cost = replay_frame::cpu::scope("buildings.fog", Some("buildings"));
            incremental::fog_changes(base.ok(), vision)?
        };
        let context = SlotContext {
            vision,
            city,
            detection: &detection,
            naval_bases: &naval_base_provinces,
            naval_bases_complete: naval_bases.is_complete(),
            available: &available,
        };
        let city_bits = match shared_city {
            Some(bits) => bits.clone(),
            None => self.city_bits(city),
        };
        let plan = match base {
            Err(reason) => Err(reason),
            Ok(base) => spans
                .measure("buildings.patch.plan", |_| {
                    let _cost = replay_frame::cpu::scope("buildings.patch.plan", Some("buildings"));
                    let hour = incremental::Inputs {
                        frame,
                        vision,
                        tokens: &tokens,
                        city: &city_bits,
                        damage_refused,
                        observation: &observation,
                        available: &available,
                        fog_changes: fog_changes.as_ref(),
                    };
                    self.affected(base, &hour)
                })?
                .map(|affected| (base, affected.slots)),
        };
        let ordinary_slots = self.ordinary_slot_count();
        let ordinary = match plan {
            Ok((base, affected)) => spans.measure("buildings.patch.slots", |_| {
                let _cost = replay_frame::cpu::scope("buildings.patch.slots", Some("buildings"));
                let (slots, tallies, totals) =
                    incremental::patch(base, &affected, &self.facts.ordered, |id| {
                        let (key, ordinal, position, province) = self.site(id);
                        let candidates = self.bucket_candidates(&key.1);
                        self.slot(id, ordinal, position, province, candidates, &context)
                    })?;
                Ok::<_, io::Error>(incremental::Ordinary::Patched {
                    base,
                    slots,
                    tallies,
                    totals,
                })
            }),
            Err(reason) => spans.measure("buildings.slots", |_| {
                let _cost = replay_frame::cpu::scope("buildings.slots", Some("buildings"));
                let mut slots = incremental::ChunkBuilder::new();
                let mut tallies = incremental::ChunkBuilder::new();
                let mut totals = incremental::SlotSummary::default();
                for ((_, bucket), positions) in &self.slots {
                    let candidates = self.bucket_candidates(bucket);
                    for (ordinal, (position, province)) in positions.iter().enumerate() {
                        let id = slots.len() as u32;
                        let (slot, tally) =
                            self.slot(id, ordinal, position, *province, candidates, &context)?;
                        totals.add(tally, &self.facts.ordered)?;
                        tallies.push(tally)?;
                        slots.push(slot)?;
                    }
                }
                Ok(incremental::Ordinary::Full {
                    slots,
                    tallies,
                    totals,
                    reason,
                })
            }),
        }?;
        let floating = spans.measure("buildings.supply", |_| {
            let _cost = replay_frame::cpu::scope("buildings.supply", Some("buildings"));
            incremental::FloatingInputs::read(
                frame,
                base.ok().and_then(|old| old.floating_for(self.id)),
            )
        })?;
        let floating_output = spans.measure("buildings.floating", |_| {
            let _cost = replay_frame::cpu::scope("buildings.floating", Some("buildings"));
            floating::Output::read(
                self,
                frame,
                vision,
                &floating,
                base.ok().filter(|old| old.floating_for(self.id).is_some()),
                fog_changes.as_ref(),
            )
        })?;
        floating_output.apply(&mut summary);
        if let Some(reason) = &city_damage.refused {
            summary.unknown_city_damage.insert(
                reason.clone(),
                rows::<tables::Provinces>(frame)?.rows().len(),
            );
        }
        let (slots, tallies, totals, path) = {
            let _cost = replay_frame::cpu::scope("buildings.chunks", Some("buildings"));
            ordinary.finish(&floating_output, ordinary_slots)?
        };
        totals.apply(&mut summary, &self.facts.ordered);
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
            available: Some(available),
            floating: Some(floating),
            floating_output: Some(floating_output),
            tokens,
            #[cfg(test)]
            gates: Arc::new(incremental::GateInputs {
                ports: naval_base_provinces,
                provinces: detection.provinces,
                states: detection.states,
            }),
            city: Some(city_bits),
            observation: Some(observation),
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
    ) -> io::Result<(PreparedSlot, Tally)> {
        let site = Site {
            state: position.state,
            at: [position.x, position.y, position.z],
            rotation: position.rotation,
            province,
            ordinal,
        };
        Ok(self.select(id, &site, candidates, |key| context.available.get(&key).copied(), context))
    }

    fn select(
        &self,
        id: u32,
        site: &Site,
        candidates: &[usize],
        row: impl Fn((bool, u32, usize)) -> Option<BuildingInput>,
        gates: &impl SlotGates,
    ) -> (PreparedSlot, Tally) {
        let selected = candidates.iter().find_map(|&index| {
            let building = &self.facts.ordered[index];
            let location = if building.provincial() {
                site.province?
            } else {
                site.state
            };
            let row = row((building.provincial(), location, index))?;
            (row.level > 0).then_some((index, row))
        });
        let mut prepared = PreparedSlot {
            source: selected.map(|(index, row)| (index, row.country)),
            id,
            drawn_at_distance: selected
                .map(|(index, _)| self.facts.ordered[index].drawn_at_distance),
            entity: None,
        };
        let mut tally = Tally::default();
        if let Some((index, row)) = selected {
            prepared.entity = self
                .entity(index, &row, site, id, &mut tally, gates)
                .map(PreparedEntity::retain);
        }
        (prepared, tally)
    }

    fn entity(
        &self,
        index: usize,
        row: &BuildingInput,
        site: &Site,
        seed: u32,
        tally: &mut Tally,
        gates: &impl SlotGates,
    ) -> Option<PreparedEntity> {
        let building = &self.facts.ordered[index];
        let province = site.province;
        let in_order =
            site.ordinal < slots_for_level(building, row.level, self.defines.max_levels) as usize;
        let healthy_admitted = building.show_on_map_meshes >= 2 || in_order;
        let destroyed_admitted =
            (!building.has_destroyed_mesh && building.show_on_map_meshes >= 2) || in_order;
        if !healthy_admitted && !destroyed_admitted {
            return None;
        }
        if province.is_some() && !building.always_shown {
            tally.mark(Tally::READ_FOG);
        }
        if province.is_some_and(|province| gates.sees(province, building.always_shown) == Some(false)) {
            tally.mark(Tally::FOGGED);
            return None;
        }
        if building.supply_node {
            tally.mark(Tally::READ_HUB);
            let on_city = gates.on_city(site.at[0], site.at[2]);
            match hub_left_to_another_layer(
                province,
                &Ports(gates),
                gates.naval_bases_complete(),
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
            match gates.detected(row) {
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
                position: site.at,
                rotation: site.rotation,
                scale: 1.0,
                animation_seed: seed,
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

struct Site {
    state: u32,
    at: [f32; 3],
    rotation: f32,
    province: Option<u32>,
    ordinal: usize,
}

pub trait SlotGates {
    fn sees(&self, province: u32, always_shown: bool) -> Option<bool>;
    fn on_city(&self, x: f32, z: f32) -> bool;
    fn naval_base(&self, province: u32) -> bool;
    fn naval_bases_complete(&self) -> bool;
    fn detected(&self, row: &BuildingInput) -> Option<bool>;
}

struct Ports<'a, G>(&'a G);

impl<G: SlotGates> NavalMembership for Ports<'_, G> {
    fn has(&self, province: u32) -> bool {
        self.0.naval_base(province)
    }
}

impl SlotGates for SlotContext<'_> {
    fn sees(&self, province: u32, always_shown: bool) -> Option<bool> {
        self.vision.sees_province_object(province, always_shown)
    }
    fn on_city(&self, x: f32, z: f32) -> bool {
        self.city.at(x, z)
    }
    fn naval_base(&self, province: u32) -> bool {
        self.naval_bases.has(province)
    }
    fn naval_bases_complete(&self) -> bool {
        self.naval_bases_complete
    }
    fn detected(&self, row: &BuildingInput) -> Option<bool> {
        self.detection.detected(row)
    }
}

#[cfg(test)]
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
    detection: &'a Detection,
    naval_bases: &'a map_display_data::copy_index::CopyIndex<u32, ()>,
    naval_bases_complete: bool,
    available: &'a available::AvailableRows,
}

trait NavalMembership {
    fn has(&self, province: u32) -> bool;
}
impl NavalMembership for map_display_data::copy_index::CopyIndex<u32, ()> {
    fn has(&self, province: u32) -> bool {
        self.get(&province).is_some()
    }
}
#[cfg(test)]
impl NavalMembership for BTreeSet<u32> {
    fn has(&self, province: u32) -> bool {
        self.contains(&province)
    }
}
fn hub_left_to_another_layer(
    province: Option<u32>,
    naval_bases: &impl NavalMembership,
    naval_bases_complete: bool,
    on_city_terrain: bool,
) -> Option<bool> {
    if on_city_terrain || province.is_some_and(|id| naval_bases.has(id)) {
        Some(true)
    } else if province.is_none() || naval_bases_complete {
        Some(false)
    } else {
        None
    }
}

use viewer_foundation::building_facts::Scope;

#[cfg(test)]
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

struct Detection {
    links: map_display_data::copy_index::CopyIndex<u32, u32>,
    ours: u32,
    provinces: map_display_data::copy_index::CopyIndex<u32, u32>,
    states: map_display_data::copy_index::CopyIndex<u32, u32>,
    targets: Option<map_display_data::copy_index::CopyIndex<Scope, bool>>,
}

impl Detection {
    #[cfg(test)]
    fn read(frame: &DecodedFrame, links: &BTreeMap<u32, u32>, ours: u32) -> io::Result<Self> {
        fn index<K: Ord + Copy, V: Copy + PartialEq>(
            mut rows: impl Iterator<Item = (K, V)>,
        ) -> io::Result<map_display_data::copy_index::CopyIndex<K, V>> {
            rows.try_fold(
                map_display_data::copy_index::CopyIndex::default(),
                |index, (key, value)| index.inserted(key, value),
            )
        }
        let slot = ours as usize;
        Ok(Self {
            links: index(links.iter().map(|(key, value)| (*key, *value)))?,
            ours,
            provinces: index(
                rows::<tables::Provinces>(frame)?
                    .rows()
                    .iter()
                    .map(|row| (row.id, row.controller)),
            )?,
            states: index(
                rows::<tables::States>(frame)?
                    .rows()
                    .iter()
                    .map(|row| (row.id, row.controller)),
            )?,
            targets: raid_targets(frame)
                .map(|targets| {
                    index(targets.into_iter().map(|(scope, bits)| {
                        (
                            scope,
                            bits.get(slot / 8)
                                .is_some_and(|byte| byte & (1 << (slot % 8)) != 0),
                        )
                    }))
                })
                .transpose()?,
        })
    }

    fn detected(&self, row: &available::BuildingInput) -> Option<bool> {
        detected(
            row,
            |provincial, location| {
                if provincial {
                    self.provinces.get(&location).copied()
                } else {
                    self.states.get(&location).copied()
                }
            },
            |raw| self.links.get(&raw).copied(),
            self.ours,
            |scope| self.targets.as_ref().map(|targets| targets.get(&scope).copied()),
        )
    }
}

pub fn detected(
    row: &BuildingInput,
    controller: impl FnOnce(bool, u32) -> Option<u32>,
    link: impl FnOnce(u32) -> Option<u32>,
    ours: u32,
    target: impl FnOnce(Scope) -> Option<Option<bool>>,
) -> Option<bool> {
    viewer_foundation::building_facts::detected(
        (row.building_type, row.location, row.status_flags, row.country),
        controller,
        link,
        ours,
        target,
    )
}

fn token_types(
    frame: &DecodedFrame,
    keys: &TokenKeys,
    index: &BTreeMap<String, usize>,
) -> io::Result<TokenTypes> {
    named_types(&TokenNames::new(frame)?, keys, index)
}

fn named_types(
    names: &TokenNames<'_>,
    keys: &TokenKeys,
    index: &BTreeMap<String, usize>,
) -> io::Result<TokenTypes> {
    let mut types = TokenTypes::default();
    for (token, spelling) in names.iter() {
        if let Some(value) = named_type(spelling, keys, index)? {
            types.0 = types.0.inserted(token, value)?;
        }
    }
    Ok(types)
}
fn named_type(
    spelling: &str,
    keys: &TokenKeys,
    index: &BTreeMap<String, usize>,
) -> io::Result<Option<map_display_data::retained::Retained<Result<usize, String>>>> {
    Ok(named_class(spelling, keys, index).map(map_display_data::retained::Retained::new))
}

fn named_class(
    spelling: &str,
    keys: &TokenKeys,
    index: &BTreeMap<String, usize>,
) -> Option<Result<usize, String>> {
    match keys.resolve(spelling) {
        Ok(Some(key)) => index.get(key).map(|index| Ok(*index)),
        Ok(None) => None,
        Err(reason) => Some(Err(reason)),
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TokenClass {
    Type(usize),
    Refused { reason: String, unnamed: bool },
}

pub const READS_FOG: u8 = 1;
pub const READS_HUB: u8 = 2;
pub const READS_DETECTION: u8 = 4;

impl MapBuildingAssets {
    pub fn ordinary_slots(&self) -> usize {
        self.ordinary_slot_count()
    }

    pub fn harbour_count(&self) -> usize {
        self.floating.len()
    }

    pub fn slot_province(&self, id: u32) -> Option<u32> {
        self.site(id).3
    }

    pub fn harbour_sea(&self, index: usize) -> u32 {
        self.floating[index].0.adjacent_sea
    }

    pub fn types(&self) -> &[BuildingType] {
        &self.facts.ordered
    }

    pub fn slot_value(
        &self,
        id: u32,
        row: impl Fn((bool, u32, usize)) -> Option<BuildingInput>,
        gates: &impl SlotGates,
    ) -> (PreparedSlot, u8) {
        let (key, ordinal, position, province) = self.site(id);
        let site = Site {
            state: position.state,
            at: [position.x, position.y, position.z],
            rotation: position.rotation,
            province,
            ordinal,
        };
        let (slot, tally) = self.select(id, &site, self.bucket_candidates(&key.1), row, gates);
        let reads = [(Tally::READ_FOG, READS_FOG), (Tally::READ_HUB, READS_HUB), (Tally::READ_DETECTION, READS_DETECTION)];
        (slot, reads.into_iter().filter(|&(flag, _)| tally.reads(flag)).fold(0, |bits, (_, bit)| bits | bit))
    }

    pub fn harbour_value(
        &self,
        index: usize,
        byte: impl FnOnce(u32) -> Option<u8>,
        deployed: impl FnOnce(u32) -> Option<bool>,
    ) -> (PreparedSlot, HarbourCounts) {
        let id = (self.ordinary_slot_count() + index) as u32;
        self.harbour(&self.floating[index].0, id, byte, deployed)
    }

    pub fn slots_of(&self, provincial: bool, location: u32, index: usize) -> impl Iterator<Item = u32> + '_ {
        self.sites().slots_of((provincial, location, index))
    }

    pub fn slots_at(&self, location: u32) -> impl Iterator<Item = u32> + '_ {
        self.sites().at_location(location)
    }

    pub fn slotted_provinces(&self) -> impl Iterator<Item = u32> + '_ {
        self.sites().provinces()
    }

    pub fn slots_in(&self, province: u32) -> impl Iterator<Item = u32> + '_ {
        self.sites().slots_in(province)
    }

    pub fn token_class(&self, token: u32, literal: Option<Option<&[u8]>>, lexical_complete: bool) -> TokenClass {
        use dashboard_text::table_access::TokenError;
        let unnamed = |error: TokenError| TokenClass::Refused { reason: error.to_string(), unnamed: true };
        if token == 0 {
            return unnamed(TokenError::Rejected);
        }
        let Some(literal) = literal else {
            return unnamed(if lexical_complete { TokenError::Absent(token) } else { TokenError::Missing(token) });
        };
        let Some(spelling) = literal.and_then(|literal| std::str::from_utf8(literal).ok()).filter(|name| !name.contains('\0')) else {
            return unnamed(TokenError::NotText(token));
        };
        match named_class(spelling, &self.facts.keys, &self.facts.index) {
            Some(Ok(index)) => TokenClass::Type(index),
            Some(Err(reason)) => TokenClass::Refused { reason, unnamed: false },
            None => TokenClass::Refused { reason: format!("token {token} ({spelling}) names no authored building type"), unnamed: false },
        }
    }

    pub fn city_templates(&self) -> Result<Vec<usize>, String> {
        let types = &self.facts.ordered;
        let infrastructure = types.iter().filter(|kind| kind.name.eq_ignore_ascii_case("infrastructure")).count();
        if infrastructure > 1 {
            return Err(format!("infrastructure: {infrastructure} ambiguous city-damage templates"));
        }
        if types.iter().any(|kind| kind.name == "industrial_complex") {
            let count = types.iter().filter(|kind| kind.name.eq_ignore_ascii_case("industrial_complex")).count();
            if count > 1 {
                return Err(format!("industrial_complex: {count} ambiguous city-damage templates"));
            }
        }
        let mut selected = Vec::with_capacity(types.len());
        selected.extend(types.iter().enumerate().filter(|(_, kind)| kind.name.eq_ignore_ascii_case("infrastructure")).map(|(index, _)| index));
        selected.extend(types.iter().enumerate().filter(|(_, kind)| kind.name == "industrial_complex").map(|(index, _)| index));
        Ok(selected)
    }
}

#[cfg(any(test, feature = "test-support"))]
impl MapBuildingAssets {
    pub fn stub(
        types: &Buildings,
        defines: Defines,
        slots: Vec<(u32, String, BuildingPosition, Option<u32>)>,
        entities: Vec<((String, usize), usize)>,
        floating: Vec<(BuildingPosition, u32)>,
        floating_mesh: Option<usize>,
    ) -> Self {
        let mut keyed: BTreeMap<SlotKey, Vec<(BuildingPosition, Option<u32>)>> = BTreeMap::new();
        for (location, bucket, position, province) in slots {
            keyed.entry((location, bucket)).or_default().push((position, province));
        }
        let mut assets = Self::assemble(
            ModelLibrary::default(),
            types,
            defines,
            slot_groups(std::iter::empty(), &BTreeMap::new()),
            keyed,
            entities.into_iter().collect(),
            BTreeMap::new(),
        );
        assets.floating = floating;
        assets.floating_mesh = floating_mesh;
        assets
    }
}

#[cfg(test)]
mod tests;

#[cfg(any(test, feature = "test-support"))]
pub mod budget_fixture;
