//! Content-selected compiled meshes and their shared actor/material library.
//! Native geometry, skin palettes, states and submission live in `models/native`.
//! Presentation clocks and historical actor admission remain reconstructed.

use super::entity_animation::{self, Rig};
use std::cell::Cell;
use std::cell::RefCell;
use std::sync::OnceLock;
use std::time::Instant;
use viewer_foundation::debug_fields;
use viewer_foundation::map::textures::load_file;
use viewer_foundation::profiling::BuilderSpans;
mod animation_dispatch;
mod draw_snapshot;
mod identity;
#[cfg(test)]
pub(crate) mod order_tests;
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
use installed_content_core::{ResolvedLayers, mesh, mesh::Descriptors};
use installed_content_image::{Rgba8Image, dds, native::Decode};

use {
    super::shadow::DEPTH_MAP_FORMAT,
    viewer_foundation::map::{
        camera::{Camera, MapSize},
        gpu_contract::{DEPTH_FORMAT, Targets},
        textures::{Bound, ColorSpace, upload},
    },
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

pub(crate) fn map_effects(state: &mesh::EntityState) -> usize {
    state
        .events
        .iter()
        .map(|event| usize::from(event.particle.is_some()) + usize::from(event.light.is_some()))
        .sum()
}

// ------------------------------------------------------------------ constants

/// `WATER_HEIGHT`. The map-object update compares `camera height - WATER_HEIGHT`
/// against the sprawl and building cutoffs.
pub const WATER_HEIGHT: f32 = 9.5;
/// Update city sprawl below CITY_SPRAWL_SHRINK_DISTANCE,
/// water-relative. DRAW_MAP_OBJECTS_CUTOFF gates only the separate transient
/// province entity, not the city list.
pub fn draws_city_sprawl(camera: &Camera) -> bool {
    camera.height - WATER_HEIGHT < camera.defines.sprawl_cutoff
}

// -------------------------------------------------------------------- effects

// -------------------------------------------------------------------- library

/// One joint of a skinning palette as the shader reads it: three rows of four,
/// the last column the translation.
///
/// The rows are what a `dot` with `(position, 1)` needs, and the 16-byte
/// alignment of a `vec4` is what a storage array of them wants, so the
/// [`mesh::Transform`]'s column form is transposed once here at load.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Pod, Zeroable)]
pub struct PaletteJoint {
    pub rows: [[f32; 4]; 3],
}

impl PaletteJoint {
    #[cfg(test)]
    pub const IDENTITY: Self = Self {
        rows: [
            [1.0, 0.0, 0.0, 0.0],
            [0.0, 1.0, 0.0, 0.0],
            [0.0, 0.0, 1.0, 0.0],
        ],
    };

    pub(crate) fn from_transform(transform: &mesh::Transform) -> Self {
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

/// One placed entity. The engine's own transform for a map object is a
/// translation, a rotation about Y and a uniform scale; nothing in
/// `map/buildings.txt`, `map/unitstacks.txt` or the sprawl generator carries
/// more than that.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Pod, Zeroable)]
pub struct ModelInstance {
    pub position: [f32; 3],
    /// Radians about +Y.
    pub rotation: f32,
    pub scale: f32,
    /// Pitch around local X, used by wagons following a sloping rail.
    pub pitch: f32,
    /// Roll around local Z, used by flying aircraft; paired with pitch at location 9.
    pub roll: f32,
    /// TrainAlphaStart.xy / TrainAlphaDir.xy; zero for an interior wagon.
    pub train_clip: [f32; 4],
    pub palette_offset: u32,
    pub animation: u32,
    pub animation_seed: u32,
    pub animation_phase: f32,
    /// Scene-owned incarnation: recreating an entity restarts its state clock
    /// even when the same mesh and seed are selected again.
    pub animation_epoch: u32,
    /// Native object-data caller: bit0 is a land figure, bit1 permits UV motion,
    /// bit2 preserves a caller-supplied basis instead of reconstructing Euler angles;
    /// bit3 supplies a native state-request random range in `animation_phase`.
    pub native_flags: u32,
    pub native_basis: [[f32; 3]; 3],
    /// Actor identity independent of the seed and dump sampling epoch.
    pub animation_identity: u32,
}

impl ModelInstance {
    pub(super) fn animation_key(self) -> (u32, u32) {
        (
            self.animation_seed,
            if self.animation_identity != 0 {
                self.animation_identity
            } else {
                self.animation_epoch
            },
        )
    }

    /// The game copies the caller's matrix and queues it unchanged.
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

/// One `[material]`: the effect's defines and the three textures.
#[derive(Clone, Debug, PartialEq)]
pub struct ModelMaterial {
    /// Indices into [`ModelLibrary::textures`].
    pub diffuse: usize,
    pub normal: usize,
    pub specular: usize,
}

/// One resolved mesh: its LOD ladder and the bounding radius a cull needs.
#[derive(Clone, Debug, PartialEq)]
pub struct ModelMesh {
    pub asset: AssetKey,
    /// The name the layer asked for: an entity name or a pdxmesh name.
    pub name: String,
    /// The `.mesh` the descriptor named.
    pub path: String,
    /// `entity.scale * pdxmesh.scale`, applied by the actor matrix and mesh import.
    pub scale: f32,
    /// `loddist`, ascending.
    pub lod_distances: Vec<f32>,
    native: Vec<native::Descriptor>,
    /// Horizontal radius of LOD 0's bounds, after `scale` and with every
    /// attachment composed in, for culling.
    pub radius: f32,
    /// Height of LOD 0's bounds above its own origin, after `scale`.
    pub height: f32,
    /// Whether a visual mod supplied any file this entity is drawn from (a mesh, animation,
    /// texture or descriptor, its attachments' included): such an entity keeps a recorded
    /// counterpart to draw when its own draw is refused ([`ModelLibrary::alternatives`]).
    pub visual: bool,
    /// The same radius over the entity's **own** geometry only. An attachment
    /// rig's is a fraction of a world unit while its composed one is metres.
    pub own_radius: f32,
    /// The farthest any point of LOD 0's bounds, attachments composed in, lies
    /// from its own origin, after `scale`: the radius of the sphere around the
    /// origin that encloses it.
    pub bound: f32,
    /// Creation-time runtime-bounds sphere for convoy launch collision.
    pub convoy_radius: Option<f32>,
    /// How many attached entities were composed into this mesh, at any depth.
    pub attachments: usize,
    /// The root mesh's squared cull distance (`cull_distance`, or the engine's default): a figure
    /// whose bounding sphere's centre lies this far from the eye or farther is not drawn.
    pub cull_distance_squared: f32,
}

/// Every entity, shared sub-mesh, material and texture one layer draws.
#[derive(Default)]
pub struct ModelLibrary {
    layers: Option<Arc<ResolvedLayers>>,
    /// The client's texture decode every texture file loads under ([`viewer_foundation::map::textures::load_file`])
    /// and the compiled mesh effects' global defines.
    decode: Decode,
    /// Every composed entity's joint palette, end to end;
    /// actor rigs index it directly.
    pub palette: Vec<PaletteJoint>,
    pub meshes: Vec<ModelMesh>,
    pub materials: Vec<ModelMaterial>,
    pub textures: Vec<dds::Texture>,
    /// The colour space each of [`textures`](ModelLibrary::textures) uploads
    /// in, from the material slot that asked for it: the capture has every
    /// `pdxmesh` `DiffuseMap` created `..._UNORM_SRGB` and its `SpecularMap`
    /// and `NormalMap` created `..._UNORM` (`viewer_foundation::map::textures::ColorSpace`).
    pub spaces: Vec<ColorSpace>,
    /// Requested name to slot in [`meshes`](ModelLibrary::meshes).
    pub index: BTreeMap<String, usize>,
    /// Texture path and colour space to slot in
    /// [`textures`](ModelLibrary::textures), so the shared `city_d.dds` of
    /// sixteen sprawl meshes is uploaded once. The space is part of the key
    /// because one file read as a diffuse and as a specular is two textures.
    paths: BTreeMap<(String, ColorSpace), usize>,
    /// `.anim` path to its decoded rest pose; `None` for a file that does not
    /// read, so it is tried once. Every rifleman entity shares one idle file.
    animations: BTreeMap<String, Option<Arc<mesh::Animation>>>,
    /// Each `.anim` a descriptor names that neither stack reads, and why: its entity stands in
    /// its bind pose, reported, never taken for an entity without an animation.
    pub animation_refusals: BTreeMap<String, String>,
    rigs: Vec<Arc<Rig>>,
    /// The game imports each mesh type once; the game instantiates actors
    /// against it. Geometry is local to the mesh and shared by all its users.
    geometry_cache: BTreeMap<native::GeometryKey, Arc<super::mesh_shader::Geometry>>,
    attachment_refusals: BTreeMap<String, usize>,
    /// Each requested name that did not bake, and why.
    pub failed: BTreeMap<String, String>,
    /// Whether the entity under composition uses a file a visual mod supplies.
    composing_visual: bool,
    /// A visual mod's entity slot to its recorded counterpart's, drawn when the mod's draw is
    /// refused ([`ModelPass`]); the counterparts are in no name's slot.
    pub alternatives: BTreeMap<usize, usize>,
    /// Whether this library's entities stand in for the same names' entities they are merged
    /// with ([`Self::isolated`]).
    alternative: bool,
    asset_namespace: Option<Arc<[identity::ContentLayer]>>,
}
debug_fields!(ModelLibrary(this) {
    decode, palette, meshes, materials, textures, spaces, index, paths, animations, rigs, attachment_refusals, failed
} {
    geometry => this.geometry_cache
});

/// One entity under composition: actor descriptors, rig, reach and refusals.
#[derive(Default)]
struct Composition {
    native: Vec<native::Descriptor>,
    reach: Reach,
    rig: Rig,
    refusals: BTreeMap<String, usize>,
}

/// How far one composed entity, attachments included, reaches around and above
/// its own origin. The pass culls on the radius and no layer reads the height
/// for anything but reporting.
#[derive(Clone, Copy, Debug, Default)]
struct Reach {
    collision_unsupported: bool,
    collision_bounds: Option<([f32; 3], [f32; 3])>,
    radius: f32,
    height: f32,
    /// The farthest point from the origin, in every direction.
    bound: f32,
    /// The radius of the root entity's own geometry, attachments excluded.
    own_radius: f32,
    /// Attached entities composed in, at any depth.
    attachments: usize,
}

impl Reach {
    /// Widens by one sub-mesh's bounds, already placed by `transform`.
    ///
    /// The eight corners are transformed rather than the two extremes, because
    /// an attachment's placement rotates: a rifle's box turned 90 degrees about
    /// Y reaches where its unrotated box does not.
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

    /// Widens by one already-placed point.
    ///
    /// A skinned sub-mesh widens by its posed vertices rather than by its
    /// `[aabb]`, because the `[aabb]` is the **bind** box and a pose is not
    /// inside it: `cavalry_horse.mesh` is 6.6 wide in x bound and 16.0 posed,
    /// and `GER_infantry.mesh` is 5.46 bound and 2.55 posed. Culling on the
    /// bind box would clip a horse at the edge of the screen.
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

/// Entity shards one library build spreads over. Bounds total threads while
/// the document loader already fans out across loaders; file reads dominate
/// and parallelize cleanly.
const BAKE_SHARDS: usize = 4;

/// Source labels stay explicit across recorded retries and counterparts.
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
    /// The layers its entities were read through; none before the first.
    pub fn layers(&self) -> Option<&ResolvedLayers> {
        self.layers.as_deref()
    }

    pub fn rig(&self, slot: usize) -> &Rig {
        &self.rigs[slot]
    }

    /// A slot's rig, shared, and its default joint palette, for a player
    /// that runs outside the pass.
    pub fn rig_source(&self, slot: usize) -> (Arc<Rig>, Vec<PaletteJoint>) {
        let rig = Arc::clone(&self.rigs[slot]);
        let palette = self.palette[rig.start..rig.end].to_vec();
        (rig, palette)
    }

    /// A library with nothing baked yet whose later bakes load under `decode`.
    pub fn empty(decode: Decode) -> Self {
        Self {
            decode,
            ..Default::default()
        }
    }

    /// The client's texture decode this library loads under.
    pub fn decode(&self) -> Decode {
        self.decode
    }

    /// Resolves and loads every name, appending one warning per name that does
    /// not resolve rather than refusing the document.
    ///
    /// `gfx/entities` and `gfx/models` are outside the content coverage policy
    /// so every file here goes through
    /// [`ResolvedLayers::read_uncovered`] and is not hash-verified.
    pub fn load(
        layers: &ResolvedLayers,
        decode: Decode,
        descriptors: &Descriptors,
        names: &[String],
        warnings: &mut Vec<installed_content_core::Diagnostic>,
    ) -> io::Result<Self> {
        Self::load_with(layers, decode, descriptors, names, warnings, |_, _| None)
    }

    /// The same load with job-local baker CPU and caller join/merge scopes.
    /// Worker wall scopes overlap; their CPU is not included in caller CPU.
    pub(crate) fn load_profiled(
        layers: &ResolvedLayers,
        decode: Decode,
        descriptors: &Descriptors,
        names: &[String],
        warnings: &mut Vec<installed_content_core::Diagnostic>,
        spans: &mut BuilderSpans,
    ) -> io::Result<Self> {
        Self::load_with_spans(
            layers,
            layers,
            decode,
            descriptors,
            names,
            warnings,
            |_, _| None,
            spans,
            BakeLabels::SHOWN,
        )
    }

    /// Loads admitted names, applying their owner's declaration check to recorded retries
    /// and draw counterparts too. Other model owners keep their own admission rules.
    pub fn load_with(
        layers: &ResolvedLayers,
        decode: Decode,
        descriptors: &Descriptors,
        names: &[String],
        warnings: &mut Vec<installed_content_core::Diagnostic>,
        refused: fn(&mesh::Roster, &str) -> Option<String>,
    ) -> io::Result<Self> {
        Self::load_with_spans(
            layers,
            layers,
            decode,
            descriptors,
            names,
            warnings,
            refused,
            &mut BuilderSpans::new(false),
            BakeLabels::SHOWN,
        )
    }

    // Keep the owner's declaration check and profiling source explicit.
    #[allow(clippy::too_many_arguments)]
    fn load_with_spans(
        layers: &ResolvedLayers,
        session: &ResolvedLayers,
        decode: Decode,
        descriptors: &Descriptors,
        names: &[String],
        warnings: &mut Vec<installed_content_core::Diagnostic>,
        refused: fn(&mesh::Roster, &str) -> Option<String>,
        spans: &mut BuilderSpans,
        labels: BakeLabels,
    ) -> io::Result<Self> {
        // Stable de-duplication first: the serial loop skipped later
        // duplicates via the index, so keeping each name's first occurrence
        // bakes exactly the serial set in the serial order.
        let mut seen = BTreeSet::new();
        let unique: Vec<&String> = names
            .iter()
            .filter(|name| seen.insert(name.as_str()))
            .collect();
        let shards = unique.len().clamp(1, BAKE_SHARDS);
        // An empty list bakes an empty library: `chunks` needs a nonzero
        // size and then yields no shard.
        let chunk = unique.len().div_ceil(shards).max(1);
        let asset_namespace = identity::namespace_for(layers, session);
        let baked = std::thread::scope(|scope| {
            let mut handles = Vec::new();
            for shard_names in unique.chunks(chunk).collect::<Vec<_>>() {
                let mut worker = spans.fork();
                let asset_namespace = Arc::clone(&asset_namespace);
                handles.push(job_priority::spawn_in(scope, move || {
                    let baked = viewer_foundation::worker_priority::at_bake_priority(|| {
                        worker.measure(labels.worker, |_| {
                            let mut library = ModelLibrary {
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
            layers: Some(Arc::new(layers.clone())),
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
        // With visual mods mounted, an entity that does not bake is baked from the recording
        // instead, by the recording's own descriptors apart from the mod's files, and named;
        // and each entity a visual mod supplies keeps its recorded counterpart, baked the same
        // way, so a refused draw of the mod's entity draws the recorded one instead. The
        // recorded stack bakes no counterparts.
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
                    &roster.descriptors,
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
                        // Drawn from the recording: no longer a failure of this library.
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
                    &roster.descriptors,
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
                installed_content_core::DiagnosticKind::Refused,
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
                installed_content_core::DiagnosticKind::Refused,
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
                installed_content_core::DiagnosticKind::Refused,
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
                installed_content_core::DiagnosticKind::Warning,
                format!(
                    "{} geometry-less entities ({}); states and map events run without geometry.",
                    meshless.len(),
                    meshless.join("; ")
                ),
            ));
        }
        // The game logs a locator whose parent_joint the skeleton lacks and
        // keeps it in entity space; say so here.
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
            warnings.push((installed_content_core::DiagnosticKind::Warning, format!(
                "These entity locators name a parent_joint the mesh lacks and stay in entity space, as the engine keeps them: {}.",
                unjointed.into_iter().collect::<Vec<_>>().join(", ")
            )));
        }
        Ok(library)
    }

    /// `names` the visual mods' descriptors leave refused, though the recording's own descriptors
    /// do not (a mod's descriptor file that does not parse may declare them), baked from the
    /// recording instead, apart from the mod's files, each named; an empty library without
    /// visual mods.
    pub fn load_recorded(
        layers: &ResolvedLayers,
        decode: Decode,
        names: &[String],
        why: impl Fn(&str) -> String,
    ) -> io::Result<Self> {
        Self::load_recorded_profiled(layers, decode, names, why, &mut BuilderSpans::new(false))
    }

    /// Recorded-only bakes retain the same source labels, including worker CPU.
    pub(crate) fn load_recorded_profiled(
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
            &roster.descriptors,
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

    /// Whether a name resolved.
    /// Whether the entity in `slot` declares `state` on its root node
    /// (the game finds it), as the animation player looks it up.
    pub fn has_state(&self, slot: usize, state: &str) -> bool {
        self.rigs
            .get(slot)
            .and_then(|rig| rig.nodes.first())
            .is_some_and(|node| node.clips.iter().any(|clip| clip.state.name == state))
    }

    pub fn slot(&self, name: &str) -> Option<usize> {
        self.index.get(name).copied()
    }

    /// Whether the baked entity draws geometry, including its attached actors.
    pub fn has_geometry(&self, slot: usize) -> bool {
        self.meshes[slot]
            .native
            .iter()
            .any(|descriptor| descriptor.lods.iter().any(|lod| !lod.is_empty()))
    }

    // Canonical draw bytes retain authored order and replace merge-local slots with texture identities.
    pub fn native_draw_snapshot(&self, slot: usize, texture_identities: &[String]) -> Vec<u8> {
        draw_snapshot::snapshot(self, slot, texture_identities)
    }

    /// Merges a shard baked on another thread. Shards hold contiguous name
    /// ranges merged in order, so first-seen dedup matches the serial loop
    /// and every rebased index lands exactly where the serial bake put it.
    pub fn merge(&mut self, mut shard: ModelLibrary) {
        if self.layers.is_none() {
            self.layers = shard.layers.clone();
            self.decode = shard.decode;
        }
        // Textures first: materials reference their slots. First seen in
        // shard order wins, as serially.
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
        // Materials by value, first seen wins, as serially.
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
        // Animations by path, first wins, as serially.
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
                    // `rigid` is a palette index like `skin_base` and moves with it; leaving it
                    // behind makes `node.rigid - rig.start` underflow the moment a shard-built rig
                    // is evaluated, which is every frame with a running presentation clock.
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
                        .geometry_cache
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

    /// This library with its file caches renamed apart, so merging it shares no texture,
    /// animation or geometry with entities baked from another stack's files of the same path;
    /// `alternative` makes its entities the merged library's counterparts of the same names
    /// rather than their slots.
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
        self.geometry_cache = std::mem::take(&mut self.geometry_cache)
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

    /// Adds one entity, or on failure removes everything its composition
    /// added so far, attachments included, so no partial rig or LOD survives.
    /// Decoded textures stay cached: another entity may use them.
    fn add(
        &mut self,
        layers: &ResolvedLayers,
        descriptors: &Descriptors,
        name: &str,
    ) -> io::Result<()> {
        let marks = (self.palette.len(), self.materials.len());
        let mut geometry_added = Vec::new();
        let added = self.compose_entity(layers, descriptors, name, &mut geometry_added);
        if added.is_err() {
            self.palette.truncate(marks.0);
            self.materials.truncate(marks.1);
            for key in geometry_added {
                self.geometry_cache.remove(&key);
            }
        }
        added
    }

    fn compose_entity(
        &mut self,
        layers: &ResolvedLayers,
        descriptors: &Descriptors,
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

    /// Builds one entity and its attached actor nodes against shared mesh geometry.
    ///
    /// `placement` carries the composed joint transforms of the rig chain above
    /// this entity, already in the **root** entity's own object space and
    /// before the root's own scale. The engine composes the same chain per
    /// frame from the posed skeleton; geometry stays mesh-local.
    /// Returns the root's mesh path, its world scale and its LOD distances.
    #[allow(clippy::too_many_arguments)] // Recursive rig traversal carries its parent placement.
    fn compose(
        &mut self,
        layers: &ResolvedLayers,
        descriptors: &Descriptors,
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
            .entities
            .get(name)
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
        // Supply a dummy object when no binding survives;
        // The game still runs its states and events through the same player.
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
                super::mesh_pose::binary_locators(&tree, &skeleton, entity.definition.scale)
                    .map_err(cannot_load)?;
            // Project only after the native skeleton and locators have validated the same tree.
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
        // The entity's own vertices sit at `placement * scale`.
        let own = placement.compose(&mesh::Transform {
            basis: [[scale, 0.0, 0.0], [0.0, scale, 0.0], [0.0, 0.0, scale]],
            translation: [0.0; 3],
        });
        // The pose the entity's selected default state holds. `None` leaves
        // every sub-mesh in its bind pose. Building damaged-state aliases
        // select their authored animation through the same descriptor path.
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
        // Some attachments have no default animation but do animate on firing.
        // Retain their influences without changing the frozen baked geometry.
        if skin_base.is_none() {
            skin_base = clips
                .iter()
                .find_map(|clip| self.push_palette(&decoded, clip.animation.as_deref(), &own));
        }
        let node_index = out.rig.nodes.len();
        out.rig.nodes.push(entity_animation::Node {
            mesh: Arc::new(skeleton_source::skeleton(&decoded)),
            native: Some(Arc::new(super::mesh_pose::Source {
                pdxmesh: entity.is_some(),
                entity_locators: descriptors.entities.get(name).map_or_else(
                    Vec::new,
                    |definition| {
                        super::mesh_pose::entity_locators(
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
            let mut geometry_cache = std::mem::take(&mut self.geometry_cache);
            let descriptor = native::descriptor(
                &decoded,
                &resolved,
                node_index,
                name,
                |key, sub| {
                    if let Some(geometry) = geometry_cache.get(&key) {
                        return Ok(Arc::clone(geometry));
                    }
                    let geometry = Arc::new(super::mesh_shader::Geometry::from_mesh(
                        sub,
                        resolved.definition.scale,
                    )?);
                    geometry_cache.insert(key.clone(), Arc::clone(&geometry));
                    geometry_added.push(key);
                    Ok(geometry)
                },
                |shape, sub| self.material(layers, directory, shape, sub, &resolved),
            );
            self.geometry_cache = geometry_cache;
            out.native.push(descriptor?);
            let shape_lods = decoded.lods();
            // Static-resource spheres and multiple-LOD unions have a different construction.
            // Do not substitute a runtime-bounds sphere for those forms.
            out.reach.collision_unsupported |= !frozen_skin || shape_lods.len() != 1;
            // Any LOD may be drawn, so culling reaches around them all. Only the first feeds
            // the collision sphere, which is unsupported with several anyway.
            for (lod, shapes) in shape_lods.iter().enumerate() {
                for (_, sub) in shapes
                    .iter()
                    .flat_map(|shape| shape.meshes.iter().map(|sub| (shape.name.as_str(), sub)))
                    .filter(|(_, sub)| super::mesh_effect::candidate(sub))
                {
                    if lod == 0 {
                        // Runtime bounds transform each bone's bind-space box, rather than a
                        // box fitted to the weighted, posed vertices. The sphere is half the
                        // resulting bounds' diagonal. Keep this independent of culling reach.
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
            // The attachments are collected before the recursion so the
            // borrow of `descriptors` through `entity` ends first.
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
                super::mesh_pose::actors(&out.rig, None, true)
            };
            for (joint_name, child, handle) in attachments {
                if !descriptors.entities.contains_key(&child) {
                    *out.refusals.entry(format!("{name} -> {child}: attachment child has no registered entity definition")).or_default() += 1;
                    continue;
                }
                let source = out.rig.nodes[node_index].native.as_ref().unwrap();
                let mode = if descriptors.resolve_entity(&child).is_some() {
                    super::mesh_pose::LookupMode::Attachment
                } else {
                    super::mesh_pose::LookupMode::AttachmentDummy
                };
                let local = match super::mesh_pose::lookup(source, &joint_name, mode) {
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
                        if mode == super::mesh_pose::LookupMode::Attachment {
                            *out.refusals
                            .entry(format!(
                                "{name} -> {child}: missing attachment point {joint_name}; native parent-root fallback"
                            ))
                            .or_default() += 1;
                        }
                        super::mesh_pose::IDENTITY
                    }
                };
                let world = match &actors[node_index].world {
                    Ok(world) => super::mesh_pose::multiply(*world, local),
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
                // A joint the animation scales to zero is the engine's own
                // switch for an attachment it is not showing.

                // A refused child leaves its parent drawn, but no partial
                // child rig, palette, geometry or reach survives.
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
                            self.geometry_cache.remove(&key);
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

    /// Appends one composed entity's joint palette and returns the slot its
    /// joint 0 landed on, or `None` when the entity has no pose to skin by.
    ///
    /// `own` is folded in, so a palette entry maps a bind-pose vertex of this
    /// entity straight into the **root** entity's object space, attachment
    /// chain and world scale included.
    ///
    /// An entity whose default state resolves no `.anim` gets no palette at
    /// all, rather than one of identity matrices. The two are the same picture
    /// â€” a skinning matrix is `pose * inverse_bind` and the bind pose's own
    /// composition is the identity â€” but only the first is the *same bits*, and
    /// the sprawl, the map buildings and the map frame are all in that case.
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

    /// One `.anim`'s rest pose, read once per file.
    fn animation(&mut self, layers: &ResolvedLayers, path: &str) -> Option<&Arc<mesh::Animation>> {
        self.composing_visual |= layers.supplied_by_visual_mod(path);
        if !self.animations.contains_key(path) {
            let read = |stack: &ResolvedLayers| {
                mesh::read_animation_in(stack, path).map_err(|error| error.to_string())
            };
            let decoded = match read(layers) {
                Ok(animation) => Some(animation),
                Err(why) => {
                    // An animation the visual mods break is the recording's own, named; one
                    // neither stack reads is refused by name.
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

    /// A material's texture at the one path the engine requests
    /// ([`installed_content_core::mesh::TextureIndex`]); the texture constructor adopts
    /// the `.dds` spelling of a `.tga` path when it exists. A texture that
    /// resolves nowhere refuses the mesh. An empty name is a blank texel.
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
        self.composing_visual |= layers.supplied_by_visual_mod(&path);
        let key = (path, space);
        if let Some(slot) = self.paths.get(&key) {
            return Ok(*slot);
        }
        let texture = load_file(
            &key.0,
            &|path| layers.read_uncovered_path(path),
            self.decode,
        )?;
        self.textures.push(texture);
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
        self.textures.push(dds::Texture::from_rgba8(Rgba8Image {
            width: 1,
            height: 1,
            pixels: vec![0, 0, 0, 0],
        }));
        self.spaces.push(space);
        let slot = self.textures.len() - 1;
        self.paths.insert(key, slot);
        slot
    }
}

/// An entity-file locator in the node's frame, whose scale includes the
/// pdxmesh scale. The engine bakes that scale into the mesh at import
/// so the entity's world matrix lacks it. Entity updates place
/// an entity locator's particles at `locator * that matrix`: position, size
/// and emitter spread all without the pdxmesh scale.
fn entity_locator(locator: &mesh::Transform, mesh_scale: f32) -> mesh::Transform {
    mesh::Transform::from_trs(
        [0.0; 3],
        [0.0, 0.0, 0.0, 1.0],
        1.0 / mesh_scale.max(f32::EPSILON),
    )
    .compose(locator)
}

/// Where one vertex lands, the way the pdxmesh skinning puts it there: a linear
/// blend of the palette matrices its four slots name, or its own position when
/// it carries no weight at all.
///
/// The pass only needs this on the CPU for the cull radius, but it is the same
/// arithmetic, so a figure that is culled wrong is a figure that is drawn
/// wrong.
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

// ---------------------------------------------------------------- the field

/// One group of instances of one mesh, the pass's cull and LOD granularity.
#[derive(Clone, Debug, PartialEq)]
pub struct ModelGroup {
    pub mesh: usize,
    pub asset: AssetKey,
    /// Slice of [`ModelField::instances`].
    pub range: Range<u32>,
    /// World bounds of the group's instances, `[x0, z0, x1, z1]`, already
    /// widened by the mesh radius.
    pub bounds: [f32; 4],
    /// Centroid used for the LOD decision.
    pub centroid: [f32; 3],
}

/// A placed instance keyed by its mesh and cull tile, as [`ModelField::rebuild`] sorts it.
pub type KeyedInstance = ((usize, i32, i32), ModelInstance, usize);

/// The placed instances of one layer.
#[derive(Clone, Debug, Default)]
pub struct ModelField {
    pub instances: Vec<ModelInstance>,
    pub groups: Vec<ModelGroup>,
    /// (group, instance) in submission order, before the port's mesh/tile sort.
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
    /// This frame's default-state actors and the state request each plays
    /// ([`ModelPass::prepare_default_states`]).
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

    /// Sorts placed instances into one group per (mesh, tile) and records each
    /// group's bounds.
    ///
    /// `tile` is the side of the cull grid in world units. The engine buckets
    /// its trees on 256-unit tiles; no recorded input ties
    /// the other layers to a particular size, so this is the viewer's own cull
    /// and not a claim about the engine.
    pub fn build(library: &ModelLibrary, placed: Vec<(usize, ModelInstance)>, tile: f32) -> Self {
        Self::build_meshes(&library.meshes, placed, tile)
    }

    /// [`Self::build`] from the mesh metadata a pass keeps.
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

    /// [`Self::build`] into this field, reusing its storage: a per-frame
    /// layer keeps `placed`, `keyed` and the field between frames. `placed`
    /// is drained; `keyed` is scratch.
    pub fn rebuild(
        &mut self,
        library: &ModelLibrary,
        placed: &mut Vec<(usize, ModelInstance)>,
        keyed: &mut Vec<KeyedInstance>,
        tile: f32,
    ) {
        self.rebuild_meshes(&library.meshes, placed, keyed, tile);
    }

    /// [`Self::rebuild`] from the mesh metadata a pass keeps.
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
        // Binding slots never decide storage or submission order.
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
        // Order by asset, unknown meshes first by slot, then tile and placement; each placed
        // mesh's asset is ranked once rather than compared per instance pair.
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
            // Equal effective assets and tiles retain producer placement order.
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

/// World units a figure standing above the water plane leans past its own ground position, per
/// unit of its height: the cotangent of the shallowest pitch the game's camera allows (53.5
/// degrees below the horizon), rounded up. The visible rectangle is already grown by
/// [`crate::map::terrain::MARGIN`], which is the lean of everything that stands on the ground, so
/// only what flies higher than that carries a lean of its own.
const LEAN: f32 = 0.75;

/// How far past its ground position a figure at `height` can appear, beyond the margin the
/// visible rectangle already carries.
fn lean(height: f32) -> f32 {
    (height.max(0.0) * LEAN - crate::map::terrain::MARGIN).max(0.0)
}

/// The world bounds, `[x0, z0, x1, z1]`, of `instances` of an entity reaching `radius` around
/// its origin at their scale, widened where an instance stands above the plane by how far it can
/// lean past its own ground position: the ground rectangle must never drop a figure the game's
/// own cull admits.
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

/// A deterministic hash of an index to `[0, 1)`, for the rotations and mesh
/// choices the engine makes from unrecorded random state.
///
/// Remaining callers lack recorded random state.
/// Sprawl uses its recovered province/x/z mixer in `sprawl.rs`.
pub fn hash01(index: u32) -> f32 {
    let mut x = index.wrapping_mul(0x9E37_79B1);
    x ^= x >> 16;
    x = x.wrapping_mul(0x85EB_CA6B);
    x ^= x >> 13;
    x = x.wrapping_mul(0xC2B2_AE35);
    x ^= x >> 16;
    // 2^24 keeps every value exactly representable in f32.
    (x >> 8) as f32 / 16_777_216.0
}

/// A deterministic rotation in `[0, TAU)`.
pub fn hash_rotation(index: u32) -> f32 {
    hash01(index) * TAU
}

// ------------------------------------------------------------------- the pass

/// What one frame of the pass needs beyond its own geometry.
#[derive(Clone, Copy)]
pub struct ModelFrame<'a> {
    pub camera: Camera,
    /// World x translations of the wrap copies to submit.
    pub offsets: &'a [f32],
    /// The visible ground rectangle groups are culled against: the padded one a kept list is
    /// admitted against ([`viewer_foundation::map::camera::Hold`]).
    pub visible: Option<[f32; 4]>,
    /// The ground rectangle the frame's own camera reaches, which the judge holds a kept list
    /// to: everything it admits the list must hold.
    pub seen: Option<[f32; 4]>,
    /// Layer opacity. Every gate in `00_graphics.lua` is a hard switch rather
    /// than a fade, so this is 1.0 for the mesh layers; the railway layer has
    /// a real fade and does not use this pass.
    pub alpha: f32,
}

/// The game's per-figure cull for one frame, for the lit figures and the shadow casters alike: the
/// camera's eye and its four side planes, each of unit normal and inside non-negative. A figure
/// is drawn only when its bounding sphere's centre lies nearer the eye than its mesh's cull
/// distance and the sphere is inside all four planes: no near or far plane, as the game tests.
/// `x` is the x of the camera it was made for: a traversal whose camera stands on another map copy
/// moves its figures onto this one before the test ([`ModelPass::game_culled`]).
/// `pad` moves every side out and the cull distance further by that many world units, so a camera
/// that travels up to `pad` from this one culls in nothing this cull leaves out
/// ([`viewer_foundation::map::camera::Hold`]).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GameCull {
    pub eye: [f32; 3],
    pub sides: [[f32; 4]; 4],
    pub x: f32,
    pub pad: f32,
}

/// Why a figure is not drawn by the game's cull.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Culled {
    Distance,
    Planes,
}

impl GameCull {
    /// The frame's cull for `camera` seen through `viewport`.
    pub fn of(camera: &Camera, viewport: [u32; 2]) -> Self {
        Self::padded(camera, viewport, 0.0)
    }

    /// [`Self::of`] with every side and the cull distance `pad` further out.
    pub fn padded(camera: &Camera, viewport: [u32; 2], pad: f32) -> Self {
        let planes = view_cone(camera, viewport);
        Self { eye: camera.eye(), sides: [planes[0], planes[1], planes[2], planes[3]], x: camera.center[0], pad }
    }

    /// Whether a figure whose bounding sphere has `centre` and `radius`, of a mesh culled beyond
    /// `cull_distance_squared`, is culled, and why.
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

/// The view volume's planes, inside non-negative and each of unit normal, so a sphere's centre
/// stands its radius outside a plane it is wholly behind.
pub fn view_cone(camera: &Camera, viewport: [u32; 2]) -> [[f32; 4]; 6] {
    camera.frustum_planes(viewport).map(|[a, b, c, d]| {
        let length = (a * a + b * b + c * c).sqrt().max(f32::MIN_POSITIVE);
        [a / length, b / length, c / length, d / length]
    })
}

/// One frame's refused actors by instance and node, with the reason.
type ActorRefusals = BTreeSet<(usize, usize, String)>;
type ActorPoses = Arc<[super::mesh_pose::Actor]>;

/// A producer clock whose requests and origin stay unchanged between presentation frames.
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
    /// Source indices in key order, retaining the last occurrence of duplicate keys.
    order: Vec<usize>,
}

#[cfg(test)]
mod clock_record_tests {
    use super::*;

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
                    // The ordinary installer must invalidate retained input, even for equal keys.
                    pass.set_clocks(BTreeMap::new());
                    assert!(pass.clock_records.is_none());
                } else if frame.is_multiple_of(5) {
                    // Stand in for defaults or another field owner overwriting producer clocks.
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
            // Duplicate original keys resolve last, before one-hop redirects resolve
            // collisions in original key order. No alternate pass is needed.
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
                        .map(|clock| clock as *const _ as usize)
                        .collect::<Vec<_>>()
                };
                let before = addresses();
                retained.set_clock_records(&records, time);
                let after: Vec<_> = retained
                    .field
                    .clocks
                    .values()
                    .map(|clock| clock as *const _ as usize)
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
    /// Uploaded when a resident mesh first needs one.
    textures: Vec<Option<Bound>>,
    materials: Vec<ModelMaterial>,
    /// Whether a material's textures are uploaded.
    ready: Vec<bool>,
    /// Meshes whose every material, and counterpart's, is uploaded or refused.
    resident: Vec<bool>,
    /// Whether shader and pipeline builds run in place, as a capture needs, rather than off the
    /// window thread while the frame draws without the part.
    settled: Cell<bool>,
    /// Meshes whose builds are made or under way ([`ModelPass::warm_pipelines`], [`ModelPass::warm_added`]).
    warmed: Cell<usize>,
    material_refusals: BTreeMap<usize, String>,
    meshes: Vec<ModelMesh>,
    field: ModelField,
    clock_records: Option<ClockRecords>,
    clock_routes: ClockRoutes,
    redirect_placed: Vec<(usize, ModelInstance)>,
    animation_dispatch: animation_dispatch::Dispatch,
    /// Placement and live-player keys, rebuilt in retained storage each frame.
    keyed: Vec<KeyedInstance>,
    alive: Vec<(usize, u32, u32)>,
    drawn: Cell<u32>,
    /// This frame's game cull, set by the scene ([`ModelPass::set_cull`]): the padded one every
    /// list is admitted against.
    cull: Cell<Option<GameCull>>,
    /// The frame's own camera's cull, which the judge holds a kept list to: every figure it
    /// admits the list must hold.
    seen: Cell<Option<GameCull>>,
    refusals: RefCell<(u64, ActorRefusals)>,
    /// Actor refusals made, ever: a caster walk that made one is walked again.
    refused: Cell<u64>,
    /// Whether the last frame's casters placed any figure.
    cast: Cell<bool>,
    caster: crate::map::shadow::CasterId,
    /// Raised by every change a caster walk reads beyond the camera, actor poses and the
    /// figures placed: hidden actors, redirects, the content stack, a library that did more than
    /// grow, too many actors given a player unread.
    revision: u64,
    /// Actors given a player since the caster list last read them: each such figure is walked
    /// again alone, its pose now moving.
    born: RefCell<Vec<(usize, u32, u32)>>,
    /// The field as the caster list last read it.
    walked: RefCell<WalkedField>,
    /// Whether a writer placed the field since the caster list last read it.
    written: Cell<bool>,
    /// The field's change for this frame's retained lists.
    frame_change: RefCell<FrameChange>,
    /// Actor poses by instance for one arena frame: animation happens before encoding, so
    /// the caster, regular and alternate queues and every wrap view share them.
    poses: RefCell<(u64, Vec<Option<ActorPoses>>)>,
    /// This pass among the scene's, which the pose worker's batches are keyed by.
    id: u64,
    /// The animation history this pass stands in, raised whenever one starts again: a pose batch
    /// in flight for the history that ended is of other samples, so it is never installed over
    /// the new one's figures ([`Self::reset_animation`]).
    history: u64,
    /// Whether this pass's figures are posed on the frame that draws them rather than taken from
    /// the worker ([`Self::pose_on_the_frame`]).
    pose_on_the_frame: bool,
    /// The poses the worker last finished for this pass, and which of its sets it is: a frame
    /// takes them by this swap and never waits ([`crate::map::poses`]). An actor the set does
    /// not hold is posed where it is read.
    posed: Arc<super::poses::Set>,
    /// Raised by every set installed: the rows of standing figures are written again when it
    /// moves, and not otherwise.
    pose_epoch: u64,
    /// Actor-local results for an immutable rig without a live player, keyed by
    /// mesh slot and whether its frozen animation is played. Placements are separate.
    static_actors: RefCell<BTreeMap<(usize, bool), ActorPoses>>,
    label: &'static str,
    rigs: Vec<Arc<Rig>>,
    default_palette: Vec<PaletteJoint>,
    players: BTreeMap<(usize, u32, u32), entity_animation::Player>,
    /// When each default-state actor appeared on the presentation clock
    /// ([`ModelPass::prepare_default_states`]).
    default_appeared: BTreeMap<(usize, u32, u32), f32>,
    /// Each mesh's default-state play, worked out when its first actor needs it.
    default_plays: Vec<Option<(Option<Arc<Rig>>, Option<DefaultPlay>)>>,
    /// This frame's default-state actors refused, by why, with their count.
    pub default_state_refusals: BTreeMap<String, usize>,
    pub events: Vec<entity_animation::Event>,
    event_namespace: u64,
    /// Placed actors, by animation key, that keep animating but draw and cast nothing.
    hidden: BTreeSet<(usize, u32, u32)>,
    hidden_actors: BTreeSet<ActorAddress>,
    /// The library's visual mod entities and their recorded counterparts
    /// ([`ModelLibrary::alternatives`]).
    alternatives: BTreeMap<usize, usize>,
    /// Mod entities drawn from their recorded counterparts since a draw was refused.
    redirects: BTreeMap<usize, usize>,
    /// Mod entities refused this frame that have a counterpart, with why; redirected before
    /// the next frame animates ([`Self::fall_back`]).
    falling_back: RefCell<BTreeMap<usize, String>>,
    /// Each redirected entity's name and why, for its warning line.
    from_recording: BTreeMap<String, String>,
    /// Whether its lit queues are drawn straight into a pass of their own, rather than kept as
    /// retained lists: such a pass stages each draw's constants in the frame's arena and so
    /// reads none of them per instance ([`ModelPass::drawn_directly`]).
    direct: bool,
}

fn event_namespace() -> u64 {
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
    NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
}

impl ModelPass {
    #[cfg(test)]
    pub(crate) fn groups_for_tests(&self) -> &[ModelGroup] {
        &self.field.groups
    }

    /// The entity player of a placed instance, under the key every producer
    /// and draw shares ([`ModelInstance::animation_key`]).
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
        use viewer_foundation::view_record::add;
        // What a figure's actors cost this frame: taken from the worker's set, evaluated here,
        // held from an earlier frame (its samples draw the same), or its mesh's static actors.
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
                .or_insert_with(|| super::mesh_pose::actors(rig, None, frozen_animation)),
        )
    }

    /// Every placed actor whose group the view reaches, as this frame shows it: its player's
    /// key, its root on the map copy the view reaches, and a number that changes when its play's
    /// samples do. The first gate's count reads it ([`crate::map::actor_motion`]).
    pub(crate) fn shown_actors(
        &self,
        rectangle: Option<[f32; 4]>,
        offsets: &[f32],
        into: &mut Vec<crate::map::actor_motion::Shown>,
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
            into.push(crate::map::actor_motion::Shown {
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

    /// The player an instance placed in `mesh` is animated by ([`ModelInstance::animation_key`]).
    /// Where this frame poses the root entity's skeleton joints of a placed instance, in its own
    /// space after its entity scale, before the instance's scale and place: what the engine's
    /// animated bounds are made of. Empty for an entity without a skeleton; `None` when the
    /// instance has no player this frame, so nothing of it is posed.
    pub fn root_joints(&self, mesh: usize, instance: &ModelInstance) -> Option<Result<Vec<[f32; 3]>, String>> {
        let slot = self.drawn_slot(mesh);
        let rig = self.rigs.get(slot)?;
        let player = self.player(mesh, instance)?;
        let root = rig.nodes.first()?;
        if root.native.as_ref().is_none_or(|source| source.skeleton.is_empty()) {
            return Some(Ok(Vec::new()));
        }
        if !(root.mesh_scale > 0.0) {
            return Some(Err(format!("unit model {mesh} has no pdxmesh scale to take its joints out of")));
        }
        let entity = root.scale / root.mesh_scale;
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

    /// This pass and the animation history it stands in, which the pose worker's jobs and sets
    /// are keyed by ([`crate::map::poses::Pass`]).
    pub(crate) fn pose_pass(&self) -> super::poses::Pass {
        (self.id, self.history)
    }

    /// Whether this pass's figures are posed on the frame that draws them, from exactly the
    /// samples it draws, rather than taken from the worker's newest set by a swap: a stopped
    /// picture (a saved frame, a picture the hours have stopped on) is exact, and the worker's
    /// set is of the frame that asked for it ([`crate::map::poses`]). Turning it on drops the set
    /// standing, so nothing of an earlier frame is left to draw.
    pub(crate) fn pose_on_the_frame(&mut self, on: bool) {
        if self.pose_on_the_frame == on {
            return;
        }
        self.pose_on_the_frame = on;
        if on {
            self.install_poses(Default::default());
        }
    }

    /// Takes `made`'s set for this pass, which it holds only when the batch was made for the
    /// animation history the pass stands in now. Nothing while its figures are posed on the
    /// frame that draws them.
    pub(crate) fn take_poses(&mut self, made: &super::poses::Made) {
        if self.pose_on_the_frame {
            return;
        }
        self.install_poses(made.set_for(self.pose_pass()));
    }

    /// Every pose this pass's placed actors need, for the worker to make: one job a placement.
    /// An actor placed many times is asked for many times and posed once, by the worker, so the
    /// frame does not pay to find that out. An actor whose player holds samples of another rig
    /// is left to be posed where it is read, as one with no player is. A pass whose figures are
    /// posed on the frame asks for none.
    ///
    /// `rectangle` is the ground this frame admits its figures against and `offsets` its wrap
    /// copies ([`ModelFrame`]): an actor whose group no copy of the view reaches is not asked for,
    /// because no draw of this frame reads its limbs. One the camera reaches before the worker
    /// has it is posed where it is read, as a figure an hour has just brought is.
    pub(crate) fn pose_jobs(
        &self,
        into: &mut super::poses::Jobs,
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

    /// Takes a finished set of poses: the worker's, or an empty one where the poses are made on
    /// the frame. Every standing figure's row is written again for the set once, and not again
    /// until the next one.
    pub(crate) fn install_poses(&mut self, posed: Arc<super::poses::Set>) {
        self.posed = posed;
        self.pose_epoch += 1;
    }

    /// The bound judge of this pass's poses ([`crate::map::poses::judge`]).
    pub(crate) fn judge_poses(&self, animation_time: f32, interval: f32) {
        super::poses::judge(self.label, &self.posed, animation_time, interval);
    }

    /// Which of the renderer's shader libraries the entity in `mesh` draws with: 1, the
    /// recording's own, for a visual mod entity's recorded counterpart, so it draws as the
    /// recording draws it; 0, the stack it was baked from, for every other.
    fn shader_library(&self, mesh: usize) -> usize {
        usize::from(self.alternatives.values().any(|&slot| slot == mesh))
    }

    /// The slot an entity placed in `mesh` draws from: its recorded counterpart once the mod's
    /// draw was refused ([`Self::fall_back`]).
    fn drawn_slot(&self, mesh: usize) -> usize {
        self.redirects.get(&mesh).copied().unwrap_or(mesh)
    }

    /// The root play position of the player keyed `(mesh, seed, identity)`
    /// ([`entity_animation::Player::root_play_position`]): `Some(None)` when
    /// its root plays no animation, including an entity without any clip,
    /// which never gets a player; `None` before an animated entity's player
    /// exists.
    pub fn root_play_position(&self, key: (usize, u32, u32)) -> Option<Option<(f32, f32)>> {
        let key = (self.drawn_slot(key.0), key.1, key.2);
        let rig = self.rigs.get(key.0)?;
        if rig.nodes.iter().all(|node| node.clips.is_empty()) {
            return Some(None);
        }
        Some(self.players.get(&key)?.root_play_position(rig))
    }

    /// Installs producer-owned state requests on the shared entity player.
    pub fn set_clocks(&mut self, clocks: BTreeMap<(usize, u32, u32), entity_animation::Clock>) {
        self.clock_records = None;
        self.install_clocks(clocks, false);
    }

    pub(crate) fn clock_actor_addresses(&self) -> impl Iterator<Item = ActorAddress> + '_ {
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

    /// Replaces all producer clocks, retaining their key order and request storage while the
    /// records are shared. Times use the same single f32 subtraction as ordinary producer clocks.
    /// Duplicate keys retain the last source record, just as collecting them into a map does.
    pub fn set_clock_records(&mut self, records: &Arc<[ClockRecord]>, time: f32) {
        let _span = viewer_foundation::profiling::span("set_clock_records");
        viewer_foundation::view_record::add(self.label, "clock_records", records.len() as u64);
        if !self.redirects.is_empty() {
            // Collect duplicate producer keys first, then resolve redirects in producer order.
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
        // A placement rebuild, default-state insertion or another field owner may have changed
        // the clock map since the last install. Replace it completely, not just the reached keys.
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

    /// Samples visible entity palettes; meshes and material buffers are immutable.
    /// Placements, event packets and palette ranges keep their traversal order.
    pub fn animate(&mut self, time: f32, rectangle: Option<[f32; 4]>, offsets: &[f32]) {
        self.fall_back();
        static PROFILE: OnceLock<bool> = OnceLock::new();
        let profile = *PROFILE.get_or_init(|| std::env::var_os("BATTLE_PLAN_ANIMATION_PROFILE").or_else(|| std::env::var_os("HOI4_ANIMATION_PROFILE")).is_some());
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
        if viewer_foundation::view_record::on() {
            use viewer_foundation::view_record::add;
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
        // Never read while the casters are not drawn: past one a figure, it is a whole change.
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

    /// Turns this frame's [`entity_animation::DEFAULT_STATE`] actors into
    /// requests of their entity's `default_state` on a clock from when each
    /// appeared: the game creates every entity instance in that state, and
    /// `use_animation = no` only spares an ambient object the `idle` request
    /// that would follow. A state with one variant is entered whatever its
    /// chance, as the game's state setter draws only among several. An actor
    /// that can reach a state with several variants, on its root or an
    /// attached entity, stays still and is counted in
    /// [`ModelPass::default_state_refusals`]: the game draws that variant from
    /// a random counter that every earlier draw for the entity has advanced
    /// since the session began, which no recording holds. Returns each
    /// played actor's state request; it chooses states by the game's rule.
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
            // A mesh's play is made from the rig the pass holds for it, and again when it holds another.
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
            // A presentation clock that starts again starts the state with it.
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

    /// Moving layers retain mesh buffers and replace only their small instance stream.
    /// A seek or a branch change starts new entity histories.
    pub fn reset_animation(&mut self) {
        self.event_namespace = event_namespace();
        self.history += 1;
        self.refresh_hidden_actors();
        self.players.clear();
        self.default_appeared.clear();
        self.events.clear();
        // A new history poses nothing yet: the worker's set, and any batch still in flight, are
        // of the history that ended, and its generation leaves them behind.
        self.install_poses(Default::default());
    }

    /// A display frame's placements where figures move between the ones their hours give: the
    /// whole field where [`Moving::standing`] carries it, then the parts the frame moves written
    /// at their own place in the field, which a frame that moves none leaves as it stands.
    pub fn update_moving(&mut self, placed: Moving, tile: f32) {
        match placed.standing {
            Some(standing) => self.update_placed(standing, tile),
            // The field stands; what a placement makes afresh for the frame does not.
            None => {
                self.events.clear();
                self.field.default_states.clear();
            }
        }
        self.move_placed(&placed.moved);
    }

    /// Writes `moved`'s parts, each at its own ordinal in the field's walk order, and the bounds
    /// and centroid of the groups they stand in. A part's group, its order, its actor and its
    /// pose are its hour's and do not move with it, so nothing of them is made again.
    fn move_placed(&mut self, moved: &[(usize, ModelInstance)]) {
        if moved.is_empty() {
            return;
        }
        let _span = viewer_foundation::profiling::span("model_move_placed");
        let mut groups: Vec<usize> = Vec::with_capacity(moved.len());
        for &(ordinal, instance) in moved {
            let Some(&(group, index)) = self.field.native_order.get(ordinal) else {
                continue;
            };
            let Some(held) = self.field.instances.get_mut(index) else {
                continue;
            };
            // Animation's own palette offset is this frame's; a root that moves does not move it.
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
        viewer_foundation::view_record::add(self.label, "field_moved_parts", moved.len() as u64);
        self.written.set(true);
    }

    /// Replaces placements while preserving the players of surviving actors.
    pub fn update_placed(&mut self, mut placed: Vec<(usize, ModelInstance)>, tile: f32) {
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

    /// Installs a producer's whole field, which needs no GPU; `field` receives the previous one,
    /// so the producer can reuse its buffers. Its meshes are made resident before they draw
    /// ([`Self::make_field_resident`]).
    pub fn replace_field(&mut self, library: &ModelLibrary, field: &mut ModelField) {
        self.sync(library);
        // Clock caches belong to the pass; installs validate their dependency values.
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

    /// Uploads what the installed field's meshes need to draw.
    pub fn make_field_resident(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        library: &ModelLibrary,
    ) -> io::Result<()> {
        let meshes: BTreeSet<usize> = self.field.groups.iter().map(|group| group.mesh).collect();
        for mesh in meshes {
            self.make_resident(device, queue, library, mesh)?;
        }
        Ok(())
    }

    pub fn new(library: &ModelLibrary, label: &'static str) -> Self {
        Self {
            native: RefCell::new(None),
            native_walk: Default::default(),
            layers: library.layers.clone(),
            decode: library.decode,
            textures: (0..library.textures.len()).map(|_| None).collect(),
            materials: library.materials.clone(),
            ready: vec![false; library.materials.len()],
            resident: Vec::new(),
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
            caster: crate::map::shadow::CasterId::next(),
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

    /// The pass's lit queues are drawn straight into a pass of their own, from constants staged
    /// in the frame's arena: its lit draws read nothing per instance. Its casters are retained as
    /// every pass's are, and read their rows per instance.
    pub fn drawn_directly(&mut self) {
        self.direct = true;
    }

    /// A pass over `rigs` with no meshes, for stepping animation events without a GPU.
    #[cfg(test)]
    pub(crate) fn with_rigs_for_tests(rigs: Vec<Arc<Rig>>, label: &'static str) -> Self {
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
                cull_distance_squared: installed_content_core::mesh::DEFAULT_CULL_DISTANCE_SQUARED,
            });
        }
        let mut pass = Self::new(&library, label);
        pass.rigs = rigs;
        pass.default_palette = vec![PaletteJoint::IDENTITY];
        pass
    }

    /// The installed field and every player's root play position, for comparing two passes
    /// stepped alike.
    #[cfg(test)]
    pub(crate) fn presentation_state(&self) -> impl PartialEq + std::fmt::Debug {
        (
            self.field.clone(),
            self.players
                .keys()
                .map(|key| (*key, self.root_play_position(*key)))
                .collect::<Vec<_>>(),
        )
    }

    /// Placed actors, keyed `(mesh, seed, identity)`, that keep their players, clocks and
    /// events but neither draw nor cast shadows until shown again.
    pub fn set_hidden(&mut self, hidden: BTreeSet<(usize, u32, u32)>) {
        if hidden != self.hidden {
            self.raise();
        }
        self.hidden = hidden;
        self.redirect_field();
        self.refresh_hidden_actors();
        self.native_walk.set_hidden(&self.hidden);
    }

    /// Resolves a producer's binding handle to its live pass-local actor address.
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

    /// Whether an actor is placed but hidden.
    pub fn hides(&self, key: ActorAddress) -> bool {
        self.hidden_actors.contains(&key)
    }

    /// Follows a library that grew since the pass last saw it (the unit
    /// library bakes entities as hours first place them). Every library only
    /// ever appends, so existing slots keep their meaning.
    /// Takes the entities `library` gained, as the next field install would.
    pub fn sync_library(&mut self, library: &ModelLibrary) {
        self.sync(library);
    }

    fn sync(&mut self, library: &ModelLibrary) {
        // A library built empty takes its content stack with its first bake, after this pass was
        // made over it; its entities draw from that stack.
        if self.layers.is_none() && library.layers.is_some() {
            self.layers = library.layers.clone();
            self.raise();
        }
        // A library that only grew leaves every drawn figure's draw as it was: a figure of a new
        // entity is walked as it joins.
        let grew = self.meshes.len() <= library.meshes.len()
            && self.textures.len() <= library.textures.len()
            && appended_only(
                (&self.default_palette, self.meshes.len()),
                (&library.palette, library.meshes.len()),
            );
        if !grew || self.alternatives.len() != library.alternatives.len() {
            self.raise();
        }
        // Recorded before the mesh arrays are extended below.
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
        if self.materials.len() < library.materials.len() {
            self.materials
                .extend_from_slice(&library.materials[self.materials.len()..]);
            self.ready.resize(library.materials.len(), false);
        }
        if self.default_palette.len() != library.palette.len() {
            // A library that only grew keeps every entity's history: a
            // newly seen entity must not restart a raid unit's intro.
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
        // Keep each producer's ordinal and effective route while refreshing its asset tie-break.
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

    /// Material content failures are actor refusals; other upload failures stay fatal.
    fn make_resident(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        library: &ModelLibrary,
        mesh: usize,
    ) -> io::Result<()> {
        if self.resident.get(mesh).copied().unwrap_or(false) {
            return Ok(());
        }
        // No kept caster draw changes: one drawn before its textures were made refused, and a
        // list a walk refused in is never kept.
        // A mod entity's recorded counterpart is resident with it, to draw the moment the mod's
        // draw is refused.
        if let Some(&counterpart) = self.alternatives.get(&mesh) {
            self.make_resident(device, queue, library, counterpart)?;
        }
        let materials: BTreeSet<usize> = library.meshes[mesh]
            .native
            .iter()
            .flat_map(|descriptor| descriptor.lods.iter().flatten())
            .map(|part| part.material_index)
            .collect();
        for material in materials {
            if !self.material_refusals.contains_key(&material)
                && let Err(why) = self.make_ready(device, queue, library, material)
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

    fn make_ready(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        library: &ModelLibrary,
        material: usize,
    ) -> io::Result<()> {
        if self.ready[material] {
            return Ok(());
        }
        let ModelMaterial {
            diffuse,
            specular,
            normal,
            ..
        } = self.materials[material];
        for slot in [diffuse, specular, normal] {
            if self.textures[slot].is_none() {
                self.textures[slot] = Some(upload(
                    device,
                    queue,
                    &format!("{} texture {slot}", self.label),
                    &library.textures[slot],
                    library.spaces[slot],
                )?);
            }
        }
        self.ready[material] = true;
        Ok(())
    }

    /// Uploads every mesh of the library, chunked within the device's limits.
    /// Warm paths call this before any hour data exists so first use never
    /// pays the library upload on a frame; a library that grows as hours place
    /// its entities uploads each one when a field first places it.
    pub fn upload_library(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        library: &ModelLibrary,
    ) -> io::Result<()> {
        self.sync(library);
        for mesh in 0..library.meshes.len() {
            self.make_resident(device, queue, library, mesh)?;
        }
        Ok(())
    }

    /// Resolves placed materials; the compiled adapter owns native geometry uploads.
    pub fn upload(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        library: &ModelLibrary,
        field: ModelField,
    ) -> io::Result<()> {
        self.sync(library);
        for group in &field.groups {
            self.make_resident(device, queue, library, group.mesh)?;
        }
        self.field = field;
        self.bind_field();
        self.redirect_field();
        self.rebuild_native_walk();
        Ok(())
    }

    /// This frame's game cull: every figure is tested by it before anything is prepared for it.
    pub fn set_cull(&self, cull: Option<GameCull>, seen: Option<GameCull>) {
        self.cull.set(cull);
        self.seen.set(seen);
    }

    /// Whether the figure of `instance` (of `mesh`, in the view moved by `offset` from the
    /// traversal's `camera`) is culled by this frame's game cull, and why; none without one. The
    /// sphere is centred half the bounds' height above the origin and reaches every point of them
    /// from there.
    pub(super) fn game_culled(&self, mesh: usize, instance: &ModelInstance, offset: f32, camera: &Camera) -> Option<Culled> {
        let cull = self.cull.get()?;
        let mesh = self.meshes.get(mesh)?;
        let scale = instance.scale.abs();
        let centre = [instance.position[0] + offset + cull.x - camera.center[0], instance.position[1] + mesh.height * scale * 0.5, instance.position[2]];
        cull.test(centre, (mesh.bound + mesh.height * 0.5) * scale, mesh.cull_distance_squared)
    }

    /// Instances the last frame drew, for reporting.
    pub fn drawn(&self) -> u32 {
        self.drawn.get()
    }

    /// Counts a refused actor draw of the entity in slot `mesh` (`None` for a refusal of the
    /// instance itself). A visual mod's entity ([`ModelMesh::visual`]) is drawn from its
    /// recorded counterpart from the next frame on, named; any other refusal is recorded in the
    /// frame's arena by name ([`shader_pipeline::gpu::Arena::refuse`]).
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
        // Each refusal by its reason and by its entity, for the flight record.
        if viewer_foundation::view_record::on() {
            viewer_foundation::view_record::add_named(self.label, &format!("refused: {reason}"), 1);
            viewer_foundation::view_record::add_named(self.label, &format!("refused {name}: {reason}"), 1);
        }
        let why = format!("refused actor {name}: {reason}");
        count(Count::RefusedActors, 1);
        if crate::map::models::profiling_refusals() {
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
            // By what was refused, not why: the message can differ between loads.
            None => arena.refuse(&format!("Map shader {}: refused actor {name}", self.label)),
        }
        let mut refusals = self.refusals.borrow_mut();
        if refusals.0 != arena.frame() {
            *refusals = (arena.frame(), BTreeSet::new());
        }
        refusals.1.insert((instance, node, why));
        self.refused.set(self.refused.get() + 1);
    }

    /// Draws each mod entity refused since the last frame from its recorded counterpart, named.
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

    /// Points every placed group of a redirected entity at its recorded counterpart.
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
        let _span = viewer_foundation::profiling::span("native_walk_install");
        self.native_walk.rebuild(
            &self.field,
            self.meshes.len(),
            &self.alternatives,
            &self.redirects,
            &self.hidden,
        );
        self.written.set(true);
    }

    /// A change a caster walk reads that only a new walk can follow.
    fn raise(&mut self) {
        self.revision += 1;
    }

    /// How the field differs from the one the caster list last read, with the instances and
    /// groups that changed. Compared only when a writer placed the field since, so a field
    /// handed over and placed again in one frame is compared once, as drawn.
    fn field_change(&self, changed: &mut Changed) -> Change {
        changed.clear();
        if !self.written.replace(false) {
            return Change::Same;
        }
        let mut walked = self.walked.borrow_mut();
        let change = walked.note(&self.field, changed);
        if viewer_foundation::view_record::on() {
            use viewer_foundation::view_record::add;
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

    /// How the field changed for this frame's retained lists, against the field they last read:
    /// made by the first list that asks in `frame`, then the same for every other.
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

    /// The actors given a player since the casters last read them, sorted, into `born`.
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

    /// Readies this frame's shadow casters (native material-selected shadow techniques,
    /// declarations and actor palettes); whether they draw what the last frame's drew, byte for
    /// byte.
    pub fn ready_casters(
        &self,
        arena: &shader_pipeline::gpu::Arena,
        frame: ModelFrame<'_>,
        native: NativeInputs<'_>,
    ) -> io::Result<bool> {
        self.prepare_casters(arena, frame, native)
    }

    pub fn caster_id(&self) -> crate::map::shadow::CasterId {
        self.caster
    }

    /// Draws the casters [`Self::ready_casters`] readied this frame.
    pub fn draw_casters(&self, arena: &shader_pipeline::gpu::Arena, pass: &mut wgpu::RenderPass<'_>) {
        self.record_casters(arena, pass);
    }
}

/// What a caster walk reads of a field: each instance but its palette offset (which animation
/// writes every frame and native draws never read), the walk order and each group's mesh, range
/// and bounds. Placements made again unchanged every frame leave it as it was.
#[derive(Default)]
struct WalkedField {
    instances: Vec<ModelInstance>,
    order: Vec<(usize, usize)>,
    groups: Vec<(usize, Range<u32>, [u32; 4])>,
    /// The last whole change's instances joined, left and out of their order, for the counts.
    reshaped: [u64; 3],
    /// What made the last change whole ([`FIELD_WHOLE`]).
    whole: usize,
}

/// What makes a field's change whole, in the flight record: another count of instances, of groups
/// or a group's mesh or range, another walk order, or an instance placed for another actor.
const FIELD_WHOLE: [&str; 4] = ["field_whole_count", "field_whole_groups", "field_whole_order", "field_whole_actor"];

/// How a field differs from the last one a caster walk read.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Change {
    Same,
    /// The same instances, actors and groups in the same order; some instances' own values
    /// (transform, animation state, flags) changed: listed in [`Changed`].
    Changed,
    /// Instances joined, left or changed their order, group or actor. The actors in both fields
    /// whose own values changed are listed in [`Changed`], by their index in the new field.
    Whole,
}

/// How the field changed for one frame's retained lists ([`ModelPass::frame_change`]).
struct FrameChange {
    frame: Option<u64>,
    /// The frame the field was read before: a list that followed that change follows this one.
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

/// A display frame's placements for a field whose figures move between the ones their hours
/// give ([`ModelPass::update_moving`]).
#[derive(Default)]
pub struct Moving {
    /// Every part where its hour places it, as `(mesh, instance)`: given only where the hours'
    /// own placements changed, so the field is built again for them and for nothing else.
    pub standing: Option<Vec<(usize, ModelInstance)>>,
    /// The parts this frame places elsewhere, as `(ordinal, instance)`, by their ordinal in the
    /// field's walk order. A part the frame before moved and this one does not carries the
    /// placement its hour gives, so it stands again.
    pub moved: Vec<(usize, ModelInstance)>,
}

/// The instances whose own values changed, and the groups whose bounds moved (in a whole
/// change, every group is new and none is listed).
#[derive(Default)]
pub(super) struct Changed {
    pub instances: Vec<usize>,
    pub groups: Vec<usize>,
}

impl Changed {
    fn clear(&mut self) {
        self.instances.clear();
        self.groups.clear();
    }
}

/// The actor an instance is: what its player, hidden state and identity follow.
fn actor(instance: &ModelInstance) -> (u32, u32, u32) {
    (
        instance.animation_seed,
        instance.animation_identity,
        instance.animation_epoch,
    )
}

impl WalkedField {
    /// Takes `field`, listing the instances and groups that changed when the instances, their
    /// actors and their order are the same.
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

    /// Takes a field of other instances or order, counting what joined, left and moved in it
    /// and listing the actors in both whose own values changed. An actor placed twice is
    /// matched by its place in each field.
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
        // Out of order: the kept instances outside the longest run that keeps its order.
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

/// What the paced report counts of the retained casters, each frame ([`take_counts`]).
#[derive(Clone, Copy)]
pub(super) enum Count {
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

pub(super) fn count(what: Count, n: u64) {
    COUNTS[what as usize].fetch_add(n, std::sync::atomic::Ordering::Relaxed);
}

/// Actor refusals met since the last [`take_refusals`], by pass, entity and reason.
static REFUSALS: std::sync::Mutex<BTreeMap<String, u64>> = std::sync::Mutex::new(BTreeMap::new());

/// Whether refusals are named for the paced report (while the harness profiles).
fn profiling_refusals() -> bool {
    viewer_foundation::profile_writer::profiling()
}

/// The actor refusals met since the last call, by pass, entity and reason, with how often: a
/// refusal met every frame is a part walked again every frame, and possibly a model missing.
pub fn take_refusals() -> BTreeMap<String, u64> {
    std::mem::take(&mut *REFUSALS.lock().unwrap_or_else(|error| error.into_inner()))
}

/// The retained lists' counts since the last call, by name: walks and kept frames of every
/// pass's casters and lit walks, nodes rewritten in kept frames, lists spliced by figure with
/// the figures walked into them and left out, and how the unit models' field changed (same, by
/// instance, whole) with its instances changed, joined, left and out of order.
pub fn take_counts() -> impl Iterator<Item = (&'static str, u64)> {
    COUNT_NAMES.iter().zip(&COUNTS).map(|(name, count)| {
        (*name, count.swap(0, std::sync::atomic::Ordering::Relaxed))
    })
}

/// [`ModelPass::sync`]'s history rule for a library that changed from
/// `before` to `after` (default palette, mesh count): the players stay when
/// it only grew ([`appended_only`]) and all start again otherwise.
fn retain_players(
    players: &mut BTreeMap<(usize, u32, u32), entity_animation::Player>,
    before: (&[PaletteJoint], usize),
    after: (&[PaletteJoint], usize),
) {
    if !appended_only(before, after) {
        players.clear();
    }
}

/// Whether a library went from `before` to `after` (default palette, mesh
/// count) by appending alone, so each known mesh keeps its index, rig and
/// palette range and its players stay valid. [`ModelLibrary::merge`] and
/// `ModelLibrary::add` append meshes, rigs and palette joints (`add` rolls
/// back only what it appended), and [`ModelPass::sync`] extends its rigs
/// from the library's tail, so this holds for every growth; anything else,
/// such as another library, starts every history again.
fn appended_only(before: (&[PaletteJoint], usize), after: (&[PaletteJoint], usize)) -> bool {
    let ((old, old_meshes), (new, new_meshes)) = (before, after);
    new_meshes >= old_meshes && new.len() >= old.len() && new[..old.len()] == *old
}

/// The CPU half of [`ModelPass::animate`]: evaluates the players of the
/// shown groups' animated instances (a default-state actor in the field's
/// `default_states` plays its request and chooses states by the game's
/// rule, [`entity_animation::Player::native_choice`]) into a palette (the default one
/// followed by one rig range per such instance, whose offset the instance
/// takes) and retires every other player.
///
/// The game's entity clocks advance off screen:
/// The game walks every quadtree node
/// through entity-event updates. The frustum test only decides visible-entity
/// work, while clip phase and elapsed time advance for every entity. So the
/// unit pass shows every group it places and the unit history, not render
/// culling, ends a figure's, hull's or raid unit's player when the game
/// destroys the entity ([`super::unit_history`]).
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

/// [`animate_players`] and, when `native_only`, without sampling the name-joined palette
/// of a rig every node of which draws natively.
#[allow(clippy::too_many_arguments)]
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
    let plan_span = viewer_foundation::profiling::span("animation_dispatch_plan");
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
    // Native actors do not read the optional name-joined palette. Keep the logical
    // offsets identical, but do not allocate or copy unused per-instance ranges.
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
        // A default-state actor plays its request; one refused, or whose
        // entity has no default state, stays still.
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
        // Clock callbacks intern content-named states. Keep those callbacks
        // in placement order; ordinary players only read registered names.
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
        // A repeated animation key shares one player: its calls stay in their original order.
        let mut tasks: Vec<_> = players
            .iter_mut()
            .filter_map(|(key, player)| by_player.remove(key).map(|work| (player, work)))
            .collect();
        drop(plan_span);
        run_span = viewer_foundation::profiling::span("animation_dispatch_run_commit");
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
    // A deactivated shared naval wrapper is absent from placements but keeps
    // its producer-owned clock. Initialize even an actor hidden before its
    // first draw, and advance only its phase/explicit requests while hidden.
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

/// Whether a group reaches the visible ground rectangle at this wrap offset.
fn visible(group: &ModelGroup, offset: f32, rectangle: Option<[f32; 4]>) -> bool {
    let Some([x0, z0, x1, z1]) = rectangle else {
        return true;
    };
    let [gx0, gz0, gx1, gz1] = group.bounds;
    gx0 + offset <= x1 && gx1 + offset >= x0 && gz0 <= z1 && gz1 >= z0
}

/// What a mesh's [`entity_animation::DEFAULT_STATE`] actors do, the same every frame: play
/// the entity's default state, or stay still for the reason given.
struct DefaultPlay {
    requests: Arc<Vec<entity_animation::Request>>,
    native: u32,
    refusal: Option<String>,
}

/// Whether a play was made from `rig`: the same rig, or none for none.
fn same_rig(made: Option<&Arc<Rig>>, rig: Option<&Arc<Rig>>) -> bool {
    match (made, rig) {
        (Some(made), Some(rig)) => Arc::ptr_eq(made, rig),
        (None, None) => true,
        _ => false,
    }
}

impl DefaultPlay {
    /// `None` when the entity has no clip of its default state.
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

/// The first state with several variants an actor of `rig` can reach from
/// its nodes' default states through `next_state`: the node, the state and
/// its variant count.
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
    use super::*;

    /// The pose worker makes a figure's limbs away from its player, from the rig and the samples
    /// the play left and nothing else ([`crate::map::poses`]): the pose it makes is bit for bit
    /// the pose the player makes, on every frame of a play.
    #[test]
    fn a_pose_made_from_its_samples_is_the_pose_its_player_makes() {
        use super::super::mesh_pose;
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
        let rig = entity_animation::pose_tests::rig(0.7, true, true);
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
        // Samples of another rig play nothing rather than reaching past their end.
        assert_eq!(mesh_pose::actors_at(&rig, &[]).len(), rig.nodes.len());
    }

    #[test]
    fn frozen_and_playerless_actors_are_placement_independent_and_reset_on_mesh_install() {
        use super::super::mesh_pose;
        let mut rig = entity_animation::pose_tests::rig(0.7, true, true);
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
                cull_distance_squared: installed_content_core::mesh::DEFAULT_CULL_DISTANCE_SQUARED,
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
        let mut eventless = entity_animation::pose_tests::rig(0.7, true, true);
        for node in &mut eventless.nodes {
            for clip in &mut node.clips {
                clip.state.events.clear();
            }
        }
        let mut event_then_quiet = entity_animation::pose_tests::rig(0.7, true, true);
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
            Arc::new(entity_animation::pose_tests::rig(0.7, true, true)),
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
                let expected = super::super::mesh_pose::actors_uncached(
                    &rigs[key.0],
                    Some(&serial[key]),
                    false,
                );
                assert_eq!(actual.len(), expected.len());
                for (a, b) in actual.iter().zip(&expected) {
                    let matrix =
                        |value: super::super::mesh_pose::Matrix| value.map(|c| c.map(f32::to_bits));
                    assert_eq!(
                        a.world.as_ref().map(|m| matrix(*m)),
                        b.world.as_ref().map(|m| matrix(*m))
                    );
                    let pose = |pose: &super::super::mesh_pose::Pose| {
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
            .map(|native| Arc::new(entity_animation::pose_tests::rig(0.7, true, native)))
            .collect();
        let default = vec![PaletteJoint::IDENTITY; rigs[0].end];
        let (mut expected, mut parallel) = (BTreeMap::new(), BTreeMap::new());
        for frame in 0..32 {
            let time = frame as f32 * 0.13;
            let mut field = ModelField::default();
            for mesh in [2, 0, 1] {
                let start = field.instances.len() as u32;
                for ordinal in 0..20 {
                    // Repeated keys must step the same player twice, not fork it.
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
