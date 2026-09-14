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
//! Dropped, with the reason:
//!
//! * Skinning. `matBones` is only written for animated entities; no layer here
//!   animates, and a skinned mesh drawn at its bind pose is what an idle
//!   entity's first animation frame very nearly is. Marked where it bites, in
//!   `map::unit_models`.
//! * `CalculateShadowCascaded`, which `pdxmesh.shader:499` has commented out:
//!   mesh shadowing is dead code in 1.19.2 (section 2.4).
//! * `CalculatePointLights`: no recorded table carries a point light.
//! * `ApplyFOW`, whose two factors the omniscient viewer leaves at zero.
//! * `UV_ANIM` and `ATLAS`, which no static map object sets, and `TRAIN`, which
//!   belongs to the train models the replay cannot place (`map::railways`).
//! * `RIM_LIGHT`, whose body is commented out in the game's own shader.

use std::{collections::BTreeMap, f32::consts::TAU, io, ops::Range};

use bytemuck::{Pod, Zeroable};
use eframe::wgpu;
use installed_content::{
    ResolvedLayers,
    image::{Rgba8Image, dds},
    mesh::{self, Descriptors},
};

use super::{
    camera::Camera,
    scene::{DEPTH_FORMAT, SAMPLE_COUNT, Targets, buffer, module, sampler_entry, texture_entry},
    textures::{Bound, Samplers, upload},
};

fn invalid(message: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message.into())
}

// ------------------------------------------------------------------ constants

/// `WATER_HEIGHT`. The map-object update compares `camera height - WATER_HEIGHT`
pub const WATER_HEIGHT: f32 = 9.5;
/// `CITY_SPRAWL_SHRINK_DISTANCE` (`00_graphics.lua`, consumer
/// `0x140b4c120:544`): despite the name it is a spawn/despawn switch, not a
/// scale (section 4.2).
pub const CITY_SPRAWL_SHRINK_DISTANCE: f32 = 220.0;
/// `DRAW_MAP_OBJECTS_CUTOFF` (`0x140b4c120:543`): the master gate over every
/// map object.
pub const DRAW_MAP_OBJECTS_CUTOFF: f32 = 1100.0;
/// `CUBEMAP_INTENSITY` (`00_graphics.lua:1066`).
pub const CUBEMAP_INTENSITY: f32 = 1.0;
/// The engine's neutral textures, which a material names when it has none.
const FALLBACK_DIRECTORY: &str = "gfx/models";

/// Whether the camera draws city sprawl (section 4.2, `0x140b4c120:544`).
///
/// The master `DRAW_MAP_OBJECTS_CUTOFF` is the outer of the two gates the
/// engine applies in the same block, and at 1100 it never binds before the
/// sprawl's own 220 does, so the pair is one comparison here.
pub fn draws_city_sprawl(height: f32) -> bool {
    height - WATER_HEIGHT < CITY_SPRAWL_SHRINK_DISTANCE.min(DRAW_MAP_OBJECTS_CUTOFF)
}

// -------------------------------------------------------------------- effects

/// The `Defines` of one `pdxmesh.shader` effect, as the pixel shader branches
/// on them (section 2.1).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Effect {
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
    }
}

/// The defines of an effect named in a `[material]` or a `meshsettings`
/// override (`pdxmesh.shader:747-993`, tabulated in section 2.1).
///
/// An effect the table does not name takes `PdxMeshStandard`'s empty define
/// set, which is the engine's own fallback shape for an unknown effect.
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
        "PdxMeshAdvanced" | "PdxMeshAdvancedSkinned" | "PdxMeshTrain" | "PdxMeshTrainSkinned" => {
            improved
        }
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

/// One vertex of a static mesh, in `VS_INPUT_PDXMESHSTANDARD` order.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Pod, Zeroable)]
pub struct ModelVertex {
    pub position: [f32; 3],
    pub normal: [f32; 3],
    pub tangent: [f32; 4],
    pub uv0: [f32; 2],
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
    /// Horizontal radius of LOD 0's bounds, after `scale`, for culling.
    pub radius: f32,
    /// Height of LOD 0's bounds above its own origin, after `scale`.
    pub height: f32,
}

/// Every mesh, material and texture one layer draws, flattened into one vertex
/// and index pair.
#[derive(Default)]
pub struct ModelLibrary {
    pub vertices: Vec<ModelVertex>,
    pub indices: Vec<u32>,
    pub meshes: Vec<ModelMesh>,
    pub materials: Vec<ModelMaterial>,
    pub textures: Vec<dds::Texture>,
    /// Requested name to slot in [`meshes`](ModelLibrary::meshes).
    pub index: BTreeMap<String, usize>,
    /// Texture path to slot in [`textures`](ModelLibrary::textures), so the
    /// shared `city_d.dds` of sixteen sprawl meshes is uploaded once.
    paths: BTreeMap<String, usize>,
}

impl ModelLibrary {
    /// Resolves and loads every name, appending one warning per name that does
    /// not resolve rather than refusing the document.
    ///
    /// `gfx/entities` and `gfx/models` are outside the content coverage policy
    /// (`RENDERING.md` section 4), so every file here goes through
    /// [`ResolvedLayers::read_uncovered`] and is not hash-verified.
    pub fn load(
        layers: &ResolvedLayers,
        descriptors: &Descriptors,
        names: &[String],
        warnings: &mut Vec<String>,
    ) -> io::Result<Self> {
        let mut library = Self::default();
        let mut missing = Vec::new();
        for name in names {
            if library.index.contains_key(name) {
                continue;
            }
            match library.add(layers, descriptors, name) {
                Ok(()) => {}
                Err(error) => missing.push(format!("{name} ({error})")),
            }
        }
        if !missing.is_empty() {
            missing.sort();
            warnings.push(format!(
                "No mesh resolved for {}; those objects are not drawn.",
                missing.join(", ")
            ));
        }
        Ok(library)
    }

    /// Whether a name resolved.
    pub fn slot(&self, name: &str) -> Option<usize> {
        self.index.get(name).copied()
    }

    fn add(
        &mut self,
        layers: &ResolvedLayers,
        descriptors: &Descriptors,
        name: &str,
    ) -> io::Result<()> {
        let resolved = descriptors
            .resolve(name)
            .ok_or_else(|| invalid("no pdxmesh or entity declares it"))?;
        let path = resolved.definition.file.clone();
        let scale = resolved.scale;
        let decoded = mesh::read(&layers.read_uncovered(&path)?)
            .map_err(|error| invalid(format!("{path}: {error}")))?;
        let directory = path.rsplit_once('/').map_or("", |(head, _)| head);
        let mut lods = Vec::new();
        let mut radius: f32 = 0.0;
        let mut height: f32 = 0.0;
        for lod in 0..decoded.shapes.len() {
            let Some(shape) = decoded.shape_for_lod(lod) else {
                lods.push(Vec::new());
                continue;
            };
            let mut parts = Vec::new();
            for sub in shape.meshes.iter().filter(|sub| sub.is_drawable()) {
                let material = self.material(layers, directory, sub, &resolved)?;
                let start = self.indices.len() as u32;
                let base = self.vertices.len() as u32;
                for vertex in 0..sub.vertex_count() {
                    let p = sub.positions[vertex];
                    self.vertices.push(ModelVertex {
                        position: [p[0] * scale, p[1] * scale, p[2] * scale],
                        normal: sub.normals[vertex],
                        tangent: sub.tangents[vertex],
                        uv0: sub.uv0[vertex],
                    });
                }
                self.indices
                    .extend(sub.indices.iter().map(|index| base + index));
                if lod == 0
                    && let Some(bounds) = &sub.bounds
                {
                    radius = radius.max(
                        (bounds.min[0].abs().max(bounds.max[0].abs()).powi(2)
                            + bounds.min[2].abs().max(bounds.max[2].abs()).powi(2))
                        .sqrt()
                            * scale,
                    );
                    height = height.max(bounds.max[1] * scale);
                }
                parts.push(ModelPart {
                    material,
                    range: start..self.indices.len() as u32,
                });
            }
            lods.push(parts);
        }
        if lods.iter().all(Vec::is_empty) {
            return Err(invalid(format!("{path} has no drawable sub-mesh")));
        }
        self.index.insert(name.to_owned(), self.meshes.len());
        self.meshes.push(ModelMesh {
            name: name.to_owned(),
            path,
            scale,
            lod_distances: decoded.lod_distances.clone(),
            lods,
            radius,
            height,
        });
        Ok(())
    }

    fn material(
        &mut self,
        layers: &ResolvedLayers,
        directory: &str,
        sub: &mesh::SubMesh,
        resolved: &mesh::ResolvedMesh<'_>,
    ) -> io::Result<usize> {
        let material = sub.material.as_ref();
        // A `meshsettings` override replaces the `.mesh`'s own effect and
        // textures for one (shape, material index) pair (section 1.4). The
        // shipped overrides that matter all name the shape, and the pass draws
        // one sub-mesh list per shape, so matching on the file names alone
        // would be wrong; the override is applied when it names no shape or
        // names this one. UNVERIFIED: the engine's match rule; only
        // `buildings.gfx:207-222` uses it in the content these layers read.
        let shader = material.map_or("", |m| m.shader.as_str());
        let effect = effect_for(shader);
        let mut diffuse = material.map_or("", |m| m.diffuse.as_str()).to_owned();
        let mut normal = material.map_or("", |m| m.normal.as_str()).to_owned();
        let mut specular = material.map_or("", |m| m.specular.as_str()).to_owned();
        let mut effect = effect;
        for setting in &resolved.definition.settings {
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
            diffuse: self.texture(layers, directory, &diffuse)?,
            normal: self.texture(layers, directory, &normal)?,
            specular: self.texture(layers, directory, &specular)?,
        };
        if let Some(slot) = self.materials.iter().position(|other| *other == entry) {
            return Ok(slot);
        }
        self.materials.push(entry);
        Ok(self.materials.len() - 1)
    }

    /// A material's texture, resolved against the `.mesh`'s own directory
    /// (section 1.4) and then against `gfx/models`, which holds the engine's
    /// `nonormal.dds` and `nospec.dds` placeholders.
    ///
    /// A name that resolves nowhere becomes a one-texel transparent black,
    /// which is what the shipped `nospec.dds` is for and what a missing
    /// diffuse deserves: the object still stands, in black.
    fn texture(
        &mut self,
        layers: &ResolvedLayers,
        directory: &str,
        file: &str,
    ) -> io::Result<usize> {
        if file.is_empty() {
            return Ok(self.blank());
        }
        for candidate in [
            format!("{directory}/{file}"),
            format!("{FALLBACK_DIRECTORY}/{file}"),
        ] {
            if let Some(slot) = self.paths.get(&candidate) {
                return Ok(*slot);
            }
            let Ok(bytes) = layers.read_uncovered(&candidate) else {
                continue;
            };
            let Ok(texture) = dds::decode(&bytes) else {
                continue;
            };
            self.textures.push(texture);
            let slot = self.textures.len() - 1;
            self.paths.insert(candidate, slot);
            return Ok(slot);
        }
        Ok(self.blank())
    }

    fn blank(&mut self) -> usize {
        if let Some(slot) = self.paths.get("") {
            return *slot;
        }
        self.textures.push(dds::Texture::from_rgba8(Rgba8Image {
            width: 1,
            height: 1,
            pixels: vec![0, 0, 0, 0],
        }));
        let slot = self.textures.len() - 1;
        self.paths.insert(String::new(), slot);
        slot
    }
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

/// The placed instances of one layer.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ModelField {
    pub instances: Vec<ModelInstance>,
    pub groups: Vec<ModelGroup>,
}

impl ModelField {
    /// Sorts placed instances into one group per (mesh, tile) and records each
    /// group's bounds.
    ///
    /// `tile` is the side of the cull grid in world units. The engine buckets
    /// its trees on 256-unit tiles (section 3.2); nothing in the decompile ties
    /// the other layers to a particular size, so this is the viewer's own cull
    /// and not a claim about the engine.
    pub fn build(library: &ModelLibrary, placed: Vec<(usize, ModelInstance)>, tile: f32) -> Self {
        let mut keyed: BTreeMap<(usize, i32, i32), Vec<ModelInstance>> = BTreeMap::new();
        for (mesh, instance) in placed {
            let key = (
                mesh,
                (instance.position[0] / tile).floor() as i32,
                (instance.position[2] / tile).floor() as i32,
            );
            keyed.entry(key).or_default().push(instance);
        }
        let mut field = Self::default();
        for ((mesh, _, _), instances) in keyed {
            let radius = library.meshes.get(mesh).map_or(0.0, |entry| entry.radius);
            let start = field.instances.len() as u32;
            let mut bounds = [f32::MAX, f32::MAX, f32::MIN, f32::MIN];
            let mut sum = [0.0f64; 3];
            for instance in &instances {
                let reach = radius * instance.scale;
                bounds[0] = bounds[0].min(instance.position[0] - reach);
                bounds[1] = bounds[1].min(instance.position[2] - reach);
                bounds[2] = bounds[2].max(instance.position[0] + reach);
                bounds[3] = bounds[3].max(instance.position[2] + reach);
                for (axis, total) in sum.iter_mut().enumerate() {
                    *total += f64::from(instance.position[axis]);
                }
            }
            let count = instances.len() as f64;
            field.instances.extend(instances);
            field.groups.push(ModelGroup {
                mesh,
                range: start..field.instances.len() as u32,
                bounds,
                centroid: [
                    (sum[0] / count) as f32,
                    (sum[1] / count) as f32,
                    (sum[2] / count) as f32,
                ],
            });
        }
        field
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
/// UNVERIFIED everywhere it is used: the engine's own generator for the sprawl
/// mesh choice and for object rotations was not recovered. Any deterministic
/// hash is indistinguishable from it at these cameras; what matters is that the
/// same index gives the same answer on every render.
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
const VERTEX_STRIDE: u64 = 48;
/// Bytes one [`ModelInstance`] occupies.
const INSTANCE_STRIDE: u64 = 20;
/// Bytes the shader reads from a per-material uniform slot.
const SLOT_SIZE: u64 = 16;
/// Bytes one slot occupies, rounded up to wgpu's uniform offset alignment.
const SLOT_STRIDE: u64 = 256;
/// World-wrap copies, as every other world layer counts them.
const COPIES: usize = 3;

/// The scene textures `pdxmesh.shader` samples that the pass does not own.
pub struct ModelInputs<'a> {
    pub snow_mud: &'a Bound,
    pub gradient1: &'a Bound,
    pub gradient2: &'a Bound,
    pub secondary: &'a Bound,
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
pub struct ModelPass {
    pipeline: wgpu::RenderPipeline,
    layout: wgpu::BindGroupLayout,
    slots: wgpu::Buffer,
    textures: Vec<Bound>,
    materials: Vec<ModelMaterial>,
    meshes: Vec<ModelMesh>,
    /// `material * COPIES + copy`.
    groups: Vec<wgpu::BindGroup>,
    vertices: Option<wgpu::Buffer>,
    indices: Option<wgpu::Buffer>,
    instances: Option<wgpu::Buffer>,
    field: ModelField,
    drawn: std::cell::Cell<u32>,
    label: &'static str,
}

impl ModelPass {
    /// Builds the pipeline and uploads the library's textures.
    pub fn new(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        globals: &wgpu::BindGroupLayout,
        format: wgpu::TextureFormat,
        library: &ModelLibrary,
        label: &'static str,
    ) -> io::Result<Self> {
        let mut entries = (0..7)
            .map(|binding| {
                texture_entry(
                    binding,
                    wgpu::ShaderStages::FRAGMENT,
                    wgpu::TextureViewDimension::D2,
                )
            })
            .collect::<Vec<_>>();
        entries.push(texture_entry(
            7,
            wgpu::ShaderStages::FRAGMENT,
            wgpu::TextureViewDimension::Cube,
        ));
        entries.push(sampler_entry(8));
        entries.push(sampler_entry(9));
        entries.push(wgpu::BindGroupLayoutEntry {
            binding: 10,
            visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Uniform,
                has_dynamic_offset: false,
                min_binding_size: wgpu::BufferSize::new(SLOT_SIZE),
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
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some(label),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &module,
                entry_point: Some("mesh_vertex"),
                compilation_options: Default::default(),
                buffers: &[
                    Some(wgpu::VertexBufferLayout {
                        array_stride: VERTEX_STRIDE,
                        step_mode: wgpu::VertexStepMode::Vertex,
                        attributes: &[
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
                        ],
                    }),
                    Some(wgpu::VertexBufferLayout {
                        array_stride: INSTANCE_STRIDE,
                        step_mode: wgpu::VertexStepMode::Instance,
                        attributes: &[
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
                        ],
                    }),
                ],
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
            // write on. The stencil half of it tags pixels for the engine's
            // transparency passthrough, which this port has no pass for.
            depth_stencil: Some(wgpu::DepthStencilState {
                format: DEPTH_FORMAT,
                depth_write_enabled: Some(true),
                depth_compare: Some(wgpu::CompareFunction::Less),
                stencil: Default::default(),
                bias: Default::default(),
            }),
            multisample: wgpu::MultisampleState {
                count: SAMPLE_COUNT,
                ..Default::default()
            },
            fragment: Some(wgpu::FragmentState {
                module: &module,
                entry_point: Some("mesh_fragment"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format,
                    // `BlendState { BlendEnable = no }`. The alpha the pixel
                    // shader returns is the emissive mask, which the engine's
                    // bloom reads; this port writes colour only, as every other
                    // world layer over the terrain does, so the city-light
                    // bloom mask the terrain wrote survives.
                    blend: None,
                    write_mask: wgpu::ColorWrites::COLOR,
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
        let mut textures = Vec::with_capacity(library.textures.len());
        for (slot, texture) in library.textures.iter().enumerate() {
            textures.push(upload(
                device,
                queue,
                &format!("{label} texture {slot}"),
                texture,
            )?);
        }
        Ok(Self {
            pipeline,
            layout,
            slots,
            textures,
            materials: library.materials.clone(),
            meshes: library.meshes.clone(),
            groups: Vec::new(),
            vertices: None,
            indices: None,
            instances: None,
            field: ModelField::default(),
            drawn: std::cell::Cell::new(0),
            label,
        })
    }

    /// Rebuilds the bind groups; needed whenever a bound scene texture is
    /// replaced.
    pub fn rebind(&mut self, device: &wgpu::Device, samplers: &Samplers, inputs: ModelInputs<'_>) {
        let mut groups = Vec::with_capacity(self.materials.len() * COPIES);
        for (index, material) in self.materials.iter().enumerate() {
            let views = [
                &self.textures[material.diffuse].view,
                &self.textures[material.specular].view,
                &self.textures[material.normal].view,
                &inputs.snow_mud.view,
                &inputs.gradient1.view,
                &inputs.gradient2.view,
                &inputs.secondary.view,
                &inputs.reflection.view,
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
                    binding: 8,
                    resource: wgpu::BindingResource::Sampler(&samplers.linear_wrap),
                });
                bindings.push(wgpu::BindGroupEntry {
                    binding: 9,
                    resource: wgpu::BindingResource::Sampler(&samplers.linear_clamp),
                });
                bindings.push(wgpu::BindGroupEntry {
                    binding: 10,
                    resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                        buffer: &self.slots,
                        offset: SLOT_STRIDE * (index * COPIES + copy) as u64,
                        size: wgpu::BufferSize::new(SLOT_SIZE),
                    }),
                });
                groups.push(device.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: Some(self.label),
                    layout: &self.layout,
                    entries: &bindings,
                }));
            }
        }
        self.groups = groups;
    }

    /// Uploads the geometry and the placed instances.
    pub fn upload(
        &mut self,
        device: &wgpu::Device,
        library: &ModelLibrary,
        field: ModelField,
    ) -> io::Result<()> {
        if library.indices.is_empty() || field.instances.is_empty() {
            self.vertices = None;
            self.indices = None;
            self.instances = None;
            self.field = ModelField::default();
            return Ok(());
        }
        self.vertices = Some(buffer(
            device,
            "pdxmesh vertices",
            wgpu::BufferUsages::VERTEX,
            bytemuck::cast_slice(&library.vertices),
        )?);
        self.indices = Some(buffer(
            device,
            "pdxmesh indices",
            wgpu::BufferUsages::INDEX,
            bytemuck::cast_slice(&library.indices),
        )?);
        self.instances = Some(buffer(
            device,
            "pdxmesh instances",
            wgpu::BufferUsages::VERTEX,
            bytemuck::cast_slice(&field.instances),
        )?);
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
        let (Some(vertices), Some(indices), Some(instances)) =
            (&self.vertices, &self.indices, &self.instances)
        else {
            return;
        };
        if self.groups.is_empty() || frame.alpha <= 0.0 {
            return;
        }
        let copies = frame.offsets.len().min(COPIES);
        let mut bytes = Vec::with_capacity(self.groups.len() * SLOT_STRIDE as usize);
        for material in &self.materials {
            for copy in 0..COPIES {
                let slot = Slot {
                    world_offset_x: frame.offsets.get(copy).copied().unwrap_or(0.0),
                    alpha: frame.alpha,
                    flags: material.effect.bits(),
                    cubemap_intensity: CUBEMAP_INTENSITY,
                };
                bytes.extend_from_slice(bytemuck::bytes_of(&slot));
                bytes.resize(bytes.len() + SLOT_STRIDE as usize - SLOT_SIZE as usize, 0);
            }
        }
        queue.write_buffer(&self.slots, 0, &bytes);

        let eye = frame.camera.eye();
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some(self.label),
            color_attachments: &[Some(targets.color_attachment(wgpu::LoadOp::Load))],
            depth_stencil_attachment: Some(targets.depth_attachment(wgpu::LoadOp::Load)),
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, globals, &[]);
        pass.set_vertex_buffer(0, vertices.slice(..));
        pass.set_vertex_buffer(1, instances.slice(..));
        pass.set_index_buffer(indices.slice(..), wgpu::IndexFormat::Uint32);
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
                    pass.set_bind_group(1, &self.groups[part.material * COPIES + copy], &[]);
                    pass.draw_indexed(part.range.clone(), 0, group.range.clone());
                    any = true;
                }
                if any {
                    drawn += group.range.end - group.range.start;
                }
            }
        }
        self.drawn.set(drawn);
    }
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

    #[test]
    fn the_sprawl_gate_matches_the_reference_and_close_zoom_heights() {
        // The reference pairs sit at 302.96 and 310.04: no sprawl.
        assert!(!draws_city_sprawl(302.960));
        assert!(!draws_city_sprawl(310.044));
        // pairD_in5 fitted at 115.293 and pairD_in10 at 50.178: sprawl.
        assert!(draws_city_sprawl(115.293));
        assert!(draws_city_sprawl(50.178));
    }

    #[test]
    fn the_sprawl_gate_is_water_relative() {
        // 229.4 - 9.5 = 219.9, just inside; 229.6 - 9.5 = 220.1, just outside.
        assert!(draws_city_sprawl(229.4));
        assert!(!draws_city_sprawl(229.6));
    }

    #[test]
    fn effects_carry_their_defines() {
        assert_eq!(
            effect_for("PdxMeshSnow"),
            Effect {
                improved_blinn_phong: true,
                emissive: true,
                snow: true,
                alpha_test: false
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
            }],
            ..ModelLibrary::default()
        }
    }

    fn at(x: f32, z: f32) -> ModelInstance {
        ModelInstance {
            position: [x, 0.0, z],
            rotation: 0.0,
            scale: 1.0,
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
}
