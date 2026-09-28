//! The shared static-mesh layer: `gfx/FX/pdxmesh.shader` ported in
//! `pdxmesh.wgsl`, with the instance library every close-zoom 3D layer builds
//! on.
//!
//! Three of the layers the engine gates off above the reference height --- city
//! sprawl (`map::sprawl`), map buildings (`map::map_buildings`) and unit models
//! (`map::unit_models`) --- are the same thing drawn from different tables: a
//! PDX entity placed at a world position with a rotation about Y and a uniform
//! scale. This module owns that machinery once. The railway layer is not one of
//! them: the engine draws rails with their own ribbon shader
//! (`gfx/models/supply/railroad.shader`, `map::railways`).
//!
//! The specification is research report
//! `C:\hoi4_scratch\replay_map\research\R3-meshes-trees-cities.md` sections 1
//! (the `.mesh` container and the `.gfx` / `.asset` descriptors), 2 (the effects
//! and `ComposeLightMesh`) and D (the camera-height gates). Section numbers
//! below refer to it.
//!
//! # What is ported and what is not
//!
//! `PixelPdxMeshStandard` branches on the `Defines` of the effect the material
//! names. Rather than compiling a pipeline per define set, the branches that
//! matter are kept at runtime behind a per-material flag word, which is how the
//! defines behave: `PDX_IMPROVED_BLINN_PHONG` chooses the material channel
//! meanings, `EMISSIVE` chooses which channel carries the emissive mask,
//! `PDX_SNOW` chooses `ComposeLightSnow` over `ComposeLightMesh`, `ALPHA_TEST`
//! clips. Every shipped effect is one of those combinations (section 2.1).
//!
//! # Skinning
//!
//! `matBones` is the engine's joint palette, and it is reproduced: a mesh that
//! declares a `[skin]` is drawn posed, not in its bind pose. The pose is
//! [`mesh::FIRST_FRAME`] of the animation the entity's **default state** plays,
//! for the time-zero dump; window playback samples `entity_animation`. Without it
//! `GER_infantry.mesh` draws as the T-pose it is authored in.
//!
//! At time zero the immutable default palette and baked rigid vertices preserve
//! historical dumps. The window adds one palette per visible instance. Rigid
//! attachments retain their bind vertices alongside the frozen vertices, so
//! joints hidden in idle can become visible without rebuilding geometry.
//! Hierarchies and animation files are cached at load; only palettes and small
//! instance records are written per frame. Repeated LOD skeletons use the first
//! inverse bind per joint index, exactly as `Mesh::palette` does.
//! Map buildings also load their authored idle and damaged animations; their
//! unrecorded clock is posed at sample zero, as documented in `map::map_buildings`.
//!
//! Dropped, with the reason:
//!
//! * `CalculateShadowCascaded`, which `pdxmesh.shader:499` has commented out:
//!   mesh shadowing is dead code in 1.19.2 (section 2.4).
//! * `UV_ANIM` and `ATLAS`, which no static map object sets.
//!
//! `TRAIN` is reproduced with white TrainColor and per-instance endpoint clipping
//! by `map::trains`; its instances also carry the rail slope.
//!
//! `RIM_LIGHT` is commented out in the game's own shader.

use super::entity_animation::{self, Rig};
use std::{
    collections::{BTreeMap, BTreeSet},
    f32::consts::TAU,
    io,
    ops::Range,
    sync::Arc,
};

use bytemuck::{Pod, Zeroable};
use eframe::wgpu;
use installed_content::{
    ResolvedLayers,
    image::{Rgba8Image, dds},
    mesh::{self, Descriptors},
};

use super::{
    camera::Camera,
    scene::{DEPTH_FORMAT, Targets, buffer, module, sampler_entry, texture_entry},
    shadow::DEPTH_MAP_FORMAT,
    textures::{Bound, ColorSpace, Samplers, upload},
};

fn invalid(message: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message.into())
}

// ------------------------------------------------------------------ constants

/// `WATER_HEIGHT`. The map-object update compares `camera height - WATER_HEIGHT`
pub const WATER_HEIGHT: f32 = 9.5;
/// How many `attach` levels one entity composes.
///
/// The shipped chains are three deep at most --- an artillery rig attaches a
/// gun and a rifleman, and the rifleman attaches his rifle --- and this is the
/// guard that keeps a modded cycle from composing for ever.
const ATTACHMENT_DEPTH: usize = 6;
/// Whether the camera draws city sprawl (section 4.2, `0x140b4c120:544`):
/// below both `CITY_SPRAWL_SHRINK_DISTANCE`, a spawn/despawn switch despite
/// its name, and the master `DRAW_MAP_OBJECTS_CUTOFF`, water-relative.
pub fn draws_city_sprawl(camera: &Camera) -> bool {
    let defines = &camera.defines;
    camera.height - WATER_HEIGHT < defines.sprawl_cutoff.min(defines.map_objects_cutoff)
}

// -------------------------------------------------------------------- effects

/// The `Defines` of one `pdxmesh.shader` effect, as the pixel shader branches
/// on them (section 2.1).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Effect {
    /// `TRAIN`: opaque TrainColor and endpoint clipping instead of texture alpha.
    pub train: bool,
    /// `PDX_IMPROVED_BLINN_PHONG`: `SpecularMap.a` is glossiness, `.g`
    /// specular, `.b` metalness, and the normal map is RRxG-packed.
    pub improved_blinn_phong: bool,
    /// `EMISSIVE`: a channel of the normal or specular map is an emissive mask.
    pub emissive: bool,
    /// `PDX_SNOW`: the mesh takes snow and `ComposeLightSnow`.
    pub snow: bool,
    /// `ALPHA_TEST`: `clip(vDiffuse.a - 1.0)`.
    pub alpha_test: bool,
}

impl Effect {
    /// The flag word the shader reads.
    pub fn bits(self) -> u32 {
        u32::from(self.improved_blinn_phong)
            | u32::from(self.emissive) << 1
            | u32::from(self.snow) << 2
            | u32::from(self.alpha_test) << 3
            | u32::from(self.train) << 4
    }
}

/// The defines of an effect named in a `[material]` or a `meshsettings`
/// override (`pdxmesh.shader:747-993`, tabulated in section 2.1).
///
/// An effect the table does not name takes `PdxMeshStandard`'s empty define
/// set, which is the engine's own fallback shape for an unknown effect.
/// Whether one `meshsettings` block overrides this sub-mesh: the shape it
/// names, or any shape when it names none, and that shape's own sub-mesh order.
fn setting_applies(setting: &mesh::MeshSetting, shape: &str, index: usize) -> bool {
    (setting.name.is_empty() || setting.name == shape) && setting.index as usize == index
}

pub fn effect_for(name: &str) -> Effect {
    let improved = Effect {
        improved_blinn_phong: true,
        emissive: true,
        ..Effect::default()
    };
    match name {
        "PdxMeshSnow" => Effect {
            snow: true,
            ..improved
        },
        // `PdxMeshSnowSkinned` declares `PDX_SNOW` only.
        "PdxMeshSnowSkinned" => Effect {
            snow: true,
            ..Effect::default()
        },
        "PdxMeshTrain" => Effect {
            train: true,
            alpha_test: true,
            ..improved
        },
        "PdxMeshTrainSkinned" => Effect {
            train: true,
            ..improved
        },
        "PdxMeshAdvanced" | "PdxMeshAdvancedSkinned" => improved,
        "PdxMeshAdvancedSnow" => Effect {
            snow: true,
            ..improved
        },
        "PdxMeshAdvancedSkinnedAlphaTest" => Effect {
            alpha_test: true,
            ..improved
        },
        "PdxMeshAlphaBlend" | "PdxMeshBorder" => Effect {
            alpha_test: true,
            ..Effect::default()
        },
        _ => Effect::default(),
    }
}

// -------------------------------------------------------------------- library

/// One vertex of a mesh, in `VS_INPUT_PDXMESHSTANDARD` order with the skin
/// streams after it.
///
/// An unskinned vertex carries four zero weights, which is the shader's signal
/// that its position, normal and tangent are already in world-relative space
/// and must be passed through untouched; see the module comment.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Pod, Zeroable)]
pub struct ModelVertex {
    pub position: [f32; 3],
    pub normal: [f32; 3],
    pub tangent: [f32; 4],
    pub uv0: [f32; 2],
    /// Slots in [`ModelLibrary::palette`], already offset by the composed
    /// entity's own base. An unused slot is 0 at weight 0.
    pub joints: [u32; mesh::INFLUENCES],
    pub weights: [f32; mesh::INFLUENCES],
    pub bind_position: [f32; 3],
    pub bind_normal: [f32; 3],
    pub bind_tangent: [f32; 3],
    /// The node's own palette slot in [`RIGID_SLOT`]; the high bit marks a
    /// vertex baked in its bind pose, which `mesh_animated` passes through.
    pub rigid_joint: u32,
}

/// The palette-index bits of [`ModelVertex::rigid_joint`].
const RIGID_SLOT: u32 = 0x7fff_ffff;

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
}

/// One sub-mesh of one LOD: which material it uses and which indices it draws.
#[derive(Clone, Debug, PartialEq)]
pub struct ModelPart {
    pub material: usize,
    pub range: Range<u32>,
}

/// One `[material]`: the effect's defines and the three textures.
#[derive(Clone, Debug, PartialEq)]
pub struct ModelMaterial {
    pub effect: Effect,
    /// Indices into [`ModelLibrary::textures`].
    pub diffuse: usize,
    pub normal: usize,
    pub specular: usize,
}

/// One resolved mesh: its LOD ladder and the bounding radius a cull needs.
#[derive(Clone, Debug, PartialEq)]
pub struct ModelMesh {
    /// The name the layer asked for: an entity name or a pdxmesh name.
    pub name: String,
    /// The `.mesh` the descriptor named.
    pub path: String,
    /// `entity.scale * pdxmesh.scale`, baked into the vertex positions.
    pub scale: f32,
    /// `loddist`, ascending.
    pub lod_distances: Vec<f32>,
    /// One entry per LOD shape, each a list of sub-mesh draws.
    pub lods: Vec<Vec<ModelPart>>,
    /// Horizontal radius of LOD 0's bounds, after `scale` and with every
    /// attachment composed in, for culling.
    pub radius: f32,
    /// Height of LOD 0's bounds above its own origin, after `scale`.
    pub height: f32,
    /// The same radius over the entity's **own** geometry only. An attachment
    /// rig's is a fraction of a world unit while its composed one is metres.
    pub own_radius: f32,
    /// How many attached entities were composed into this mesh, at any depth.
    pub attachments: usize,
}

/// Every mesh, material and texture one layer draws, flattened into one vertex
/// and index pair.
#[derive(Default)]
pub struct ModelLibrary {
    pub vertices: Vec<ModelVertex>,
    pub indices: Vec<u32>,
    /// Every composed entity's joint palette, end to end;
    /// [`ModelVertex::joints`] indexes it directly.
    pub palette: Vec<PaletteJoint>,
    pub meshes: Vec<ModelMesh>,
    pub materials: Vec<ModelMaterial>,
    pub textures: Vec<dds::Texture>,
    /// The colour space each of [`textures`](ModelLibrary::textures) uploads
    /// in, from the material slot that asked for it: the capture has every
    /// `pdxmesh` `DiffuseMap` created `..._UNORM_SRGB` and its `SpecularMap`
    /// and `NormalMap` created `..._UNORM` (`map::textures::ColorSpace`).
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
    rigs: Vec<Arc<Rig>>,
}

/// One entity under composition: the LOD part lists it is filling and how far
/// it has reached so far.
#[derive(Default)]
struct Composition {
    lods: Vec<Vec<ModelPart>>,
    reach: Reach,
    rig: Rig,
}

/// How far one composed entity, attachments included, reaches around and above
/// its own origin. The pass culls on the radius and no layer reads the height
/// for anything but reporting.
#[derive(Clone, Copy, Debug, Default)]
struct Reach {
    radius: f32,
    height: f32,
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
        if own {
            self.own_radius = self.own_radius.max(radius);
        }
    }
}

/// Entity shards one library build spreads over. Bounds total threads while
/// the document loader already fans out across loaders; file reads dominate
/// and parallelize cleanly.
const BAKE_SHARDS: usize = 4;

impl ModelLibrary {
    /// Resolves and loads every name, appending one warning per name that does
    /// not resolve rather than refusing the document.
    ///
    /// `gfx/entities` and `gfx/models` are outside the content coverage policy
    /// ([archived rendering notes](#) section 4), so every file here goes through
    /// [`ResolvedLayers::read_uncovered`] and is not hash-verified.
    pub fn load(
        layers: &ResolvedLayers,
        descriptors: &Descriptors,
        names: &[String],
        warnings: &mut Vec<String>,
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
        let chunk = unique.len().div_ceil(shards);
        let baked: Vec<(ModelLibrary, Vec<String>)> = std::thread::scope(|scope| {
            let mut handles = Vec::new();
            for shard_names in unique.chunks(chunk).collect::<Vec<_>>() {
                handles.push(scope.spawn(move || {
                    let mut library = ModelLibrary::default();
                    let mut missing = Vec::new();
                    for name in shard_names {
                        match library.add(layers, descriptors, name) {
                            Ok(()) => {}
                            Err(error) => missing.push(format!("{name} ({error})")),
                        }
                    }
                    (library, missing)
                }));
            }
            handles
                .into_iter()
                .map(|handle| handle.join().expect("entity baker panicked"))
                .collect()
        });
        let mut library = ModelLibrary::default();
        let mut missing = Vec::new();
        for (shard, shard_missing) in baked {
            missing.extend(shard_missing);
            library.merge(shard);
        }
        if !missing.is_empty() {
            missing.sort();
            warnings.push(format!(
                "No mesh resolved for {}; those objects are not drawn.",
                missing.join(", ")
            ));
        }
        // 14227D8C0 logs a locator whose parent_joint the skeleton lacks and
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
            warnings.push(format!(
                "These entity locators name a parent_joint the mesh lacks and stay in entity space, as the engine keeps them: {}.",
                unjointed.into_iter().collect::<Vec<_>>().join(", ")
            ));
        }
        Ok(library)
    }

    /// Whether a name resolved.
    pub fn slot(&self, name: &str) -> Option<usize> {
        self.index.get(name).copied()
    }

    /// Merges a shard baked on another thread. Shards hold contiguous name
    /// ranges merged in order, so first-seen dedup matches the serial loop
    /// and every rebased index lands exactly where the serial bake put it.
    pub(crate) fn merge(&mut self, shard: ModelLibrary) {
        // Textures first: materials reference their slots. First seen in
        // shard order wins, as serially.
        let mut texture_keys: Vec<Option<&(String, ColorSpace)>> = vec![None; shard.textures.len()];
        for (key, slot) in &shard.paths {
            texture_keys[*slot] = Some(key);
        }
        let mut texture_map = Vec::with_capacity(shard.textures.len());
        for (slot, texture) in shard.textures.iter().enumerate() {
            let key = texture_keys[slot].expect("every texture has its path key");
            texture_map.push(match self.paths.get(key) {
                Some(&global) => global,
                None => {
                    let global = self.textures.len();
                    self.textures.push(texture.clone());
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
        let palette_base = self.palette.len();
        let vertex_base = self.vertices.len() as u32;
        let index_base = self.indices.len() as u32;
        self.palette.extend(shard.palette);
        self.vertices
            .extend(shard.vertices.into_iter().map(|mut vertex| {
                // Only weighted slots name palette joints; the rest are literal
                // zero, exactly as the serial bake left them.
                for (joint, weight) in vertex.joints.iter_mut().zip(vertex.weights.iter()) {
                    if *weight != 0.0 {
                        *joint += palette_base as u32;
                    }
                }
                // The rigid slot is a palette index too, under the animated flag.
                vertex.rigid_joint = ((vertex.rigid_joint & RIGID_SLOT) + palette_base as u32)
                    | (vertex.rigid_joint & !RIGID_SLOT);
                vertex
            }));
        self.indices
            .extend(shard.indices.into_iter().map(|index| index + vertex_base));
        assert_eq!(shard.meshes.len(), shard.rigs.len(), "one rig per mesh");
        for (mut mesh, mut rig) in shard.meshes.into_iter().zip(shard.rigs) {
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
            for lod in &mut mesh.lods {
                for part in lod {
                    part.material = material_map[part.material];
                    part.range.start += index_base;
                    part.range.end += index_base;
                }
            }
            let slot = self.meshes.len();
            self.index.insert(mesh.name.clone(), slot);
            self.meshes.push(mesh);
            self.rigs.push(rig);
        }
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
        let marks = (
            self.vertices.len(),
            self.indices.len(),
            self.palette.len(),
            self.materials.len(),
        );
        let added = self.compose_entity(layers, descriptors, name);
        if added.is_err() {
            self.vertices.truncate(marks.0);
            self.indices.truncate(marks.1);
            self.palette.truncate(marks.2);
            self.materials.truncate(marks.3);
        }
        added
    }

    fn compose_entity(
        &mut self,
        layers: &ResolvedLayers,
        descriptors: &Descriptors,
        name: &str,
    ) -> io::Result<()> {
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
        )?;
        out.rig.end = self.palette.len();
        let Composition { lods, reach, rig } = out;
        if lods.iter().all(Vec::is_empty) {
            return Err(invalid(format!("{path} has no drawable sub-mesh")));
        }
        let (radius, height) = (reach.radius, reach.height);
        self.rigs.push(Arc::new(rig));
        self.index.insert(name.to_owned(), self.meshes.len());
        self.meshes.push(ModelMesh {
            name: name.to_owned(),
            path,
            scale,
            lod_distances,
            lods,
            radius,
            height,
            own_radius: reach.own_radius,
            attachments: reach.attachments,
        });
        Ok(())
    }

    /// Bakes one entity and, recursively, everything it attaches into `lods`.
    ///
    /// `placement` carries the composed joint transforms of the rig chain above
    /// this entity, already in the **root** entity's own object space and
    /// before the root's own scale, so an attachment three levels down is one
    /// matrix multiply away from its vertices. The engine composes the same
    /// chain per frame from the posed skeleton. Retain this frozen composition
    /// for time-zero dumps and the bind geometry/rig for window animation.
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
    ) -> io::Result<(String, f32, Vec<f32>)> {
        let entity = descriptors
            .resolve_entity(name)
            .ok_or_else(|| invalid("no pdxmesh or entity declares it"))?;
        let resolved = mesh::ResolvedMesh {
            definition: entity.definition,
            scale: entity.scale,
        };
        let path = entity.definition.file.clone();
        let scale = entity.scale;
        let decoded = mesh::read(&layers.read_uncovered(&path)?)
            .map_err(|error| invalid(format!("{path}: {error}")))?;
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
            .animation
            .map(|animation| animation.file.clone())
            .and_then(|path| self.animation(layers, &path).cloned());
        let mut skin_base = self.push_palette(&decoded, animation.as_deref(), &own);
        let frozen_skin = skin_base.is_some();
        let rigid = self.palette.len() as u32;
        self.palette.push(PaletteJoint::from_transform(&own));
        let inherited = descriptors.inherit(name);
        let mut clips = Vec::new();
        if let Some(inherited) = &inherited {
            for variants in inherited.states.values() {
                for state in variants {
                    let path = entity
                        .definition
                        .animations
                        .iter()
                        .find(|(id, _)| id == &state.animation)
                        .and_then(|(_, key)| descriptors.animations.get(key))
                        .map(|a| a.file.clone());
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
        let rigid_vertex = rigid
            | if !frozen_skin && skin_base.is_some() {
                !RIGID_SLOT
            } else {
                0
            };
        let node_index = out.rig.nodes.len();
        let mut skeleton = decoded.clone();
        skeleton.shapes.clear();
        // Each LOD may repeat the skeleton, sometimes with placeholder zero
        // inverse binds. Mesh::palette/joint choose the first occurrence of
        // each index; keep that same canonical skeleton for per-frame writes.
        let mut joint_indices = BTreeSet::new();
        skeleton
            .skeleton
            .retain(|joint| joint_indices.insert(joint.index));
        out.rig.nodes.push(entity_animation::Node {
            mesh: Arc::new(skeleton),
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
            mesh_scale: entity.definition.scale,
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
                                    entity_locator(&locator.transform, entity.definition.scale),
                                    locator.parent_joint.clone(),
                                ),
                            )
                        })
                        .collect()
                })
                .unwrap_or_default(),
        });
        let shape_lods = decoded.lods();
        if out.lods.len() < shape_lods.len() {
            out.lods.resize(shape_lods.len(), Vec::new());
        }
        for (lod, shapes) in shape_lods.iter().enumerate() {
            let mut parts = Vec::new();
            for (shape, index, sub) in shapes
                .iter()
                .flat_map(|shape| {
                    shape
                        .meshes
                        .iter()
                        .enumerate()
                        .map(|(index, sub)| (shape.name.as_str(), index, sub))
                })
                .filter(|(_, _, sub)| sub.is_drawable())
            {
                let material = self.material(layers, directory, shape, index, sub, &resolved)?;
                let start = self.indices.len() as u32;
                let base = self.vertices.len() as u32;
                // A skinned vertex reaches the shader in its own bind space and
                // the palette carries `own`; an unskinned one is baked here, as
                // every vertex used to be.
                let skin = skin_base.and_then(|base| Some((base, sub.skin.as_ref()?)));
                let mut posed = false;
                for vertex in 0..sub.vertex_count() {
                    let influences = match skin {
                        Some((_, skin)) => skin.influences(vertex),
                        None => Vec::new(),
                    };
                    let mut joints = [0u32; mesh::INFLUENCES];
                    let mut weights = [0.0f32; mesh::INFLUENCES];
                    for (slot, (joint, weight)) in influences.iter().enumerate() {
                        // The `-1` slots are already dropped, so the base is
                        // added to a joint that exists.
                        joints[slot] = skin.map_or(0, |(base, _)| base) + joint;
                        weights[slot] = *weight;
                    }
                    let tangent = sub.tangents[vertex];
                    let entry = if influences.is_empty() || !frozen_skin {
                        // Either an unskinned sub-mesh or, in three shipped
                        // meshes, a vertex bound to nothing: both keep the
                        // baked bind position they always had.
                        let rotated = own.transform_vector([tangent[0], tangent[1], tangent[2]]);
                        ModelVertex {
                            position: own.transform_point(sub.positions[vertex]),
                            // The composed transform is a rotation and a
                            // uniform scale, so a direction only needs the
                            // rotation; the shader normalises what it gets.
                            normal: own.transform_vector(sub.normals[vertex]),
                            tangent: [rotated[0], rotated[1], rotated[2], tangent[3]],
                            uv0: sub.uv0[vertex],
                            joints,
                            weights,
                            bind_position: sub.positions[vertex],
                            bind_normal: sub.normals[vertex],
                            bind_tangent: [tangent[0], tangent[1], tangent[2]],
                            rigid_joint: rigid_vertex,
                        }
                    } else {
                        ModelVertex {
                            position: sub.positions[vertex],
                            normal: sub.normals[vertex],
                            tangent,
                            uv0: sub.uv0[vertex],
                            joints,
                            weights,
                            bind_position: sub.positions[vertex],
                            bind_normal: sub.normals[vertex],
                            bind_tangent: [tangent[0], tangent[1], tangent[2]],
                            rigid_joint: rigid_vertex,
                        }
                    };
                    if lod == 0 && frozen_skin && !influences.is_empty() {
                        posed = true;
                        out.reach
                            .widen_point(skinned(&self.palette, &entry), depth == 0);
                    }
                    self.vertices.push(entry);
                }
                self.indices
                    .extend(sub.indices.iter().map(|index| base + index));
                if lod == 0
                    && !posed
                    && let Some(bounds) = &sub.bounds
                {
                    out.reach.widen(&own, bounds, depth == 0);
                }
                parts.push(ModelPart {
                    material,
                    range: start..self.indices.len() as u32,
                });
            }
            out.lods[lod].extend(parts);
        }
        if depth < ATTACHMENT_DEPTH {
            // The attachments are collected before the recursion so the
            // borrow of `descriptors` through `entity` ends first.
            let attachments: Vec<(String, String, String)> = entity
                .attachments
                .iter()
                .map(|attachment| {
                    (
                        attachment.joint.clone(),
                        attachment.entity.clone(),
                        attachment.handle().to_owned(),
                    )
                })
                .collect();
            for (joint_name, child, handle) in attachments {
                let Some(joint) = joint_transform(&decoded, animation.as_deref(), &joint_name)
                else {
                    continue;
                };
                // A joint the animation scales to zero is the engine's own
                // switch for an attachment it is not showing.

                // An attachment that does not resolve leaves the parent drawn
                // rather than refusing the whole entity.
                if self
                    .compose(
                        layers,
                        descriptors,
                        &child,
                        own.compose(&joint),
                        depth + 1,
                        Some((node_index, joint_name, handle)),
                        out,
                    )
                    .is_ok()
                {
                    out.reach.attachments += 1;
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
    /// — a skinning matrix is `pose * inverse_bind` and the bind pose's own
    /// composition is the identity — but only the first is the *same bits*, and
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
        if !self.animations.contains_key(path) {
            let decoded = layers
                .read_uncovered(path)
                .ok()
                .and_then(|bytes| mesh::read_animation(&bytes).ok())
                .map(Arc::new);
            self.animations.insert(path.to_owned(), decoded);
        }
        self.animations.get(path)?.as_ref()
    }

    fn material(
        &mut self,
        layers: &ResolvedLayers,
        directory: &str,
        shape: &str,
        index: usize,
        sub: &mesh::SubMesh,
        resolved: &mesh::ResolvedMesh<'_>,
    ) -> io::Result<usize> {
        let material = sub.material.as_ref();
        // A `meshsettings` override replaces the `.mesh`'s own effect and
        // textures for one (shape, material index) pair (section 1.4): the
        // shape it names and that shape's own sub-mesh order. A mesh with
        // several materials gets a different texture set per index --- the
        // special project facilities give their platform, their gun and their
        // bunker three --- so applying every block to every sub-mesh paints
        // the whole model in the last block's textures. A block that names no
        // shape applies to any.
        let shader = material.map_or("", |m| m.shader.as_str());
        let effect = effect_for(shader);
        let mut diffuse = material.map_or("", |m| m.diffuse.as_str()).to_owned();
        let mut normal = material.map_or("", |m| m.normal.as_str()).to_owned();
        let mut specular = material.map_or("", |m| m.specular.as_str()).to_owned();
        let mut effect = effect;
        for setting in resolved
            .definition
            .settings
            .iter()
            .filter(|setting| setting_applies(setting, shape, index))
        {
            if let Some(texture) = &setting.diffuse {
                diffuse = texture.clone();
            }
            if let Some(texture) = &setting.normal {
                normal = texture.clone();
            }
            if let Some(texture) = &setting.specular {
                specular = texture.clone();
            }
            if let Some(shader) = &setting.shader {
                effect = effect_for(shader);
            }
        }
        let entry = ModelMaterial {
            effect,
            diffuse: self.texture(layers, directory, &diffuse, ColorSpace::Srgb)?,
            normal: self.texture(layers, directory, &normal, ColorSpace::Linear)?,
            specular: self.texture(layers, directory, &specular, ColorSpace::Linear)?,
        };
        if let Some(slot) = self.materials.iter().position(|other| *other == entry) {
            return Ok(slot);
        }
        self.materials.push(entry);
        Ok(self.materials.len() - 1)
    }

    /// A material's texture at the one path the engine requests
    /// ([`installed_content::mesh::TextureIndex`]); the texture constructor adopts
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
        let path = layers
            .texture_index()?
            .resolve(directory, file)
            .map_err(invalid)?;
        let dds = path
            .strip_suffix(".tga")
            .map(|stem| format!("{stem}.dds"))
            .into_iter();
        for candidate in dds.chain(std::iter::once(path.clone())) {
            let key = (candidate, space);
            if let Some(slot) = self.paths.get(&key) {
                return Ok(*slot);
            }
            let bytes = match layers.read_uncovered(&key.0) {
                Ok(bytes) => bytes,
                Err(error) if error.kind() == io::ErrorKind::NotFound => continue,
                Err(error) => return Err(error),
            };
            let texture =
                dds::decode(&bytes).map_err(|error| invalid(format!("{}: {error}", key.0)))?;
            self.textures.push(texture);
            self.spaces.push(space);
            let slot = self.textures.len() - 1;
            self.paths.insert(key, slot);
            return Ok(slot);
        }
        Err(invalid(format!("texture {path} is not in the content")))
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
/// (`load_mesh_type` 1422F55B0 passes `meshtype+0x30` to `import_mesh`
/// 14224F250, the skeleton builder 142250010 and the `.mesh` locator reader
/// 14224E890), so the entity's world matrix lacks it, and `14226EF30` places
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

/// Where one attachment hangs, in its parent mesh's own object space.
///
/// **The animation the entity's default state plays is the source**, at
/// [`mesh::FIRST_FRAME`]. The engine attaches to the posed skeleton and this is
/// the same joint transform [`ModelLibrary::push_palette`] builds the skinning
/// matrix from, so an attachment and the hand that carries it are one pose.
/// That pose also carries the uniform scale the engine switches an attachment
/// off with, which the `.mesh` cannot: `GER_infantry_idle_rifle.anim` holds
/// `Left_Hand_node`, `mid_back_node` and `Root_node_2` at scale 0 so the
/// rifleman carries one of his four rifles and neither cigarette, and
/// `cavalry_frame`'s idle holds `infantry` and `horse` at 0 in favour of the
/// combined model on its `cavalry` joint.
///
/// The `.mesh` `[skeleton]`'s inverse bind matrix is the fallback, for a joint
/// the animation does not name or an entity whose state names no animation. It
/// is not the first choice because three of the four shipped rigs prove it is
/// authoring residue: `artillery_frame.mesh` writes an all-zero `tx` for 33 of
/// its 41 joints, `cavalry_frame.mesh` writes 1e12 on two diagonals, and
/// `motorized_frame.mesh` puts its `vehicle` joint at `(-1, 0, 0)` unrotated
/// and unscaled where the idle animation puts it at `(0.074, 0, 1.256)` turned
/// 26.4 degrees about +Y at scale 0.7.
///
/// The pose is composed down the skeleton's parent chain
/// ([`mesh::Mesh::pose`]) because the `.anim` states each joint relative to its
/// parent; the inverse bind is **not**, because `tx` is already a world matrix.
/// The two agree where they can be compared: on `motorized_frame.mesh` the
/// `infantry` joint's `tx` translates by `(-1.88863, 0, 0)` and its animation
/// rest is `(1.88863, 0, 0)` under an identity root.
fn joint_transform(
    decoded: &mesh::Mesh,
    animation: Option<&mesh::Animation>,
    joint: &str,
) -> Option<mesh::Transform> {
    animation
        .and_then(|animation| decoded.pose(animation, mesh::FIRST_FRAME, joint))
        .or_else(|| decoded.bind_pose(joint))
}

/// Where one vertex lands, the way `pdxmesh.wgsl` puts it there: a linear
/// blend of the palette matrices its four slots name, or its own position when
/// it carries no weight at all.
///
/// The pass only needs this on the CPU for the cull radius, but it is the same
/// arithmetic, so a figure that is culled wrong is a figure that is drawn
/// wrong.
fn skinned(palette: &[PaletteJoint], vertex: &ModelVertex) -> [f32; 3] {
    let mut out = [0.0f32; 3];
    let mut any = false;
    for (joint, weight) in vertex.joints.iter().zip(vertex.weights) {
        if weight == 0.0 {
            continue;
        }
        let Some(joint) = palette.get(*joint as usize) else {
            continue;
        };
        any = true;
        for (axis, row) in joint.rows.iter().enumerate() {
            out[axis] += weight
                * (row[0] * vertex.position[0]
                    + row[1] * vertex.position[1]
                    + row[2] * vertex.position[2]
                    + row[3]);
        }
    }
    if any { out } else { vertex.position }
}

// ---------------------------------------------------------------- the field

/// One group of instances of one mesh, the pass's cull and LOD granularity.
#[derive(Clone, Debug, PartialEq)]
pub struct ModelGroup {
    pub mesh: usize,
    /// Slice of [`ModelField::instances`].
    pub range: Range<u32>,
    /// World bounds of the group's instances, `[x0, z0, x1, z1]`, already
    /// widened by the mesh radius.
    pub bounds: [f32; 4],
    /// Centroid used for the LOD decision.
    pub centroid: [f32; 3],
}

/// A placed instance keyed by its mesh and cull tile, as [`ModelField::rebuild`] sorts it.
pub type KeyedInstance = ((usize, i32, i32), ModelInstance);

/// The placed instances of one layer.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ModelField {
    pub instances: Vec<ModelInstance>,
    pub groups: Vec<ModelGroup>,
}

impl ModelField {
    /// Uploaded bytes of the instance stream.
    pub fn byte_len(&self) -> usize {
        bytemuck::cast_slice::<ModelInstance, u8>(&self.instances).len()
    }

    /// Sorts placed instances into one group per (mesh, tile) and records each
    /// group's bounds.
    ///
    /// `tile` is the side of the cull grid in world units. The engine buckets
    /// its trees on 256-unit tiles (section 3.2); nothing in the decompile ties
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
        // Sorted once instead of a tree insert per instance: the same key
        // order the map held, without an allocation per instance.
        keyed.clear();
        keyed.extend(placed.drain(..).map(|(mesh, instance)| {
            (
                (
                    mesh,
                    (instance.position[0] / tile).floor() as i32,
                    (instance.position[2] / tile).floor() as i32,
                ),
                instance,
            )
        }));
        keyed.sort_by_key(|entry| entry.0);
        let field = self;
        field.instances.clear();
        field.groups.clear();
        let mut cursor = 0;
        while cursor < keyed.len() {
            // Keys sort mesh first, so one run of equal keys is one of the
            // map's groups, in the map's iteration order.
            let (mesh, _, _) = keyed[cursor].0;
            let start = cursor;
            while cursor < keyed.len() && keyed[cursor].0 == keyed[start].0 {
                cursor += 1;
            }
            let radius = meshes.get(mesh).map_or(0.0, |entry| entry.radius);
            let instances_start = field.instances.len() as u32;
            let mut bounds = [f32::MAX, f32::MAX, f32::MIN, f32::MIN];
            let mut sum = [0.0f64; 3];
            for ((_, _, _), instance) in &keyed[start..cursor] {
                let reach = radius * instance.scale;
                bounds[0] = bounds[0].min(instance.position[0] - reach);
                bounds[1] = bounds[1].min(instance.position[2] - reach);
                bounds[2] = bounds[2].max(instance.position[0] + reach);
                bounds[3] = bounds[3].max(instance.position[2] + reach);
                for (axis, total) in sum.iter_mut().enumerate() {
                    *total += f64::from(instance.position[axis]);
                }
                field.instances.push(*instance);
            }
            let count = (cursor - start) as f64;
            field.groups.push(ModelGroup {
                mesh,
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

/// The LOD a distance selects: the largest `i` with `loddist[i] <= distance`,
/// clamped to the shapes the mesh actually declares (section 3.5).
pub fn lod_for_distance(lod_distances: &[f32], distance: f32, shapes: usize) -> usize {
    if shapes == 0 {
        return 0;
    }
    let mut lod = 0;
    for (index, limit) in lod_distances.iter().enumerate() {
        if *limit <= distance {
            lod = index;
        }
    }
    lod.min(shapes - 1)
}

/// A deterministic hash of an index to `[0, 1)`, for the rotations and mesh
/// choices the engine makes with a generator the decompile does not give up.
///
/// UNVERIFIED for remaining callers without recorded native random state.
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

/// Bytes one [`ModelVertex`] occupies.
const VERTEX_STRIDE: u64 = 120;
/// Bytes one [`ModelInstance`] occupies.
const INSTANCE_STRIDE: u64 = 64;
/// Bytes the shader reads from a per-material uniform slot.
const SLOT_SIZE: u64 = 16;
/// Bytes one [`PaletteJoint`] occupies: three `vec4<f32>` rows.
const PALETTE_JOINT_SIZE: u64 = 48;
/// Bytes one slot occupies, rounded up to wgpu's uniform offset alignment.
const SLOT_STRIDE: u64 = 256;
/// World-wrap copies, as every other world layer counts them.
const COPIES: usize = 3;

/// The scene textures `pdxmesh.shader` samples that the pass does not own.
pub struct ModelInputs<'a> {
    pub snow_mud: &'a Bound,
    pub reflection: &'a Bound,
}

/// What one frame of the pass needs beyond its own geometry.
#[derive(Clone, Copy)]
pub struct ModelFrame<'a> {
    pub camera: Camera,
    /// World x translations of the wrap copies to submit.
    pub offsets: &'a [f32],
    /// The visible ground rectangle groups are culled against.
    pub visible: Option<[f32; 4]>,
    /// Layer opacity. Every gate in `00_graphics.lua` is a hard switch rather
    /// than a fade, so this is 1.0 for the mesh layers; the railway layer has
    /// a real fade and does not use this pass.
    pub alpha: f32,
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct Slot {
    world_offset_x: f32,
    alpha: f32,
    flags: u32,
    cubemap_intensity: f32,
}

/// The GPU static-mesh pass. One per layer: each owns its own library, its own
/// textures and its own instance buffer.
/// The two vertex buffer layouts every mesh pipeline takes: the mesh vertices
/// and the per-instance records. Shared so the lit pass and the shadow caster
/// cannot drift apart.
fn mesh_buffers() -> [Option<wgpu::VertexBufferLayout<'static>>; 2] {
    const VERTEX: [wgpu::VertexAttribute; 10] = [
        wgpu::VertexAttribute {
            format: wgpu::VertexFormat::Float32x3,
            offset: 0,
            shader_location: 0,
        },
        wgpu::VertexAttribute {
            format: wgpu::VertexFormat::Float32x3,
            offset: 12,
            shader_location: 1,
        },
        wgpu::VertexAttribute {
            format: wgpu::VertexFormat::Float32x4,
            offset: 24,
            shader_location: 2,
        },
        wgpu::VertexAttribute {
            format: wgpu::VertexFormat::Float32x2,
            offset: 40,
            shader_location: 3,
        },
        // The skinning pair: four joint indices and their weights.
        wgpu::VertexAttribute {
            format: wgpu::VertexFormat::Uint32x4,
            offset: 48,
            shader_location: 7,
        },
        wgpu::VertexAttribute {
            format: wgpu::VertexFormat::Float32x4,
            offset: 64,
            shader_location: 8,
        },
        wgpu::VertexAttribute {
            format: wgpu::VertexFormat::Float32x3,
            offset: 80,
            shader_location: 12,
        },
        wgpu::VertexAttribute {
            format: wgpu::VertexFormat::Float32x3,
            offset: 92,
            shader_location: 13,
        },
        wgpu::VertexAttribute {
            format: wgpu::VertexFormat::Float32x3,
            offset: 104,
            shader_location: 14,
        },
        wgpu::VertexAttribute {
            format: wgpu::VertexFormat::Uint32,
            offset: 116,
            shader_location: 15,
        },
    ];
    const INSTANCE: [wgpu::VertexAttribute; 6] = [
        wgpu::VertexAttribute {
            format: wgpu::VertexFormat::Float32x3,
            offset: 0,
            shader_location: 4,
        },
        wgpu::VertexAttribute {
            format: wgpu::VertexFormat::Float32,
            offset: 12,
            shader_location: 5,
        },
        wgpu::VertexAttribute {
            format: wgpu::VertexFormat::Float32,
            offset: 16,
            shader_location: 6,
        },
        wgpu::VertexAttribute {
            format: wgpu::VertexFormat::Float32x2,
            offset: 20,
            shader_location: 9,
        },
        wgpu::VertexAttribute {
            format: wgpu::VertexFormat::Float32x4,
            offset: 28,
            shader_location: 10,
        },
        wgpu::VertexAttribute {
            format: wgpu::VertexFormat::Uint32,
            offset: 44,
            shader_location: 11,
        },
    ];
    [
        Some(wgpu::VertexBufferLayout {
            array_stride: VERTEX_STRIDE,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: &VERTEX,
        }),
        Some(wgpu::VertexBufferLayout {
            array_stride: INSTANCE_STRIDE,
            step_mode: wgpu::VertexStepMode::Instance,
            attributes: &INSTANCE,
        }),
    ]
}

pub struct ModelPass {
    /// `BlendState` (all channels: the alpha is the emissive mask the bloom
    /// seed reads) and, for `TRAIN` materials, `BlendStateAlphaTestTrain`'s
    /// `WriteMask = "RED|GREEN|BLUE"`, which leaves that mask alone.
    pipelines: [wgpu::RenderPipeline; 2],
    /// `pdxmesh.shader`'s `PdxMesh*Shadow` family, which writes the mesh's
    /// depth into the light's map (`map::shadow`). Every map object this pass
    /// draws takes `VertexPdxMeshStandardShadow`; only the alpha test tells the
    /// `PixelPdxMeshStandardShadow` and `PixelPdxMeshAlphaBlendShadow` variants
    /// apart, and the material's flag word already carries it.
    caster: wgpu::RenderPipeline,
    layout: wgpu::BindGroupLayout,
    slots: wgpu::Buffer,
    /// `matBones`, every composed entity's joints end to end.
    palette: wgpu::Buffer,
    /// The library palette length [`ModelPass::palette`] was built from.
    palette_len: usize,
    /// Uploaded when a resident mesh first needs one.
    textures: Vec<Option<Bound>>,
    materials: Vec<ModelMaterial>,
    /// Whether a material's textures are uploaded and its groups can be built.
    ready: Vec<bool>,
    meshes: Vec<ModelMesh>,
    /// `material * COPIES + copy`; `None` until the material is ready and the
    /// scene's inputs are known.
    groups: Vec<Option<wgpu::BindGroup>>,
    /// The scene inputs of the last [`ModelPass::rebind`], for groups built
    /// later.
    inputs: Option<BoundInputs>,
    /// Geometry is uploaded per mesh, only for meshes a field places, into
    /// chunk buffers no larger than the device allows.
    chunks: Vec<Chunk>,
    packer: Packer,
    /// Per library mesh: where its geometry lives, once uploaded.
    resident: Vec<Option<Placement>>,
    /// Meshes that cannot be uploaded on this device, with the reason; they
    /// are not drawn.
    refused: BTreeMap<usize, String>,
    instances: Option<wgpu::Buffer>,
    field: ModelField,
    drawn: std::cell::Cell<u32>,
    label: &'static str,
    rigs: Vec<Arc<Rig>>,
    default_palette: Vec<PaletteJoint>,
    players: BTreeMap<(usize, u32, u32), entity_animation::Player>,
    pub events: Vec<entity_animation::Event>,
}

/// Clones of the scene's views and samplers a material's bind group needs.
struct BoundInputs {
    snow_mud: wgpu::TextureView,
    reflection: wgpu::TextureView,
    linear_wrap: wgpu::Sampler,
    linear_clamp: wgpu::Sampler,
}

/// One pair of geometry buffers.
struct Chunk {
    vertices: wgpu::Buffer,
    indices: wgpu::Buffer,
}

/// Target sizes of one geometry chunk. A mesh larger than the target, but
/// within the device's buffer limit, gets a chunk of its own.
const CHUNK_VERTEX_BYTES: u64 = 64 << 20;
const CHUNK_INDEX_BYTES: u64 = 16 << 20;

/// Where one mesh's geometry lives: its chunk, and the first vertex and index
/// it occupies there.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Placement {
    chunk: usize,
    vertex: u64,
    index: u64,
    /// The mesh's first index in the library, which its parts' ranges count from.
    library_index: u64,
}

/// The room left in each chunk, in elements.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct ChunkSpace {
    vertex_cap: u64,
    vertex_len: u64,
    index_cap: u64,
    index_len: u64,
}

/// Chunk bookkeeping, apart from the device so the packing rule is testable.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct Packer {
    chunks: Vec<ChunkSpace>,
}

impl Packer {
    /// Places a mesh of `vertices` and `indices` elements in the first chunk
    /// with room, else in a new chunk, which the caller then allocates at the
    /// returned capacities. `limit` is the device's largest buffer in bytes. A
    /// mesh that alone exceeds it is refused with the reason.
    fn place(
        &mut self,
        vertices: u64,
        indices: u64,
        limit: u64,
    ) -> Result<(Placement, Option<ChunkSpace>), String> {
        let vertex_bytes = vertices.saturating_mul(VERTEX_STRIDE);
        let index_bytes = indices.saturating_mul(4);
        if vertex_bytes > limit || index_bytes > limit {
            return Err(format!(
                "needs {vertex_bytes} vertex bytes and {index_bytes} index bytes, above the device's {limit}-byte buffer limit"
            ));
        }
        for (chunk, space) in self.chunks.iter_mut().enumerate() {
            if space.vertex_cap - space.vertex_len >= vertices
                && space.index_cap - space.index_len >= indices
            {
                let placement = Placement {
                    chunk,
                    vertex: space.vertex_len,
                    index: space.index_len,
                    library_index: 0,
                };
                space.vertex_len += vertices;
                space.index_len += indices;
                return Ok((placement, None));
            }
        }
        let space = ChunkSpace {
            vertex_cap: CHUNK_VERTEX_BYTES.min(limit).max(vertex_bytes) / VERTEX_STRIDE,
            vertex_len: vertices,
            index_cap: CHUNK_INDEX_BYTES.min(limit).max(index_bytes) / 4,
            index_len: indices,
        };
        self.chunks.push(space);
        Ok((
            Placement {
                chunk: self.chunks.len() - 1,
                vertex: 0,
                index: 0,
                library_index: 0,
            },
            Some(space),
        ))
    }
}

/// A mesh's contiguous index span in its library and the vertices those
/// indices name; `None` for a mesh with nothing to draw.
fn mesh_span(library: &ModelLibrary, mesh: usize) -> Option<(Range<u32>, Range<u32>)> {
    let parts = library.meshes.get(mesh)?.lods.iter().flatten();
    let (mut first, mut last) = (u32::MAX, 0);
    for part in parts.filter(|part| !part.range.is_empty()) {
        first = first.min(part.range.start);
        last = last.max(part.range.end);
    }
    if first >= last {
        return None;
    }
    let indices = library.indices.get(first as usize..last as usize)?;
    let low = *indices.iter().min()?;
    let high = *indices.iter().max()?;
    Some((first..last, low..high + 1))
}

impl ModelPass {
    /// Samples visible entity palettes; meshes and material buffers are immutable.
    /// Returns true only when the palette allocation grew and bindings need refreshing.
    pub fn animate(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        time: f32,
        rectangle: Option<[f32; 4]>,
        offsets: &[f32],
    ) -> io::Result<bool> {
        static PROFILE: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
        let profile = *PROFILE.get_or_init(|| std::env::var_os("BATTLE_PLAN_ANIMATION_PROFILE").or_else(|| std::env::var_os("HOI4_ANIMATION_PROFILE")).is_some());
        let started = profile.then(std::time::Instant::now);
        self.events.clear();
        if time <= 0.0 {
            self.players.clear();
            for instance in &mut self.field.instances {
                instance.palette_offset = 0;
            }
            if let Some(instances) = &self.instances {
                queue.write_buffer(instances, 0, bytemuck::cast_slice(&self.field.instances));
            }
            return Ok(false);
        }
        let mut palette = self.default_palette.clone();
        let mut alive = BTreeSet::new();
        for instance in &mut self.field.instances {
            instance.palette_offset = 0;
        }
        for group in &self.field.groups {
            if !offsets
                .iter()
                .any(|offset| visible(group, *offset, rectangle))
            {
                continue;
            }
            let Some(rig) = self.rigs.get(group.mesh) else {
                continue;
            };
            if rig.nodes.iter().all(|n| n.clips.is_empty()) {
                continue;
            }
            for index in group.range.clone() {
                let instance = &mut self.field.instances[index as usize];
                let key = (
                    group.mesh,
                    instance.animation_seed,
                    instance.animation_epoch,
                );
                alive.insert(key);
                let player = self
                    .players
                    .entry(key)
                    .or_insert_with(|| entity_animation::Player::new(rig));
                let start = palette.len();
                palette.extend_from_slice(&self.default_palette[rig.start..rig.end]);
                instance.palette_offset = (start - rig.start) as u32;
                player.evaluate(
                    rig,
                    *instance,
                    time,
                    &mut palette[start..],
                    &mut self.events,
                );
            }
        }
        self.players.retain(|key, _| alive.contains(key));
        let bytes = bytemuck::cast_slice(&palette);
        let grew = self.palette.size() < bytes.len() as u64;
        if grew {
            let capacity = (bytes.len() as u64).next_power_of_two();
            if capacity > device.limits().max_storage_buffer_binding_size {
                return Err(invalid(
                    "animated joint palettes exceed device binding limit",
                ));
            }
            self.palette = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("animated joint palettes"),
                size: capacity,
                usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
        }
        queue.write_buffer(&self.palette, 0, bytes);
        if let Some(instances) = &self.instances {
            queue.write_buffer(instances, 0, bytemuck::cast_slice(&self.field.instances));
        }
        if let Some(started) = started {
            eprintln!(
                "Animation: time={time:.6} players={} events={} palette_bytes={} instance_bytes={} cpu_us={}",
                alive.len(),
                self.events.len(),
                bytes.len(),
                self.field.instances.len() * std::mem::size_of::<ModelInstance>(),
                started.elapsed().as_micros()
            );
        }
        Ok(grew)
    }

    /// Moving layers retain mesh buffers and replace only their small instance stream.
    /// A seek or a branch change starts new entity histories.
    pub fn reset_animation(&mut self) {
        self.players.clear();
        self.events.clear();
    }

    /// Replaces the placed instances of a pass whose library is uploaded.
    pub fn update_placed(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        placed: Vec<(usize, ModelInstance)>,
        tile: f32,
    ) -> io::Result<()> {
        // Retire the players of entities no longer placed, so a recreated
        // entity starts a fresh clock rather than replaying old events.
        let alive: BTreeSet<_> = placed
            .iter()
            .map(|(mesh, instance)| (*mesh, instance.animation_seed, instance.animation_epoch))
            .collect();
        self.players.retain(|key, _| alive.contains(key));
        self.events.clear();
        let field = ModelField::build_meshes(&self.meshes, placed, tile);
        let bytes = bytemuck::cast_slice(&field.instances);
        if !bytes.is_empty() {
            if let Some(instances) = &self.instances
                && instances.size() >= bytes.len() as u64
            {
                queue.write_buffer(instances, 0, bytes);
            } else {
                self.instances = Some(buffer(
                    device,
                    "placed pdxmesh instances",
                    wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
                    bytes,
                )?);
            }
        }
        self.field = field;
        Ok(())
    }

    pub fn update_instances(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        library: &ModelLibrary,
        field: &mut ModelField,
    ) -> io::Result<()> {
        if self.chunks.is_empty() && !field.instances.is_empty() {
            let min_capacity = field.byte_len() as u64;
            return self.upload(device, queue, library, std::mem::take(field), min_capacity);
        }
        self.sync(device, library)?;
        for group in &field.groups {
            self.make_resident(device, queue, library, group.mesh)?;
        }
        let bytes = bytemuck::cast_slice(&field.instances);
        if !bytes.is_empty() {
            if let Some(instances) = &self.instances
                && instances.size() >= bytes.len() as u64
            {
                queue.write_buffer(instances, 0, bytes);
            } else {
                self.instances = Some(buffer(
                    device,
                    "moving pdxmesh instances",
                    wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
                    bytes,
                )?);
            }
        }
        // The caller gets the previous frame's field back to rebuild into.
        std::mem::swap(&mut self.field, field);
        Ok(())
    }
    /// Builds the pipeline; textures and geometry upload as meshes become
    /// resident ([`ModelPass::upload_library`], [`ModelPass::upload`]).
    pub fn new(
        device: &wgpu::Device,
        samples: u32,
        _queue: &wgpu::Queue,
        globals: &wgpu::BindGroupLayout,
        format: wgpu::TextureFormat,
        library: &ModelLibrary,
        label: &'static str,
    ) -> io::Result<Self> {
        let mut entries = (0..4)
            .map(|binding| {
                texture_entry(
                    binding,
                    wgpu::ShaderStages::FRAGMENT,
                    wgpu::TextureViewDimension::D2,
                )
            })
            .collect::<Vec<_>>();
        entries.push(texture_entry(
            4,
            wgpu::ShaderStages::FRAGMENT,
            wgpu::TextureViewDimension::Cube,
        ));
        entries.push(sampler_entry(5));
        entries.push(sampler_entry(6));
        entries.push(wgpu::BindGroupLayoutEntry {
            binding: 7,
            visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Uniform,
                has_dynamic_offset: false,
                min_binding_size: wgpu::BufferSize::new(SLOT_SIZE),
            },
            count: None,
        });
        // `matBones`. A uniform block would not hold it: the unit library's
        // palette is thousands of joints and the 64 KiB uniform limit is a few
        // hundred.
        entries.push(wgpu::BindGroupLayoutEntry {
            binding: 8,
            visibility: wgpu::ShaderStages::VERTEX,
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Storage { read_only: true },
                has_dynamic_offset: false,
                min_binding_size: wgpu::BufferSize::new(PALETTE_JOINT_SIZE),
            },
            count: None,
        });
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("pdxmesh inputs"),
            entries: &entries,
        });
        let module = module(device, "pdxmesh", include_str!("pdxmesh.wgsl"));
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some(label),
            bind_group_layouts: &[Some(globals), Some(&layout)],
            immediate_size: 0,
        });
        let pipeline = |write_mask| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(label),
                layout: Some(&pipeline_layout),
                vertex: wgpu::VertexState {
                    module: &module,
                    entry_point: Some("mesh_vertex"),
                    compilation_options: Default::default(),
                    buffers: &mesh_buffers(),
                },
                primitive: wgpu::PrimitiveState {
                    // The `.mesh` winding is D3D left-handed
                    // (`pdxmesh.shader:145-154`) and wgpu's front face is
                    // counter-clockwise, so a face test here would cull the wrong
                    // half. These meshes are closed solids drawn with a depth test,
                    // where drawing both faces is indistinguishable from culling
                    // the right one.
                    cull_mode: None,
                    ..Default::default()
                },
                // `Effect PdxMeshStandard` takes
                // `DepthStencilStateDisableTransparencyPassthrough`: depth test and
                // write on, with LessEqual (frame33840 event10850). The stencil half tags pixels for the engine's
                // transparency passthrough, which this port has no pass for.
                depth_stencil: Some(wgpu::DepthStencilState {
                    format: DEPTH_FORMAT,
                    depth_write_enabled: Some(true),
                    depth_compare: Some(wgpu::CompareFunction::LessEqual),
                    stencil: Default::default(),
                    bias: Default::default(),
                }),
                multisample: wgpu::MultisampleState {
                    count: samples,
                    ..Default::default()
                },
                fragment: Some(wgpu::FragmentState {
                    module: &module,
                    entry_point: Some("mesh_fragment"),
                    compilation_options: Default::default(),
                    targets: &[Some(wgpu::ColorTargetState {
                        format,
                        blend: None,
                        write_mask,
                    })],
                }),
                multiview_mask: None,
                cache: None,
            })
        };
        let caster = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("PdxMeshStandardShadow"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &module,
                entry_point: Some("mesh_shadow_vertex"),
                compilation_options: Default::default(),
                buffers: &mesh_buffers(),
            },
            primitive: wgpu::PrimitiveState {
                cull_mode: None,
                ..Default::default()
            },
            depth_stencil: Some(wgpu::DepthStencilState {
                format: DEPTH_FORMAT,
                depth_write_enabled: Some(true),
                depth_compare: Some(wgpu::CompareFunction::Less),
                stencil: Default::default(),
                bias: Default::default(),
            }),
            multisample: Default::default(),
            fragment: Some(wgpu::FragmentState {
                module: &module,
                entry_point: Some("mesh_shadow_fragment"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: DEPTH_MAP_FORMAT,
                    blend: None,
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            multiview_mask: None,
            cache: None,
        });
        let slots = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("pdxmesh ConstantBuffer(1, 28)"),
            size: SLOT_STRIDE * (library.materials.len().max(1) * COPIES) as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let (palette, default_palette) = palette_buffer(device, &library.palette)?;
        Ok(Self {
            pipelines: [
                pipeline(wgpu::ColorWrites::ALL),
                pipeline(wgpu::ColorWrites::COLOR),
            ],
            caster,
            layout,
            slots,
            palette,
            palette_len: library.palette.len(),
            textures: (0..library.textures.len()).map(|_| None).collect(),
            materials: library.materials.clone(),
            ready: vec![false; library.materials.len()],
            meshes: library.meshes.clone(),
            groups: (0..library.materials.len() * COPIES)
                .map(|_| None)
                .collect(),
            inputs: None,
            chunks: Vec::new(),
            packer: Packer::default(),
            resident: vec![None; library.meshes.len()],
            refused: BTreeMap::new(),
            instances: None,
            field: ModelField::default(),
            drawn: std::cell::Cell::new(0),
            label,
            rigs: library.rigs.clone(),
            default_palette,
            players: BTreeMap::new(),
            events: Vec::new(),
        })
    }

    /// Rebuilds the bind groups; needed whenever a bound scene texture is
    /// replaced. The inputs are kept so a material that becomes ready later
    /// gets its groups too.
    pub fn rebind(&mut self, device: &wgpu::Device, samplers: &Samplers, inputs: ModelInputs<'_>) {
        self.inputs = Some(BoundInputs {
            snow_mud: inputs.snow_mud.view.clone(),
            reflection: inputs.reflection.view.clone(),
            linear_wrap: samplers.linear_wrap.clone(),
            linear_clamp: samplers.linear_clamp.clone(),
        });
        self.rebuild_groups(device);
    }

    /// Builds the groups of every ready material.
    fn rebuild_groups(&mut self, device: &wgpu::Device) {
        for group in &mut self.groups {
            *group = None;
        }
        for material in 0..self.materials.len() {
            if self.ready[material] {
                self.build_groups(device, material);
            }
        }
    }

    fn build_groups(&mut self, device: &wgpu::Device, index: usize) {
        let Some(inputs) = &self.inputs else {
            return;
        };
        let material = &self.materials[index];
        let (Some(diffuse), Some(specular), Some(normal)) = (
            &self.textures[material.diffuse],
            &self.textures[material.specular],
            &self.textures[material.normal],
        ) else {
            return;
        };
        let views = [
            &diffuse.view,
            &specular.view,
            &normal.view,
            &inputs.snow_mud,
            &inputs.reflection,
        ];
        for copy in 0..COPIES {
            let mut bindings = views
                .iter()
                .enumerate()
                .map(|(binding, view)| wgpu::BindGroupEntry {
                    binding: binding as u32,
                    resource: wgpu::BindingResource::TextureView(view),
                })
                .collect::<Vec<_>>();
            bindings.push(wgpu::BindGroupEntry {
                binding: 5,
                resource: wgpu::BindingResource::Sampler(&inputs.linear_wrap),
            });
            bindings.push(wgpu::BindGroupEntry {
                binding: 6,
                resource: wgpu::BindingResource::Sampler(&inputs.linear_clamp),
            });
            bindings.push(wgpu::BindGroupEntry {
                binding: 7,
                resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                    buffer: &self.slots,
                    offset: SLOT_STRIDE * (index * COPIES + copy) as u64,
                    size: wgpu::BufferSize::new(SLOT_SIZE),
                }),
            });
            bindings.push(wgpu::BindGroupEntry {
                binding: 8,
                resource: self.palette.as_entire_binding(),
            });
            self.groups[index * COPIES + copy] =
                Some(device.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: Some(self.label),
                    layout: &self.layout,
                    entries: &bindings,
                }));
        }
    }

    /// Follows a library that grew since the pass last saw it (the unit
    /// library bakes entities as hours first place them). Every library only
    /// ever appends, so existing slots keep their meaning.
    fn sync(&mut self, device: &wgpu::Device, library: &ModelLibrary) -> io::Result<()> {
        let mut regroup = false;
        if self.meshes.len() < library.meshes.len() {
            self.meshes
                .extend_from_slice(&library.meshes[self.meshes.len()..]);
            self.rigs
                .extend_from_slice(&library.rigs[self.rigs.len()..]);
            self.resident.resize(library.meshes.len(), None);
        }
        if self.textures.len() < library.textures.len() {
            self.textures.resize_with(library.textures.len(), || None);
        }
        if self.materials.len() < library.materials.len() {
            self.materials
                .extend_from_slice(&library.materials[self.materials.len()..]);
            self.ready.resize(library.materials.len(), false);
            self.groups
                .resize_with(library.materials.len() * COPIES, || None);
            let needed = SLOT_STRIDE * (library.materials.len() * COPIES) as u64;
            if self.slots.size() < needed {
                self.slots = device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some("pdxmesh ConstantBuffer(1, 28)"),
                    size: needed.next_power_of_two(),
                    usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                    mapped_at_creation: false,
                });
                regroup = true;
            }
        }
        if self.palette_len != library.palette.len() {
            let (palette, joints) = palette_buffer(device, &library.palette)?;
            self.palette = palette;
            self.default_palette = joints;
            self.palette_len = library.palette.len();
            // Library slots only append; new meshes must not restart the
            // retained unit entities' animation clocks.
            regroup = true;
        }
        if regroup {
            self.rebuild_groups(device);
        }
        Ok(())
    }

    /// Uploads one mesh's geometry and its materials' textures, once. A mesh
    /// the device cannot hold is refused with a named reason and not drawn.
    fn make_resident(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        library: &ModelLibrary,
        mesh: usize,
    ) -> io::Result<()> {
        if self.resident.get(mesh).is_none_or(Option::is_some) || self.refused.contains_key(&mesh) {
            return Ok(());
        }
        let Some((indices, vertices)) = mesh_span(library, mesh) else {
            return Ok(());
        };
        let limit = device.limits().max_buffer_size;
        let (mut placement, fresh) = match self.packer.place(
            u64::from(vertices.end - vertices.start),
            u64::from(indices.end - indices.start),
            limit,
        ) {
            Ok(placed) => placed,
            Err(reason) => {
                let reason = format!(
                    "{} mesh {} ({}) {reason}; it is not drawn",
                    self.label, library.meshes[mesh].name, library.meshes[mesh].path
                );
                eprintln!("Warning: {reason}");
                self.refused.insert(mesh, reason);
                return Ok(());
            }
        };
        if let Some(space) = fresh {
            let buffer = |label: &str, size: u64, usage| {
                device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some(label),
                    size: size.max(wgpu::COPY_BUFFER_ALIGNMENT),
                    usage: usage | wgpu::BufferUsages::COPY_DST,
                    mapped_at_creation: false,
                })
            };
            self.chunks.push(Chunk {
                vertices: buffer(
                    "pdxmesh vertices",
                    space.vertex_cap * VERTEX_STRIDE,
                    wgpu::BufferUsages::VERTEX,
                ),
                indices: buffer(
                    "pdxmesh indices",
                    space.index_cap * 4,
                    wgpu::BufferUsages::INDEX,
                ),
            });
        }
        let chunk = &self.chunks[placement.chunk];
        queue.write_buffer(
            &chunk.vertices,
            placement.vertex * VERTEX_STRIDE,
            bytemuck::cast_slice(&library.vertices[vertices.start as usize..vertices.end as usize]),
        );
        let base = placement.vertex as u32;
        let rebased: Vec<u32> = library.indices[indices.start as usize..indices.end as usize]
            .iter()
            .map(|index| index - vertices.start + base)
            .collect();
        queue.write_buffer(
            &chunk.indices,
            placement.index * 4,
            bytemuck::cast_slice(&rebased),
        );
        placement.library_index = u64::from(indices.start);
        self.resident[mesh] = Some(placement);
        let materials: BTreeSet<usize> = library.meshes[mesh]
            .lods
            .iter()
            .flatten()
            .map(|part| part.material)
            .collect();
        for material in materials {
            self.make_ready(device, queue, library, material)?;
        }
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
        self.build_groups(device, material);
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
        self.sync(device, library)?;
        for mesh in 0..library.meshes.len() {
            self.make_resident(device, queue, library, mesh)?;
        }
        Ok(())
    }

    /// Uploads the geometry the field places and the placed instances.
    ///
    /// `min_capacity` sizes a new instance buffer: the units layer passes the
    /// all-bands size so zooming between height bands rewrites the same buffer
    /// instead of reallocating mid-gesture; other layers pass their bytes.
    pub fn upload(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        library: &ModelLibrary,
        field: ModelField,
        min_capacity: u64,
    ) -> io::Result<()> {
        if library.indices.is_empty() || field.instances.is_empty() {
            self.instances = None;
            self.field = ModelField::default();
            return Ok(());
        }
        self.sync(device, library)?;
        for group in &field.groups {
            self.make_resident(device, queue, library, group.mesh)?;
        }
        // The instance stream is rewritten whenever the hour changes, but the
        // buffer is kept while it fits: reallocating it stalls the frame that
        // crosses into an hour with more instances.
        let bytes = bytemuck::cast_slice(&field.instances);
        if let Some(instances) = &self.instances
            && instances.size() >= bytes.len() as u64
        {
            queue.write_buffer(instances, 0, bytes);
        } else {
            let capacity = (bytes.len() as u64).max(min_capacity).max(64);
            let grown = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("pdxmesh instances"),
                size: capacity,
                usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
            queue.write_buffer(&grown, 0, bytes);
            self.instances = Some(grown);
            if super::counter_move::profiling() {
                super::counter_move::profile_print(format!(
                    "PROFILE instances buffer {} bytes capacity {capacity} pass {}",
                    bytes.len(),
                    self.label,
                ));
            }
        }
        self.field = field;
        Ok(())
    }

    /// Instances the last frame drew, for reporting.
    pub fn drawn(&self) -> u32 {
        self.drawn.get()
    }

    /// Draws the layer, once per visible world-wrap copy.
    pub fn draw(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        queue: &wgpu::Queue,
        targets: &Targets,
        globals: &wgpu::BindGroup,
        frame: ModelFrame<'_>,
    ) {
        self.drawn.set(0);
        let Some(copies) = self.upload_slots(queue, frame) else {
            return;
        };
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some(self.label),
            color_attachments: &[Some(targets.scene_attachment(wgpu::LoadOp::Load))],
            depth_stencil_attachment: Some(targets.depth_attachment(wgpu::LoadOp::Load)),
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        let [standard, train] = &self.pipelines;
        let drawn = self.record(&mut pass, [standard, train], globals, frame, copies);
        self.drawn.set(drawn);
    }

    /// The `PdxMesh*Shadow` draw into the light's depth map. `frame.visible` is
    /// the caster rectangle (`map::shadow::caster_rect`) and `frame.camera` the
    /// real camera, so the LOD the shadow is cut from is the LOD that is drawn.
    pub fn draw_casters(
        &self,
        pass: &mut wgpu::RenderPass<'_>,
        queue: &wgpu::Queue,
        globals: &wgpu::BindGroup,
        frame: ModelFrame<'_>,
    ) {
        let Some(copies) = self.upload_slots(queue, frame) else {
            return;
        };
        self.record(pass, [&self.caster; 2], globals, frame, copies);
    }

    /// Fills `ConstantBuffer(1, 28)` for every material and wrap copy and
    /// reports how many copies this frame submits.
    fn upload_slots(&self, queue: &wgpu::Queue, frame: ModelFrame<'_>) -> Option<usize> {
        if self.chunks.is_empty() {
            return None;
        }
        self.instances.as_ref()?;
        if self.groups.is_empty() || frame.alpha <= 0.0 {
            return None;
        }
        let mut bytes = Vec::with_capacity(self.groups.len() * SLOT_STRIDE as usize);
        for material in &self.materials {
            for copy in 0..COPIES {
                let slot = Slot {
                    world_offset_x: frame.offsets.get(copy).copied().unwrap_or(0.0),
                    alpha: frame.alpha,
                    flags: material.effect.bits(),
                    cubemap_intensity: frame.camera.defines.cubemap,
                };
                bytes.extend_from_slice(bytemuck::bytes_of(&slot));
                bytes.resize(bytes.len() + SLOT_STRIDE as usize - SLOT_SIZE as usize, 0);
            }
        }
        queue.write_buffer(&self.slots, 0, &bytes);
        Some(frame.offsets.len().min(COPIES))
    }

    /// The group walk the colour and caster pipelines share, and the instances
    /// it drew. `pipelines[1]` draws `TRAIN` materials.
    fn record(
        &self,
        pass: &mut wgpu::RenderPass<'_>,
        pipelines: [&wgpu::RenderPipeline; 2],
        globals: &wgpu::BindGroup,
        frame: ModelFrame<'_>,
        copies: usize,
    ) -> u32 {
        let Some(instances) = &self.instances else {
            return 0;
        };
        let eye = frame.camera.eye();
        let mut current = None;
        let mut bound = None;
        pass.set_bind_group(0, globals, &[]);
        pass.set_vertex_buffer(1, instances.slice(..));
        let mut drawn = 0u32;
        for copy in 0..copies {
            let offset = frame.offsets[copy];
            for group in &self.field.groups {
                if !visible(group, offset, frame.visible) {
                    continue;
                }
                let Some(mesh) = self.meshes.get(group.mesh) else {
                    continue;
                };
                // A mesh the field placed is resident unless this device
                // refused it.
                let Some(placement) = self.resident.get(group.mesh).copied().flatten() else {
                    continue;
                };
                if bound != Some(placement.chunk) {
                    let chunk = &self.chunks[placement.chunk];
                    pass.set_vertex_buffer(0, chunk.vertices.slice(..));
                    pass.set_index_buffer(chunk.indices.slice(..), wgpu::IndexFormat::Uint32);
                    bound = Some(placement.chunk);
                }
                let shift = placement.index as i64 - placement.library_index as i64;
                let distance = ((group.centroid[0] + offset - eye[0]).powi(2)
                    + (group.centroid[1] - eye[1]).powi(2)
                    + (group.centroid[2] - eye[2]).powi(2))
                .sqrt();
                let lod = lod_for_distance(&mesh.lod_distances, distance, mesh.lods.len());
                let mut any = false;
                for part in &mesh.lods[lod] {
                    if part.range.is_empty() {
                        continue;
                    }
                    let wanted = usize::from(self.materials[part.material].effect.train);
                    if current != Some(wanted) {
                        pass.set_pipeline(pipelines[wanted]);
                        current = Some(wanted);
                    }
                    let Some(material) = &self.groups[part.material * COPIES + copy] else {
                        continue;
                    };
                    pass.set_bind_group(1, material, &[]);
                    let range = (i64::from(part.range.start) + shift) as u32
                        ..(i64::from(part.range.end) + shift) as u32;
                    pass.draw_indexed(range, 0, group.range.clone());
                    any = true;
                }
                if any {
                    drawn += group.range.end - group.range.start;
                }
            }
        }
        drawn
    }
}

/// The `matBones` storage buffer of a library palette, and the joints it holds.
/// A library with no skinned mesh still needs a bindable buffer, and one
/// identity joint is the smallest legal one. No vertex names it: an unskinned
/// vertex carries four zero weights and never reads the palette at all. A
/// palette larger than the device binds is refused by name.
fn palette_buffer(
    device: &wgpu::Device,
    palette: &[PaletteJoint],
) -> io::Result<(wgpu::Buffer, Vec<PaletteJoint>)> {
    let joints = if palette.is_empty() {
        vec![PaletteJoint::IDENTITY]
    } else {
        palette.to_vec()
    };
    let bytes = (joints.len() as u64) * PALETTE_JOINT_SIZE;
    let limits = device.limits();
    if bytes > limits.max_storage_buffer_binding_size || bytes > limits.max_buffer_size {
        return Err(invalid(format!(
            "pdxmesh matBones needs {bytes} bytes, above the device's storage binding limit of {}",
            limits.max_storage_buffer_binding_size
        )));
    }
    let buffer = buffer(
        device,
        "pdxmesh matBones",
        wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
        bytemuck::cast_slice(&joints),
    )?;
    Ok((buffer, joints))
}

/// Whether a group reaches the visible ground rectangle at this wrap offset.
fn visible(group: &ModelGroup, offset: f32, rectangle: Option<[f32; 4]>) -> bool {
    let Some([x0, z0, x1, z1]) = rectangle else {
        return true;
    };
    let [gx0, gz0, gx1, gz1] = group.bounds;
    gx0 + offset <= x1 && gx1 + offset >= x0 && gz0 <= z1 && gz1 >= z0
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Five thousand meshes of about a megabyte each, far above any single
    /// buffer, pack into chunks no larger than the device allows; a mesh that
    /// alone exceeds the limit is refused by name, and nothing overlaps.
    #[test]
    fn a_large_mesh_set_packs_into_bounded_chunks_and_an_oversized_mesh_is_refused() {
        const LIMIT: u64 = 256 << 20;
        let mut packer = Packer::default();
        let mut used: Vec<Vec<(u64, u64, u64, u64)>> = Vec::new();
        let (mut total_vertices, mut placed) = (0_u64, 0);
        for mesh in 0..5_000_u64 {
            let vertices = 8_000 + (mesh * 7_919) % 4_000;
            let indices = vertices * 3;
            let (placement, fresh) = packer.place(vertices, indices, LIMIT).unwrap();
            if let Some(space) = fresh {
                assert!(space.vertex_cap * VERTEX_STRIDE <= LIMIT);
                assert!(space.index_cap * 4 <= LIMIT);
                used.push(Vec::new());
            }
            used[placement.chunk].push((placement.vertex, vertices, placement.index, indices));
            total_vertices += vertices;
            placed += 1;
        }
        assert_eq!(placed, 5_000);
        assert!(
            total_vertices * VERTEX_STRIDE > 4 * LIMIT,
            "the set is far above one buffer"
        );
        for (chunk, meshes) in used.iter().enumerate() {
            let space = packer.chunks[chunk];
            let mut vertex_end = 0;
            let mut index_end = 0;
            for &(vertex, vertices, index, indices) in meshes {
                assert!(vertex >= vertex_end && index >= index_end, "no overlap");
                vertex_end = vertex + vertices;
                index_end = index + indices;
            }
            assert!(vertex_end <= space.vertex_cap && index_end <= space.index_cap);
        }
        // A mesh above the target but within the limit gets its own chunk.
        let big = (CHUNK_VERTEX_BYTES / VERTEX_STRIDE) * 2;
        let (placement, fresh) = packer.place(big, 3, LIMIT).unwrap();
        assert_eq!(placement.vertex, 0);
        assert!(fresh.unwrap().vertex_cap >= big);
        // One mesh alone above the limit is refused, and nothing is placed.
        let before = packer.clone();
        let error = packer
            .place(LIMIT / VERTEX_STRIDE + 1, 3, LIMIT)
            .unwrap_err();
        assert!(error.contains("buffer limit"), "{error}");
        assert_eq!(packer, before);
    }

    #[test]
    fn a_mesh_span_covers_every_lod_and_the_vertices_it_names() {
        let mut library = ModelLibrary {
            vertices: vec![ModelVertex::default(); 10],
            indices: vec![0, 1, 2, 5, 6, 7, 6, 8, 9],
            ..ModelLibrary::default()
        };
        let mesh = |lods: Vec<Vec<ModelPart>>| ModelMesh {
            name: "m".into(),
            path: "m.mesh".into(),
            scale: 1.0,
            lod_distances: Vec::new(),
            lods,
            radius: 0.0,
            height: 0.0,
            own_radius: 0.0,
            attachments: 0,
        };
        library.meshes.push(mesh(vec![vec![ModelPart {
            material: 0,
            range: 0..3,
        }]]));
        library.meshes.push(mesh(vec![
            vec![ModelPart {
                material: 0,
                range: 3..6,
            }],
            vec![ModelPart {
                material: 0,
                range: 6..9,
            }],
        ]));
        library.meshes.push(mesh(vec![vec![]]));
        assert_eq!(mesh_span(&library, 0), Some((0..3, 0..3)));
        assert_eq!(mesh_span(&library, 1), Some((3..9, 5..10)));
        assert_eq!(mesh_span(&library, 2), None);
        assert_eq!(mesh_span(&library, 9), None);
    }

    /// Every palette index a shard carries is rebased onto the merged palette:
    /// the rig's range, each node's skin and rigid slots, and the vertices'
    /// weighted and rigid slots. A slot left behind indexes another entity's
    /// joints, and `entity_animation` subtracts `rig.start` from the node's.
    #[test]
    fn merging_a_shard_rebases_every_palette_slot() {
        let joint = PaletteJoint::from_transform(&mesh::Transform::IDENTITY);
        let shard_mesh = ModelMesh {
            name: "shard".to_owned(),
            path: "shard.mesh".to_owned(),
            scale: 1.0,
            lod_distances: Vec::new(),
            lods: vec![Vec::new()],
            radius: 0.0,
            height: 0.0,
            own_radius: 0.0,
            attachments: 0,
        };
        let mut shard = ModelLibrary {
            palette: vec![joint; 3],
            vertices: vec![ModelVertex {
                joints: [2, 0, 0, 0],
                weights: [1.0, 0.0, 0.0, 0.0],
                rigid_joint: 1 | !RIGID_SLOT,
                ..ModelVertex::default()
            }],
            meshes: vec![shard_mesh.clone()],
            rigs: vec![Arc::new(Rig {
                nodes: vec![entity_animation::Node {
                    mesh: Arc::new(mesh::Mesh::default()),
                    clips: Vec::new(),
                    default: "idle".to_owned(),
                    follows_parent: true,
                    parent: None,
                    scale: 1.0,
                    mesh_scale: 1.0,
                    skin_base: Some(0),
                    rigid: 1,
                    locators: BTreeMap::new(),
                }],
                start: 0,
                end: 3,
            })],
            ..ModelLibrary::default()
        };
        shard.index.insert("shard".to_owned(), 0);
        let mut library = ModelLibrary {
            palette: vec![joint; 5],
            ..ModelLibrary::default()
        };
        library.merge(shard);
        let rig = &library.rigs[0];
        assert_eq!((rig.start, rig.end), (5, 8));
        assert_eq!(rig.nodes[0].rigid, 6);
        assert_eq!(rig.nodes[0].skin_base, Some(5));
        assert_eq!(library.vertices[0].joints[0], 7);
        assert_eq!(library.vertices[0].rigid_joint, 6 | !RIGID_SLOT);
        // The node's own slot is inside the rig's range, which is what
        // `Player::evaluate` indexes.
        assert!((rig.start..rig.end).contains(&(rig.nodes[0].rigid as usize)));
    }

    #[test]
    fn a_mesh_setting_overrides_one_shape_and_one_material_index() {
        // `buildings.gfx:357-384` gives facility_landShape three texture sets,
        // one per material index; applying them all would repaint the model.
        let setting = |name: &str, index: u32| mesh::MeshSetting {
            name: name.to_owned(),
            index,
            ..mesh::MeshSetting::default()
        };
        assert!(setting_applies(
            &setting("facility_landShape", 1),
            "facility_landShape",
            1
        ));
        assert!(!setting_applies(
            &setting("facility_landShape", 1),
            "facility_landShape",
            2
        ));
        assert!(!setting_applies(
            &setting("facility_landShape", 1),
            "collisionShape",
            1
        ));
        assert!(setting_applies(&setting("", 0), "anyShape", 0));
    }

    #[test]
    fn the_sprawl_gate_is_water_relative_below_both_defines() {
        let at = |height| {
            super::super::camera::Placement {
                center: [0.0, 0.0],
                height,
            }
            .on(
                super::super::camera::MapSize::TEST,
                super::super::camera::MapDefines::test(),
            )
        };
        // The reference pairs at 302.96 draw no sprawl; 50.178 does.
        assert!(!draws_city_sprawl(&at(302.960)));
        assert!(draws_city_sprawl(&at(50.178)));
        // 229.4 - 9.5 = 219.9, just inside; 229.6 - 9.5 = 220.1, just outside.
        assert!(draws_city_sprawl(&at(229.4)));
        assert!(!draws_city_sprawl(&at(229.6)));
    }

    #[test]
    fn an_entity_locator_ignores_the_baked_pdxmesh_scale() {
        let entity_scale = 0.8;
        let mesh_scale = 0.14;
        let node = mesh::Transform::from_trs(
            [10.0, 0.0, 20.0],
            [0.0, 0.0, 0.0, 1.0],
            entity_scale * mesh_scale,
        );
        let mut locator = mesh::Transform::IDENTITY;
        locator.translation = [0.0, 1.1, -1.0];
        let world = node.compose(&entity_locator(&locator, mesh_scale));
        let at = world.transform_point([0.0; 3]);
        let expected = [10.0, 1.1 * entity_scale, 20.0 - entity_scale];
        for axis in 0..3 {
            assert!((at[axis] - expected[axis]).abs() < 1e-5, "{at:?}");
        }
        assert!((world.uniform_scale() - entity_scale).abs() < 1e-5);
    }

    #[test]
    fn effects_carry_their_defines() {
        assert_eq!(
            effect_for("PdxMeshSnow"),
            Effect {
                improved_blinn_phong: true,
                emissive: true,
                snow: true,
                alpha_test: false,
                train: false,
            }
        );
        assert_eq!(
            effect_for("PdxMeshSnowSkinned"),
            Effect {
                snow: true,
                ..Effect::default()
            }
        );
        assert!(effect_for("PdxMeshAdvanced").improved_blinn_phong);
        assert!(!effect_for("PdxMeshAdvanced").snow);
        assert!(effect_for("PdxMeshStandard") == Effect::default());
        assert!(effect_for("something else") == Effect::default());
        assert_eq!(effect_for("PdxMeshSnow").bits(), 0b0111);
    }

    #[test]
    fn the_lod_is_the_last_distance_the_eye_passes() {
        let distances = [0.0, 100.0, 200.0, 500.0];
        assert_eq!(lod_for_distance(&distances, 0.0, 4), 0);
        assert_eq!(lod_for_distance(&distances, 99.0, 4), 0);
        assert_eq!(lod_for_distance(&distances, 100.0, 4), 1);
        assert_eq!(lod_for_distance(&distances, 4000.0, 4), 3);
        // A mesh that declares fewer shapes than distances is clamped.
        assert_eq!(lod_for_distance(&distances, 4000.0, 2), 1);
        assert_eq!(lod_for_distance(&[], 4000.0, 1), 0);
    }

    #[test]
    fn a_palette_joint_is_the_transform_transposed_into_rows() {
        // A quarter turn about +Y at scale 2, then a shift: the rows are what
        // a `dot` with `(position, 1)` needs.
        let half = std::f32::consts::FRAC_PI_4;
        let transform =
            mesh::Transform::from_trs([1.0, 2.0, 3.0], [0.0, half.sin(), 0.0, half.cos()], 2.0);
        let joint = PaletteJoint::from_transform(&transform);
        for point in [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.3, -0.4, 0.5]] {
            let want = transform.transform_point(point);
            let got = skinned(
                &[joint],
                &ModelVertex {
                    position: point,
                    weights: [1.0, 0.0, 0.0, 0.0],
                    ..ModelVertex::default()
                },
            );
            assert!(
                got.iter()
                    .zip(want)
                    .all(|(value, want)| (value - want).abs() < 1e-5),
                "{got:?} against {want:?}"
            );
        }
    }

    #[test]
    fn a_vertex_with_no_weight_keeps_its_own_position() {
        // This is the case that has to be exact rather than close: an
        // unskinned mesh must reach the shader as the bytes the loader baked.
        let vertex = ModelVertex {
            position: [1.25, -0.5, 7.0],
            ..ModelVertex::default()
        };
        assert_eq!(skinned(&[], &vertex), vertex.position);
        assert_eq!(skinned(&[PaletteJoint::IDENTITY], &vertex), vertex.position);
    }

    #[test]
    fn weights_blend_the_joints_they_name() {
        let left = PaletteJoint::from_transform(&mesh::Transform {
            translation: [-10.0, 0.0, 0.0],
            ..mesh::Transform::IDENTITY
        });
        let right = PaletteJoint::from_transform(&mesh::Transform {
            translation: [10.0, 0.0, 0.0],
            ..mesh::Transform::IDENTITY
        });
        let at = |weights: [f32; 4]| {
            skinned(
                &[left, right],
                &ModelVertex {
                    position: [0.0, 1.0, 0.0],
                    joints: [0, 1, 0, 0],
                    weights,
                    ..ModelVertex::default()
                },
            )
        };
        assert_eq!(at([1.0, 0.0, 0.0, 0.0]), [-10.0, 1.0, 0.0]);
        assert_eq!(at([0.0, 1.0, 0.0, 0.0]), [10.0, 1.0, 0.0]);
        assert_eq!(at([0.5, 0.5, 0.0, 0.0]), [0.0, 1.0, 0.0]);
        // A slot naming a joint outside the palette contributes nothing rather
        // than reading past its end.
        assert_eq!(
            skinned(
                &[left],
                &ModelVertex {
                    position: [0.0, 1.0, 0.0],
                    joints: [0, 99, 0, 0],
                    weights: [1.0, 1.0, 0.0, 0.0],
                    ..ModelVertex::default()
                }
            ),
            [-10.0, 1.0, 0.0]
        );
    }

    #[test]
    fn the_vertex_stride_matches_what_the_pipeline_declares() {
        assert_eq!(size_of::<ModelVertex>() as u64, VERTEX_STRIDE);
        assert_eq!(size_of::<ModelInstance>() as u64, INSTANCE_STRIDE);
        assert_eq!(size_of::<PaletteJoint>() as u64, PALETTE_JOINT_SIZE);
        let buffers = mesh_buffers();
        let attributes = buffers[1].as_ref().unwrap().attributes;
        let angles = &attributes[3];
        assert_eq!(angles.format, wgpu::VertexFormat::Float32x2);
        assert_eq!(
            angles.offset as usize,
            std::mem::offset_of!(ModelInstance, pitch)
        );
        assert_eq!(
            angles.offset as usize + 4,
            std::mem::offset_of!(ModelInstance, roll)
        );
        assert_eq!(
            attributes[4].offset as usize,
            std::mem::offset_of!(ModelInstance, train_clip)
        );
        assert_eq!(
            attributes[5].offset as usize,
            std::mem::offset_of!(ModelInstance, palette_offset)
        );
    }

    #[test]
    fn a_posed_vertex_widens_the_reach_where_the_bind_box_would_not() {
        // `cavalry_horse.mesh` is 6.6 wide bound and 16.0 posed, so a cull on
        // the `[aabb]` would clip it.
        let mut reach = Reach::default();
        reach.widen(
            &mesh::Transform::IDENTITY,
            &mesh::Bounds {
                min: [-3.3, 0.0, -3.3],
                max: [3.3, 8.0, 3.3],
            },
            true,
        );
        let bound = reach.radius;
        reach.widen_point([8.0, 9.0, 0.0], true);
        assert!(reach.radius > bound);
        assert_eq!(reach.radius, 8.0);
        assert_eq!(reach.height, 9.0);
        assert_eq!(reach.own_radius, 8.0);
    }

    #[test]
    fn the_hash_is_deterministic_and_in_range() {
        for index in 0..1000u32 {
            let value = hash01(index);
            assert!((0.0..1.0).contains(&value), "{index} gave {value}");
            assert_eq!(value, hash01(index));
        }
        assert_ne!(hash01(1), hash01(2));
    }

    fn library_with_one_mesh(radius: f32) -> ModelLibrary {
        ModelLibrary {
            meshes: vec![ModelMesh {
                name: "m".into(),
                path: "m.mesh".into(),
                scale: 1.0,
                lod_distances: Vec::new(),
                lods: vec![Vec::new()],
                radius,
                height: 1.0,
                own_radius: radius,
                attachments: 0,
            }],
            ..ModelLibrary::default()
        }
    }

    fn at(x: f32, z: f32) -> ModelInstance {
        ModelInstance {
            position: [x, 0.0, z],
            rotation: 0.0,
            scale: 1.0,
            pitch: 0.0,
            train_clip: [0.0; 4],
            ..ModelInstance::default()
        }
    }

    #[test]
    fn the_field_groups_by_mesh_and_tile_and_widens_by_the_radius() {
        let library = library_with_one_mesh(2.0);
        let field = ModelField::build(
            &library,
            vec![
                (0, at(10.0, 10.0)),
                (0, at(20.0, 10.0)),
                (0, at(300.0, 10.0)),
            ],
            256.0,
        );
        assert_eq!(field.instances.len(), 3);
        assert_eq!(field.groups.len(), 2);
        assert_eq!(field.groups[0].bounds, [8.0, 8.0, 22.0, 12.0]);
        assert_eq!(field.groups[0].centroid, [15.0, 0.0, 10.0]);
        assert_eq!(field.groups[1].range, 2..3);
    }

    #[test]
    fn a_group_outside_the_visible_rectangle_is_culled() {
        let library = library_with_one_mesh(0.0);
        let field = ModelField::build(&library, vec![(0, at(10.0, 10.0))], 256.0);
        let group = &field.groups[0];
        assert!(visible(group, 0.0, None));
        assert!(visible(group, 0.0, Some([0.0, 0.0, 20.0, 20.0])));
        assert!(!visible(group, 0.0, Some([100.0, 0.0, 200.0, 20.0])));
        // The wrap copy moves the group, not the rectangle.
        assert!(visible(group, 100.0, Some([100.0, 0.0, 200.0, 20.0])));
    }

    #[test]
    fn unit_lifetime_gpu_stream_and_hidden_animation_clock() {
        use super::super::unit_history::{
            History, Kind,
            tests::{GATES, target},
        };
        let _serial = super::super::shader_tests::GPU_TESTS
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
        let Ok(adapter) = pollster::block_on(instance.request_adapter(&Default::default())) else {
            eprintln!("SKIP unit lifetime GPU test: no adapter");
            return;
        };
        let (device, queue) =
            pollster::block_on(adapter.request_device(&crate::render::device_descriptor(&adapter)))
                .unwrap();
        let mut entries = vec![wgpu::BindGroupLayoutEntry {
            binding: 0,
            visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Uniform,
                has_dynamic_offset: false,
                min_binding_size: None,
            },
            count: None,
        }];
        for binding in [1, 3, 4, 5, 7, 8] {
            entries.push(texture_entry(
                binding,
                wgpu::ShaderStages::FRAGMENT,
                wgpu::TextureViewDimension::D2,
            ));
        }
        entries.extend([sampler_entry(2), sampler_entry(6)]);
        let globals = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: None,
            entries: &entries,
        });
        let mut library = library_with_one_mesh(1.0);
        library.palette = vec![PaletteJoint::IDENTITY];
        library.rigs.push(Arc::new(Rig {
            start: 0,
            end: 1,
            nodes: vec![entity_animation::Node {
                mesh: Arc::new(mesh::Mesh::default()),
                clips: vec![entity_animation::Clip {
                    state: mesh::EntityState {
                        name: "idle".into(),
                        chance: 1.0,
                        speed: 1.0,
                        events: vec![mesh::EntityEvent {
                            time: 0.2,
                            trigger_once: true,
                            particle: Some("synthetic".into()),
                            ..Default::default()
                        }],
                        ..Default::default()
                    },
                    animation: None,
                }],
                default: "idle".into(),
                follows_parent: false,
                parent: None,
                scale: 1.0,
                mesh_scale: 1.0,
                skin_base: None,
                rigid: 0,
                locators: BTreeMap::new(),
            }],
        }));
        let mut pass = ModelPass::new(
            &device,
            1,
            &queue,
            &globals,
            super::super::scene::SCENE_FORMAT,
            &library,
            "synthetic unit lifetime",
        )
        .unwrap();
        // Read back the actual instance stream animate/update_placed write.
        let bytes = std::mem::size_of::<ModelInstance>() as u64;
        pass.instances = Some(device.create_buffer(&wgpu::BufferDescriptor {
            label: None,
            size: bytes,
            usage: wgpu::BufferUsages::VERTEX
                | wgpu::BufferUsages::COPY_DST
                | wgpu::BufferUsages::COPY_SRC,
            mapped_at_creation: false,
        }));
        let mut target = target(40, Kind::Land);
        target.instance.animation = entity_animation::MOVE; // no reconstructed idle phase
        let mut history = History::default();
        history.set(vec![target], Arc::default(), false);
        let faded = {
            let mut frame = |time, shown, in_view| {
                if time == 10.75 {
                    // Discovering another mesh in a later hour grows the
                    // palette without replacing this live entity.
                    library.palette.push(PaletteJoint::IDENTITY);
                    pass.sync(&device, &library).unwrap();
                }
                let placed = history.step(time, GATES, [shown; 2], true, |_| in_view);
                pass.update_placed(&device, &queue, placed, 128.0).unwrap();
                pass.animate(&device, &queue, time, None, &[0.0]).unwrap();
                (
                    pass.field.instances.clone(),
                    pass.events.iter().map(|e| e.age).collect::<Vec<_>>(),
                )
            };
            let (first, events) = frame(10.0, true, true);
            assert!(
                events.is_empty(),
                "creation clock starts now, not at global time zero"
            );
            let epoch = first[0].animation_epoch;
            let (_, events) = frame(10.5, true, true);
            assert!((events[0] - 0.3).abs() < 1e-5);
            let (hidden, events) = frame(10.75, true, false);
            assert_eq!(hidden[0].animation_epoch, epoch);
            assert!((events[0] - 0.55).abs() < 1e-5, "hidden entity advances");
            let (back, events) = frame(11.0, true, true);
            assert_eq!(back[0].animation_epoch, epoch);
            assert!((events[0] - 0.8).abs() < 1e-5);
            for _ in 0..100 {
                frame(11.0, true, false);
            }
            let (fresh, events) = frame(12.0, true, true);
            assert_ne!(fresh[0].animation_epoch, epoch);
            assert!(events.is_empty());
            frame(12.5, true, true);
            let (faded, _) = frame(12.75, false, true);
            assert_eq!(faded[0].scale, 0.5);
            faded
        };
        let read = device.create_buffer(&wgpu::BufferDescriptor {
            label: None,
            size: bytes,
            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let mut encoder = device.create_command_encoder(&Default::default());
        encoder.copy_buffer_to_buffer(pass.instances.as_ref().unwrap(), 0, &read, 0, bytes);
        queue.submit([encoder.finish()]);
        read.slice(..)
            .map_async(wgpu::MapMode::Read, |r| r.unwrap());
        device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
        let readback = read.slice(..).get_mapped_range().unwrap();
        let actual: &ModelInstance = bytemuck::from_bytes(&readback);
        assert_eq!(*actual, faded[0]);
    }
}
