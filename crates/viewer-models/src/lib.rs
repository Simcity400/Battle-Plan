use content_reader_mesh::mesh::MeshProducts as _;
#[cfg(test)]
#[global_allocator]
static TEST_ALLOCATOR: engine_heap::Heap = engine_heap::Heap;


pub mod aircraft_events;
pub mod entity_animation;
pub mod mesh_pose;
pub mod motion_view;
pub mod poses;
pub mod sound_events;
pub mod strike_effects;

use crate::entity_animation::Rig;
use std::cell::Cell;
use std::cell::RefCell;
use std::sync::OnceLock;
use std::time::Instant;
use viewer_runtime::debug_fields;
use viewer_gpu::textures::{load_file, locate_file};
use viewer_runtime::profiling::BuilderSpans;
mod animation_dispatch;
mod draw_snapshot;
mod identity;
#[cfg(any(test, feature = "test-support"))]
#[path = "order_fixture.rs"]
pub mod order_fixture;
#[cfg(test)]
mod order_tests;
pub use identity::{ActorAddress, AssetKey};
mod native;
mod skeleton_source;
pub use native::{
    Inputs as NativeInputs, ObjectConstants as NativeObjectConstants, Queue as NativeQueue,
};
use std::{
    collections::{BTreeMap, BTreeSet},
    f32::consts::TAU,
    io,
    ops::Range,
    sync::Arc,
};

use bytemuck::{Pod, Zeroable};
use eframe::wgpu;
use content_reader_layers::ResolvedLayers;
use content_reader_mesh::mesh::{self, DescriptorView, Descriptors};
use content_reader_image::{Rgba8Image, dds, native::Decode};

use viewer_scene_base::shadow::DEPTH_MAP_FORMAT;
use viewer_gpu::{
    camera::{Camera, MapSize},
    gpu_contract::{DEPTH_FORMAT, Targets},
    textures::{Bound, ColorSpace, upload},
};

fn invalid(message: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message.into())
}

pub(crate) fn missing_mesh(entity: &mesh::InheritedEntity<'_>, name: &str) -> String {
    match entity.mesh.or(entity.unbound_mesh) {
        Some(mesh) => format!("entity {name} references pdxmesh {mesh}, which is not registered"),
        None => format!("entity {name} has no pdxmesh binding"),
    }
}

pub fn map_effects(state: &mesh::EntityState) -> usize {
    state
        .events
        .iter()
        .map(|event| usize::from(event.particle.is_some()) + usize::from(event.light.is_some()))
        .sum()
}


pub const WATER_HEIGHT: f32 = 9.5;
pub fn draws_city_sprawl(camera: &Camera) -> bool {
    camera.height - WATER_HEIGHT < camera.defines.sprawl_cutoff
}



#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Pod, Zeroable)]
pub struct PaletteJoint {
    pub rows: [[f32; 4]; 3],
}

impl PaletteJoint {
    #[cfg(any(test, feature = "test-support"))]
    pub const IDENTITY: Self = Self {
        rows: [
            [1.0, 0.0, 0.0, 0.0],
            [0.0, 1.0, 0.0, 0.0],
            [0.0, 0.0, 1.0, 0.0],
        ],
    };

    pub fn from_transform(transform: &mesh::Transform) -> Self {
        let [a, b, c] = transform.basis;
        let t = transform.translation;
        Self {
            rows: [
                [a[0], b[0], c[0], t[0]],
                [a[1], b[1], c[1], t[1]],
                [a[2], b[2], c[2], t[2]],
            ],
        }
    }
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Pod, Zeroable)]
pub struct ModelInstance {
    pub position: [f32; 3],
    pub rotation: f32,
    pub scale: f32,
    pub pitch: f32,
    pub roll: f32,
    pub train_clip: [f32; 4],
    pub palette_offset: u32,
    pub animation: u32,
    pub animation_seed: u32,
    pub animation_phase: f32,
    pub animation_epoch: u32,
    pub native_flags: u32,
    pub native_basis: [[f32; 3]; 3],
    pub animation_identity: u32,
}

impl From<ModelInstance> for map_display_data::buildings::Instance {
    fn from(value: ModelInstance) -> Self {
        Self {
            position: value.position,
            rotation: value.rotation,
            scale: value.scale,
            animation_seed: value.animation_seed,
        }
    }
}
impl From<map_display_data::buildings::Instance> for ModelInstance {
    fn from(value: map_display_data::buildings::Instance) -> Self {
        Self {
            position: value.position,
            rotation: value.rotation,
            scale: value.scale,
            animation_seed: value.animation_seed,
            ..Self::default()
        }
    }
}

impl ModelInstance {
    pub fn animation_key(self) -> (u32, u32) {
        (
            self.animation_seed,
            if self.animation_identity != 0 {
                self.animation_identity
            } else {
                self.animation_epoch
            },
        )
    }

    fn native_transform(self) -> io::Result<mesh::Transform> {
        if self.native_flags & 4 != 0 {
            return Ok(mesh::Transform {
                basis: self.native_basis.map(|axis| axis.map(|v| v * self.scale)),
                translation: self.position,
            });
        }
        if self.roll != 0. {
            return Err(invalid(
                "native mesh roll requires the caller's original basis",
            ));
        }
        Ok(entity_animation::world_transform(self))
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct ModelMaterial {
    pub diffuse: usize,
    pub normal: usize,
    pub specular: usize,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ModelMesh {
    pub asset: AssetKey,
    pub name: String,
    pub path: String,
    pub scale: f32,
    pub lod_distances: Vec<f32>,
    native: Vec<native::Descriptor>,
    pub radius: f32,
    pub height: f32,
    pub visual: bool,
    pub own_radius: f32,
    pub bound: f32,
    pub convoy_radius: Option<f32>,
    pub attachments: usize,
    pub cull_distance_squared: f32,
}

#[derive(Default)]
pub struct ModelLibrary {
    layers: Option<Arc<ResolvedLayers>>,
    decode: Decode,
    pub palette: Vec<PaletteJoint>,
    pub meshes: Vec<ModelMesh>,
    pub materials: Vec<ModelMaterial>,
    pub textures: Vec<ModelTexture>,
    pub spaces: Vec<ColorSpace>,
    uploads: Arc<Uploads>,
    pub index: BTreeMap<String, usize>,
    paths: BTreeMap<(String, ColorSpace), usize>,
    animations: BTreeMap<String, Option<Arc<mesh::Animation>>>,
    pub animation_refusals: BTreeMap<String, String>,
    rigs: Vec<Arc<Rig>>,
    geometry_sources: BTreeMap<native::GeometryKey, Arc<viewer_scene_base::mesh_shader::ModelGeometry>>,
    attachment_refusals: BTreeMap<String, usize>,
    pub failed: BTreeMap<String, String>,
    composing_visual: bool,
    pub alternatives: BTreeMap<usize, usize>,
    alternative: bool,
    asset_namespace: Option<Arc<[identity::ContentLayer]>>,
}

#[derive(Clone)]
pub struct ModelTexture(Arc<TextureSource>);

enum TextureSource {
    Held(dds::Texture),
    Deferred {
        path: String,
        layers: Arc<ResolvedLayers>,
        visual: bool,
    },
}

impl ModelTexture {
    pub fn held(texture: dds::Texture) -> Self {
        Self(Arc::new(TextureSource::Held(texture)))
    }

    fn deferred(path: String, layers: Arc<ResolvedLayers>, visual: bool) -> Self {
        Self(Arc::new(TextureSource::Deferred {
            path,
            layers,
            visual,
        }))
    }

    #[cfg(test)]
    pub(crate) fn deferred_for_tests(path: String, layers: Arc<ResolvedLayers>) -> Self {
        Self::deferred(path, layers, false)
    }

    pub fn source(&self) -> Option<dds::Texture> {
        match self.0.as_ref() {
            TextureSource::Held(texture) => Some(texture.clone()),
            TextureSource::Deferred { .. } => None,
        }
    }

    pub fn path(&self) -> Option<&str> {
        match self.0.as_ref() {
            TextureSource::Held(_) => None,
            TextureSource::Deferred { path, .. } => Some(path.as_str()),
        }
    }

    fn is_visual(&self) -> bool {
        match self.0.as_ref() {
            TextureSource::Held(_) => false,
            TextureSource::Deferred { visual, .. } => *visual,
        }
    }

    #[cfg(test)]
    pub(crate) fn decode(&self, decode: Decode) -> io::Result<dds::Texture> {
        match self.0.as_ref() {
            TextureSource::Held(texture) => Ok(texture.clone()),
            TextureSource::Deferred { path, layers, .. } => {
                load_file(path, &|path| layers.read_uncovered_path(path), decode)
            }
        }
    }

    fn upload(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        label: &str,
        space: ColorSpace,
        decode: Decode,
    ) -> io::Result<Bound> {
        match self.0.as_ref() {
            TextureSource::Held(texture) => upload(device, queue, label, texture, space),
            TextureSource::Deferred { path, layers, .. } => {
                let texture = load_file(path, &|path| layers.read_uncovered_path(path), decode)
                    .map_err(|error| {
                        invalid(format!(
                            "{path} could not be loaded for this map: {error}"
                        ))
                    })?;
                upload(device, queue, label, &texture, space)
            }
        }
    }
}

#[cfg(test)]
impl PartialEq for ModelTexture {
    fn eq(&self, other: &Self) -> bool {
        match (self.0.as_ref(), other.0.as_ref()) {
            (TextureSource::Held(a), TextureSource::Held(b)) => a == b,
            (
                TextureSource::Deferred {
                    path: a,
                    visual: av,
                    ..
                },
                TextureSource::Deferred {
                    path: b,
                    visual: bv,
                    ..
                },
            ) => a == b && av == bv,
            _ => false,
        }
    }
}

impl std::fmt::Debug for ModelTexture {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ModelTexture")
            .field("path", &self.path())
            .field(
                "held",
                &self
                    .source()
                    .map(|texture| (texture.width, texture.height, texture.format)),
            )
            .finish()
    }
}

#[derive(Default)]
struct Uploads(std::sync::Mutex<Vec<(wgpu::Device, std::sync::Weak<DeviceTextures>)>>);

impl Uploads {
    fn on(&self, device: &wgpu::Device) -> Arc<DeviceTextures> {
        let mut entries = self.0.lock().unwrap_or_else(|error| error.into_inner());
        entries.retain(|(_, textures)| textures.strong_count() != 0);
        if let Some(shared) = entries
            .iter()
            .find_map(|(owner, textures)| (owner == device).then(|| textures.upgrade()).flatten())
        {
            return shared;
        }
        let shared = Arc::new(DeviceTextures::default());
        entries.push((device.clone(), Arc::downgrade(&shared)));
        shared
    }
}

impl DeviceTextures {
    fn texture(
        &self,
        source: (&ModelTexture, ColorSpace, Decode),
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        label: &str,
        slot: usize,
    ) -> io::Result<Arc<Bound>> {
        let mut slots = self.0.lock().unwrap_or_else(|error| error.into_inner());
        if let Some(Some(bound)) = slots.get(slot) {
            return Ok(Arc::clone(bound));
        }
        let (texture, space, decode) = source;
        let bound = Arc::new(texture.upload(device, queue, label, space, decode)?);
        if slots.len() <= slot {
            slots.resize(slot + 1, None);
        }
        slots[slot] = Some(Arc::clone(&bound));
        Ok(bound)
    }
}

#[derive(Default)]
pub struct DeviceTextures(std::sync::Mutex<Vec<Option<Arc<Bound>>>>);

debug_fields!(ModelLibrary(this) {
    decode, palette, meshes, materials, textures, spaces, index, paths, animations, rigs, attachment_refusals, failed
} {
    geometry => this.geometry_sources.len()
});

#[derive(Default)]
struct Composition {
    native: Vec<native::Descriptor>,
    reach: Reach,
    rig: Rig,
    refusals: BTreeMap<String, usize>,
}

#[derive(Clone, Copy, Debug, Default)]
struct Reach {
    collision_unsupported: bool,
    collision_bounds: Option<([f32; 3], [f32; 3])>,
    radius: f32,
    height: f32,
    bound: f32,
    own_radius: f32,
    attachments: usize,
}

impl Reach {
    fn widen(&mut self, transform: &mesh::Transform, bounds: &mesh::Bounds, own: bool) {
        for corner in 0..8u8 {
            self.widen_point(
                transform.transform_point([
                    if corner & 1 == 0 {
                        bounds.min[0]
                    } else {
                        bounds.max[0]
                    },
                    if corner & 2 == 0 {
                        bounds.min[1]
                    } else {
                        bounds.max[1]
                    },
                    if corner & 4 == 0 {
                        bounds.min[2]
                    } else {
                        bounds.max[2]
                    },
                ]),
                own,
            );
        }
    }

    fn collision_point(&mut self, p: [f32; 3]) {
        let (low, high) = self.collision_bounds.get_or_insert((p, p));
        for axis in 0..3 {
            low[axis] = low[axis].min(p[axis]);
            high[axis] = high[axis].max(p[axis]);
        }
    }

    fn collision_radius(&self) -> Option<f32> {
        let (low, high) = self.collision_bounds?;
        let radius = (0..3)
            .map(|axis| (high[axis] - low[axis]).powi(2))
            .sum::<f32>()
            .sqrt()
            * 0.5;
        (radius.is_finite() && self.attachments == 0 && !self.collision_unsupported)
            .then_some(radius)
    }

    fn widen_point(&mut self, point: [f32; 3], own: bool) {
        let radius = (point[0] * point[0] + point[2] * point[2]).sqrt();
        self.radius = self.radius.max(radius);
        self.height = self.height.max(point[1]);
        self.bound = self.bound.max(radius.hypot(point[1]));
        if own {
            self.own_radius = self.own_radius.max(radius);
        }
    }
}

const BAKE_SHARDS: usize = 4;

#[derive(Clone, Copy)]
struct BakeLabels {
    worker: &'static str,
    join: &'static str,
    merge: &'static str,
}

impl BakeLabels {
    const SHOWN: Self = Self {
        worker: "units.ensure.bake_worker.shown",
        join: "units.ensure.bake_join_shown",
        merge: "units.ensure.bake_merge_shown",
    };
    const RECORDED: Self = Self {
        worker: "units.ensure.bake_worker.recorded",
        join: "units.ensure.bake_join_recorded",
        merge: "units.ensure.bake_merge_recorded",
    };
}

impl ModelLibrary {
    pub fn layers(&self) -> Option<&ResolvedLayers> {
        self.layers.as_deref()
    }

    pub fn rig(&self, slot: usize) -> &Rig {
        &self.rigs[slot]
    }

    pub fn rig_source(&self, slot: usize) -> (Arc<Rig>, Vec<PaletteJoint>) {
        let rig = Arc::clone(&self.rigs[slot]);
        let palette = self.palette[rig.start..rig.end].to_vec();
        (rig, palette)
    }

    pub fn empty(decode: Decode) -> Self {
        Self {
            decode,
            ..Default::default()
        }
    }

    pub fn decode(&self) -> Decode {
        self.decode
    }

    pub fn load<'a>(
        layers: &ResolvedLayers,
        decode: Decode,
        descriptors: impl Into<DescriptorView<'a>>,
        names: &[String],
        warnings: &mut Vec<content_reader_parser::Diagnostic>,
    ) -> io::Result<Self> {
        Self::load_with(layers, decode, descriptors, names, warnings, |_, _| None)
    }

    pub fn load_profiled(
        layers: &ResolvedLayers,
        decode: Decode,
        descriptors: &Descriptors,
        names: &[String],
        warnings: &mut Vec<content_reader_parser::Diagnostic>,
        spans: &mut BuilderSpans,
    ) -> io::Result<Self> {
        Self::load_with_spans(
            layers,
            layers,
            decode,
            descriptors.into(),
            names,
            warnings,
            |_, _| None,
            spans,
            BakeLabels::SHOWN,
        )
    }

    pub fn load_with<'a>(
        layers: &ResolvedLayers,
        decode: Decode,
        descriptors: impl Into<DescriptorView<'a>>,
        names: &[String],
        warnings: &mut Vec<content_reader_parser::Diagnostic>,
        refused: fn(&mesh::Roster, &str) -> Option<String>,
    ) -> io::Result<Self> {
        Self::load_with_spans(
            layers,
            layers,
            decode,
            descriptors.into(),
            names,
            warnings,
            refused,
            &mut BuilderSpans::new(false),
            BakeLabels::SHOWN,
        )
    }

    #[expect(clippy::too_many_arguments, reason = "the layers, the decoder and descriptors, the names, and the warnings, spans and labels it records")]
    fn load_with_spans(
        layers: &ResolvedLayers,
        session: &ResolvedLayers,
        decode: Decode,
        descriptors: DescriptorView<'_>,
        names: &[String],
        warnings: &mut Vec<content_reader_parser::Diagnostic>,
        refused: fn(&mesh::Roster, &str) -> Option<String>,
        spans: &mut BuilderSpans,
        labels: BakeLabels,
    ) -> io::Result<Self> {
        let mut seen = BTreeSet::new();
        let unique: Vec<&String> = names
            .iter()
            .filter(|name| seen.insert(name.as_str()))
            .collect();
        let shards = unique.len().clamp(1, BAKE_SHARDS);
        let chunk = unique.len().div_ceil(shards).max(1);
        let asset_namespace = identity::namespace_for(layers, session);
        let shared_layers = Arc::new(layers.clone());
        let baked = std::thread::scope(|scope| {
            let mut handles = Vec::new();
            for shard_names in unique.chunks(chunk).collect::<Vec<_>>() {
                let mut worker = spans.fork();
                let asset_namespace = Arc::clone(&asset_namespace);
                let shared_layers = Arc::clone(&shared_layers);
                handles.push(job_priority::spawn_in(scope, move || {
                    let baked = viewer_runtime::worker_priority::at_bake_priority(|| {
                        worker.measure(labels.worker, |_| {
                            let mut library = ModelLibrary {
                                layers: Some(shared_layers),
                                asset_namespace: Some(asset_namespace),
                                decode,
                                ..Default::default()
                            };
                            let mut missing = Vec::new();
                            for name in shard_names {
                                if let Err(error) = library.add(layers, descriptors, name) {
                                    missing.push(((*name).clone(), error.to_string()));
                                }
                            }
                            (library, missing)
                        })
                    });
                    (baked, worker)
                }));
            }
            spans.measure(labels.join, |_| {
                handles
                    .into_iter()
                    .map(|handle| handle.join().expect("entity baker panicked"))
                    .collect::<Vec<_>>()
            })
        });
        let mut library = ModelLibrary {
            layers: Some(shared_layers),
            decode,
            ..Default::default()
        };
        let mut missing = Vec::new();
        spans.measure(labels.merge, |spans| {
            for ((shard, shard_missing), worker) in baked {
                spans.append(worker);
                for (name, error) in shard_missing {
                    library.failed.insert(name.clone(), error.clone());
                    missing.push((name, error));
                }
                library.merge(shard);
            }
        });
        layers.release_parsers(&["binary asset grammar"]);
        if let Some(recorded) = layers.without_overlays()
            && let Some(roster) = layers.recorded_descriptor_roster()?
        {
            if !missing.is_empty() {
                let names: Vec<String> = missing
                    .iter()
                    .filter(|(name, _)| refused(&roster, name).is_none())
                    .map(|(name, _)| name.clone())
                    .collect();
                let baked = Self::load_with_spans(
                    &recorded,
                    session,
                    decode,
                    (&roster.descriptors).into(),
                    &names,
                    &mut Vec::new(),
                    |_, _| None,
                    spans,
                    BakeLabels::RECORDED,
                )?;
                spans.measure(BakeLabels::RECORDED.merge, |_| {
                    library.merge(baked.isolated(false));
                });
                missing.retain(|(name, error)| {
                    let drawn = library.slot(name).is_some();
                    if drawn {
                        library.failed.remove(name);
                        layers.note_overlay_refusal(format!("model {name} ({error})"));
                    }
                    !drawn
                });
            }
            let visual: Vec<String> = library
                .meshes
                .iter()
                .filter(|mesh| mesh.visual && refused(&roster, &mesh.name).is_none())
                .map(|mesh| mesh.name.clone())
                .collect();
            if !visual.is_empty() {
                let counterparts = Self::load_with_spans(
                    &recorded,
                    session,
                    decode,
                    (&roster.descriptors).into(),
                    &visual,
                    &mut Vec::new(),
                    |_, _| None,
                    spans,
                    BakeLabels::RECORDED,
                )?;
                spans.measure(BakeLabels::RECORDED.merge, |_| {
                    library.merge(counterparts.isolated(true));
                });
            }
        }
        if !library.animation_refusals.is_empty() {
            warnings.push((
                content_reader_parser::DiagnosticKind::Refused,
                format!(
                    "Model animations that do not read, so their entities stand in their bind \
                     pose: {}.",
                    library
                        .animation_refusals
                        .iter()
                        .map(|(path, why)| format!("{path} ({why})"))
                        .collect::<Vec<_>>()
                        .join("; ")
                ),
            ));
        }
        if !library.attachment_refusals.is_empty() {
            warnings.push((
                content_reader_parser::DiagnosticKind::Refused,
                format!(
                    "Attached entities refused: {}.",
                    library
                        .attachment_refusals
                        .iter()
                        .map(|(reason, count)| format!("{reason} ({count})"))
                        .collect::<Vec<_>>()
                        .join("; ")
                ),
            ));
        }
        if !missing.is_empty() {
            let mut missing: Vec<String> = missing
                .into_iter()
                .map(|(name, error)| format!("{name} ({error})"))
                .collect();
            missing.sort();
            warnings.push((
                content_reader_parser::DiagnosticKind::Refused,
                format!(
                    "No mesh resolved for {}; those objects are not drawn.",
                    missing.join(", ")
                ),
            ));
        }
        let mut meshless = Vec::new();
        for mesh in &library.meshes {
            if mesh.path.is_empty()
                && let Some(entity) = descriptors.inherit(&mesh.name)
            {
                meshless.push(missing_mesh(&entity, &mesh.name));
            }
        }
        if !meshless.is_empty() {
            warnings.push((
                content_reader_parser::DiagnosticKind::Warning,
                format!(
                    "{} geometry-less entities ({}); states and map events run without geometry.",
                    meshless.len(),
                    meshless.join("; ")
                ),
            ));
        }
        let mut unjointed = BTreeSet::new();
        for (mesh, rig) in library.meshes.iter().zip(&library.rigs) {
            for node in &rig.nodes {
                for (locator, (_, joint)) in &node.locators {
                    if let Some(joint) = joint
                        && node.mesh.joint(joint).is_none()
                    {
                        unjointed.insert(format!("{} {locator} ({joint})", mesh.name));
                    }
                }
            }
        }
        if !unjointed.is_empty() {
            warnings.push((content_reader_parser::DiagnosticKind::Warning, format!(
                "These entity locators name a parent_joint the mesh lacks and stay in entity space, as the engine keeps them: {}.",
                unjointed.into_iter().collect::<Vec<_>>().join(", ")
            )));
        }
        Ok(library)
    }

    pub fn load_recorded(
        layers: &ResolvedLayers,
        decode: Decode,
        names: &[String],
        why: impl Fn(&str) -> String,
    ) -> io::Result<Self> {
        Self::load_recorded_profiled(layers, decode, names, why, &mut BuilderSpans::new(false))
    }

    pub fn load_recorded_profiled(
        layers: &ResolvedLayers,
        decode: Decode,
        names: &[String],
        why: impl Fn(&str) -> String,
        spans: &mut BuilderSpans,
    ) -> io::Result<Self> {
        let (Some(recorded), Some(roster)) = (
            layers.without_overlays(),
            layers.recorded_descriptor_roster()?,
        ) else {
            return Ok(Self::empty(decode));
        };
        let baked = Self::load_with_spans(
            &recorded,
            layers,
            decode,
            (&roster.descriptors).into(),
            names,
            &mut Vec::new(),
            |_, _| None,
            spans,
            BakeLabels::RECORDED,
        )?;
        for name in names.iter().filter(|name| baked.slot(name).is_some()) {
            layers.note_overlay_refusal(format!("model {name} ({})", why(name)));
        }
        Ok(baked.isolated(false))
    }

    pub fn has_state(&self, slot: usize, state: &str) -> bool {
        self.rigs
            .get(slot)
            .and_then(|rig| rig.nodes.first())
            .is_some_and(|node| node.clips.iter().any(|clip| clip.state.name == state))
    }

    pub fn slot(&self, name: &str) -> Option<usize> {
        self.index.get(name).copied()
    }

    pub fn has_geometry(&self, slot: usize) -> bool {
        self.meshes[slot]
            .native
            .iter()
            .any(|descriptor| descriptor.lods.iter().any(|lod| !lod.is_empty()))
    }

    pub fn native_draw_snapshot(&self, slot: usize, texture_identities: &[String]) -> Vec<u8> {
        draw_snapshot::snapshot(self, slot, texture_identities)
    }

    pub fn merge(&mut self, mut shard: ModelLibrary) {
        if self.layers.is_none() {
            self.layers = shard.layers.clone();
            self.decode = shard.decode;
        }
        let mut texture_keys: Vec<Option<&(String, ColorSpace)>> = vec![None; shard.textures.len()];
        for (key, slot) in &shard.paths {
            texture_keys[*slot] = Some(key);
        }
        let textures = std::mem::take(&mut shard.textures);
        let mut texture_map = Vec::with_capacity(textures.len());
        for (slot, texture) in textures.into_iter().enumerate() {
            let key = texture_keys[slot].expect("every texture has its path key");
            texture_map.push(match self.paths.get(key) {
                Some(&global) => global,
                None => {
                    let global = self.textures.len();
                    self.textures.push(texture);
                    self.spaces.push(shard.spaces[slot]);
                    self.paths.insert(key.clone(), global);
                    global
                }
            });
        }
        let mut material_map = Vec::with_capacity(shard.materials.len());
        for material in &shard.materials {
            let mut material = material.clone();
            material.diffuse = texture_map[material.diffuse];
            material.normal = texture_map[material.normal];
            material.specular = texture_map[material.specular];
            material_map.push(match self.materials.iter().position(|m| *m == material) {
                Some(slot) => slot,
                None => {
                    self.materials.push(material);
                    self.materials.len() - 1
                }
            });
        }
        for (path, animation) in shard.animations {
            self.animations.entry(path).or_insert(animation);
        }
        for (path, why) in shard.animation_refusals {
            self.animation_refusals.entry(path).or_insert(why);
        }
        for (reason, count) in shard.attachment_refusals {
            *self.attachment_refusals.entry(reason).or_default() += count;
        }
        self.failed.extend(shard.failed);
        let palette_base = self.palette.len();
        self.palette.extend(shard.palette);
        let mesh_base = self.meshes.len();
        for (from, to) in &shard.alternatives {
            self.alternatives.insert(mesh_base + from, mesh_base + to);
        }
        let counterparts: BTreeSet<usize> = shard.alternatives.values().copied().collect();
        assert_eq!(shard.meshes.len(), shard.rigs.len(), "one rig per mesh");
        for (shard_slot, (mut mesh, mut rig)) in
            shard.meshes.into_iter().zip(shard.rigs).enumerate()
        {
            {
                let rig = Arc::get_mut(&mut rig).expect("shard-local rig");
                rig.start += palette_base;
                rig.end += palette_base;
                for node in &mut rig.nodes {
                    node.rigid += palette_base as u32;
                    if let Some(skin_base) = node.skin_base.as_mut() {
                        *skin_base += palette_base as u32;
                    }
                }
            }
            for descriptor in &mut mesh.native {
                for part in descriptor.lods.iter_mut().flatten() {
                    part.material_index = material_map[part.material_index];
                    part.geometry = self
                        .geometry_sources
                        .entry(part.geometry_key.clone())
                        .or_insert_with(|| Arc::clone(&part.geometry))
                        .clone();
                }
            }
            let slot = self.meshes.len();
            if shard.alternative {
                if let Some(&original) = self.index.get(&mesh.name) {
                    self.alternatives.insert(original, slot);
                }
            } else if !counterparts.contains(&shard_slot) {
                self.index.insert(mesh.name.clone(), slot);
            }
            self.meshes.push(mesh);
            self.rigs.push(rig);
        }
    }

    pub fn isolated(mut self, alternative: bool) -> Self {
        let apart = |path: &str| format!("recorded:{path}");
        self.paths = std::mem::take(&mut self.paths)
            .into_iter()
            .map(|((path, space), slot)| ((apart(&path), space), slot))
            .collect();
        self.animations = std::mem::take(&mut self.animations)
            .into_iter()
            .map(|(path, animation)| (apart(&path), animation))
            .collect();
        self.geometry_sources = std::mem::take(&mut self.geometry_sources)
            .into_iter()
            .map(|(mut key, geometry)| {
                key.0 = apart(&key.0);
                (key, geometry)
            })
            .collect();
        for mesh in &mut self.meshes {
            mesh.asset.recorded(alternative);
            for descriptor in &mut mesh.native {
                for part in descriptor.lods.iter_mut().flatten() {
                    part.geometry_key.0 = apart(&part.geometry_key.0);
                }
            }
        }
        self.alternative = alternative;
        self
    }

    fn add<'a>(
        &mut self,
        layers: &ResolvedLayers,
        descriptors: impl Into<DescriptorView<'a>>,
        name: &str,
    ) -> io::Result<()> {
        let marks = (self.palette.len(), self.materials.len());
        let mut geometry_added = Vec::new();
        let added = self.compose_entity(layers, descriptors.into(), name, &mut geometry_added);
        if added.is_err() {
            self.palette.truncate(marks.0);
            self.materials.truncate(marks.1);
            for key in geometry_added {
                self.geometry_sources.remove(&key);
            }
        }
        added
    }

    fn compose_entity(
        &mut self,
        layers: &ResolvedLayers,
        descriptors: DescriptorView<'_>,
        name: &str,
        geometry_added: &mut Vec<native::GeometryKey>,
    ) -> io::Result<()> {
        if self.layers.is_none() {
            self.layers = Some(Arc::new(layers.clone()));
        }
        self.composing_visual = false;
        let mut out = Composition::default();
        out.rig.start = self.palette.len();
        let (path, scale, lod_distances) = self.compose(
            layers,
            descriptors,
            name,
            mesh::Transform::IDENTITY,
            0,
            None,
            &mut out,
            geometry_added,
            &BTreeSet::new(),
        )?;
        out.rig.end = self.palette.len();
        let Composition {
            native,
            reach,
            rig,
            refusals,
        } = out;
        if !path.is_empty() && native.iter().all(|d| d.lods.iter().all(Vec::is_empty)) {
            return Err(invalid(format!("{path} has no drawable sub-mesh")));
        }
        let (radius, height) = (reach.radius, reach.height);
        self.rigs.push(Arc::new(rig));
        for (reason, count) in refusals {
            *self.attachment_refusals.entry(reason).or_default() += count;
        }
        self.index.insert(name.to_owned(), self.meshes.len());
        let namespace = self
            .asset_namespace
            .get_or_insert_with(|| identity::namespace(layers));
        let cull_distance_squared = descriptors
            .resolve_entity(name)
            .map_or(mesh::DEFAULT_CULL_DISTANCE_SQUARED, |entity| entity.definition.cull_distance_squared);
        self.meshes.push(ModelMesh {
            asset: AssetKey::selected(name, Arc::clone(namespace), self.decode),
            name: name.to_owned(),
            path,
            scale,
            lod_distances,
            native,
            radius,
            height,
            visual: self.composing_visual,
            own_radius: reach.own_radius,
            bound: reach.bound,
            convoy_radius: reach.collision_radius(),
            attachments: reach.attachments,
            cull_distance_squared,
        });
        Ok(())
    }

    #[expect(clippy::too_many_arguments, reason = "the layers and descriptors, the part's name, placement and parent, and the composition it fills")]
    fn compose(
        &mut self,
        layers: &ResolvedLayers,
        descriptors: DescriptorView<'_>,
        name: &str,
        placement: mesh::Transform,
        depth: usize,
        parent: Option<(usize, String, String)>,
        out: &mut Composition,
        geometry_added: &mut Vec<native::GeometryKey>,
        ancestors: &BTreeSet<String>,
    ) -> io::Result<(String, f32, Vec<f32>)> {
        let mut ancestors = ancestors.clone();
        if !ancestors.insert(name.to_owned()) {
            return Err(invalid(format!("attachment cycle through {name}")));
        }
        if descriptors
            .entity(name)
            .is_some_and(|entity| entity.clone.is_some())
        {
            return Err(invalid(format!(
                "entity {name}: raw clone descriptors lack the native registration snapshot; load the registered content Roster"
            )));
        }
        let inherited = descriptors.inherit(name);
        if let Some(inherited) = &inherited {
            for source in &inherited.sources {
                self.composing_visual |= layers.supplied_by_visual_mod(source);
            }
        }
        let entity = descriptors.resolve_entity(name);
        if entity.is_none() && inherited.is_none() {
            return Err(invalid("no pdxmesh or entity declares it"));
        }
        let (path, decoded, native_skeleton, binary_locators) = if let Some(entity) = &entity {
            let path = entity.definition.file.clone();
            let count: usize = inherited
                .as_ref()
                .into_iter()
                .flat_map(|entity| entity.states.values())
                .flatten()
                .map(|state| map_effects(state))
                .sum();
            let cannot_load = |error| {
                invalid(format!(
                    "{name}: pdxmesh {} ({path}) cannot be loaded: {error}; {count} map effects omitted",
                    entity.definition.name
                ))
            };
            self.composing_visual |= layers.supplied_by_visual_mod(&path);
            let tree = mesh::parse_in(layers, &path).map_err(cannot_load)?;
            let skeleton =
                mesh::native_skeleton(&tree, entity.definition.scale).map_err(cannot_load)?;
            let locators =
                crate::mesh_pose::binary_locators(&tree, &skeleton, entity.definition.scale)
                    .map_err(cannot_load)?;
            let decoded = mesh::read_tree(&tree).map_err(cannot_load)?;
            (path, decoded, skeleton, locators)
        } else {
            (String::new(), mesh::Mesh::default(), Vec::new(), Vec::new())
        };
        let scale = entity.as_ref().map_or_else(
            || inherited.as_ref().and_then(|e| e.scale).unwrap_or(1.),
            |entity| entity.scale,
        );
        let mesh_scale = entity.as_ref().map_or(1., |entity| entity.definition.scale);
        let directory = path.rsplit_once('/').map_or("", |(head, _)| head);
        let own = placement.compose(&mesh::Transform {
            basis: [[scale, 0.0, 0.0], [0.0, scale, 0.0], [0.0, 0.0, scale]],
            translation: [0.0; 3],
        });
        let animation = entity
            .as_ref()
            .and_then(|entity| entity.animation)
            .map(|animation| animation.file.clone())
            .and_then(|path| self.animation(layers, &path).cloned());
        let mut skin_base = self.push_palette(&decoded, animation.as_deref(), &own);
        let frozen_skin = skin_base.is_some();
        let rigid = self.palette.len() as u32;
        self.palette.push(PaletteJoint::from_transform(&own));
        let mut clips = Vec::new();
        if let Some(inherited) = &inherited {
            for variants in inherited.states.values() {
                for state in variants {
                    let path = entity.as_ref().and_then(|entity| {
                        entity
                            .definition
                            .animations
                            .iter()
                            .find(|(id, _)| id == &state.animation)
                            .and_then(|(_, key)| descriptors.animations.get(key))
                            .map(|a| a.file.clone())
                    });
                    let animation = path.and_then(|path| self.animation(layers, &path).cloned());
                    clips.push(entity_animation::Clip {
                        state: (*state).clone(),
                        animation,
                    });
                }
            }
        }
        if skin_base.is_none() {
            skin_base = clips
                .iter()
                .find_map(|clip| self.push_palette(&decoded, clip.animation.as_deref(), &own));
        }
        let node_index = out.rig.nodes.len();
        out.rig.nodes.push(entity_animation::Node {
            mesh: Arc::new(skeleton_source::skeleton(&decoded)),
            native: Some(Arc::new(crate::mesh_pose::Source {
                pdxmesh: entity.is_some(),
                entity_locators: descriptors.entity(name).map_or_else(
                    Vec::new,
                    |definition| {
                        crate::mesh_pose::entity_locators(
                            &definition.locators,
                            &native_skeleton,
                            entity
                                .as_ref()
                                .map(|entity| entity.definition.name.as_str()),
                        )
                    },
                ),
                binary_locators,
                skeleton: native_skeleton,
                mesh_scale,
                entity_scale: inherited.as_ref().and_then(|e| e.scale).unwrap_or(1.),
                frozen: animation.clone(),
                game_data: inherited.as_ref().and_then(|e| e.game_data).cloned(),
            })),
            clips,
            default: inherited
                .as_ref()
                .and_then(|e| e.default_state)
                .unwrap_or("idle")
                .to_owned(),
            follows_parent: inherited
                .as_ref()
                .and_then(|e| e.get_state_from_parent)
                .unwrap_or(true),
            parent,
            scale,
            mesh_scale,
            skin_base,
            rigid,
            locators: inherited
                .as_ref()
                .map(|e| {
                    e.locators
                        .iter()
                        .map(|(name, locator)| {
                            (
                                (*name).to_owned(),
                                (
                                    entity_locator(&locator.transform, mesh_scale),
                                    locator.parent_joint.clone(),
                                ),
                            )
                        })
                        .collect()
                })
                .unwrap_or_default(),
        });
        if let Some(entity) = &entity {
            let resolved = mesh::ResolvedMesh {
                definition: entity.definition,
                scale,
            };
            let mut geometry_sources = std::mem::take(&mut self.geometry_sources);
            let source_layers = Arc::clone(self.layers.as_ref().expect("layers set before composing"));
            let descriptor = native::descriptor(
                &decoded,
                &resolved,
                node_index,
                name,
                |key, sub| {
                    if let Some(geometry) = geometry_sources.get(&key) {
                        return Ok(Arc::clone(geometry));
                    }
                    let geometry = native::installed_geometry(
                        Arc::clone(&source_layers),
                        key.clone(),
                        sub,
                    )?;
                    geometry_sources.insert(key.clone(), Arc::clone(&geometry));
                    geometry_added.push(key);
                    Ok(geometry)
                },
                |shape, sub| self.material(layers, directory, shape, sub, &resolved),
            );
            self.geometry_sources = geometry_sources;
            out.native.push(descriptor?);
            let shape_lods = decoded.lods();
            out.reach.collision_unsupported |= !frozen_skin || shape_lods.len() != 1;
            for (lod, shapes) in shape_lods.iter().enumerate() {
                for (_, sub) in shapes
                    .iter()
                    .flat_map(|shape| shape.meshes.iter().map(|sub| (shape.name.as_str(), sub)))
                    .filter(|(_, sub)| viewer_scene_base::mesh_effect::candidate(sub))
                {
                    if lod == 0 {
                        let mut bone_bounds: BTreeMap<i32, ([f32; 3], [f32; 3])> =
                            BTreeMap::new();
                        for (vertex, &position) in sub.positions.iter().enumerate() {
                            let joints: Vec<i32> = sub
                                .skin
                                .as_ref()
                                .map(|skin| {
                                    skin.influences(vertex)
                                        .into_iter()
                                        .map(|(joint, _)| joint as i32)
                                        .collect()
                                })
                                .unwrap_or_default();
                            for joint in if joints.is_empty() { vec![-1] } else { joints } {
                                let (low, high) =
                                    bone_bounds.entry(joint).or_insert((position, position));
                                for axis in 0..3 {
                                    low[axis] = low[axis].min(position[axis]);
                                    high[axis] = high[axis].max(position[axis]);
                                }
                            }
                        }
                        for (joint, (low, high)) in bone_bounds {
                            for corner in 0..8 {
                                let p = [0, 1, 2].map(|axis| {
                                    if corner & (1 << axis) == 0 {
                                        low[axis]
                                    } else {
                                        high[axis]
                                    }
                                });
                                let p = if joint >= 0
                                    && let Some(base) = skin_base
                                {
                                    skinned(&self.palette, p, &[(joint as u32, 1.0)], base)
                                } else {
                                    own.transform_point(p)
                                };
                                out.reach.collision_point(p);
                            }
                        }
                    }
                    let mut posed = false;
                    if frozen_skin && let (Some(base), Some(skin)) = (skin_base, &sub.skin) {
                        for vertex in 0..sub.vertex_count() {
                            let influences = skin.influences(vertex);
                            if !influences.is_empty() {
                                posed = true;
                                out.reach.widen_point(
                                    skinned(
                                        &self.palette,
                                        sub.positions[vertex],
                                        &influences,
                                        base,
                                    ),
                                    depth == 0,
                                );
                            }
                        }
                    }
                    if !posed && let Some(bounds) = &sub.bounds {
                        out.reach.widen(&own, bounds, depth == 0);
                    }
                }
            }
        }
        {
            let attachments: Vec<(String, String, String)> = inherited
                .as_ref()
                .into_iter()
                .flat_map(|entity| &entity.attachments)
                .map(|attachment| {
                    (
                        attachment.joint.clone(),
                        attachment.entity.clone(),
                        attachment.handle().to_owned(),
                    )
                })
                .collect();
            let actors = if attachments.is_empty() {
                Arc::from([])
            } else {
                crate::mesh_pose::actors(&out.rig, None, true)
            };
            for (joint_name, child, handle) in attachments {
                if !descriptors.has_entity(&child) {
                    *out.refusals.entry(format!("{name} -> {child}: attachment child has no registered entity definition")).or_default() += 1;
                    continue;
                }
                let source = out.rig.nodes[node_index].native.as_ref().unwrap();
                let mode = if descriptors.resolve_entity(&child).is_some() {
                    crate::mesh_pose::LookupMode::Attachment
                } else {
                    crate::mesh_pose::LookupMode::AttachmentDummy
                };
                let local = match crate::mesh_pose::lookup(source, &joint_name, mode) {
                    Some(binding) => match binding.local(
                        actors[node_index]
                            .pose
                            .as_ref()
                            .map(|pose| Some(pose.joints.as_slice()))
                            .map_err(Clone::clone),
                    ) {
                        Ok(local) => local,
                        Err(reason) => {
                            *out.refusals
                                .entry(format!("{name} -> {child}: {reason}"))
                                .or_default() += 1;
                            continue;
                        }
                    },
                    None => {
                        if mode == crate::mesh_pose::LookupMode::Attachment {
                            *out.refusals
                            .entry(format!(
                                "{name} -> {child}: missing attachment point {joint_name}; native parent-root fallback"
                            ))
                            .or_default() += 1;
                        }
                        crate::mesh_pose::IDENTITY
                    }
                };
                let world = match &actors[node_index].world {
                    Ok(world) => crate::mesh_pose::multiply(*world, local),
                    Err(reason) => {
                        *out.refusals
                            .entry(format!("{name} -> {child}: {reason}"))
                            .or_default() += 1;
                        continue;
                    }
                };
                let placement = mesh::Transform {
                    basis: [
                        world[0][..3].try_into().unwrap(),
                        world[1][..3].try_into().unwrap(),
                        world[2][..3].try_into().unwrap(),
                    ],
                    translation: world[3][..3].try_into().unwrap(),
                };

                let mark = (
                    self.palette.len(),
                    self.materials.len(),
                    out.native.len(),
                    out.rig.nodes.len(),
                    geometry_added.len(),
                    out.reach,
                );
                match self.compose(
                    layers,
                    descriptors,
                    &child,
                    placement,
                    depth + 1,
                    Some((node_index, joint_name, handle)),
                    out,
                    geometry_added,
                    &ancestors,
                ) {
                    Ok(_) => out.reach.attachments += 1,
                    Err(error) => {
                        self.palette.truncate(mark.0);
                        self.materials.truncate(mark.1);
                        out.native.truncate(mark.2);
                        out.rig.nodes.truncate(mark.3);
                        for key in geometry_added.drain(mark.4..) {
                            self.geometry_sources.remove(&key);
                        }
                        out.reach = mark.5;
                        *out.refusals
                            .entry(format!("{name} -> {child}: {error}"))
                            .or_default() += 1;
                    }
                }
            }
        }
        Ok((path, scale, decoded.lod_distances.clone()))
    }

    fn push_palette(
        &mut self,
        decoded: &mesh::Mesh,
        animation: Option<&mesh::Animation>,
        own: &mesh::Transform,
    ) -> Option<u32> {
        let animation = animation?;
        if decoded.skeleton.is_empty() {
            return None;
        }
        let base = u32::try_from(self.palette.len()).ok()?;
        self.palette.extend(
            decoded
                .palette(animation, mesh::FIRST_FRAME)
                .iter()
                .map(|joint| PaletteJoint::from_transform(&own.compose(joint))),
        );
        Some(base)
    }

    fn animation(&mut self, layers: &ResolvedLayers, path: &str) -> Option<&Arc<mesh::Animation>> {
        self.composing_visual |= layers.supplied_by_visual_mod(path);
        if !self.animations.contains_key(path) {
            let read = |stack: &ResolvedLayers| {
                mesh::read_animation_in(stack, path).map_err(|error| error.to_string())
            };
            let decoded = match read(layers) {
                Ok(animation) => Some(animation),
                Err(why) => {
                    match layers.without_overlays().map(|recorded| read(&recorded)) {
                        Some(Ok(animation)) => {
                            layers.note_overlay_refusal(format!("animation {path} ({why})"));
                            Some(animation)
                        }
                        _ => {
                            layers.note_refused(format!("model animation {path}"));
                            self.animation_refusals.insert(path.to_owned(), why);
                            None
                        }
                    }
                }
            };
            self.animations.insert(path.to_owned(), decoded);
        }
        self.animations.get(path)?.as_ref()
    }

    fn material(
        &mut self,
        layers: &ResolvedLayers,
        directory: &str,
        shape: &str,
        sub: &mesh::SubMesh,
        resolved: &mesh::ResolvedMesh<'_>,
    ) -> io::Result<usize> {
        let material = resolved.definition.material(shape, sub);
        let entry = ModelMaterial {
            diffuse: self.texture(layers, directory, &material.diffuse, ColorSpace::Srgb)?,
            normal: self.texture(layers, directory, &material.normal, ColorSpace::Linear)?,
            specular: self.texture(layers, directory, &material.specular, ColorSpace::Linear)?,
        };
        if let Some(slot) = self.materials.iter().position(|other| *other == entry) {
            return Ok(slot);
        }
        self.materials.push(entry);
        Ok(self.materials.len() - 1)
    }

    fn texture(
        &mut self,
        layers: &ResolvedLayers,
        directory: &str,
        file: &str,
        space: ColorSpace,
    ) -> io::Result<usize> {
        if file.is_empty() {
            return Ok(self.blank(space));
        }
        let path = layers.texture_index()?.resolve(directory, file)?;
        let key = (path, space);
        if let Some(slot) = self.paths.get(&key) {
            self.composing_visual |= self.textures[*slot].is_visual();
            return Ok(*slot);
        }
        let (_, layer) = locate_file(&key.0, &|path| layers.locate_uncovered_path(path))?;
        let visual = layers.is_overlay_layer(layer);
        self.composing_visual |= visual;
        let stack = Arc::clone(
            self.layers
                .get_or_insert_with(|| Arc::new(layers.clone())),
        );
        self.textures
            .push(ModelTexture::deferred(key.0.clone(), stack, visual));
        self.spaces.push(space);
        let slot = self.textures.len() - 1;
        self.paths.insert(key, slot);
        Ok(slot)
    }

    fn blank(&mut self, space: ColorSpace) -> usize {
        let key = (String::new(), space);
        if let Some(slot) = self.paths.get(&key) {
            return *slot;
        }
        self.textures.push(ModelTexture::held(dds::Texture::from_rgba8(Rgba8Image {
            width: 1,
            height: 1,
            pixels: vec![0, 0, 0, 0],
        })));
        self.spaces.push(space);
        let slot = self.textures.len() - 1;
        self.paths.insert(key, slot);
        slot
    }

    pub fn device_textures(&self, device: &wgpu::Device) -> Arc<DeviceTextures> {
        self.uploads.on(device)
    }

    #[cfg(test)]
    fn gpu_texture(
        &self,
        shared: &DeviceTextures,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        label: &str,
        slot: usize,
    ) -> io::Result<Arc<Bound>> {
        shared.texture((&self.textures[slot], self.spaces[slot], self.decode), device, queue, label, slot)
    }
}

fn drawn_sources(library: &ModelLibrary) -> Vec<(ModelTexture, ColorSpace)> {
    library
        .textures
        .iter()
        .zip(&library.spaces)
        .map(|(texture, space)| (texture.clone(), *space))
        .collect()
}

fn entity_locator(locator: &mesh::Transform, mesh_scale: f32) -> mesh::Transform {
    mesh::Transform::from_trs(
        [0.0; 3],
        [0.0, 0.0, 0.0, 1.0],
        1.0 / mesh_scale.max(f32::EPSILON),
    )
    .compose(locator)
}

fn skinned(
    palette: &[PaletteJoint],
    position: [f32; 3],
    influences: &[(u32, f32)],
    base: u32,
) -> [f32; 3] {
    let mut out = [0.0f32; 3];
    let mut any = false;
    for &(joint, weight) in influences {
        if weight == 0.0 {
            continue;
        }
        let Some(joint) = palette.get((base + joint) as usize) else {
            continue;
        };
        any = true;
        for (axis, row) in joint.rows.iter().enumerate() {
            out[axis] += weight
                * (row[0] * position[0] + row[1] * position[1] + row[2] * position[2] + row[3]);
        }
    }
    if any { out } else { position }
}


#[derive(Clone, Debug, PartialEq)]
pub struct ModelGroup {
    pub mesh: usize,
    pub asset: AssetKey,
    pub range: Range<u32>,
    pub bounds: [f32; 4],
    pub centroid: [f32; 3],
}

pub type KeyedInstance = ((usize, i32, i32), ModelInstance, usize);

#[derive(Clone, Debug, Default)]
pub struct ModelField {
    pub instances: Vec<ModelInstance>,
    pub groups: Vec<ModelGroup>,
    pub native_order: Vec<(usize, usize)>,
    tile: Option<f32>,
    bindings: BTreeMap<usize, AssetKey>,
    producer_order: Vec<(ClockKey, usize)>,
    producer_scratch: Vec<(ClockKey, usize)>,
    clock_priorities: BTreeMap<ClockKey, usize>,
    clock_sources: Vec<ClockKey>,
    ordered_clocks: Vec<(ClockKey, usize, Option<AssetKey>)>,
    group_work: Vec<Option<usize>>,
    #[cfg(test)]
    clock_order_builds: usize,
    #[cfg(test)]
    producer_sorts: usize,
    pub clocks: BTreeMap<(usize, u32, u32), entity_animation::Clock>,
    pub default_states: BTreeMap<(usize, u32, u32), u32>,
}

type ClockKey = (usize, u32, u32);

fn source_order<'a>(
    native_order: &'a [(usize, usize)],
    groups: &'a [ModelGroup],
) -> impl Iterator<Item = (usize, usize)> + 'a {
    native_order.iter().copied().chain(
        groups
            .iter()
            .enumerate()
            .take(if native_order.is_empty() {
                groups.len()
            } else {
                0
            })
            .flat_map(|(group, entry)| {
                entry
                    .range
                    .clone()
                    .map(move |index| (group, index as usize))
            }),
    )
}

impl PartialEq for ModelField {
    fn eq(&self, other: &Self) -> bool {
        self.instances == other.instances
            && self.groups == other.groups
            && self.native_order == other.native_order
            && self.tile == other.tile
            && self.clocks == other.clocks
            && self.default_states == other.default_states
            && self.producer_order == other.producer_order
    }
}

impl ModelField {
    fn source_order(&self) -> impl Iterator<Item = (usize, usize)> + '_ {
        source_order(&self.native_order, &self.groups)
    }

    fn producer_ordinal(&self, key: ClockKey) -> usize {
        self.producer_order
            .binary_search_by_key(&key, |entry| entry.0)
            .map(|at| self.producer_order[at].1)
            .unwrap_or(usize::MAX)
    }

    fn prepare_clock_order(&mut self) {
        let phase_keys = || {
            self.clocks
                .iter()
                .filter_map(|(key, clock)| clock.phase.is_some().then_some(*key))
        };
        if phase_keys().eq(self.clock_sources.iter().copied())
            && self.ordered_clocks.iter().all(|(key, ordinal, asset)| {
                *ordinal
                    == self
                        .clock_priorities
                        .get(key)
                        .copied()
                        .unwrap_or_else(|| self.producer_ordinal(*key))
                    && asset.as_ref() == self.bindings.get(&key.0)
            })
        {
            return;
        }
        self.clock_sources.clear();
        self.clock_sources.extend(
            self.clocks
                .iter()
                .filter_map(|(key, clock)| clock.phase.is_some().then_some(*key)),
        );
        self.ordered_clocks.clear();
        for &key in &self.clock_sources {
            let ordinal = self
                .clock_priorities
                .get(&key)
                .copied()
                .unwrap_or_else(|| self.producer_ordinal(key));
            self.ordered_clocks
                .push((key, ordinal, self.bindings.get(&key.0).cloned()));
        }
        self.ordered_clocks.sort_unstable_by(|a, b| {
            (a.1, &a.2, a.0.1, a.0.2, a.0.0).cmp(&(b.1, &b.2, b.0.1, b.0.2, b.0.0))
        });
        #[cfg(test)]
        {
            self.clock_order_builds += 1;
        }
    }

    fn clock_order(&self) -> impl Iterator<Item = ClockKey> + '_ {
        self.ordered_clocks.iter().map(|entry| entry.0)
    }

    pub fn build(library: &ModelLibrary, placed: Vec<(usize, ModelInstance)>, tile: f32) -> Self {
        Self::build_meshes(&library.meshes, placed, tile)
    }

    pub fn build_meshes(
        meshes: &[ModelMesh],
        mut placed: Vec<(usize, ModelInstance)>,
        tile: f32,
    ) -> Self {
        let mut field = Self::default();
        let mut keyed = Vec::with_capacity(placed.len());
        field.rebuild_meshes(meshes, &mut placed, &mut keyed, tile);
        field
    }

    pub fn rebuild(
        &mut self,
        library: &ModelLibrary,
        placed: &mut Vec<(usize, ModelInstance)>,
        keyed: &mut Vec<KeyedInstance>,
        tile: f32,
    ) {
        self.rebuild_meshes(&library.meshes, placed, keyed, tile);
    }

    pub fn rebuild_meshes(
        &mut self,
        meshes: &[ModelMesh],
        placed: &mut Vec<(usize, ModelInstance)>,
        keyed: &mut Vec<KeyedInstance>,
        tile: f32,
    ) {
        self.rebuild_meshes_inner(meshes, placed, keyed, tile, false);
    }

    fn rebuild_meshes_inner(
        &mut self,
        meshes: &[ModelMesh],
        placed: &mut Vec<(usize, ModelInstance)>,
        keyed: &mut Vec<KeyedInstance>,
        tile: f32,
        regroup: bool,
    ) {
        keyed.clear();
        keyed.extend(
            placed
                .drain(..)
                .enumerate()
                .map(|(ordinal, (mesh, instance))| {
                    (
                        (
                            mesh,
                            (instance.position[0] / tile).floor() as i32,
                            (instance.position[2] / tile).floor() as i32,
                        ),
                        instance,
                        ordinal,
                    )
                }),
        );
        let mut ranked: Vec<usize> = keyed
            .iter()
            .map(|((mesh, _, _), _, _)| *mesh)
            .filter(|&mesh| mesh < meshes.len())
            .collect();
        ranked.sort_unstable();
        ranked.dedup();
        ranked.sort_by(|&a, &b| meshes[a].asset.cmp(&meshes[b].asset));
        let mut ranks = vec![0u32; meshes.len()];
        let mut rank = 0;
        for (at, &mesh) in ranked.iter().enumerate() {
            if at == 0 || meshes[ranked[at - 1]].asset.cmp(&meshes[mesh].asset).is_ne() {
                rank += 1;
            }
            ranks[mesh] = rank;
        }
        keyed.sort_unstable_by_key(|&((mesh, x, z), _, ordinal)| match ranks.get(mesh) {
            Some(&rank) => (rank, 0, x, z, ordinal),
            None => (0, mesh, x, z, ordinal),
        });
        let field = self;
        field.instances.clear();
        field.groups.clear();
        field.tile = Some(tile);
        field.clocks.clear();
        let mut bindings_changed = false;
        let mut producers_changed = false;
        let mut first_ordinals = 0;
        let mut previous_mesh = None;
        for &((mesh, _, _), instance, ordinal) in keyed.iter() {
            if previous_mesh != Some(mesh) {
                if let Some(asset) = meshes.get(mesh) {
                    let binding = field.bindings.entry(mesh).or_default();
                    if *binding != asset.asset {
                        *binding = asset.asset.clone();
                        bindings_changed = true;
                    }
                }
                previous_mesh = Some(mesh);
            }
            if !regroup {
                let (seed, identity) = instance.animation_key();
                let key = (mesh, seed, identity);
                match field
                    .producer_order
                    .binary_search_by_key(&key, |entry| entry.0)
                {
                    Ok(at) => {
                        let first = field.producer_order[at].1;
                        first_ordinals += usize::from(first == ordinal);
                        producers_changed |= ordinal < first;
                    }
                    Err(_) => producers_changed = true,
                }
            }
        }
        producers_changed |= !regroup && first_ordinals != field.producer_order.len();
        if producers_changed {
            field.producer_scratch.clear();
            field.producer_scratch.extend(keyed.iter().map(
                |&((mesh, _, _), instance, ordinal)| {
                    let (seed, identity) = instance.animation_key();
                    ((mesh, seed, identity), ordinal)
                },
            ));
            field.producer_scratch.sort_unstable();
            field.producer_scratch.dedup_by_key(|entry| entry.0);
            std::mem::swap(&mut field.producer_order, &mut field.producer_scratch);
            #[cfg(test)]
            {
                field.producer_sorts += 1;
            }
        }
        if bindings_changed || producers_changed {
            field.clock_priorities.clear();
        }
        field.native_order.resize(keyed.len(), (0, 0));
        let mut cursor = 0;
        while cursor < keyed.len() {
            let (mesh, _, _) = keyed[cursor].0;
            let start = cursor;
            while cursor < keyed.len() {
                let (next, x, z) = keyed[cursor].0;
                let (_, first_x, first_z) = keyed[start].0;
                if (
                    meshes.get(next).map(|mesh| &mesh.asset),
                    meshes.get(next).is_none().then_some(next),
                    x,
                    z,
                ) != (
                    meshes.get(mesh).map(|mesh| &mesh.asset),
                    meshes.get(mesh).is_none().then_some(mesh),
                    first_x,
                    first_z,
                ) {
                    break;
                }
                cursor += 1;
            }
            let radius = meshes.get(mesh).map_or(0.0, |entry| entry.radius);
            let instances_start = field.instances.len() as u32;
            let bounds = reach_bounds(radius, keyed[start..cursor].iter().map(|entry| &entry.1));
            let mut sum = [0.0f64; 3];
            for ((_, _, _), instance, ordinal) in &keyed[start..cursor] {
                field.native_order[*ordinal] = (field.groups.len(), field.instances.len());
                for (axis, total) in sum.iter_mut().enumerate() {
                    *total += f64::from(instance.position[axis]);
                }
                field.instances.push(*instance);
            }
            let count = (cursor - start) as f64;
            field.groups.push(ModelGroup {
                mesh,
                asset: meshes
                    .get(mesh)
                    .map(|mesh| mesh.asset.clone())
                    .unwrap_or_default(),
                range: instances_start..field.instances.len() as u32,
                bounds,
                centroid: [
                    (sum[0] / count) as f32,
                    (sum[1] / count) as f32,
                    (sum[2] / count) as f32,
                ],
            });
        }
    }
}

const LEAN: f32 = 0.75;

fn lean(height: f32) -> f32 {
    (height.max(0.0) * LEAN - viewer_scene_base::line_lod::MARGIN).max(0.0)
}

fn reach_bounds<'a>(radius: f32, instances: impl Iterator<Item = &'a ModelInstance>) -> [f32; 4] {
    let mut bounds = [f32::MAX, f32::MAX, f32::MIN, f32::MIN];
    for instance in instances {
        let reach = radius * instance.scale + lean(instance.position[1]);
        bounds[0] = bounds[0].min(instance.position[0] - reach);
        bounds[1] = bounds[1].min(instance.position[2] - reach);
        bounds[2] = bounds[2].max(instance.position[0] + reach);
        bounds[3] = bounds[3].max(instance.position[2] + reach);
    }
    bounds
}

pub fn hash01(index: u32) -> f32 {
    let mut x = index.wrapping_mul(0x9E37_79B1);
    x ^= x >> 16;
    x = x.wrapping_mul(0x85EB_CA6B);
    x ^= x >> 13;
    x = x.wrapping_mul(0xC2B2_AE35);
    x ^= x >> 16;
    (x >> 8) as f32 / 16_777_216.0
}

pub fn hash_rotation(index: u32) -> f32 {
    hash01(index) * TAU
}


#[derive(Clone, Copy)]
pub struct ModelFrame<'a> {
    pub camera: Camera,
    pub offsets: &'a [f32],
    pub visible: Option<[f32; 4]>,
    pub seen: Option<[f32; 4]>,
    pub alpha: f32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GameCull {
    pub eye: [f32; 3],
    pub sides: [[f32; 4]; 4],
    pub x: f32,
    pub pad: f32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Culled {
    Distance,
    Planes,
}

impl GameCull {
    pub fn of(camera: &Camera, viewport: [u32; 2]) -> Self {
        Self::padded(camera, viewport, 0.0)
    }

    pub fn padded(camera: &Camera, viewport: [u32; 2], pad: f32) -> Self {
        let planes = view_cone(camera, viewport);
        Self { eye: camera.eye(), sides: [planes[0], planes[1], planes[2], planes[3]], x: camera.center[0], pad }
    }

    pub fn test(&self, centre: [f32; 3], radius: f32, cull_distance_squared: f32) -> Option<Culled> {
        let [dx, dy, dz] = [centre[0] - self.eye[0], centre[1] - self.eye[1], centre[2] - self.eye[2]];
        let distance = cull_distance_squared.sqrt() + self.pad;
        if dx * dx + dy * dy + dz * dz >= distance * distance {
            return Some(Culled::Distance);
        }
        let reach = radius + self.pad;
        self.sides
            .iter()
            .any(|[a, b, c, d]| a * centre[0] + b * centre[1] + c * centre[2] + d < -reach)
            .then_some(Culled::Planes)
    }
}

pub fn view_cone(camera: &Camera, viewport: [u32; 2]) -> [[f32; 4]; 6] {
    camera.frustum_planes(viewport).map(|[a, b, c, d]| {
        let length = (a * a + b * b + c * c).sqrt().max(f32::MIN_POSITIVE);
        [a / length, b / length, c / length, d / length]
    })
}

type ActorRefusals = BTreeSet<(usize, usize, String)>;
type ActorPoses = Arc<[crate::mesh_pose::Actor]>;

#[derive(Clone, Debug, PartialEq)]
pub struct ClockRecord {
    pub key: (usize, u32, u32),
    pub start: f32,
    pub requests: Arc<Vec<entity_animation::Request>>,
}

impl ClockRecord {
    fn clock(&self, time: f32) -> entity_animation::Clock {
        entity_animation::Clock {
            time: time - self.start,
            phase: None,
            requests: Arc::clone(&self.requests),
        }
    }
}

#[derive(Default)]
struct ClockRoutes {
    effective: bool,
    redirects: BTreeMap<usize, usize>,
    sources: Vec<ClockKey>,
    routes: Vec<ClockRoute>,
    #[cfg(test)]
    builds: usize,
}

struct ClockRoute {
    source: ClockKey,
    target: ClockKey,
    ordinal: usize,
    asset: Option<AssetKey>,
}

struct ClockRecords {
    source: Arc<[ClockRecord]>,
    order: Vec<usize>,
}

#[cfg(test)]
mod clock_record_tests {
    use crate::*;

    #[test]
    fn retained_clocks_replace_exact_keys_requests_and_times_across_400_frames() {
        let mut retained = ModelPass::new(&ModelLibrary::default(), "clock records");
        let mut records: Arc<[ClockRecord]> = Arc::default();
        let mut random = 0xc3a1_851du32;
        let mut next = || {
            random ^= random << 13;
            random ^= random >> 17;
            random ^= random << 5;
            random
        };
        let mut saw = [false; 4];
        for frame in 0..400u32 {
            let roll = next();
            if frame.is_multiple_of(11) {
                let request = |state: String| {
                    Arc::new(vec![entity_animation::Request {
                        time: -0.0,
                        phase_time: None,
                        state,
                    }])
                };
                let mut source = vec![
                    ClockRecord {
                        key: (3, 7, 9),
                        start: 30.0,
                        requests: request("source first, redirect winner".into()),
                    },
                    ClockRecord {
                        key: (1, 7, 9),
                        start: 10.0,
                        requests: request("original key first".into()),
                    },
                    ClockRecord {
                        key: (1, 7, 9),
                        start: 12.0,
                        requests: request("duplicate original key last".into()),
                    },
                ];
                for index in 0..roll % 29 {
                    source.push(ClockRecord {
                        key: ((roll as usize + index as usize) % 5, index % 5, index % 3),
                        start: [0.0, -0.0, 1.0e-30, 1.0e30, frame as f32 / 3.0][index as usize % 5],
                        requests: request(format!("synthetic_{frame}_{index}")),
                    });
                }
                if frame.is_multiple_of(33) {
                    source.clear();
                }
                records = source.into();
            }
            let redirects = match frame / 19 % 3 {
                0 => BTreeMap::new(),
                1 => BTreeMap::from([(1, 2), (3, 2)]),
                _ => BTreeMap::from([(1, 2), (2, 4), (3, 2)]),
            };
            {
                let pass = &mut retained;
                pass.redirects.clone_from(&redirects);
                if frame.is_multiple_of(17) {
                    pass.update_placed(Vec::new(), 256.0);
                }
                if frame.is_multiple_of(23) {
                    let mut field = ModelField::default();
                    pass.replace_field(&ModelLibrary::default(), &mut field);
                }
                if frame.is_multiple_of(7) {
                    pass.set_clocks(BTreeMap::new());
                    assert!(pass.clock_records.is_none());
                } else if frame.is_multiple_of(5) {
                    pass.field.clocks.insert(
                        (99, 0, 0),
                        entity_animation::Clock {
                            time: -9.0,
                            phase: Some(entity_animation::Phase {
                                time: 17.0,
                                root_advances_hidden: true,
                                instance: ModelInstance::default(),
                            }),
                            requests: Arc::default(),
                        },
                    );
                }
            }
            let time = [0.0, -0.0, 1.0e-30, 1.0e30, frame as f32 * 0.125][roll as usize % 5];
            retained.set_clock_records(&records, time);
            let originals: BTreeMap<_, _> =
                records.iter().map(|record| (record.key, record)).collect();
            let expected: BTreeMap<_, _> = originals
                .into_iter()
                .map(|((mesh, seed, identity), record)| {
                    (
                        (
                            redirects.get(&mesh).copied().unwrap_or(mesh),
                            seed,
                            identity,
                        ),
                        record,
                    )
                })
                .collect();
            assert_eq!(
                retained.field.clocks.keys().collect::<Vec<_>>(),
                expected.keys().collect::<Vec<_>>()
            );
            for (key, record) in expected {
                let clock = &retained.field.clocks[&key];
                assert_eq!(
                    clock.time.to_bits(),
                    (time - record.start).to_bits(),
                    "frame {frame} key={key:?}"
                );
                assert!(clock.phase.is_none());
                assert!(Arc::ptr_eq(&clock.requests, &record.requests));
            }
            assert!(retained.players.is_empty() && retained.events.is_empty());
            if redirects.is_empty() {
                let addresses = || {
                    retained
                        .field
                        .clocks
                        .values()
                        .map(|clock| std::ptr::from_ref(clock) as usize)
                        .collect::<Vec<_>>()
                };
                let before = addresses();
                retained.set_clock_records(&records, time);
                let after: Vec<_> = retained
                    .field
                    .clocks
                    .values()
                    .map(|clock| std::ptr::from_ref(clock) as usize)
                    .collect();
                assert_eq!(before, after, "unchanged clocks keep their map storage");
                assert!(Arc::ptr_eq(
                    &retained.clock_records.as_ref().unwrap().source,
                    &records
                ));
            }
            saw[0] |= records.is_empty();
            saw[1] |= !records.is_empty();
            saw[2] |= redirects.len() == 2;
            saw[3] |= redirects.len() == 3;
        }
        assert!(saw.into_iter().all(|seen| seen));
    }
}

pub struct ModelPass {
    native: RefCell<Option<native::Renderer>>,
    native_walk: native::walk::Layout,
    layers: Option<Arc<ResolvedLayers>>,
    decode: Decode,
    textures: Vec<Option<Arc<Bound>>>,
    sources: Vec<(ModelTexture, ColorSpace)>,
    uploads: Arc<Uploads>,
    shared_textures: Option<Arc<DeviceTextures>>,
    materials: Vec<ModelMaterial>,
    ready: Vec<bool>,
    resident: Vec<bool>,
    field_resident: bool,
    settled: Cell<bool>,
    warmed: Cell<usize>,
    material_refusals: BTreeMap<usize, String>,
    meshes: Vec<ModelMesh>,
    field: ModelField,
    clock_records: Option<ClockRecords>,
    clock_routes: ClockRoutes,
    redirect_placed: Vec<(usize, ModelInstance)>,
    animation_dispatch: animation_dispatch::Dispatch,
    keyed: Vec<KeyedInstance>,
    alive: Vec<(usize, u32, u32)>,
    drawn: Cell<u32>,
    cull: Cell<Option<GameCull>>,
    seen: Cell<Option<GameCull>>,
    refusals: RefCell<(u64, ActorRefusals)>,
    refused: Cell<u64>,
    cast: Cell<bool>,
    caster: viewer_scene_base::shadow::CasterId,
    revision: u64,
    born: RefCell<Vec<(usize, u32, u32)>>,
    walked: RefCell<WalkedField>,
    written: Cell<bool>,
    frame_change: RefCell<FrameChange>,
    poses: RefCell<(u64, Vec<Option<ActorPoses>>)>,
    id: u64,
    history: u64,
    pose_on_the_frame: bool,
    posed: Arc<crate::poses::Set>,
    pose_epoch: u64,
    static_actors: RefCell<BTreeMap<(usize, bool), ActorPoses>>,
    label: &'static str,
    rigs: Vec<Arc<Rig>>,
    default_palette: Vec<PaletteJoint>,
    players: BTreeMap<(usize, u32, u32), entity_animation::Player>,
    default_appeared: BTreeMap<(usize, u32, u32), f32>,
    default_plays: Vec<Option<(Option<Arc<Rig>>, Option<DefaultPlay>)>>,
    pub default_state_refusals: BTreeMap<String, usize>,
    pub events: Vec<entity_animation::Event>,
    event_namespace: u64,
    hidden: BTreeSet<(usize, u32, u32)>,
    hidden_actors: BTreeSet<ActorAddress>,
    alternatives: BTreeMap<usize, usize>,
    redirects: BTreeMap<usize, usize>,
    falling_back: RefCell<BTreeMap<usize, String>>,
    from_recording: BTreeMap<String, String>,
    direct: bool,
}

fn event_namespace() -> u64 {
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
    NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
}

impl ModelPass {
    #[cfg(any(test, feature = "test-support"))]
    pub fn groups_for_tests(&self) -> &[ModelGroup] {
        &self.field.groups
    }

    pub fn player(
        &self,
        mesh: usize,
        instance: &ModelInstance,
    ) -> Option<&entity_animation::Player> {
        let (seed, identity) = instance.animation_key();
        self.players.get(&(self.drawn_slot(mesh), seed, identity))
    }

    fn actors(
        &self,
        mesh: usize,
        key: (usize, u32, u32),
        rig: &Rig,
        player: Option<&entity_animation::Player>,
        frozen_animation: bool,
    ) -> ActorPoses {
        use viewer_runtime::view_record::add;
        if let Some(player) = player {
            if let Some(posed) = self.posed.get((key, mesh)) {
                add(self.label, "poses_worked", 1);
                return posed;
            }
            let (actors, held) = player.native_actors_held(rig);
            add(self.label, if held { "poses_held" } else { "poses_evaluated" }, 1);
            return actors;
        }
        add(self.label, "actors_static", 1);
        Arc::clone(
            self.static_actors
                .borrow_mut()
                .entry((mesh, frozen_animation))
                .or_insert_with(|| crate::mesh_pose::actors(rig, None, frozen_animation)),
        )
    }

    pub fn shown_actors(
        &self,
        rectangle: Option<[f32; 4]>,
        offsets: &[f32],
        into: &mut Vec<viewer_scene_base::actor_motion::Shown>,
    ) {
        for (group_at, index) in self.field.source_order() {
            let (Some(group), Some(instance)) =
                (self.field.groups.get(group_at), self.field.instances.get(index))
            else {
                continue;
            };
            let Some(offset) = offsets
                .iter()
                .copied()
                .find(|offset| visible(group, *offset, rectangle))
            else {
                continue;
            };
            let (seed, identity) = instance.animation_key();
            let key = (self.drawn_slot(group.mesh), seed, identity);
            into.push(viewer_scene_base::actor_motion::Shown {
                key,
                at: [
                    instance.position[0] + offset,
                    instance.position[1],
                    instance.position[2],
                ],
                samples: self
                    .players
                    .get(&key)
                    .map_or(0, entity_animation::Player::sample_bits),
            });
        }
    }

    pub fn root_joints(&self, mesh: usize, instance: &ModelInstance) -> Option<Result<Vec<[f32; 3]>, String>> {
        let slot = self.drawn_slot(mesh);
        let rig = self.rigs.get(slot)?;
        let player = self.player(mesh, instance)?;
        let root = rig.nodes.first()?;
        let Some(source) = root.native.as_ref() else {
            return Some(Ok(Vec::new()));
        };
        if source.skeleton.is_empty() {
            return Some(Ok(Vec::new()));
        }
        let entity = source.entity_scale;
        let actors = self.actors(slot, self.player_key(mesh, instance), rig, Some(player), false);
        let pose = match actors.first().map(|actor| &actor.pose) {
            Some(Ok(pose)) => pose,
            Some(Err(why)) => return Some(Err(why.clone())),
            None => return Some(Err(format!("unit model {mesh} has no root actor"))),
        };
        Some(Ok(pose.joints.iter().map(|joint| [0, 1, 2].map(|axis| joint[3][axis] * entity)).collect()))
    }

    fn player_key(&self, mesh: usize, instance: &ModelInstance) -> (usize, u32, u32) {
        let (seed, identity) = instance.animation_key();
        (self.drawn_slot(mesh), seed, identity)
    }

    pub fn pose_pass(&self) -> crate::poses::Pass {
        (self.id, self.history)
    }

    pub fn pose_on_the_frame(&mut self, on: bool) {
        if self.pose_on_the_frame == on {
            return;
        }
        self.pose_on_the_frame = on;
        if on {
            self.install_poses(Default::default());
        }
    }

    pub fn take_poses(&mut self, made: &crate::poses::Made) {
        if self.pose_on_the_frame {
            return;
        }
        self.install_poses(made.set_for(self.pose_pass()));
    }

    pub fn pose_jobs(
        &self,
        into: &mut crate::poses::Jobs,
        rectangle: Option<[f32; 4]>,
        offsets: &[f32],
    ) {
        if self.pose_on_the_frame {
            return;
        }
        for (group_at, index) in self.field.source_order() {
            let (Some(group), Some(instance)) =
                (self.field.groups.get(group_at), self.field.instances.get(index))
            else {
                continue;
            };
            if !offsets.iter().any(|offset| visible(group, *offset, rectangle)) {
                continue;
            }
            let key = (self.player_key(group.mesh, instance), group.mesh);
            let (Some(rig), Some(player)) = (self.rigs.get(group.mesh), self.players.get(&key.0))
            else {
                continue;
            };
            if player.samples().len() < rig.nodes.len() {
                continue;
            }
            into.push(self.pose_pass(), key, rig, player.samples());
        }
    }

    pub fn install_poses(&mut self, posed: Arc<crate::poses::Set>) {
        self.posed = posed;
        self.pose_epoch += 1;
    }

    pub fn judge_poses(&self, animation_time: f32, interval: f32) {
        crate::poses::judge(self.label, &self.posed, animation_time, interval);
    }

    fn shader_library(&self, mesh: usize) -> usize {
        usize::from(self.alternatives.values().any(|&slot| slot == mesh))
    }

    fn drawn_slot(&self, mesh: usize) -> usize {
        self.redirects.get(&mesh).copied().unwrap_or(mesh)
    }

    pub fn root_play_position(&self, key: (usize, u32, u32)) -> Option<Option<(f32, f32)>> {
        let key = (self.drawn_slot(key.0), key.1, key.2);
        let rig = self.rigs.get(key.0)?;
        if rig.nodes.iter().all(|node| node.clips.is_empty()) {
            return Some(None);
        }
        Some(self.players.get(&key)?.root_play_position(rig))
    }

    pub fn set_clocks(&mut self, clocks: BTreeMap<(usize, u32, u32), entity_animation::Clock>) {
        self.clock_records = None;
        self.install_clocks(clocks, false);
    }

    pub fn clock_actor_addresses(&self) -> impl Iterator<Item = ActorAddress> + '_ {
        self.clock_routes
            .routes
            .iter()
            .map(|route| route.target)
            .chain(self.clock_routes.sources.iter().copied().take(
                if self.clock_routes.redirects.is_empty() {
                    self.clock_routes.sources.len()
                } else {
                    0
                },
            ))
            .map(|key| self.actor_address(key))
    }

    fn install_clocks(
        &mut self,
        mut clocks: BTreeMap<ClockKey, entity_animation::Clock>,
        effective: bool,
    ) {
        for (&source, clock) in &clocks {
            if clock.phase.is_some() {
                let target = self.redirects.get(&source.0).copied().unwrap_or(source.0);
                for mesh in [source.0, target] {
                    if let Some(asset) = self.meshes.get(mesh) {
                        self.field
                            .bindings
                            .entry(mesh)
                            .or_insert_with(|| asset.asset.clone());
                    }
                }
            }
        }
        let routes = &mut self.clock_routes;
        let sources_changed = !routes.sources.iter().copied().eq(clocks.keys().copied());
        if self.redirects.is_empty() {
            if sources_changed || !routes.redirects.is_empty() || routes.effective != effective {
                routes.sources.clear();
                routes.sources.extend(clocks.keys().copied());
                routes.routes.clear();
                routes.redirects.clear();
                routes.effective = effective;
                #[cfg(test)]
                {
                    routes.builds += 1;
                }
            }
            self.field.clock_priorities.clear();
            self.field.clocks = clocks;
            return;
        }
        if sources_changed
            || routes.effective != effective
            || routes.redirects != self.redirects
            || (self.field.clock_priorities.is_empty() && !routes.routes.is_empty())
            || !routes.routes.iter().all(|route| {
                let ordinal = if effective {
                    self.field
                        .clock_priorities
                        .get(&route.source)
                        .copied()
                        .unwrap_or_else(|| self.field.producer_ordinal(route.source))
                } else {
                    self.field.producer_ordinal(route.source)
                };
                route.ordinal == ordinal
                    && route.asset.as_ref()
                        == self
                            .field
                            .bindings
                            .get(&route.source.0)
                            .or_else(|| self.meshes.get(route.source.0).map(|mesh| &mesh.asset))
            })
        {
            routes.sources.clear();
            routes.sources.extend(clocks.keys().copied());
            routes.routes.clear();
            for &source in &routes.sources {
                let target = (
                    self.redirects.get(&source.0).copied().unwrap_or(source.0),
                    source.1,
                    source.2,
                );
                for mesh in [source.0, target.0] {
                    if let Some(asset) = self.meshes.get(mesh) {
                        self.field
                            .bindings
                            .entry(mesh)
                            .or_insert_with(|| asset.asset.clone());
                    }
                }
                routes.routes.push(ClockRoute {
                    source,
                    target,
                    ordinal: if effective {
                        self.field
                            .clock_priorities
                            .get(&source)
                            .copied()
                            .unwrap_or_else(|| self.field.producer_ordinal(source))
                    } else {
                        self.field.producer_ordinal(source)
                    },
                    asset: self.field.bindings.get(&source.0).cloned(),
                });
            }
            routes.routes.sort_unstable_by(|a, b| {
                (a.ordinal, &a.asset, a.source.1, a.source.2, a.source.0)
                    .cmp(&(b.ordinal, &b.asset, b.source.1, b.source.2, b.source.0))
            });
            routes.redirects.clone_from(&self.redirects);
            routes.effective = effective;
            #[cfg(test)]
            {
                routes.builds += 1;
            }
            self.field.clock_priorities.clear();
            for route in &routes.routes {
                self.field
                    .clock_priorities
                    .insert(route.target, route.ordinal);
            }
        }
        self.field.clocks.clear();
        for route in &routes.routes {
            self.field
                .clocks
                .insert(route.target, clocks.remove(&route.source).unwrap());
        }
    }

    pub fn set_clock_records(&mut self, records: &Arc<[ClockRecord]>, time: f32) {
        let _span = viewer_runtime::profiling::span("set_clock_records");
        viewer_runtime::view_record::add(self.label, "clock_records", records.len() as u64);
        if !self.redirects.is_empty() {
            self.set_clocks(
                records
                    .iter()
                    .map(|record| (record.key, record.clock(time)))
                    .collect(),
            );
            return;
        }
        if self
            .clock_records
            .as_ref()
            .is_none_or(|installed| !Arc::ptr_eq(&installed.source, records))
        {
            let order: BTreeMap<_, _> = records
                .iter()
                .enumerate()
                .map(|(index, record)| (record.key, index))
                .collect();
            self.clock_records = Some(ClockRecords {
                source: Arc::clone(records),
                order: order.into_values().collect(),
            });
        }
        let installed = self.clock_records.as_ref().unwrap();
        if self.field.clocks.len() == installed.order.len()
            && self
                .field
                .clocks
                .iter_mut()
                .zip(&installed.order)
                .all(|((key, clock), &index)| {
                    let record = &records[index];
                    if *key != record.key
                        || clock.phase.is_some()
                        || !Arc::ptr_eq(&clock.requests, &record.requests)
                    {
                        return false;
                    }
                    clock.time = time - record.start;
                    true
                })
        {
            return;
        }
        let clocks = installed
            .order
            .iter()
            .map(|&index| {
                let record = &records[index];
                (record.key, record.clock(time))
            })
            .collect();
        self.install_clocks(clocks, false);
    }

    pub fn animate(&mut self, time: f32, rectangle: Option<[f32; 4]>, offsets: &[f32]) {
        self.fall_back();
        static PROFILE: OnceLock<bool> = OnceLock::new();
        let profile = *PROFILE.get_or_init(|| std::env::var_os("BATTLEPLAN_ANIMATION_PROFILE").or_else(|| std::env::var_os("HOI4_ANIMATION_PROFILE")).is_some());
        let started = profile.then(Instant::now);
        self.events.clear();
        if time <= 0.0
            && !self
                .field
                .clocks
                .values()
                .any(|clock| clock.phase.is_some())
        {
            self.players.clear();
            for instance in &mut self.field.instances {
                instance.palette_offset = 0;
            }
            return;
        }
        self.field.default_states = self.prepare_default_states(time);
        let shown = |group: &ModelGroup| {
            offsets
                .iter()
                .any(|offset| visible(group, *offset, rectangle))
        };
        if viewer_runtime::view_record::on() {
            use viewer_runtime::view_record::add;
            add(self.label, "animate_instances", self.field.instances.len() as u64);
            add(self.label, "animate_groups", self.field.groups.len() as u64);
            add(
                self.label,
                "animate_groups_shown",
                self.field.groups.iter().filter(|group| shown(group)).count() as u64,
            );
        }
        let born = self.born.get_mut();
        let palette = if self.animation_dispatch.animate(
            &mut self.players,
            &self.rigs,
            self.default_palette.len(),
            &mut self.field,
            time,
            &shown,
            &mut self.events,
            self.label,
        ) {
            Vec::new()
        } else {
            animate_players_native(
                &mut self.players,
                &self.rigs,
                &self.default_palette,
                &mut self.field,
                time,
                &shown,
                &mut self.events,
                true,
                born,
            )
        };
        born.extend_from_slice(self.animation_dispatch.created());
        if born.len() > self.field.instances.len() {
            born.clear();
            self.raise();
        }
        for event in &mut self.events {
            event.key.pass = self.event_namespace;
        }
        if let Some(started) = started {
            eprintln!(
                "Animation: time={time:.6} players={} events={} palette_bytes={} instance_bytes={} cpu_us={}",
                self.players.len(),
                self.events.len(),
                std::mem::size_of_val(palette.as_slice()),
                self.field.instances.len() * std::mem::size_of::<ModelInstance>(),
                started.elapsed().as_micros()
            );
        }
    }

    fn prepare_default_states(&mut self, time: f32) -> BTreeMap<(usize, u32, u32), u32> {
        self.default_state_refusals.clear();
        let mut native = BTreeMap::new();
        let mut stilled = BTreeMap::<usize, usize>::new();
        let ModelField {
            native_order,
            groups,
            instances,
            clocks,
            ..
        } = &mut self.field;
        for (group_at, index) in source_order(native_order, groups) {
            let group = &groups[group_at];
            let instance = instances[index];
            if instance.animation != entity_animation::DEFAULT_STATE {
                continue;
            }
            if self.default_plays.len() <= group.mesh {
                self.default_plays.resize_with(group.mesh + 1, || None);
            }
            let rig = self.rigs.get(group.mesh);
            let made = &mut self.default_plays[group.mesh];
            if made.as_ref().is_none_or(|(from, _)| !same_rig(from.as_ref(), rig)) {
                *made = Some((rig.cloned(), DefaultPlay::of(rig.map(|rig| &**rig), self.meshes.get(group.mesh))));
            }
            let Some(play) = made.as_ref().and_then(|(_, play)| play.as_ref()) else {
                continue;
            };
            if play.refusal.is_some() {
                *stilled.entry(group.mesh).or_default() += 1;
                continue;
            }
            let (seed, identity) = instance.animation_key();
            let key = (group.mesh, seed, identity);
            let appeared = self.default_appeared.entry(key).or_insert(time);
            *appeared = appeared.min(time);
            clocks.insert(
                key,
                entity_animation::Clock {
                    time: time - *appeared,
                    phase: None,
                    requests: Arc::clone(&play.requests),
                },
            );
            native.insert(key, play.native);
        }
        for (mesh, count) in stilled {
            if let Some((_, Some(DefaultPlay {
                refusal: Some(refusal),
                ..
            }))) = &self.default_plays[mesh]
            {
                *self
                    .default_state_refusals
                    .entry(refusal.clone())
                    .or_default() += count;
            }
        }
        self.default_appeared
            .retain(|key, _| native.contains_key(key));
        native
    }

    pub fn reset_animation(&mut self) {
        self.event_namespace = event_namespace();
        self.history += 1;
        self.refresh_hidden_actors();
        self.players.clear();
        self.default_appeared.clear();
        self.events.clear();
        self.install_poses(Default::default());
    }

    pub fn update_moving(&mut self, placed: Moving, tile: f32) {
        match placed.standing {
            Some(standing) => self.update_placed(standing, tile),
            None => {
                self.events.clear();
                self.field.default_states.clear();
            }
        }
        self.move_placed(&placed.moved);
    }

    fn move_placed(&mut self, moved: &[(usize, ModelInstance)]) {
        if moved.is_empty() {
            return;
        }
        let _span = viewer_runtime::profiling::span("model_move_placed");
        let mut groups: Vec<usize> = Vec::with_capacity(moved.len());
        for &(ordinal, instance) in moved {
            let Some(&(group, index)) = self.field.native_order.get(ordinal) else {
                continue;
            };
            let Some(held) = self.field.instances.get_mut(index) else {
                continue;
            };
            *held = ModelInstance {
                palette_offset: held.palette_offset,
                ..instance
            };
            if groups.last() != Some(&group) {
                groups.push(group);
            }
        }
        groups.sort_unstable();
        groups.dedup();
        for at in groups {
            let Some(group) = self.field.groups.get(at) else {
                continue;
            };
            let radius = self.meshes.get(group.mesh).map_or(0.0, |mesh| mesh.radius);
            let range = group.range.start as usize..group.range.end as usize;
            let instances = &self.field.instances[range];
            let bounds = reach_bounds(radius, instances.iter());
            let mut sum = [0.0f64; 3];
            for instance in instances {
                for (axis, total) in sum.iter_mut().enumerate() {
                    *total += f64::from(instance.position[axis]);
                }
            }
            let count = instances.len() as f64;
            let group = &mut self.field.groups[at];
            group.bounds = bounds;
            group.centroid = std::array::from_fn(|axis| (sum[axis] / count) as f32);
        }
        viewer_runtime::view_record::add(self.label, "field_moved_parts", moved.len() as u64);
        self.written.set(true);
    }

    pub fn update_placed(&mut self, mut placed: Vec<(usize, ModelInstance)>, tile: f32) {
        self.field_resident = false;
        self.alive.clear();
        self.alive.extend(placed.iter().map(|(mesh, instance)| {
            let (seed, identity) = instance.animation_key();
            (
                self.redirects.get(mesh).copied().unwrap_or(*mesh),
                seed,
                identity,
            )
        }));
        self.alive.sort_unstable();
        self.players.retain(|key, _| {
            self.alive.binary_search(key).is_ok()
                || self
                    .field
                    .clocks
                    .get(key)
                    .is_some_and(|clock| clock.phase.is_some())
        });
        self.events.clear();
        self.field.default_states.clear();
        self.field
            .rebuild_meshes(&self.meshes, &mut placed, &mut self.keyed, tile);
        self.redirect_field();
        self.rebuild_native_walk();
    }

    pub fn replace_field(&mut self, library: &ModelLibrary, field: &mut ModelField) {
        self.sync(library);
        std::mem::swap(
            &mut self.field.clock_priorities,
            &mut field.clock_priorities,
        );
        std::mem::swap(&mut self.field.clock_sources, &mut field.clock_sources);
        std::mem::swap(&mut self.field.ordered_clocks, &mut field.ordered_clocks);
        #[cfg(test)]
        std::mem::swap(
            &mut self.field.clock_order_builds,
            &mut field.clock_order_builds,
        );
        std::mem::swap(&mut self.field, field);
        self.bind_field();
        let clocks = std::mem::take(&mut self.field.clocks);
        self.install_clocks(clocks, false);
        self.redirect_field();
        self.rebuild_native_walk();
    }

    pub fn make_field_resident(&mut self, device: &wgpu::Device, queue: &wgpu::Queue) -> io::Result<()> {
        if self.field_resident {
            return Ok(());
        }
        let shown = |group: &&ModelGroup| {
            self.field.instances[group.range.start as usize..group.range.end as usize]
                .iter()
                .any(|instance| {
                    let (seed, identity) = instance.animation_key();
                    !self.hidden.contains(&(group.mesh, seed, identity))
                })
        };
        let meshes: BTreeSet<usize> = self.field.groups.iter().filter(shown).map(|group| group.mesh).collect();
        for mesh in meshes {
            self.make_resident(device, queue, mesh)?;
        }
        self.field_resident = true;
        Ok(())
    }

    pub fn new(library: &ModelLibrary, label: &'static str) -> Self {
        Self {
            native: RefCell::new(None),
            native_walk: Default::default(),
            layers: library.layers.clone(),
            decode: library.decode,
            textures: (0..library.textures.len()).map(|_| None).collect(),
            sources: drawn_sources(library),
            uploads: Arc::clone(&library.uploads),
            shared_textures: None,
            materials: library.materials.clone(),
            ready: vec![false; library.materials.len()],
            resident: Vec::new(),
            field_resident: false,
            settled: Cell::new(false),
            warmed: Cell::new(0),
            material_refusals: BTreeMap::new(),
            meshes: library.meshes.clone(),
            field: ModelField::default(),
            clock_records: None,
            clock_routes: ClockRoutes::default(),
            redirect_placed: Vec::new(),
            animation_dispatch: Default::default(),
            keyed: Vec::new(),
            alive: Vec::new(),
            drawn: Cell::new(0),
            cull: Cell::new(None),
            seen: Cell::new(None),
            refusals: Default::default(),
            refused: Cell::new(0),
            cast: Cell::new(false),
            caster: viewer_scene_base::shadow::CasterId::next(),
            revision: 0,
            born: RefCell::default(),
            walked: RefCell::default(),
            written: Cell::new(true),
            frame_change: RefCell::default(),
            poses: Default::default(),
            id: event_namespace(),
            history: 0,
            pose_on_the_frame: false,
            posed: Default::default(),
            pose_epoch: 0,
            static_actors: Default::default(),
            label,
            alternatives: library.alternatives.clone(),
            redirects: BTreeMap::new(),
            falling_back: RefCell::default(),
            from_recording: BTreeMap::new(),
            rigs: library.rigs.clone(),
            default_palette: library.palette.clone(),
            players: BTreeMap::new(),
            default_appeared: BTreeMap::new(),
            default_plays: Vec::new(),
            default_state_refusals: BTreeMap::new(),
            events: Vec::new(),
            event_namespace: event_namespace(),
            hidden: BTreeSet::new(),
            hidden_actors: BTreeSet::new(),
            direct: false,
        }
    }

    pub fn drawn_directly(&mut self) {
        self.direct = true;
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn with_rigs_for_tests(rigs: Vec<Arc<Rig>>, label: &'static str) -> Self {
        let mut library = ModelLibrary::default();
        for at in 0..rigs.len() {
            library.meshes.push(ModelMesh {
                asset: AssetKey::synthetic(&format!("actor_{at}")),
                name: format!("actor_{at}"),
                path: String::new(),
                scale: 1.,
                lod_distances: Vec::new(),
                native: Vec::new(),
                radius: 0.,
                height: 0.,
                visual: false,
                own_radius: 0.,
                bound: 0.,
                convoy_radius: None,
                attachments: 0,
                cull_distance_squared: content_reader_mesh::mesh::DEFAULT_CULL_DISTANCE_SQUARED,
            });
        }
        let mut pass = Self::new(&library, label);
        pass.rigs = rigs;
        pass.default_palette = vec![PaletteJoint::IDENTITY];
        pass
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn presentation_state(&self) -> impl PartialEq + std::fmt::Debug {
        (
            self.field.clone(),
            self.players
                .keys()
                .map(|key| (*key, self.root_play_position(*key)))
                .collect::<Vec<_>>(),
        )
    }

    pub fn set_hidden(&mut self, hidden: BTreeSet<(usize, u32, u32)>) {
        if hidden != self.hidden {
            self.raise();
            self.field_resident = false;
        }
        self.hidden = hidden;
        self.redirect_field();
        self.refresh_hidden_actors();
        self.native_walk.set_hidden(&self.hidden);
    }

    pub fn actor_address(&self, key: (usize, u32, u32)) -> ActorAddress {
        let asset = self.meshes[self.drawn_slot(key.0)].asset.clone();
        (self.event_namespace, asset, key.1, key.2)
    }

    fn refresh_hidden_actors(&mut self) {
        self.hidden_actors = self
            .hidden
            .iter()
            .map(|&key| self.actor_address(key))
            .collect();
    }

    pub fn hides(&self, key: ActorAddress) -> bool {
        self.hidden_actors.contains(&key)
    }

    pub fn sync_library(&mut self, library: &ModelLibrary) {
        self.sync(library);
    }

    fn sync(&mut self, library: &ModelLibrary) {
        if self.layers.is_none() && library.layers.is_some() {
            self.layers = library.layers.clone();
            self.raise();
        }
        let grew = self.meshes.len() <= library.meshes.len()
            && self.textures.len() <= library.textures.len()
            && appended_only(
                (&self.default_palette, self.meshes.len()),
                (&library.palette, library.meshes.len()),
            );
        if !grew || self.alternatives.len() != library.alternatives.len() {
            self.raise();
        }
        let known_meshes = self.meshes.len();
        let route_change = known_meshes < library.meshes.len()
            || self.alternatives.len() != library.alternatives.len();
        if self.meshes.len() < library.meshes.len() {
            self.static_actors.get_mut().clear();
            self.meshes
                .extend_from_slice(&library.meshes[self.meshes.len()..]);
            self.rigs
                .extend_from_slice(&library.rigs[self.rigs.len()..]);
        }
        if self.alternatives.len() != library.alternatives.len() {
            self.alternatives = library.alternatives.clone();
        }
        self.textures.resize_with(library.textures.len(), || None);
        if !grew || self.sources.len() != library.textures.len() {
            self.sources = drawn_sources(library);
        }
        if self.materials.len() < library.materials.len() {
            self.materials
                .extend_from_slice(&library.materials[self.materials.len()..]);
            self.ready.resize(library.materials.len(), false);
        }
        if self.default_palette.len() != library.palette.len() {
            retain_players(
                &mut self.players,
                (&self.default_palette, known_meshes),
                (&library.palette, library.meshes.len()),
            );
            self.default_palette = library.palette.clone();
        }
        if route_change {
            self.resident.clear();
            self.default_plays.clear();
            self.bind_field();
            self.rebuild_native_walk();
        }
    }

    fn bind_field(&mut self) {
        self.field_resident = false;
        let mut changed = false;
        for group in &mut self.field.groups {
            let Some(mesh) = self.meshes.get(group.mesh) else {
                continue;
            };
            let rebind = group.asset != mesh.asset;
            if rebind {
                group.asset = mesh.asset.clone();
                changed = true;
            }
            if rebind || !self.field.bindings.contains_key(&group.mesh) {
                self.field.bindings.insert(group.mesh, mesh.asset.clone());
                changed = true;
            }
        }
        for &(slot, _, _) in self
            .field
            .clocks
            .keys()
            .chain(self.clock_routes.sources.iter())
        {
            if let Some(mesh) = self.meshes.get(slot)
                && !self.field.bindings.contains_key(&slot)
            {
                self.field.bindings.insert(slot, mesh.asset.clone());
                changed = true;
            }
        }
        if !changed {
            return;
        }
        self.clock_records = None;
        self.field.clock_sources.clear();
        self.field.ordered_clocks.clear();
        self.field.clock_priorities.clear();
        for route in &mut self.clock_routes.routes {
            route.asset = self.field.bindings.get(&route.source.0).cloned();
        }
        self.clock_routes.routes.sort_unstable_by(|a, b| {
            (a.ordinal, &a.asset, a.source.1, a.source.2, a.source.0)
                .cmp(&(b.ordinal, &b.asset, b.source.1, b.source.2, b.source.0))
        });
        for route in &self.clock_routes.routes {
            self.field
                .clock_priorities
                .insert(route.target, route.ordinal);
        }
        self.refresh_hidden_actors();
        self.poses.get_mut().1.clear();
    }

    fn make_resident(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, mesh: usize) -> io::Result<()> {
        if self.resident.get(mesh).copied().unwrap_or(false) {
            return Ok(());
        }
        if let Some(&counterpart) = self.alternatives.get(&mesh) {
            self.make_resident(device, queue, counterpart)?;
        }
        let materials: BTreeSet<usize> = self.meshes[mesh]
            .native
            .iter()
            .flat_map(|descriptor| descriptor.lods.iter().flatten())
            .map(|part| part.material_index)
            .collect();
        for material in materials {
            if !self.material_refusals.contains_key(&material)
                && let Err(why) = self.make_ready(device, queue, material)
            {
                if why.kind() != io::ErrorKind::InvalidData {
                    return Err(why);
                }
                self.material_refusals.insert(material, why.to_string());
            }
        }
        if self.resident.len() <= mesh {
            self.resident.resize(mesh + 1, false);
        }
        self.resident[mesh] = true;
        Ok(())
    }

    fn make_ready(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, material: usize) -> io::Result<()> {
        if self.ready[material] {
            return Ok(());
        }
        let ModelMaterial {
            diffuse,
            specular,
            normal,
            ..
        } = self.materials[material];
        let shared = Arc::clone(self.shared_textures.get_or_insert_with(|| self.uploads.on(device)));
        for slot in [diffuse, specular, normal] {
            if self.textures[slot].is_none() {
                let (texture, space) = &self.sources[slot];
                self.textures[slot] = Some(shared.texture(
                    (texture, *space, self.decode),
                    device,
                    queue,
                    &format!("{} texture {slot}", self.label),
                    slot,
                )?);
            }
        }
        self.ready[material] = true;
        Ok(())
    }

    #[cfg(test)]
    pub fn upload_library(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        library: &ModelLibrary,
    ) -> io::Result<()> {
        self.sync(library);
        for mesh in 0..library.meshes.len() {
            self.make_resident(device, queue, mesh)?;
        }
        Ok(())
    }

    pub fn upload(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        library: &ModelLibrary,
        field: ModelField,
    ) -> io::Result<()> {
        self.set_field(library, field);
        self.make_field_resident(device, queue)
    }

    pub fn set_field(&mut self, library: &ModelLibrary, field: ModelField) {
        self.sync(library);
        self.field = field;
        self.bind_field();
        self.redirect_field();
        self.rebuild_native_walk();
    }

    pub fn set_cull(&self, cull: Option<GameCull>, seen: Option<GameCull>) {
        self.cull.set(cull);
        self.seen.set(seen);
    }

    pub(crate) fn game_culled(&self, mesh: usize, instance: &ModelInstance, offset: f32, camera: &Camera) -> Option<Culled> {
        let cull = self.seen.get()?;
        let mesh = self.meshes.get(mesh)?;
        let scale = instance.scale.abs();
        let centre = [instance.position[0] + offset + cull.x - camera.center[0], instance.position[1] + mesh.height * scale * 0.5, instance.position[2]];
        cull.test(centre, (mesh.bound + mesh.height * 0.5) * scale, mesh.cull_distance_squared)
    }

    pub fn shows_animation(
        &self,
        camera: &Camera,
        rectangle: Option<[f32; 4]>,
        offsets: &[f32],
    ) -> bool {
        self.drawn() > 0
            && !self.players.is_empty()
            && self.field.source_order().any(|(group, index)| {
                let group = &self.field.groups[group];
                let instance = &self.field.instances[index];
                let (seed, identity) = instance.animation_key();
                let key = self.player_key(group.mesh, instance);
                let Some(rig) = self.rigs.get(key.0) else {
                    return false;
                };
                !self.hidden.contains(&(group.mesh, seed, identity))
                    && self
                        .players
                        .get(&key)
                        .is_some_and(|player| player.animating(rig))
                    && offsets.iter().any(|&offset| {
                        visible(group, offset, rectangle)
                            && self
                                .game_culled(group.mesh, instance, offset, camera)
                                .is_none()
                    })
            })
    }

    pub fn drawn(&self) -> u32 {
        self.drawn.get()
    }

    fn refuse_actor(
        &self,
        arena: &shader_pipeline::gpu::Arena,
        mesh: Option<usize>,
        instance: usize,
        node: usize,
        name: &str,
        why: impl std::fmt::Display,
    ) {
        let reason = why.to_string();
        if viewer_runtime::view_record::on() {
            viewer_runtime::view_record::add_named(self.label, &format!("refused: {reason}"), 1);
            viewer_runtime::view_record::add_named(self.label, &format!("refused {name}: {reason}"), 1);
        }
        let why = format!("refused actor {name}: {reason}");
        count(Count::RefusedActors, 1);
        if crate::profiling_refusals() {
            let mut named = REFUSALS.lock().unwrap_or_else(|error| error.into_inner());
            *named.entry(format!("{} {why}", self.label)).or_default() += 1;
        }
        match mesh.filter(|&mesh| {
            self.meshes.get(mesh).is_some_and(|mesh| mesh.visual)
                && self.alternatives.contains_key(&mesh)
        }) {
            Some(mesh) => {
                self.falling_back
                    .borrow_mut()
                    .entry(mesh)
                    .or_insert_with(|| why.clone());
            }
            None => arena.refuse(&format!("Map shader {}: refused actor {name}", self.label)),
        }
        let mut refusals = self.refusals.borrow_mut();
        if refusals.0 != arena.frame() {
            *refusals = (arena.frame(), BTreeSet::new());
        }
        refusals.1.insert((instance, node, why));
        self.refused.set(self.refused.get() + 1);
    }

    fn fall_back(&mut self) {
        let refused = self.falling_back.get_mut();
        if refused.is_empty() {
            return;
        }
        let refused = std::mem::take(refused);
        let mut changed = false;
        for (mesh, why) in refused {
            let Some(&counterpart) = self.alternatives.get(&mesh) else {
                continue;
            };
            if self.redirects.insert(mesh, counterpart).is_none() {
                changed = true;
                let name = self.meshes[mesh].name.clone();
                if let Some(layers) = &self.layers {
                    layers.note_overlay_refusal(format!("model {name} ({why})"));
                }
                self.from_recording.insert(name, why);
            }
        }
        if changed {
            self.raise();
            self.redirect_field();
        }
    }

    fn redirect_field(&mut self) {
        if self.redirects.is_empty() {
            return;
        }
        let mut changed = false;
        for group in &mut self.field.groups {
            if let Some(&to) = self.redirects.get(&group.mesh) {
                changed |= group.mesh != to;
                group.mesh = to;
                group.asset = self.meshes[to].asset.clone();
                let radius = self.meshes.get(to).map_or(0.0, |mesh| mesh.radius);
                let instances =
                    &self.field.instances[group.range.start as usize..group.range.end as usize];
                group.bounds = reach_bounds(radius, instances.iter());
            }
        }
        if self
            .field
            .clocks
            .keys()
            .any(|key| self.redirects.contains_key(&key.0))
        {
            let clocks = std::mem::take(&mut self.field.clocks);
            self.install_clocks(clocks, true);
        }
        if self
            .hidden
            .iter()
            .any(|key| self.redirects.contains_key(&key.0))
        {
            self.hidden = std::mem::take(&mut self.hidden)
                .into_iter()
                .map(|(mesh, seed, identity)| {
                    (
                        self.redirects.get(&mesh).copied().unwrap_or(mesh),
                        seed,
                        identity,
                    )
                })
                .collect();
            self.refresh_hidden_actors();
        }
        if changed {
            if let Some(tile) = self.field.tile {
                self.redirect_placed.clear();
                self.redirect_placed
                    .extend(self.field.source_order().map(|(group, instance)| {
                        (
                            self.field.groups[group].mesh,
                            self.field.instances[instance],
                        )
                    }));
                let clocks = std::mem::take(&mut self.field.clocks);
                let priorities = std::mem::take(&mut self.field.clock_priorities);
                self.field.rebuild_meshes_inner(
                    &self.meshes,
                    &mut self.redirect_placed,
                    &mut self.keyed,
                    tile,
                    true,
                );
                self.field.clocks = clocks;
                self.field.clock_priorities = priorities;
            }
            self.rebuild_native_walk();
        } else {
            self.native_walk.set_hidden(&self.hidden);
        }
    }

    fn rebuild_native_walk(&mut self) {
        let _span = viewer_runtime::profiling::span("native_walk_install");
        self.native_walk.rebuild(
            &self.field,
            self.meshes.len(),
            &self.alternatives,
            &self.redirects,
            &self.hidden,
        );
        self.written.set(true);
    }

    fn raise(&mut self) {
        self.revision += 1;
    }

    fn field_change(&self, changed: &mut Changed) -> Change {
        changed.clear();
        if !self.written.replace(false) {
            return Change::Same;
        }
        let mut walked = self.walked.borrow_mut();
        let change = walked.note(&self.field, changed);
        if viewer_runtime::view_record::on() {
            use viewer_runtime::view_record::add;
            add(self.label, "field_placed", 1);
            add(self.label, "field_instances", self.field.instances.len() as u64);
            match change {
                Change::Same => add(self.label, "field_same", 1),
                Change::Changed => {
                    add(self.label, "field_changed", 1);
                    add(self.label, "field_changed_instances", changed.instances.len() as u64);
                    add(self.label, "field_moved_groups", changed.groups.len() as u64);
                }
                Change::Whole => {
                    add(self.label, "field_whole", 1);
                    add(self.label, FIELD_WHOLE[walked.whole], 1);
                    let [joined, left, reordered] = walked.reshaped;
                    add(self.label, "field_joined", joined);
                    add(self.label, "field_left", left);
                    add(self.label, "field_reordered", reordered);
                }
            }
        }
        if self.label == "unit models" {
            count(Count::UnitInstances, self.field.instances.len() as u64);
            match change {
                Change::Same => count(Count::UnitFieldSame, 1),
                Change::Changed => {
                    count(Count::UnitFieldChanged, 1);
                    count(Count::UnitChanged, changed.instances.len() as u64);
                }
                Change::Whole => {
                    count(Count::UnitFieldWhole, 1);
                    let [joined, left, reordered] = walked.reshaped;
                    count(Count::UnitJoined, joined);
                    count(Count::UnitLeft, left);
                    count(Count::UnitReordered, reordered);
                }
            }
        }
        change
    }

    fn frame_change(&self, frame: u64) -> std::cell::Ref<'_, FrameChange> {
        if self.frame_change.borrow().frame != Some(frame) {
            let mut cached = self.frame_change.borrow_mut();
            let cached = &mut *cached;
            cached.since = cached.frame;
            cached.frame = Some(frame);
            cached.change = self.field_change(&mut cached.changed);
            self.take_born(&mut cached.born);
        }
        self.frame_change.borrow()
    }

    fn take_born(&self, born: &mut Vec<(usize, u32, u32)>) {
        born.clear();
        std::mem::swap(born, &mut *self.born.borrow_mut());
        born.sort_unstable();
        born.dedup();
    }

    pub fn refusal_lines(&self, frame: u64) -> Vec<String> {
        let mut counts = BTreeMap::<&str, usize>::new();
        let refusals = self.refusals.borrow();
        if refusals.0 == frame {
            for (_, _, why) in &refusals.1 {
                *counts.entry(why).or_default() += 1;
            }
        }
        for player in self.players.values() {
            for (why, count) in player.event_refusals() {
                *counts.entry(why).or_default() += count;
            }
        }
        counts
            .into_iter()
            .map(|(why, count)| format!("Map shader {}: {count} {why}", self.label))
            .chain(self.from_recording.iter().map(|(name, why)| {
                format!(
                    "Map shader {}: visual mod entity {name} is drawn from the recording ({why})",
                    self.label
                )
            }))
            .collect()
    }

    pub fn ready_casters(
        &self,
        arena: &shader_pipeline::gpu::Arena,
        frame: ModelFrame<'_>,
        native: NativeInputs<'_>,
    ) -> io::Result<bool> {
        self.prepare_casters(arena, frame, native)
    }

    pub fn caster_id(&self) -> viewer_scene_base::shadow::CasterId {
        self.caster
    }

    pub fn draw_casters(&self, arena: &shader_pipeline::gpu::Arena, pass: &mut wgpu::RenderPass<'_>) {
        self.record_casters(arena, pass);
    }
}

#[derive(Default)]
struct WalkedField {
    instances: Vec<ModelInstance>,
    order: Vec<(usize, usize)>,
    groups: Vec<(usize, Range<u32>, [u32; 4])>,
    reshaped: [u64; 3],
    whole: usize,
}

const FIELD_WHOLE: [&str; 4] = ["field_whole_count", "field_whole_groups", "field_whole_order", "field_whole_actor"];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Change {
    Same,
    Changed,
    Whole,
}

struct FrameChange {
    frame: Option<u64>,
    pub since: Option<u64>,
    pub change: Change,
    pub changed: Changed,
    pub born: Vec<(usize, u32, u32)>,
}

impl Default for FrameChange {
    fn default() -> Self {
        Self {
            frame: None,
            since: None,
            change: Change::Whole,
            changed: Changed::default(),
            born: Vec::new(),
        }
    }
}

#[derive(Default)]
pub struct Moving {
    pub standing: Option<Vec<(usize, ModelInstance)>>,
    pub moved: Vec<(usize, ModelInstance)>,
}

#[derive(Default)]
pub(crate) struct Changed {
    pub instances: Vec<usize>,
    pub groups: Vec<usize>,
}

impl Changed {
    fn clear(&mut self) {
        self.instances.clear();
        self.groups.clear();
    }
}

fn actor(instance: &ModelInstance) -> (u32, u32, u32) {
    (
        instance.animation_seed,
        instance.animation_identity,
        instance.animation_epoch,
    )
}

impl WalkedField {
    fn note(&mut self, field: &ModelField, changed: &mut Changed) -> Change {
        let walked = |instance: &ModelInstance| ModelInstance {
            palette_offset: 0,
            ..*instance
        };
        let unshaped = if self.instances.len() != field.instances.len() {
            Some(0)
        } else if self.groups.len() != field.groups.len()
            || !self.groups.iter().zip(&field.groups).all(|(kept, now)| kept.0 == now.mesh && kept.1 == now.range)
        {
            Some(1)
        } else if self.order != field.native_order {
            Some(2)
        } else {
            None
        };
        if let Some(whole) = unshaped {
            self.whole = whole;
            self.reshape(field, changed);
            return Change::Whole;
        }
        for (index, (kept, now)) in self.instances.iter().zip(&field.instances).enumerate() {
            let now = walked(now);
            if bytemuck::bytes_of(kept) == bytemuck::bytes_of(&now) {
                continue;
            }
            if actor(kept) != actor(&now) {
                self.whole = 3;
                self.reshape(field, changed);
                return Change::Whole;
            }
            changed.instances.push(index);
        }
        for &index in &changed.instances {
            self.instances[index] = walked(&field.instances[index]);
        }
        for (at, (kept, now)) in self.groups.iter_mut().zip(&field.groups).enumerate() {
            let bounds = now.bounds.map(f32::to_bits);
            if kept.2 != bounds {
                kept.2 = bounds;
                changed.groups.push(at);
            }
        }
        if changed.instances.is_empty() && changed.groups.is_empty() {
            Change::Same
        } else {
            Change::Changed
        }
    }

    fn reshape(&mut self, field: &ModelField, changed: &mut Changed) {
        changed.clear();
        let keyed = |instances: &[ModelInstance], groups: &mut dyn Iterator<Item = (usize, Range<u32>)>| {
            let mut keyed = Vec::with_capacity(instances.len());
            for (mesh, range) in groups {
                for at in range.start..range.end {
                    if let Some(instance) = instances.get(at as usize) {
                        keyed.push(((mesh, actor(instance)), at as usize));
                    }
                }
            }
            keyed.sort_unstable();
            keyed
        };
        let before = keyed(
            &self.instances,
            &mut self.groups.iter().map(|(mesh, range, _)| (*mesh, range.clone())),
        );
        let after = keyed(
            &field.instances,
            &mut field.groups.iter().map(|group| (group.mesh, group.range.clone())),
        );
        let (mut joined, mut left, mut kept) = (0, 0, Vec::new());
        let (mut a, mut b) = (0, 0);
        while a < before.len() || b < after.len() {
            match (before.get(a), after.get(b)) {
                (Some(old), Some(new)) if old.0 == new.0 => {
                    let now = ModelInstance {
                        palette_offset: 0,
                        ..field.instances[new.1]
                    };
                    if bytemuck::bytes_of(&self.instances[old.1]) != bytemuck::bytes_of(&now) {
                        changed.instances.push(new.1);
                    }
                    kept.push((old.1, new.1));
                    a += 1;
                    b += 1;
                }
                (Some(old), Some(new)) if old.0 < new.0 => {
                    left += 1;
                    a += 1;
                }
                (Some(_), None) => {
                    left += 1;
                    a += 1;
                }
                _ => {
                    joined += 1;
                    b += 1;
                }
            }
        }
        kept.sort_unstable();
        let mut tails: Vec<usize> = Vec::new();
        for &(_, new) in &kept {
            let at = tails.partition_point(|&tail| tail < new);
            if at == tails.len() {
                tails.push(new);
            } else {
                tails[at] = new;
            }
        }
        self.reshaped = [joined, left, (kept.len() - tails.len()) as u64];
        changed.instances.sort_unstable();
        self.take(field);
    }

    fn take(&mut self, field: &ModelField) {
        self.order.clone_from(&field.native_order);
        self.instances.clear();
        self.instances.extend(field.instances.iter().map(|instance| ModelInstance {
            palette_offset: 0,
            ..*instance
        }));
        self.groups.clear();
        self.groups.extend(
            field
                .groups
                .iter()
                .map(|group| (group.mesh, group.range.clone(), group.bounds.map(f32::to_bits))),
        );
    }
}

#[derive(Clone, Copy)]
pub(crate) enum Count {
    CasterWalks,
    CasterKept,
    CasterRewritten,
    CasterSpliced,
    CasterFiguresWalked,
    CasterFiguresLeft,
    LitWalks,
    LitKept,
    LitSpliced,
    LitFiguresWalked,
    LitFiguresLeft,
    FirstParts,
    FirstGeometryBytes,
    PackedHere,
    WarmedNodes,
    RefusedActors,
    UnitFieldSame,
    UnitFieldChanged,
    UnitFieldWhole,
    UnitChanged,
    UnitJoined,
    UnitLeft,
    UnitReordered,
    UnitInstances,
}

const COUNT_NAMES: [&str; 24] = [
    "caster_walks",
    "caster_kept",
    "caster_rewritten",
    "caster_spliced",
    "caster_figures_walked",
    "caster_figures_left",
    "lit_walks",
    "lit_kept",
    "lit_spliced",
    "lit_figures_walked",
    "lit_figures_left",
    "first_parts",
    "first_geometry_bytes",
    "packed_here",
    "warmed_nodes",
    "refused_actors",
    "unit_field_same",
    "unit_field_changed",
    "unit_field_whole",
    "unit_changed",
    "unit_joined",
    "unit_left",
    "unit_reordered",
    "unit_instances",
];

static COUNTS: [std::sync::atomic::AtomicU64; COUNT_NAMES.len()] =
    [const { std::sync::atomic::AtomicU64::new(0) }; COUNT_NAMES.len()];

pub(crate) fn count(what: Count, n: u64) {
    COUNTS[what as usize].fetch_add(n, std::sync::atomic::Ordering::Relaxed);
}

static REFUSALS: std::sync::Mutex<BTreeMap<String, u64>> = std::sync::Mutex::new(BTreeMap::new());

fn profiling_refusals() -> bool {
    viewer_runtime::profile_writer::profiling()
}

pub fn take_refusals() -> BTreeMap<String, u64> {
    std::mem::take(&mut *REFUSALS.lock().unwrap_or_else(|error| error.into_inner()))
}

pub fn take_counts() -> impl Iterator<Item = (&'static str, u64)> {
    COUNT_NAMES.iter().zip(&COUNTS).map(|(name, count)| {
        (*name, count.swap(0, std::sync::atomic::Ordering::Relaxed))
    })
}

fn retain_players(
    players: &mut BTreeMap<(usize, u32, u32), entity_animation::Player>,
    before: (&[PaletteJoint], usize),
    after: (&[PaletteJoint], usize),
) {
    if !appended_only(before, after) {
        players.clear();
    }
}

fn appended_only(before: (&[PaletteJoint], usize), after: (&[PaletteJoint], usize)) -> bool {
    let ((old, old_meshes), (new, new_meshes)) = (before, after);
    new_meshes >= old_meshes && new.len() >= old.len() && new[..old.len()] == *old
}

pub fn animate_players(
    players: &mut BTreeMap<(usize, u32, u32), entity_animation::Player>,
    rigs: &[Arc<Rig>],
    default_palette: &[PaletteJoint],
    field: &mut ModelField,
    time: f32,
    shown: &dyn Fn(&ModelGroup) -> bool,
    events: &mut Vec<entity_animation::Event>,
) -> Vec<PaletteJoint> {
    animate_players_native(
        players,
        rigs,
        default_palette,
        field,
        time,
        shown,
        events,
        false,
        &mut Vec::new(),
    )
}

#[expect(clippy::too_many_arguments, reason = "the players and rigs, the field it poses, the time and filter, and the events and players it records")]
fn animate_players_native(
    players: &mut BTreeMap<(usize, u32, u32), entity_animation::Player>,
    rigs: &[Arc<Rig>],
    default_palette: &[PaletteJoint],
    field: &mut ModelField,
    time: f32,
    shown: &dyn Fn(&ModelGroup) -> bool,
    events: &mut Vec<entity_animation::Event>,
    native_only: bool,
    created: &mut Vec<(usize, u32, u32)>,
) -> Vec<PaletteJoint> {
    struct Call<'a> {
        key: (usize, u32, u32),
        asset: &'a AssetKey,
        rig: &'a Rig,
        instance: ModelInstance,
        count: usize,
        clocked: bool,
        events: Vec<entity_animation::Event>,
    }
    struct Work<'a> {
        rig: &'a Rig,
        instance: ModelInstance,
        palette: &'a mut [PaletteJoint],
        events: &'a mut Vec<entity_animation::Event>,
    }
    let plan_span = viewer_runtime::profiling::span("animation_dispatch_plan");
    field.prepare_clock_order();
    field.group_work.clear();
    field.group_work.extend(field.groups.iter().map(|group| {
        if !shown(group) {
            return None;
        }
        let rig = rigs.get(group.mesh)?;
        if rig.nodes.iter().all(|node| node.clips.is_empty()) {
            return None;
        }
        Some(
            if native_only && rig.nodes.iter().all(|node| node.native.is_some()) {
                0
            } else {
                rig.end - rig.start
            },
        )
    }));
    let mut calls = Vec::new();
    let mut palette = if native_only {
        Vec::new()
    } else {
        default_palette.to_vec()
    };
    let mut palette_end = default_palette.len();
    let phase_only = time <= 0.0 && field.clocks.values().any(|clock| clock.phase.is_some());
    let mut alive = BTreeSet::new();
    for instance in &mut field.instances {
        instance.palette_offset = 0;
    }
    let ModelField {
        native_order,
        groups,
        instances,
        group_work,
        clocks,
        default_states,
        ..
    } = field;
    for (group_at, index) in source_order(native_order, groups) {
        let Some(count) = group_work[group_at] else {
            continue;
        };
        let group = &groups[group_at];
        let rig = &rigs[group.mesh];
        let instance = &mut instances[index];
        if instance.animation == entity_animation::STOPPED {
            continue;
        }
        let (seed, identity) = instance.animation_key();
        let key = (group.mesh, seed, identity);
        if phase_only && clocks.get(&key).is_none_or(|clock| clock.phase.is_none()) {
            continue;
        }
        let request = default_states.get(&key).copied();
        if instance.animation == entity_animation::DEFAULT_STATE && request.is_none() {
            continue;
        }
        alive.insert(key);
        let player = players.entry(key).or_insert_with(|| {
            created.push(key);
            if request.is_some() {
                entity_animation::Player::native_choice(rig)
            } else {
                entity_animation::Player::new(rig)
            }
        });
        let start = palette.len();
        if count != 0 {
            palette.extend_from_slice(&default_palette[rig.start..rig.end]);
        }
        instance.palette_offset = (palette_end - rig.start) as u32;
        palette_end += rig.end - rig.start;
        let playing = ModelInstance {
            animation: request.unwrap_or(instance.animation),
            ..*instance
        };
        let mut emitted = Vec::new();
        let clock = clocks.get(&key);
        if let Some(clock) = clock {
            let palette = if count == 0 {
                &mut []
            } else {
                &mut palette[start..]
            };
            player.evaluate_clock(rig, playing, clock, palette, &mut emitted);
        }
        calls.push(Call {
            key,
            asset: &group.asset,
            rig,
            instance: playing,
            count,
            clocked: clock.is_some(),
            events: emitted,
        });
    }
    let run_span;
    {
        let mut by_player = BTreeMap::<_, Vec<Work<'_>>>::new();
        let mut tail = &mut palette[if native_only {
            0
        } else {
            default_palette.len()
        }..];
        for call in &mut calls {
            let (palette, rest) = tail.split_at_mut(call.count);
            tail = rest;
            if !call.clocked {
                by_player.entry(call.key).or_default().push(Work {
                    rig: call.rig,
                    instance: call.instance,
                    palette,
                    events: &mut call.events,
                });
            }
        }
        let mut tasks: Vec<_> = players
            .iter_mut()
            .filter_map(|(key, player)| by_player.remove(key).map(|work| (player, work)))
            .collect();
        drop(plan_span);
        run_span = viewer_runtime::profiling::span("animation_dispatch_run_commit");
        for (player, work) in &mut tasks {
            for work in work.iter_mut() {
                let palette = if work.palette.is_empty() {
                    &mut []
                } else {
                    &mut *work.palette
                };
                player.evaluate(work.rig, work.instance, time, palette, work.events);
            }
        }
    }
    for call in calls {
        events.extend(call.events.into_iter().map(|mut event| {
            event.key.model = call.asset.clone();
            event
        }));
    }
    for key in field.clock_order() {
        let clock = &field.clocks[&key];
        if clock.phase.is_none() || alive.contains(&key) {
            continue;
        }
        let Some(rig) = rigs.get(key.0) else {
            continue;
        };
        players
            .entry(key)
            .or_insert_with(|| {
                created.push(key);
                entity_animation::Player::new(rig)
            })
            .advance_hidden(rig, clock);
        alive.insert(key);
    }
    players.retain(|key, _| alive.contains(key));
    drop(run_span);
    palette
}

fn visible(group: &ModelGroup, offset: f32, rectangle: Option<[f32; 4]>) -> bool {
    let Some([x0, z0, x1, z1]) = rectangle else {
        return true;
    };
    let [gx0, gz0, gx1, gz1] = group.bounds;
    gx0 + offset <= x1 && gx1 + offset >= x0 && gz0 <= z1 && gz1 >= z0
}

struct DefaultPlay {
    requests: Arc<Vec<entity_animation::Request>>,
    native: u32,
    refusal: Option<String>,
}

fn same_rig(made: Option<&Arc<Rig>>, rig: Option<&Arc<Rig>>) -> bool {
    match (made, rig) {
        (Some(made), Some(rig)) => Arc::ptr_eq(made, rig),
        (None, None) => true,
        _ => false,
    }
}

impl DefaultPlay {
    fn of(rig: Option<&Rig>, mesh: Option<&ModelMesh>) -> Option<Self> {
        let rig = rig?;
        let root = rig.nodes.first()?;
        if !root
            .clips
            .iter()
            .any(|clip| clip.state.name == root.default)
        {
            return None;
        }
        let refusal = drawn_variants(rig).map(|(node, reached, variants)| {
            let entity = mesh.map_or("", |mesh| &mesh.name);
            let part = if node == 0 {
                String::new()
            } else {
                format!(" (attached entity {node})")
            };
            format!(
                "entity {entity}{part} reaches state {reached} of {variants} variants, which the game draws from a per-entity random counter no recording holds; it stays still and silent"
            )
        });
        Some(Self {
            requests: Arc::new(vec![entity_animation::Request {
                time: 0.0,
                phase_time: None,
                state: root.default.clone(),
            }]),
            native: entity_animation::named_request(&root.default),
            refusal,
        })
    }
}

fn drawn_variants(rig: &entity_animation::Rig) -> Option<(usize, String, usize)> {
    for (index, node) in rig.nodes.iter().enumerate() {
        let mut seen = BTreeSet::new();
        let mut pending = vec![node.default.clone()];
        while let Some(state) = pending.pop() {
            if !seen.insert(state.clone()) {
                continue;
            }
            let variants: Vec<_> = node
                .clips
                .iter()
                .filter(|clip| clip.state.name == state)
                .collect();
            if variants.len() > 1 {
                return Some((index, state, variants.len()));
            }
            pending.extend(
                variants
                    .iter()
                    .filter_map(|clip| clip.state.next_state.clone()),
            );
        }
    }
    None
}

#[cfg(test)]
mod naval_composition_tests;
#[cfg(test)]
pub(crate) mod tests;

#[cfg(test)]
mod parallel_animation_tests {
    use crate::*;

    #[test]
    fn a_pose_made_from_its_samples_is_the_pose_its_player_makes() {
        use crate::mesh_pose;
        let matrix = |value: mesh_pose::Matrix| value.map(|column| column.map(f32::to_bits));
        let bits = |actors: &[mesh_pose::Actor]| {
            actors
                .iter()
                .map(|actor| {
                    (
                        actor.world.as_ref().map(|world| matrix(*world)).map_err(Clone::clone),
                        actor
                            .pose
                            .as_ref()
                            .map(|pose| {
                                (
                                    pose.joints.iter().copied().map(matrix).collect::<Vec<_>>(),
                                    pose.bones.iter().copied().map(matrix).collect::<Vec<_>>(),
                                )
                            })
                            .map_err(Clone::clone),
                    )
                })
                .collect::<Vec<_>>()
        };
        let rig = entity_animation::pose_rig(0.7, true, true);
        let mut player = entity_animation::Player::new(&rig);
        let instance = ModelInstance {
            animation: entity_animation::IDLE,
            ..Default::default()
        };
        let mut moved = false;
        let mut last = None;
        for frame in 1..=8_u32 {
            player.evaluate(&rig, instance, f32::from(frame as u16) / 60.0, &mut [], &mut Vec::new());
            let worked = bits(&mesh_pose::actors_at(&rig, player.samples()));
            assert_eq!(worked, bits(&mesh_pose::actors(&rig, Some(&player), false)), "frame {frame}");
            moved |= last.is_some_and(|last| last != worked);
            last = Some(worked);
        }
        assert!(moved, "the fixture never poses differently, so it proves nothing");
        assert_eq!(mesh_pose::actors_at(&rig, &[]).len(), rig.nodes.len());
    }

    #[test]
    fn frozen_and_playerless_actors_are_placement_independent_and_reset_on_mesh_install() {
        use crate::mesh_pose;
        let mut rig = entity_animation::pose_rig(0.7, true, true);
        for node in &mut rig.nodes {
            Arc::make_mut(node.native.as_mut().unwrap()).frozen = node.clips[0].animation.clone();
        }
        let mut library = ModelLibrary {
            rigs: vec![Arc::new(rig)],
            meshes: vec![ModelMesh {
                asset: AssetKey::synthetic("synthetic"),
                name: "synthetic".into(),
                path: String::new(),
                scale: 1.,
                lod_distances: Vec::new(),
                native: Vec::new(),
                radius: 1.,
                height: 1.,
                visual: false,
                own_radius: 1.,
                bound: 1.,
                convoy_radius: None,
                attachments: 1,
                cull_distance_squared: content_reader_mesh::mesh::DEFAULT_CULL_DISTANCE_SQUARED,
            }],
            ..Default::default()
        };
        let mut pass = ModelPass::new(&library, "static actor parity");
        let bits = |actors: &[mesh_pose::Actor]| {
            let matrix = |value: mesh_pose::Matrix| value.map(|column| column.map(f32::to_bits));
            actors
                .iter()
                .map(|actor| {
                    (
                        actor
                            .world
                            .as_ref()
                            .map(|world| matrix(*world))
                            .map_err(Clone::clone),
                        actor
                            .pose
                            .as_ref()
                            .map(|pose| {
                                (
                                    pose.joints.iter().copied().map(matrix).collect::<Vec<_>>(),
                                    pose.bones.iter().copied().map(matrix).collect::<Vec<_>>(),
                                )
                            })
                            .map_err(Clone::clone),
                    )
                })
                .collect::<Vec<_>>()
        };
        let mut retained = Vec::new();
        for instance in [
            ModelInstance {
                position: [1., 2., 3.],
                scale: 0.3,
                animation: entity_animation::STOPPED,
                ..Default::default()
            },
            ModelInstance {
                position: [700., -4., 90.],
                scale: 2.1,
                rotation: 0.7,
                pitch: -0.2,
                animation: entity_animation::IDLE,
                animation_seed: 81,
                animation_identity: 9,
                ..Default::default()
            },
        ] {
            pass.update_placed(vec![(0, instance)], 256.);
            let instance = &pass.field.instances[0];
            let rig = &pass.rigs[0];
            for frozen in [false, true] {
                let cached = pass.actors(0, pass.player_key(0, instance), rig, pass.player(0, instance), frozen);
                let fresh = mesh_pose::actors(rig, None, frozen);
                assert_eq!(bits(&cached), bits(&fresh));
                let transform = instance.native_transform().unwrap();
                let [a, b, c] = transform.basis;
                let t = transform.translation;
                let world = [
                    [a[0], a[1], a[2], 0.],
                    [b[0], b[1], b[2], 0.],
                    [c[0], c[1], c[2], 0.],
                    [t[0], t[1], t[2], 1.],
                ];
                let placed = |actor: &mesh_pose::Actor| {
                    actor
                        .world
                        .as_ref()
                        .map(|local| {
                            mesh_pose::multiply(world, *local)
                                .map(|column| column.map(f32::to_bits))
                        })
                        .map_err(Clone::clone)
                };
                assert_eq!(
                    cached.iter().map(placed).collect::<Vec<_>>(),
                    fresh.iter().map(placed).collect::<Vec<_>>(),
                );
                retained.push(cached);
            }
        }
        assert!(Arc::ptr_eq(&retained[0], &retained[2]));
        assert!(Arc::ptr_eq(&retained[1], &retained[3]));
        assert_ne!(bits(&retained[0]), bits(&retained[1]));
        library.meshes.push(library.meshes[0].clone());
        library.rigs.push(Arc::clone(&library.rigs[0]));
        pass.sync(&library);
        assert!(pass.static_actors.borrow().is_empty());
        let installed = pass.actors(0, (0, 0, 0), &pass.rigs[0], None, true);
        assert!(!Arc::ptr_eq(&retained[1], &installed));
        assert_eq!(bits(&retained[1]), bits(&installed));
    }

    #[test]
    fn native_only_batches_match_serial_poses_events_and_offsets() {
        let mut eventless = entity_animation::pose_rig(0.7, true, true);
        for node in &mut eventless.nodes {
            for clip in &mut node.clips {
                clip.state.events.clear();
            }
        }
        let mut event_then_quiet = entity_animation::pose_rig(0.7, true, true);
        for node in &mut event_then_quiet.nodes {
            for clip in &mut node.clips {
                if clip.state.name == "idle" {
                    for event in &mut clip.state.events {
                        event.keep_particle = false;
                    }
                } else {
                    clip.state.events.clear();
                }
            }
        }
        let rigs = [
            Arc::new(eventless),
            Arc::new(entity_animation::pose_rig(0.7, true, true)),
            Arc::new(event_then_quiet),
        ];
        let default = vec![PaletteJoint::IDENTITY; rigs[0].end];
        let (mut serial, mut borrowed) = (BTreeMap::new(), BTreeMap::new());
        for (frame, time) in [0., 0.13, 0.7, 0.71, 1.2, 2.1, 4.9, 0.2, 1.3]
            .into_iter()
            .enumerate()
        {
            let mut field = ModelField::default();
            for mesh in [1, 0, 2] {
                let start = field.instances.len() as u32;
                for seed in [9, 2, 8, 1, 9, 0, 7] {
                    field.instances.push(ModelInstance {
                        scale: 1.,
                        animation_seed: seed,
                        animation: if time < 0.7 {
                            entity_animation::IDLE
                        } else {
                            entity_animation::MOVE
                        },
                        ..Default::default()
                    });
                }
                field.groups.push(ModelGroup {
                    mesh,
                    asset: AssetKey::synthetic(&format!("actor_{mesh}")),
                    range: start..field.instances.len() as u32,
                    bounds: [0.; 4],
                    centroid: [0.; 3],
                });
                for seed in [9, 8] {
                    field.clocks.insert(
                        (mesh, seed, 0),
                        entity_animation::Clock {
                            time,
                            phase: None,
                            requests: Arc::new(vec![entity_animation::Request {
                                time: 0.,
                                phase_time: None,
                                state: "move".into(),
                            }]),
                        },
                    );
                }
            }
            let mut other = field.clone();
            let (mut a, mut b) = (Vec::new(), Vec::new());
            animate_players(
                &mut serial,
                &rigs,
                &default,
                &mut field,
                time,
                &|_| true,
                &mut a,
            );
            let unused = animate_players_native(
                &mut borrowed,
                &rigs,
                &default,
                &mut other,
                time,
                &|_| true,
                &mut b,
                true,
                &mut Vec::new(),
            );
            assert!(
                unused.is_empty(),
                "native actors allocate no unused palette"
            );
            assert_eq!(field, other, "logical palette offsets stay identical");
            assert_eq!(a.len(), b.len());
            for (a, b) in a.iter().zip(&b) {
                assert_eq!(a.definition, b.definition);
                assert_eq!(
                    (
                        a.key.clone(),
                        a.seed,
                        a.age.to_bits(),
                        a.state,
                        a.emission_end.map(f32::to_bits),
                        a.hidden
                    ),
                    (
                        b.key.clone(),
                        b.seed,
                        b.age.to_bits(),
                        b.state,
                        b.emission_end.map(f32::to_bits),
                        b.hidden
                    )
                );
                assert_eq!(
                    bytemuck::bytes_of(&PaletteJoint::from_transform(&a.transform)),
                    bytemuck::bytes_of(&PaletteJoint::from_transform(&b.transform))
                );
            }
            assert_eq!(
                serial.keys().collect::<Vec<_>>(),
                borrowed.keys().collect::<Vec<_>>()
            );
            for (key, player) in &borrowed {
                let actual = player.native_actors(&rigs[key.0]);
                let expected = crate::mesh_pose::actors_uncached(
                    &rigs[key.0],
                    Some(&serial[key]),
                    false,
                );
                assert_eq!(actual.len(), expected.len());
                for (a, b) in actual.iter().zip(&expected) {
                    let matrix =
                        |value: crate::mesh_pose::Matrix| value.map(|c| c.map(f32::to_bits));
                    assert_eq!(
                        a.world.as_ref().map(|m| matrix(*m)),
                        b.world.as_ref().map(|m| matrix(*m))
                    );
                    let pose = |pose: &crate::mesh_pose::Pose| {
                        (
                            pose.joints.iter().copied().map(matrix).collect::<Vec<_>>(),
                            pose.bones.iter().copied().map(matrix).collect::<Vec<_>>(),
                        )
                    };
                    assert_eq!(
                        a.pose.as_ref().map(pose),
                        b.pose.as_ref().map(pose),
                        "frame={frame} key={key:?}"
                    );
                }
            }
        }
    }

    #[test]
    fn borrowed_player_batches_match_serial_palettes_packets_and_lifetimes() {
        let rigs: Vec<_> = [false, true, true]
            .into_iter()
            .map(|native| Arc::new(entity_animation::pose_rig(0.7, true, native)))
            .collect();
        let default = vec![PaletteJoint::IDENTITY; rigs[0].end];
        let (mut expected, mut parallel) = (BTreeMap::new(), BTreeMap::new());
        for frame in 0..32 {
            let time = frame as f32 * 0.13;
            let mut field = ModelField::default();
            for mesh in [2, 0, 1] {
                let start = field.instances.len() as u32;
                for ordinal in 0..20 {
                    let seed = (ordinal % 17) as u32;
                    field.instances.push(ModelInstance {
                        scale: 1.,
                        animation_seed: seed,
                        animation: match frame % 13 {
                            0..=3 => entity_animation::IDLE,
                            4..=8 => entity_animation::MOVE,
                            _ => entity_animation::ATTACK,
                        },
                        ..Default::default()
                    });
                }
                field.groups.push(ModelGroup {
                    mesh,
                    asset: AssetKey::synthetic(&format!("actor_{mesh}")),
                    range: start..field.instances.len() as u32,
                    bounds: [0.; 4],
                    centroid: [0.; 3],
                });
            }
            field.clocks.insert(
                (2, 0, 0),
                entity_animation::Clock {
                    time,
                    phase: None,
                    requests: Arc::new(vec![entity_animation::Request {
                        time: 0.,
                        phase_time: None,
                        state: "move".into(),
                    }]),
                },
            );
            if frame == 15 {
                field.groups.remove(1);
            }
            let mut other = field.clone();
            let (mut a, mut b) = (Vec::new(), Vec::new());
            let a_palette = animate_players(
                &mut expected,
                &rigs,
                &default,
                &mut field,
                time,
                &|_| true,
                &mut a,
            );
            let b_palette = animate_players_native(
                &mut parallel,
                &rigs,
                &default,
                &mut other,
                time,
                &|_| true,
                &mut b,
                false,
                &mut Vec::new(),
            );
            assert_eq!(
                bytemuck::cast_slice::<_, u8>(&a_palette),
                bytemuck::cast_slice::<_, u8>(&b_palette)
            );
            assert_eq!(field, other);
            assert_eq!(
                expected.keys().collect::<Vec<_>>(),
                parallel.keys().collect::<Vec<_>>()
            );
            assert_eq!(a.len(), b.len());
            for (a, b) in a.iter().zip(&b) {
                assert_eq!(a.definition, b.definition);
                assert_eq!(
                    bytemuck::bytes_of(&PaletteJoint::from_transform(&a.transform)),
                    bytemuck::bytes_of(&PaletteJoint::from_transform(&b.transform)),
                );
                assert_eq!(
                    (a.age.to_bits(), a.seed, a.key.clone(), a.state),
                    (b.age.to_bits(), b.seed, b.key.clone(), b.state)
                );
            }
            for (key, player) in &expected {
                assert_eq!(player.root_entry(), parallel[key].root_entry());
                assert_eq!(
                    player.root_play_position(&rigs[key.0]),
                    parallel[key].root_play_position(&rigs[key.0])
                );
            }
        }
    }
}

#[cfg(any(test, feature = "test-support"))]
pub mod test_support;
