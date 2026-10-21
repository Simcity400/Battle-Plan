//! Compiled mesh passes with retained material state and ordered submission.
use super::*;
pub(super) mod walk;
mod plans;
mod plan_tail;
use crate::map::{mesh_pose, mesh_shader, shader_scene};
use installed_content_core::pdxshader::program::Stage;
use shader_pipeline::{
    binding,
    gpu::{CONSTANT_BINDINGS, TEXTURE_BINDINGS},
    library::{Library, Selection},
};
use viewer_foundation::map::gpu_contract::SCENE_FORMAT;

pub(super) type GeometryKey = (String, u32, usize, usize);

#[derive(Clone, PartialEq)]
pub(super) struct Part {
    pub geometry: Arc<mesh_shader::Geometry>,
    pub geometry_key: GeometryKey,
    pub material: mesh::Material,
    pub material_index: usize,
}

impl std::fmt::Debug for Part {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Part")
            .field("geometry", &self.geometry_key)
            .field("material", &self.material)
            .field("material_index", &self.material_index)
            .finish()
    }
}

#[derive(Clone, Debug, PartialEq)]
pub(super) struct Descriptor {
    pub name: String,
    pub node: usize,
    pub distances: Vec<f32>,
    pub lods: Vec<Vec<Part>>,
}

/// The game groups accepted materials, then appends the unnumbered records.
fn lods<T>(records: impl IntoIterator<Item = (Option<i32>, T)>) -> io::Result<Vec<Vec<T>>> {
    let mut lods = Vec::new();
    let mut unnumbered = Vec::new();
    for (lod, record) in records {
        match lod.unwrap_or(-1) {
            -1 => unnumbered.push(record),
            lod if lod >= 0 => {
                let index = lod as usize;
                if lods.len() <= index {
                    lods.try_reserve(index + 1 - lods.len())
                        .map_err(io::Error::other)?;
                    lods.resize_with(index + 1, Vec::new);
                }
                lods[index].push(record);
            }
            lod => return Err(invalid(format!("native mesh invalid negative LOD {lod}"))),
        }
    }
    if !unnumbered.is_empty() {
        lods.push(unnumbered);
    }
    Ok(lods)
}

pub(super) fn descriptor(
    decoded: &mesh::Mesh,
    resolved: &mesh::ResolvedMesh<'_>,
    node: usize,
    name: &str,
    mut geometry: impl FnMut(GeometryKey, &mesh::SubMesh) -> io::Result<Arc<mesh_shader::Geometry>>,
    mut material_index: impl FnMut(&str, &mesh::SubMesh) -> io::Result<usize>,
) -> io::Result<Descriptor> {
    let mut records = Vec::new();
    for (shape_index, shape) in decoded.shapes.iter().enumerate() {
        for (sub_index, sub) in shape.meshes.iter().enumerate() {
            if !super::super::mesh_effect::candidate(sub) {
                continue;
            }
            let geometry_key = (
                resolved.definition.file.clone(),
                resolved.definition.scale.to_bits(),
                shape_index,
                sub_index,
            );
            records.push((
                shape.lod,
                Part {
                    geometry: geometry(geometry_key.clone(), sub).map_err(|error| {
                        invalid(format!(
                            "{} shape {:?} record {}: {error}",
                            resolved.definition.file, shape.name, sub.child_index
                        ))
                    })?,
                    geometry_key,
                    material: resolved.definition.material(&shape.name, sub),
                    material_index: material_index(&shape.name, sub)?,
                },
            ));
        }
    }
    Ok(Descriptor {
        name: name.to_owned(),
        node,
        distances: decoded.lod_distances.iter().map(|v| v * v).collect(),
        lods: lods(records)?,
    })
}

impl Descriptor {
    /// The game compares y²+x²+z² against the squared ladder backwards;
    ///below every threshold (or without a ladder), use the last descriptor LOD.
    fn lod(&self, position: [f32; 3], eye: [f32; 3]) -> io::Result<Option<usize>> {
        let Some(last) = self.lods.len().checked_sub(1) else {
            return Ok(None);
        };
        let [x, y, z] = std::array::from_fn(|i| position[i] - eye[i]);
        let distance = y * y + x * x + z * z;
        let index = self
            .distances
            .iter()
            .rposition(|v| *v <= distance)
            .unwrap_or(last);
        if index >= self.lods.len() {
            return Err(invalid(format!(
                "native mesh LOD {index} is outside {} descriptor levels",
                self.lods.len()
            )));
        }
        Ok(Some(index))
    }
}

#[derive(Clone, Copy)]
pub struct Inputs<'a> {
    pub samples: u32,
    pub frames: &'a [&'a [u8]],
    pub grid: &'a [u8],
    /// Physical pixel slots 4..13, independent of material samplers.
    pub textures: [&'a wgpu::TextureView; 10],
}

/// The game appends regular/alternate/caster queues without sorting.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Queue {
    Regular,
    Alternate,
    Caster,
}

impl Queue {
    fn accepts(self, blended: bool, forced: bool) -> bool {
        match self {
            Self::Regular => !blended && !forced,
            Self::Alternate => blended || forced,
            Self::Caster => true,
        }
    }
}

/// The game zeroes CPU scratch once per queue invocation. Later actors without
///userdata retain its preceding bytes. The mode0 buffer selects a
///new allocation on each upload; a VS-only bind retains PS's snapshot.
pub struct ObjectConstants {
    bytes: [u8; 208],
    pixel: Option<[u8; 208]>,
    pub retained: super::super::shader_scene::State,
}

impl Default for ObjectConstants {
    fn default() -> Self {
        Self {
            bytes: [0; 208],
            pixel: None,
            retained: Default::default(),
        }
    }
}

impl ObjectConstants {
    /// A preceding world producer replaced physical PS b1. Its allocation remains in the
    /// retained stream; a mesh without userdata must not rebind an older CPU snapshot.
    pub fn forget_pixel_snapshot(&mut self) {
        self.pixel = None;
    }

    pub fn begin(&mut self) {
        self.bytes.fill(0);
    }
}

/// One queue invocation's retained binds, applied after its draws. Each native bind replaces
/// only its own slot (a null pixel sampler keeps the previous one), so the last value per slot
/// equals binding every draw in turn.
struct Pending {
    part: Option<usize>,
    /// The part whose program last bound a sampler with each texture.
    samplers: [Option<usize>; TEXTURE_BINDINGS],
    skin: Option<usize>,
    frame: Option<usize>,
    grid: bool,
}

impl Default for Pending {
    fn default() -> Self {
        Self {
            part: None,
            samplers: [None; TEXTURE_BINDINGS],
            skin: None,
            frame: None,
            grid: false,
        }
    }
}

impl Pending {
    fn record(&mut self, key: usize, resident: &Resident, textures: u32) {
        if self.part != Some(key) {
            let mut samplers = textures & resident.samplers;
            while samplers != 0 {
                let slot = samplers.trailing_zeros() as usize;
                self.samplers[slot] = Some(key);
                samplers &= samplers - 1;
            }
            if resident.geometry.cpu_skin.is_some() {
                self.skin = Some(key);
            }
        }
        self.part = Some(key);
    }

    fn apply_sources<'a>(
        &self,
        resident: impl Fn(usize) -> &'a Resident,
        textures: Textures,
        buffers: Constants,
        retained: &mut shader_scene::State,
    ) {
        let Some(part) = self.part else {
            return;
        };
        resident(part).retained.select(retained);
        let stage = |key: u32| {
            if key.is_multiple_of(2) {
                Stage::Vertex
            } else {
                Stage::Pixel
            }
        };
        let stream = &mut retained.stream;
        for (key, bytes) in (0..).zip(buffers) {
            let Some(bytes) = bytes else {
                continue;
            };
            if key / 2 == 0 {
                stream.bind_partial_constants(0, stage(key), Some(bytes), &[0..348, 352..408]);
            } else {
                stream.bind_constants(key / 2, stage(key), Some(bytes));
            }
        }
        for (key, texture) in (0..).zip(textures) {
            let Some(texture) = texture else {
                continue;
            };
            let sampler =
                self.samplers[key as usize].and_then(|part| resident(part).retained.sampler(key));
            stream.bind_texture(key / 2, stage(key), Some(texture.clone()), sampler);
        }
        let selected = resident(part);
        retained.bind_vertex(
            0,
            selected.geometry.native_stride,
            selected.geometry.cpu_vertices.clone(),
        );
        if let Some(skin) = self
            .skin
            .and_then(|part| resident(part).geometry.cpu_skin.clone())
        {
            retained.bind_vertex(1, 16, skin);
        }
    }
}

/// A draw's constant blocks and textures by `binding(slot, stage)`.
type Constants<'a> = [Option<&'a [u8]>; CONSTANT_BINDINGS];
type Textures<'a> = [Option<&'a wgpu::TextureView>; TEXTURE_BINDINGS];

struct Resident {
    pipeline: mesh_shader::Pass,
    required_constants: Vec<u32>,
    required_textures: Vec<u32>,
    constant_mask: u32,
    texture_mask: u32,
    /// The lit program's native binds, which later passes retain.
    retained: shader_scene::Program,
    geometry: SharedGeometry,
    caster_texture: bool,
    blended: bool,
    /// Physical slots this material updates, independent of shader reads.
    samplers: u32,
    /// Published only at the successful descriptor commit. Stale programs stay cold until use.
    program: Option<(ResourceStamp, plan_tail::Program)>,
}

enum PrepareError {
    Actor(io::Error),
    Shared(io::Error),
}

impl From<io::Error> for PrepareError {
    fn from(error: io::Error) -> Self {
        Self::Shared(error)
    }
}

struct PreparedPart {
    key: usize,
    bindings: shader_pipeline::gpu::Bindings,
    textures: u32,
    material: usize,
    count: u32,
}

impl Resident {
    fn check_bindings(&self, buffers: &Constants, textures: &Textures) -> Result<(), PrepareError> {
        for &slot in &self.required_constants {
            if buffers[slot as usize].is_none() {
                return Err(PrepareError::Actor(invalid(format!(
                    "native constant buffer {} has not been bound",
                    slot / 2
                ))));
            }
        }
        for &slot in &self.required_textures {
            if textures[slot as usize].is_none() {
                return Err(PrepareError::Actor(invalid(format!(
                    "native texture slot {} has not been bound",
                    slot / 2
                ))));
            }
        }
        Ok(())
    }
}

#[derive(Clone)]
struct SharedGeometry {
    stride: u64,
    native_stride: u64,
    cpu_vertices: Arc<[u8]>,
    cpu_skin: Option<Arc<[u8]>>,
    /// Pooled geometry: vertex chunk and first vertex, index chunk and first index.
    vertices: (usize, i32),
    indices: (usize, u32),
    /// One past the largest palette index the skin stream names.
    skin_bones: usize,
}

/// Dense resident slots for one mesh: node/LOD/part, then caster/forced selection.
type MeshSlots = Vec<Vec<Vec<[Option<usize>; 4]>>>;

pub(super) struct Renderer {
    /// The mesh effects of the stack the entities were baked from, then of the recording alone:
    /// a visual mod entity's recorded counterpart ([`super::ModelLibrary::alternatives`]) is
    /// drawn with the recording's own shaders, as the recording draws it.
    libraries: [Library; 2],
    cache: shader_pipeline::gpu::Cache,
    residents: Vec<Resident>,
    /// Mesh/node/LOD/part, then caster/forced selection. Sized from demanded content.
    slots: Vec<MeshSlots>,
    resource_ids: ResourceIdentities,
    geometry: BTreeMap<GeometryKey, SharedGeometry>,
    /// Same deferred mesh/node queue decisions, addressed without tree lookups.
    queue_slots: Vec<Vec<Option<[bool; 2]>>>,
    frame_handles: walk::FrameHandles,
    binding_storage: shader_pipeline::gpu::BindingSessionStorage,
    scratch: plans::Scratch,
    geometry_seen: Vec<bool>,
    prepared: Vec<PreparedPart>,
    candidates: Vec<(usize, shader_pipeline::gpu::BindingProgram)>,
    /// Vertex pools by stream stride and skinning; the index pool.
    pools: BTreeMap<(u64, bool), Pool>,
    indices: Pool,
    /// Command packet storage returned after this pass has been encoded.
    draws: DrawList,
}

#[derive(Clone, Copy, Default, PartialEq, Eq)]
struct ResourceStamp {
    local: u64,
    globals: u64,
}

#[derive(Default)]
struct ResourceIdentities {
    local: Vec<Option<wgpu::TextureView>>,
    globals: Option<[wgpu::TextureView; 10]>,
    stamp: ResourceStamp,
}

impl ResourceIdentities {
    fn observe(
        &mut self,
        local: &[Option<Bound>],
        globals: Option<[&wgpu::TextureView; 10]>,
    ) -> ResourceStamp {
        let unchanged = self.local.len() == local.len()
            && self
                .local
                .iter()
                .zip(local)
                .all(|(old, new)| match (old, new) {
                    (None, None) => true,
                    (Some(old), Some(new)) => *old == new.view,
                    _ => false,
                });
        if !unchanged {
            self.local = local
                .iter()
                .map(|texture| texture.as_ref().map(|texture| texture.view.clone()))
                .collect();
            self.stamp.local += 1;
        }
        if let Some(globals) = globals
            && self
                .globals
                .as_ref()
                .is_none_or(|last| !last.iter().eq(globals))
        {
            self.globals = Some(globals.map(Clone::clone));
            self.stamp.globals += 1;
        }
        ResourceStamp {
            local: self.stamp.local,
            globals: globals.map_or(0, |_| self.stamp.globals),
        }
    }
}

type VertexKey = ((u64, bool), usize);

/// Prepared in native order on the presentation thread; encoding reads only GPU handles.
#[derive(Default)]
pub(crate) struct DrawList {
    draws: Vec<RecordedDraw>,
    vertices: BTreeMap<VertexKey, [Option<wgpu::Buffer>; 2]>,
    indices: BTreeMap<usize, wgpu::Buffer>,
}

struct RecordedDraw {
    bindings: shader_pipeline::gpu::Bindings,
    reference: u32,
    vertices: VertexKey,
    indices: usize,
    first: u32,
    count: u32,
    base: i32,
}

impl DrawList {
    pub(crate) fn record(&self, pass: &mut wgpu::RenderPass<'_>) {
        self.record_slice(pass, &self.draws);
    }

    fn record_slice(&self, pass: &mut wgpu::RenderPass<'_>, draws: &[RecordedDraw]) {
        if draws.is_empty() {
            return;
        }
        let mut previous: Option<&RecordedDraw> = None;
        let mut vertices = None;
        let mut indices = None;
        pass.set_blend_constant(wgpu::Color::WHITE);
        for draw in draws {
            if previous.is_none_or(|last| last.reference != draw.reference) {
                pass.set_stencil_reference(draw.reference);
            }
            if vertices != Some(draw.vertices) {
                for (slot, stream) in self.vertices[&draw.vertices].iter().enumerate() {
                    if let Some(stream) = stream {
                        pass.set_vertex_buffer(slot as u32, stream.slice(..));
                    }
                }
                vertices = Some(draw.vertices);
            }
            if indices != Some(draw.indices) {
                pass.set_index_buffer(
                    self.indices[&draw.indices].slice(..),
                    wgpu::IndexFormat::Uint32,
                );
                indices = Some(draw.indices);
            }
            draw.bindings
                .bind_after(pass, previous.map(|last| &last.bindings));
            pass.draw_indexed(draw.first..draw.first + draw.count, draw.base, 0..1);
            previous = Some(draw);
        }
    }

    /// Encode contiguous slices independently; all passes load/store the same attachments.
    /// Submission retains slice order and no pass samples these attachments between slices.
    fn encode(
        lists: &[&Self],
        device: &wgpu::Device,
        targets: &Targets,
        admitted: &viewer_foundation::parallel::Admitted,
        preceding: &mut wgpu::CommandEncoder,
    ) -> Vec<wgpu::CommandBuffer> {
        let total = lists.iter().map(|list| list.draws.len()).sum::<usize>();
        let count = total.min(admitted.capacity());
        if count == 0 {
            admitted.run(0, &|_, _| {});
            return Vec::new();
        }
        let preceding = std::sync::Mutex::new(Some(std::mem::replace(
            preceding,
            device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("map") }),
        )));
        // The first buffer stays before every slice, whichever encoder finishes first.
        let outputs: Vec<_> = (0..=count).map(|_| std::sync::Mutex::new(None)).collect();
        let finish = |_: &dyn Fn() -> bool| {
            if let Some(encoder) = preceding.lock().unwrap_or_else(|e| e.into_inner()).take() {
                *outputs[0].lock().unwrap_or_else(|e| e.into_inner()) = Some(encoder.finish());
            }
        };
        let wall_profile = viewer_foundation::profile_writer::profiling();
        let (count, timing) = viewer_foundation::parallel::profile_run(wall_profile, || {
            admitted.run_beside(
                count,
                &|index, count| {
                    let std::ops::Range { start, end } = draw_slice(total, count, index);
                    let mut encoder =
                        device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
                            label: Some("native model slice"),
                        });
                    {
                        let mut colour = targets.scene_attachment(wgpu::LoadOp::Load);
                        // No producer samples the resolved scene between these slices. Keep every
                        // multisample value, but resolve only after the last slice's draws.
                        if index + 1 < count {
                            colour.resolve_target = None;
                        }
                        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                            label: Some("native model slice"),
                            color_attachments: &[Some(colour)],
                            depth_stencil_attachment: Some(
                                targets.depth_attachment(wgpu::LoadOp::Load),
                            ),
                            ..Default::default()
                        });
                        let mut base = 0;
                        for list in lists {
                            let next = base + list.draws.len();
                            if base < end && next > start {
                                list.record_slice(
                                    &mut pass,
                                    &list.draws[start.saturating_sub(base)
                                        ..(end - base).min(list.draws.len())],
                                );
                            }
                            base = next;
                            if base >= end {
                                break;
                            }
                        }
                    }
                    *outputs[index + 1].lock().unwrap_or_else(|e| e.into_inner()) =
                        Some(encoder.finish());
                },
                Some(&finish),
            )
        });
        // Busy builders, or helpers that did not take a turn before the caller finished,
        // leave the preceding encoder here. No commands are submitted until Scene is done.
        let clock = wall_profile.then(std::time::Instant::now);
        finish(&|| true);
        let caller = timing.own + clock.map_or(std::time::Duration::ZERO, |clock| clock.elapsed());

        if wall_profile {
            viewer_foundation::profiling::record_lap("native_encode_caller", caller);
            viewer_foundation::profiling::record_lap("native_turns_finish_wait", timing.wait);
        }
        outputs
            .into_iter()
            .take(count + 1)
            .map(|output| {
                output
                    .into_inner()
                    .unwrap_or_else(|e| e.into_inner())
                    .expect("native slice encoded")
            })
            .collect()
    }

    fn clear(&mut self) {
        self.draws.clear();
        self.vertices.clear();
        self.indices.clear();
    }
}

/// Balanced contiguous runs, sized from the actual draw count and borrowed worker count.
fn draw_slice(total: usize, count: usize, index: usize) -> std::ops::Range<usize> {
    let width = total / count;
    let extra = total % count;
    let start = index * width + index.min(extra);
    start..start + width + usize::from(index < extra)
}

/// Resident geometry in shared chunk buffers, so a draw of another part moves
/// `base_vertex`/`first_index` instead of rebinding streams. Records of one stride share
/// numbering; a skinned pool keeps each chunk's skin stream at the same record numbers.
#[derive(Default)]
struct Pool {
    chunks: Vec<PoolChunk>,
}

struct PoolChunk {
    streams: [Option<wgpu::Buffer>; 2],
    capacity: u64,
    used: u64,
}

/// Records per chunk unless one part needs more.
const POOL_RECORDS: u64 = 1 << 16;

impl Pool {
    /// Place `count` records of `streams` (bytes per record in `strides`); returns the chunk
    /// and first record. Bytes reach the GPU at the arena's next flush.
    fn place(
        &mut self,
        arena: &shader_pipeline::gpu::Arena,
        usage: wgpu::BufferUsages,
        strides: [u64; 2],
        streams: [Option<&[u8]>; 2],
        count: u64,
    ) -> io::Result<(usize, u64)> {
        let device = arena.device();
        let fits = |chunk: &PoolChunk| chunk.used + count <= chunk.capacity;
        if self.chunks.last().is_none_or(|chunk| !fits(chunk)) {
            let capacity = POOL_RECORDS.max(count);
            let mut chunk = PoolChunk {
                streams: [None, None],
                capacity,
                used: 0,
            };
            for (stream, (stride, bytes)) in
                chunk.streams.iter_mut().zip(strides.iter().zip(streams))
            {
                if bytes.is_none() {
                    continue;
                }
                let size = (capacity * stride).next_multiple_of(wgpu::COPY_BUFFER_ALIGNMENT);
                if size > device.limits().max_buffer_size {
                    return Err(invalid(format!(
                        "native mesh pool: {size}-byte buffer exceeds GPU limit {}",
                        device.limits().max_buffer_size
                    )));
                }
                *stream = Some(device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some("native mesh pool"),
                    size,
                    usage: usage | wgpu::BufferUsages::COPY_DST,
                    mapped_at_creation: false,
                }));
            }
            self.chunks.push(chunk);
        }
        let index = self.chunks.len() - 1;
        let chunk = &mut self.chunks[index];
        let first = chunk.used;
        for ((stream, stride), bytes) in chunk.streams.iter().zip(strides).zip(streams) {
            if let (Some(buffer), Some(bytes)) = (stream, bytes) {
                arena.write(buffer, first * stride, bytes);
            }
        }
        chunk.used += count;
        Ok((index, first))
    }
}

/// What a part draws with, from its built pass: the compiled program of the technique the pass
/// was built from (the recorded one when a visual mod's replacement fell back, never the
/// technique asked for), and the retained program: that technique, its vertex layouts at the
/// geometry's compact stride, and its state.
fn part_program(
    pipeline: &mesh_shader::Pass,
    geometry: &mesh_shader::Geometry,
) -> io::Result<(Arc<shader_pipeline::Compiled>, shader_scene::Program)> {
    let selection = &pipeline.selection;
    let compiled = selection
        .effect
        .compiled(selection.index)
        .map_err(invalid)?;
    let mut layouts = mesh_shader::vertex_layouts(
        &compiled.vertex.inputs,
        geometry.uv1,
        !geometry.skin.is_empty(),
    )?;
    // Retained declarations bind the game's compact stride.
    layouts[0].stride = geometry.stride;
    let effect = &selection.effect.source;
    let technique = &effect.techniques[selection.index];
    let mut state = shader_pipeline::gpu::State::authored(effect, technique).map_err(invalid)?;
    state
        .samplers
        .retain(|key, _| (0..3).any(|slot| *key == binding(slot, Stage::Pixel)));
    state.samplers.extend(mesh_shader::map_samplers());
    let retained = shader_scene::Program::new(
        selection,
        &layouts,
        &state,
        &effect.rasterizer(technique),
        &effect.depth_stencil(technique),
    );
    Ok((compiled, retained))
}

fn select(library: &mut Library, part: &Part, caster: bool) -> io::Result<Selection> {
    let defines = if part.geometry.uv1 {
        vec!["PDX_MESH_UV1".to_owned()]
    } else {
        Vec::new()
    };
    let effect = library
        .load(
            part.material
                .shader_file
                .as_deref()
                .unwrap_or("gfx/FX/pdxmesh.lua")
                .as_bytes(),
            &defines,
        )
        .map_err(invalid)?;
    let base = part.material.shader.as_deref().unwrap_or_default();
    let skinned = !part.geometry.skin.is_empty();
    let suffix = match (skinned, caster) {
        (false, false) => "",
        (true, false) => "Skinned",
        (false, true) => "Shadow",
        (true, true) => "SkinnedShadow",
    };
    let name = format!("{base}{suffix}");
    if !base.is_empty()
        && let Some(selection) = effect.select(&name).map_err(invalid)?
    {
        return Ok(selection);
    }
    // Install the fallback without material UV defines.
    library
        .load(b"gfx/FX/pdxmesh.lua", &[])
        .map_err(invalid)?
        .select(&format!("PdxMeshColor{suffix}"))
        .map_err(invalid)?
        .ok_or_else(|| invalid("native PdxMeshColor fallback is unavailable"))
}

/// The game appends Alpha to the skin suffix; absent Alpha retains the base
/// variant (not PdxMeshColor). Draws use this handle as the alpha variant.
fn select_alpha(library: &mut Library, part: &Part) -> io::Result<Selection> {
    let base = select(library, part, false)?;
    let defines = if part.geometry.uv1 {
        vec!["PDX_MESH_UV1".to_owned()]
    } else {
        Vec::new()
    };
    let effect = library
        .load(
            part.material
                .shader_file
                .as_deref()
                .unwrap_or("gfx/FX/pdxmesh.lua")
                .as_bytes(),
            &defines,
        )
        .map_err(invalid)?;
    let name = part.material.shader.as_deref().unwrap_or_default();
    let skin = if part.geometry.skin.is_empty() {
        ""
    } else {
        "Skinned"
    };
    Ok(effect
        .select(&format!("{name}{skin}Alpha"))
        .map_err(invalid)?
        .unwrap_or(base))
}

/// Select whether the material uses a caster texture and the alternate queue.
/// The shadow-state check runs only when the lit state did not admit a texture.
fn material_flags(
    lit: &installed_content_core::pdxshader::BlendState,
    shadow: &installed_content_core::pdxshader::BlendState,
    diffuse: bool,
) -> (bool, bool) {
    let mut textured = diffuse && (lit.enable || lit.alpha_test);
    let mut alternate = lit.enable;
    if !textured && (shadow.enable || shadow.alpha_test) {
        alternate = true;
        textured = diffuse;
    }
    (textured, alternate)
}

fn matrix(transform: &mesh::Transform) -> [[f32; 4]; 4] {
    let [a, b, c] = transform.basis;
    let t = transform.translation;
    [
        [a[0], a[1], a[2], 0.],
        [b[0], b[1], b[2], 0.],
        [c[0], c[1], c[2], 0.],
        [t[0], t[1], t[2], 1.],
    ]
}

impl ModelPass {
    pub(crate) fn has_placements(&self) -> bool {
        !self.field.native_order.is_empty()
    }

    pub fn reset_drawn(&self) {
        self.drawn.set(0);
    }

    #[allow(clippy::too_many_arguments)]
    pub fn draw_native(
        &self,
        arena: &shader_pipeline::gpu::Arena,
        encoder: &mut wgpu::CommandEncoder,
        targets: &Targets,
        frame: ModelFrame<'_>,
        inputs: Inputs<'_>,
        queue: Queue,
        object: &mut ObjectConstants,
    ) -> io::Result<()> {
        if self.field.native_order.is_empty() {
            return Ok(());
        }
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some(self.label),
            color_attachments: &[Some(targets.scene_attachment(wgpu::LoadOp::Load))],
            depth_stencil_attachment: Some(targets.depth_attachment(wgpu::LoadOp::Load)),
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        self.record_native(arena, &mut pass, frame, inputs, queue, object)
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn record_native(
        &self,
        arena: &shader_pipeline::gpu::Arena,
        pass: &mut wgpu::RenderPass<'_>,
        frame: ModelFrame<'_>,
        inputs: Inputs<'_>,
        queue: Queue,
        object: &mut ObjectConstants,
    ) -> io::Result<()> {
        let draws = self.prepare_native(arena, frame, inputs, queue, object)?;
        {
            let wall_profile = viewer_foundation::profile_writer::profiling();
            let clock = wall_profile.then(std::time::Instant::now);
            draws.record(pass);
            let caller = clock.map_or(std::time::Duration::ZERO, |clock| clock.elapsed());

            if wall_profile {
                viewer_foundation::profiling::record_lap("native_encode_caller", caller);
                viewer_foundation::profiling::record_lap(
                    "native_turns_finish_wait",
                    std::time::Duration::ZERO,
                );
            }
        }
        self.recycle_native(draws);
        Ok(())
    }

    pub(crate) fn recycle_native(&self, mut draws: DrawList) {
        draws.clear();
        if let Some(native) = self.native.borrow_mut().as_mut() {
            native.draws = draws;
        }
    }

    /// Preparation retains native order; encoding lends only the immutable command packets.
    pub(crate) fn encode_native(
        lists: &[(&Self, DrawList)],
        device: &wgpu::Device,
        targets: &Targets,
        admitted: &viewer_foundation::parallel::Admitted,
        preceding: &mut wgpu::CommandEncoder,
    ) -> Vec<wgpu::CommandBuffer> {
        DrawList::encode(
            &lists.iter().map(|(_, list)| list).collect::<Vec<_>>(),
            device,
            targets,
            admitted,
            preceding,
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn prepare_native(
        &self,
        arena: &shader_pipeline::gpu::Arena,
        frame: ModelFrame<'_>,
        inputs: Inputs<'_>,
        queue: Queue,
        object: &mut ObjectConstants,
    ) -> io::Result<DrawList> {
        // One clock per queue invocation, only under normal PROFILE.
        let _wall = viewer_foundation::profiling::span(if queue == Queue::Caster {
            "native_caster_prepare"
        } else {
            "native_lit_prepare"
        });
        if self.field.native_order.is_empty() {
            return Ok(DrawList::default());
        }
        let caster = queue == Queue::Caster;
        let caller = plan_tail::Caller::select(self.label);
        // The game forces every train actor and attachment.
        let forced = self.label == "supply trains";
        let mut renderer = self.native.borrow_mut();
        if renderer.is_none() {
            let layers = self
                .layers
                .as_ref()
                .ok_or_else(|| invalid("mesh content stack unavailable"))?;
            *renderer = Some(Renderer {
                libraries: {
                    let globals = super::super::shader_sources::globals(
                        [frame.camera.map.width, frame.camera.map.height],
                        self.decode,
                    );
                    let recorded = layers
                        .without_overlays()
                        .map_or_else(|| layers.clone(), Arc::new);
                    [
                        Library::new(layers.clone()).with_globals(globals.clone()),
                        Library::new(recorded).with_globals(globals),
                    ]
                },
                cache: Default::default(),
                residents: Vec::new(),
                slots: Vec::new(),
                resource_ids: ResourceIdentities::default(),
                geometry: BTreeMap::new(),
                queue_slots: Vec::new(),
                frame_handles: Default::default(),
                binding_storage: Default::default(),
                scratch: Default::default(),
                geometry_seen: Vec::new(),
                prepared: Vec::new(),
                candidates: Vec::new(),
                pools: BTreeMap::new(),
                indices: Pool::default(),
                draws: DrawList::default(),
            });
        }
        let native = renderer.as_mut().expect("initialized above");
        let stamp = native
            .resource_ids
            .observe(&self.textures, (!caster).then_some(inputs.textures));
        native.slots.resize_with(self.meshes.len(), Vec::new);
        native.queue_slots.resize_with(self.meshes.len(), Vec::new);
        let layout = &self.native_walk;
        let mut frame_handles = std::mem::take(&mut native.frame_handles);
        let dense_handles = layout.installed && frame_handles.begin(layout.players.len());

        let mut commands = std::mem::take(&mut native.draws);
        let device = arena.device();
        let mut poses = self.poses.borrow_mut();
        if poses.0 != arena.frame() {
            *poses = (arena.frame(), Vec::new());
        }
        poses.1.resize_with(self.field.instances.len(), || None);
        let eye = frame.camera.eye();
        let mut drawn = 0;
        let mut last_material = None;
        let mut pending_sources = plans::PendingSources::default();
        let mut scratch = std::mem::take(&mut native.scratch);
        scratch.begin(object);
        let mut binding_session = shader_pipeline::gpu::BindingSession::with_storage(
            arena, std::mem::take(&mut native.binding_storage));
        native.geometry_seen.clear();
        native.geometry_seen.resize(native.residents.len(), false);
        let mut prepared = std::mem::take(&mut native.prepared);
        let mut candidates = std::mem::take(&mut native.candidates);
        // Physical pixel slots 4..13; material slots 0..2 are replaced per draw.
        let mut textures: Textures = [None; TEXTURE_BINDINGS];
        let mut texture_mask = 0;
        if !caster {
            for (i, texture) in inputs.textures.iter().enumerate() {
                let slot = binding(i as u32 + 4, Stage::Pixel) as usize;
                textures[slot] = Some(*texture);
                texture_mask |= 1 << slot;
            }
        }
        for (view, offset) in frame.offsets.iter().enumerate() {
            for (group_index, index, row) in layout.routes(&self.field) {
                let group = &self.field.groups[group_index];
                let library = row.map_or_else(|| self.shader_library(group.mesh), |row| row.library);
                if !visible(group, *offset, frame.visible) {
                    continue;
                }
                let mesh = &self.meshes[group.mesh];
                {
                    let instance = self.field.instances[index];
                    let hidden = row.map_or_else(|| {
                        let (seed, identity) = instance.animation_key();
                        self.hidden.contains(&(group.mesh, seed, identity))
                    }, |row| row.hidden);
                    if hidden { continue; }

                    let instance_world = match instance.native_transform() {
                        Ok(world) => world,
                        Err(why) => {
                            // The instance's own recorded transform, whatever draws it.

                            self.refuse_actor(arena, None, index, usize::MAX, &mesh.name, why);
                            continue;
                        }
                    };

                    let Some(rig) = self.rigs.get(group.mesh) else {
                        self.refuse_actor(
                            arena,
                            Some(group.mesh),
                            index,
                            usize::MAX,
                            &mesh.name,
                            "native actor rig unavailable",
                        );
                        continue;
                    };
                    let actors = poses.1[index].get_or_insert_with(|| {
                        // Default-state refusal keeps the original frozen-choice law.
                        let frozen = !matches!(
                            instance.animation,
                            entity_animation::STOPPED | entity_animation::DEFAULT_STATE
                        );
                        if dense_handles {
                            let row = row.expect("installed actor route");
                            frame_handles.resolve(row.player, row.mesh, frozen, || {
                                self.actors(group.mesh, rig, self.players.get(&layout.players[row.player]), frozen)
                            })
                        } else {
                            // Optional handle allocation cannot add a refusal or suppress
                            // the original actor demand. No separate preparation path.
                            self.actors(group.mesh, rig, self.player(group.mesh, &instance), frozen)
                        }
                    });

                    for descriptor in &mesh.native {
                        let source_before = scratch.checkpoint();
                        candidates.clear();

                        let mut submit = || -> Result<(), PrepareError> {
                            let actor = &actors[descriptor.node];
                            let world = mesh_pose::multiply(
                                matrix(&instance_world),
                                *actor
                                    .world
                                    .as_ref()
                                    .map_err(|why| PrepareError::Actor(invalid(why.clone())))?,
                            );
                            let position = [world[3][0] + offset, world[3][1], world[3][2]];

                            let Some(lod) =
                                descriptor.lod(position, eye).map_err(PrepareError::Actor)?
                            else {
                                return Ok(());
                            };

                            for source in &descriptor.lods[lod] {
                                if let Some(why) =
                                    self.material_refusals.get(&source.material_index)
                                {
                                    return Err(PrepareError::Actor(invalid(why.clone())));
                                }
                            }

                            // No rest palette: the game returns without a played clip;
                            // Draws still upload the allocated bytes.
                            let skin_pose = descriptor.lods[lod]
                                .iter()
                                .any(|source| !source.geometry.skin.is_empty())
                                .then(|| {
                                    actor.pose.as_ref().map_err(|why| {
                                        PrepareError::Actor(invalid(format!("skin: {why}")))
                                    })
                                })
                                .transpose()?;

                            let queues = &mut native.queue_slots[group.mesh];
                            if queues.len() <= descriptor.node {
                                queues.resize(descriptor.node + 1, None);
                            }
                            if queues[descriptor.node].is_none() {
                                let mut admitted = [false; 2];
                                for part in descriptor.lods.iter().flatten() {
                                    let lit = select(&mut native.libraries[library], part, false)?;
                                    let shadow =
                                        select(&mut native.libraries[library], part, true)?;
                                    let (_, alternate) = material_flags(
                                        &lit.effect
                                            .source
                                            .blend(&lit.effect.source.techniques[lit.index]),
                                        &shadow
                                            .effect
                                            .source
                                            .blend(&shadow.effect.source.techniques[shadow.index]),
                                        !part.material.diffuse.is_empty(),
                                    );
                                    admitted[usize::from(alternate)] = true;
                                }
                                queues[descriptor.node] = Some(admitted);
                            }
                            let flags = queues[descriptor.node].unwrap();
                            if !match queue {
                                Queue::Regular => flags[0] && !forced,
                                Queue::Alternate => flags[1] || forced,
                                Queue::Caster => flags[0],
                            } {
                                return Ok(());
                            }

                            // The game propagates the ROOT actor payload to every child.
                            let root = rig.nodes[0].native.as_ref().ok_or_else(|| {
                                PrepareError::Actor(invalid("native root actor source unavailable"))
                            })?;
                            let userdata = if caster {
                                None
                            } else {
                                caller.evaluate(root, instance, frame.alpha).map_err(PrepareError::Actor)?
                            };

                            scratch.update(&world, userdata);
                            let (vertex, pixel) = (scratch.vertex(), scratch.pixel());

                            prepared.clear();

                            let nodes = &mut native.slots[group.mesh];
                            nodes.resize_with(rig.nodes.len(), Vec::new);
                            if nodes[descriptor.node].is_empty() {
                                nodes[descriptor.node] = descriptor
                                    .lods
                                    .iter()
                                    .map(|parts| vec![[None; 4]; parts.len()])
                                    .collect();
                            }

                            let mut submitted_mask = 0;
                            for (part, source) in descriptor.lods[lod].iter().enumerate() {
                                let variants = usize::from(caster) * 2 + usize::from(forced);
                                let key = match native.slots[group.mesh][descriptor.node][lod][part]
                                    [variants]
                                {
                                    Some(key) => key,
                                    None => {
                                        let selection = if forced && !caster {
                                            select_alpha(&mut native.libraries[library], source)?
                                        } else {
                                            select(&mut native.libraries[library], source, caster)?
                                        };
                                        let source_blend = |s: &Selection| {
                                            s.effect
                                                .source
                                                .blend(&s.effect.source.techniques[s.index])
                                        };
                                        let lit =
                                            select(&mut native.libraries[library], source, false)?;
                                        let shadow =
                                            select(&mut native.libraries[library], source, true)?;
                                        let (caster_texture, blended) = material_flags(
                                            &source_blend(&lit),
                                            &source_blend(&shadow),
                                            !source.material.diffuse.is_empty(),
                                        );
                                        let pipeline = if caster {
                                            mesh_shader::Pass::caster(
                                                &mut native.cache,
                                                device,
                                                &selection,
                                                &source.geometry,
                                                DEPTH_MAP_FORMAT,
                                                DEPTH_FORMAT,
                                            )?
                                        } else {
                                            mesh_shader::Pass::new(
                                                &mut native.cache,
                                                device,
                                                &selection,
                                                &source.geometry,
                                                SCENE_FORMAT,
                                                DEPTH_FORMAT,
                                                inputs.samples,
                                            )?
                                        };
                                        let (compiled, retained) =
                                            part_program(&pipeline, &source.geometry)?;
                                        let geometry = match native
                                            .geometry
                                            .get(&source.geometry_key)
                                        {
                                            Some(geometry) => geometry.clone(),
                                            None => {
                                                let stride =
                                                    if source.geometry.uv1 { 56 } else { 48 };
                                                let skin: Option<&[u8]> =
                                                    (!source.geometry.skin.is_empty()).then(|| {
                                                        bytemuck::cast_slice(&source.geometry.skin)
                                                    });
                                                let packed = source
                                                    .geometry
                                                    .packed(source.geometry.uv1)
                                                    .map_err(PrepareError::Actor)?;
                                                let vertices = native
                                                    .pools
                                                    .entry((stride, skin.is_some()))
                                                    .or_default()
                                                    .place(
                                                        arena,
                                                        wgpu::BufferUsages::VERTEX,
                                                        [stride, 16],
                                                        [Some(bytemuck::cast_slice(&packed)), skin],
                                                        packed.len() as u64 * 4 / stride,
                                                    )?;
                                                let indices = native.indices.place(
                                                    arena,
                                                    wgpu::BufferUsages::INDEX,
                                                    [4, 0],
                                                    [
                                                        Some(bytemuck::cast_slice(
                                                            &source.geometry.indices,
                                                        )),
                                                        None,
                                                    ],
                                                    source.geometry.indices.len() as u64,
                                                )?;
                                                let geometry = SharedGeometry {
                                                    stride,
                                                    native_stride: source.geometry.stride,
                                                    cpu_vertices: bytemuck::cast_slice(
                                                        &source.geometry.vertices,
                                                    )
                                                    .into(),
                                                    cpu_skin: skin.map(Into::into),
                                                    skin_bones: source
                                                        .geometry
                                                        .skin
                                                        .iter()
                                                        .flat_map(|v| v.indices)
                                                        .map(|i| usize::from(i) + 1)
                                                        .max()
                                                        .unwrap_or(0),
                                                    vertices: (
                                                        vertices.0,
                                                        i32::try_from(vertices.1)
                                                            .map_err(io::Error::other)?,
                                                    ),
                                                    indices: (
                                                        indices.0,
                                                        u32::try_from(indices.1)
                                                            .map_err(io::Error::other)?,
                                                    ),
                                                };
                                                native.geometry.insert(
                                                    source.geometry_key.clone(),
                                                    geometry.clone(),
                                                );
                                                geometry
                                            }
                                        };
                                        let samplers = (0..TEXTURE_BINDINGS as u32)
                                            .filter(|slot| retained.sampler(*slot).is_some())
                                            .fold(0, |mask, slot| mask | (1 << slot));
                                        let key = native.residents.len();
                                        native.residents.push(Resident {
                                            retained,
                                            geometry,
                                            pipeline,
                                            required_constants: compiled
                                                .vertex
                                                .buffers
                                                .iter()
                                                .chain(&compiled.pixel.buffers)
                                                .map(|buffer| buffer.binding)
                                                .collect(),
                                            required_textures: compiled
                                                .vertex
                                                .textures
                                                .iter()
                                                .chain(&compiled.pixel.textures)
                                                .map(|(slot, _)| *slot)
                                                .collect(),
                                            constant_mask: compiled.vertex.buffers.iter().chain(&compiled.pixel.buffers)
                                                .fold(0, |mask, buffer| mask | (1 << buffer.binding)),
                                            texture_mask: compiled.vertex.textures.iter().chain(&compiled.pixel.textures)
                                                .fold(0, |mask, (slot, _)| mask | (1 << slot)),
                                            caster_texture,
                                            blended,
                                            samplers,
                                            program: None,
                                        });
                                        native.slots[group.mesh][descriptor.node][lod][part]
                                            [variants] = Some(key);
                                        key
                                    }
                                };

                                let gpu = &native.residents[key];

                                let material = &self.materials[source.material_index];
                                for (slot, index) in
                                    [material.diffuse, material.specular, material.normal]
                                        .into_iter()
                                        .enumerate()
                                {
                                    let slot = binding(slot as u32, Stage::Pixel) as usize;
                                    let bound = &mut textures[slot];
                                    if caster
                                        && (slot != binding(0, Stage::Pixel) as usize
                                            || !gpu.caster_texture)
                                    {
                                        *bound = None;
                                        texture_mask &= !(1 << slot);
                                        continue;
                                    }
                                    let texture =
                                        self.textures[index].as_ref().ok_or_else(|| {
                                            PrepareError::Actor(invalid(
                                                "native mesh material is not resident",
                                            ))
                                        })?;
                                    *bound = Some(&texture.view);
                                    texture_mask |= 1 << slot;
                                }

                                let skin = if gpu.geometry.cpu_skin.is_some() {
                                    let pose = skin_pose.expect("skinned geometry checked above");
                                    if gpu.geometry.skin_bones > pose.bones.len() {
                                        return Err(PrepareError::Actor(invalid(format!(
                                            "skin: vertex reads outside {} initialized bones",
                                            pose.bones.len()
                                        ))));
                                    }
                                    Some(bytemuck::cast_slice(&pose.bones))
                                } else { None };
                                let supplies = plan_tail::Supplies::new(
                                    inputs.frames[view], vertex, pixel, skin,
                                    (!caster).then_some(inputs.grid));
                                // Missing inputs retain their original ordered diagnostic demand.
                                if supplies.present() & gpu.constant_mask != gpu.constant_mask
                                    || texture_mask & gpu.texture_mask != gpu.texture_mask
                                {
                                    gpu.check_bindings(&supplies.constants(), &textures)?;
                                }

                                let bindings = match &gpu.program {
                                    Some((installed, program)) if *installed == stamp => {
                                        match program.recipe() {
                                            Some(recipe) => binding_session.bind_ordered(key, program, recipe.ordered(supplies)),
                                            None => binding_session.bind_with(key, program, |binding| supplies.get(binding)),
                                        }.map_err(invalid)?
                                    }
                                    _ => {
                                        let (bindings, program) = gpu
                                            .pipeline
                                            .pipeline
                                            .bind_program_with(
                                                arena,
                                                |binding| supplies.get(binding),
                                                |binding| textures[binding as usize],
                                            )
                                            .map_err(invalid)?;
                                        candidates.push((key, program));
                                        bindings
                                    }
                                };

                                if queue.accepts(gpu.blended, forced) {
                                    submitted_mask |= supplies.present();
                                    prepared.push(PreparedPart {
                                        key,
                                        bindings,
                                        textures: texture_mask,
                                        material: source.material_index,
                                        count: source.geometry.indices.len() as u32,
                                    });
                                }
                            }

                            // A failed later part leaves all successful-program candidates cold.
                            for (key, program) in candidates.drain(..) {
                                native.residents[key].program = Some((stamp, plan_tail::Program::new(program)));
                            }
                            // Prepare every part before the first command for this actor.
                            if !caster && !prepared.is_empty() {
                                pending_sources.constants_mask(submitted_mask, view, &scratch, index, descriptor.node);
                            }
                            for PreparedPart {
                                key,
                                bindings,
                                textures,
                                material,
                                count,
                            } in prepared.drain(..)
                            {
                                let gpu = &native.residents[key];
                                if !caster {
                                    pending_sources.state.record(key, gpu, textures);
                                }
                                let pool = (gpu.geometry.stride, gpu.geometry.cpu_skin.is_some());
                                let vertices = (pool, gpu.geometry.vertices.0);
                                native.geometry_seen.resize(native.residents.len(), false);
                                let first_geometry = !native.geometry_seen[key];
                                native.geometry_seen[key] = true;
                                if first_geometry {
                                commands.vertices.entry(vertices).or_insert_with(|| {
                                    native.pools[&pool].chunks[vertices.1].streams.clone()
                                });
                                commands
                                    .indices
                                    .entry(gpu.geometry.indices.0)
                                    .or_insert_with(|| {
                                        native.indices.chunks[gpu.geometry.indices.0].streams[0]
                                            .as_ref()
                                            .expect("index stream")
                                            .clone()
                                    });
                                }
                                commands.draws.push(RecordedDraw {
                                    bindings,
                                    reference: gpu.pipeline.reference(),
                                    vertices,
                                    indices: gpu.geometry.indices.0,
                                    first: gpu.geometry.indices.1,
                                    count,
                                    base: gpu.geometry.vertices.1,
                                });
                                last_material = Some(material);
                                drawn += 1;
                            }
                            Ok(())
                        };
                        if let Err(why) = submit() {
                            let why = match why {
                                PrepareError::Actor(why) => why,
                                PrepareError::Shared(why) => {
                                    scratch.sync(object);
                                    return Err(why);
                                }
                            };
                            scratch.restore(source_before);
                            let name = if descriptor.name == mesh.name {
                                mesh.name.clone()
                            } else {
                                format!("{}/{}", mesh.name, descriptor.name)
                            };
                            self.refuse_actor(
                                arena,
                                Some(group.mesh),
                                index,
                                descriptor.node,
                                &name,
                                why,
                            );
                        }
                    }
                }
            }
        }
        if !caster {
            // Failed binds must not replace the last submitted material's retained textures.
            if let Some(material) = last_material {
                let material = &self.materials[material];
                for (slot, index) in [material.diffuse, material.specular, material.normal]
                    .into_iter()
                    .enumerate()
                {
                    textures[binding(slot as u32, Stage::Pixel) as usize] =
                        self.textures[index].as_ref().map(|texture| &texture.view);
                }
            }
            pending_sources.apply(native, textures, inputs, &scratch, &poses.1, &mut object.retained);
            self.drawn.set(self.drawn.get() + drawn);
        }
        scratch.sync(object);
        native.scratch = scratch;
        native.binding_storage = binding_session.into_storage();
        native.frame_handles = frame_handles;
        prepared.clear();
        candidates.clear();
        native.prepared = prepared;
        native.candidates = candidates;
        Ok(commands)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn draw_slices_cover_native_order_without_empty_runs() {
        for total in [0, 1, 2, 3, 7, 32, 1109, 14475] {
            for threads in 1..=32 {
                let count = total.min(threads);
                let ranges: Vec<_> = (0..count)
                    .map(|index| draw_slice(total, count, index))
                    .collect();
                assert!(ranges.iter().all(|range| !range.is_empty()));
                assert_eq!(
                    ranges.into_iter().flatten().collect::<Vec<_>>(),
                    (0..total).collect::<Vec<_>>()
                );
            }
        }
    }

    /// Native packets fully establish state at each boundary. Splitting between arbitrary
    /// draws preserves MSAA samples, colour blending, depth, stencil and pooled geometry.
    #[test]
    #[ignore = "requires a GPU"]
    fn ordered_encoder_slices_keep_blend_depth_stencil_and_dynamic_bindings() {
        use shader_pipeline::{
            Buffer, Compiled, ScalarKind, Shader, TextureKind, VertexInput, gpu,
        };
        use viewer_foundation::{
            map::gpu_contract::{buffer, read_texture},
            parallel::{Spawn, Workers},
        };
        engine_executable::executable_tables::initialize_for_tests()
            .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
        let (_serial, device, queue) = viewer_foundation::map::test_support::gpu::device();
        let program = installed_content_core::pdxshader::program::read_with(b"slices.shader", &|path| {
            Some(if path == b"slices.shader" {
                b"Effect Draw { }".to_vec()
            } else {
                Vec::new()
            })
        })
        .unwrap();
        let shader = |wgsl: &str| Shader {
            wgsl: wgsl.into(),
            buffers: Vec::new(),
            textures: Vec::new(),
            samplers: Vec::new(),
            sampling_pairs: Vec::new(),
            inputs: Vec::new(),
            clock: Default::default(),
        };
        let mut vertex = shader(
            r#"
            struct Block { colour: vec4<f32>, shift: vec4<f32> }
            @group(0) @binding(0) var<uniform> block: Block;
            struct Out { @builtin(position) position: vec4<f32>, @location(0) colour: vec4<f32> }
            @vertex fn main(@location(0) p: vec3<f32>) -> Out {
                var out: Out;
                out.position = vec4<f32>(p + block.shift.xyz, 1.0);
                out.colour = block.colour;
                return out;
            }
        "#,
        );
        vertex.buffers.push(Buffer {
            binding: 0,
            size: 32,
            reads: std::iter::once(0..32).collect(),
        });
        vertex.inputs.push(VertexInput {
            kind: ScalarKind::Float,
            location: 0,
            semantic: "POSITION".into(),
            components: 3,
        });
        let mut pixel = shader(
            r#"
            @group(1) @binding(1) var image: texture_2d<f32>;
            @fragment fn main(@location(0) colour: vec4<f32>) -> @location(0) vec4<f32> {
                return colour * textureLoad(image, vec2<i32>(0), 0);
            }
        "#,
        );
        pixel.textures.push((1, TextureKind::D2));
        let compiled = Compiled { vertex, pixel };
        let views: Vec<_> = [[255, 255, 255, 255], [128, 255, 128, 255]]
            .into_iter()
            .map(|texel| {
                let texture = device.create_texture(&wgpu::TextureDescriptor {
                    label: Some("slice texture"),
                    size: wgpu::Extent3d {
                        width: 1,
                        height: 1,
                        depth_or_array_layers: 1,
                    },
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: wgpu::TextureDimension::D2,
                    format: wgpu::TextureFormat::Rgba8Unorm,
                    usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
                    view_formats: &[],
                });
                queue.write_texture(
                    texture.as_image_copy(),
                    &texel,
                    wgpu::TexelCopyBufferLayout::default(),
                    texture.size(),
                );
                texture.create_view(&Default::default())
            })
            .collect();
        // Distinct vertex/index chunks, with nonzero first_index and base_vertex.
        let vertices: Vec<_> = [0.0_f32, 0.125]
            .into_iter()
            .map(|shift| {
                let positions = [
                    [0.; 3],
                    [-0.9 + shift, -0.8, 0.],
                    [0.8, -0.7 + shift, 0.],
                    [-0.1, 0.9, 0.],
                ];
                buffer(
                    &device,
                    "slice vertices",
                    wgpu::BufferUsages::VERTEX,
                    bytemuck::cast_slice(&positions),
                )
                .unwrap()
            })
            .collect();
        let indices: Vec<_> = [[0_u32, 0, 1, 2], [0_u32, 2, 1, 0]]
            .into_iter()
            .map(|data| {
                buffer(
                    &device,
                    "slice indices",
                    wgpu::BufferUsages::INDEX,
                    bytemuck::cast_slice(&data),
                )
                .unwrap()
            })
            .collect();
        struct Pool;
        impl Spawn for Pool {
            fn try_spawn(
                &self,
                job: Box<dyn FnOnce() + Send>,
            ) -> Result<(), Box<dyn FnOnce() + Send>> {
                std::thread::spawn(job);
                Ok(())
            }
            fn threads(&self) -> usize {
                3
            }
        }
        let assist =
            viewer_foundation::frame_assist::FrameAssist::with_helpers_for_tests(Arc::new(Pool), 1)
                .unwrap();
        let workers = Workers::new(Arc::new(assist));
        struct Busy;
        impl Spawn for Busy {
            fn try_spawn(
                &self,
                job: Box<dyn FnOnce() + Send>,
            ) -> Result<(), Box<dyn FnOnce() + Send>> {
                Err(job)
            }
            fn threads(&self) -> usize {
                3
            }
        }
        let busy = Workers::new(Arc::new(Busy));
        for samples in [1, 4] {
            let arena = gpu::Arena::new(&device);
            let state = gpu::State::authored(&program, &program.techniques[0]).unwrap();
            let pipelines: Vec<_> = (0..3)
                .map(|mode| {
                    let mut pass = gpu::Pass::packed(&compiled, &state, SCENE_FORMAT, samples);
                    pass.primitive.cull_mode = None;
                    let face = wgpu::StencilFaceState {
                        compare: if mode == 0 {
                            wgpu::CompareFunction::Always
                        } else {
                            wgpu::CompareFunction::Equal
                        },
                        pass_op: if mode == 0 {
                            wgpu::StencilOperation::Replace
                        } else {
                            wgpu::StencilOperation::Keep
                        },
                        ..Default::default()
                    };
                    pass.depth_stencil = Some(wgpu::DepthStencilState {
                        format: DEPTH_FORMAT,
                        depth_write_enabled: Some(mode != 1),
                        depth_compare: Some(wgpu::CompareFunction::LessEqual),
                        stencil: wgpu::StencilState {
                            front: face,
                            back: face,
                            read_mask: 0xff,
                            write_mask: 0xff,
                        },
                        bias: Default::default(),
                    });
                    if mode == 1 {
                        pass.targets[0].as_mut().unwrap().blend =
                            Some(wgpu::BlendState::ALPHA_BLENDING);
                    }
                    gpu::Pipeline::for_pass(
                        &device,
                        &compiled,
                        &program,
                        &program.techniques[0],
                        &state,
                        &pass,
                    )
                    .unwrap()
                })
                .collect();
            // Cross list boundaries and split the first state's consecutive run.
            let mut lists = [
                DrawList::default(),
                DrawList::default(),
                DrawList::default(),
            ];
            for list in &mut lists {
                for (index, vertex) in vertices.iter().enumerate() {
                    list.vertices
                        .insert(((12, false), index), [Some(vertex.clone()), None]);
                    list.indices.insert(index, indices[index].clone());
                }
            }
            for index in 0..19 {
                let mode = if index == 0 {
                    0
                } else if index < 17 {
                    1
                } else {
                    2
                };
                let block = [
                    index as f32 / 32.,
                    0.5,
                    0.75,
                    0.25,
                    (index % 3) as f32 / 16.,
                    0.,
                    if index == 17 {
                        0.9
                    } else if index == 18 {
                        0.25
                    } else {
                        0.5
                    },
                    0.,
                ];
                let bindings = pipelines[mode]
                    .bind_with(
                        &arena,
                        |_| Some(bytemuck::cast_slice(&block)),
                        |_| Some(&views[index % views.len()]),
                    )
                    .unwrap();
                lists[usize::from(index >= 11) * 2]
                    .draws
                    .push(RecordedDraw {
                        bindings,
                        reference: if index == 18 { 2 } else { 1 },
                        vertices: ((12, false), index % 2),
                        indices: index % 2,
                        first: 1,
                        count: 3,
                        base: 1,
                    });
            }
            arena.flush(&queue);
            let render = |workers: Option<(&Workers, usize)>, empty: bool| {
                let targets = Targets::new(&device, [16, 16], [16, 16], samples);
                let mut encoder = device.create_command_encoder(&Default::default());
                {
                    let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                        color_attachments: &[Some(
                            targets.scene_attachment(wgpu::LoadOp::Clear(wgpu::Color::BLACK)),
                        )],
                        depth_stencil_attachment: Some(
                            targets.depth_attachment(wgpu::LoadOp::Clear(1.)),
                        ),
                        ..Default::default()
                    });
                    if workers.is_none() && !empty {
                        for list in &lists {
                            list.record(&mut pass);
                        }
                    }
                }
                // Like Scene, finish the preceding clear beside the slices when a helper
                // takes it, but always submit it first. Busy builders finish it here instead.
                let slices = workers.map(|(workers, helpers)| {
                    workers.with_admitted(workers.threads(), |admitted| {
                        admitted.wait_for_helpers(helpers);
                        let lists = if empty {
                            Vec::new()
                        } else {
                            lists.iter().collect()
                        };
                        DrawList::encode(&lists, &device, &targets, admitted, &mut encoder)
                    })
                });
                let mut commands = slices.unwrap_or_default();
                commands.push(encoder.finish());
                queue.submit(commands);
                read_texture(&device, &queue, targets.scene_texture(), 8).unwrap()
            };
            for empty in [false, true] {
                let reference = render(None, empty);
                assert_eq!(
                    reference
                        .chunks_exact(8)
                        .any(|pixel| pixel[..6].iter().any(|byte| *byte != 0)),
                    !empty
                );
                assert_eq!(
                    render(Some((&Workers::default(), 0)), empty),
                    reference,
                    "serial {samples}x MSAA empty={empty}"
                );
                assert_eq!(
                    render(Some((&workers, workers.threads() - 1)), empty),
                    reference,
                    "borrowed {samples}x MSAA empty={empty}"
                );
                assert_eq!(
                    render(Some((&busy, 0)), empty),
                    reference,
                    "busy {samples}x MSAA empty={empty}"
                );
            }
        }
    }

    /// A part whose mesh pass fell back from a visual mod's replacement draws with the
    /// recorded technique: its compiled program, vertex layouts (the recorded POSITION and
    /// TEXCOORD0, where the replacement's COLOR0 has none), state (the recorded blend) and
    /// retained program, never the technique it asked for.
    #[test]
    fn a_part_draws_with_the_technique_its_pass_fell_back_to() {
        let (pass, asked, recorded, geometry) = crate::map::mesh_shader::tests::fallen_back_pass();
        assert!(!Arc::ptr_eq(&asked.effect, &recorded));
        let (compiled, retained) = part_program(&pass, &geometry).unwrap();
        assert!(Arc::ptr_eq(&retained.selection().effect, &recorded));
        assert_eq!(compiled.vertex.inputs[1].semantic, "TEXCOORD0");
        let layouts = retained.layouts();
        assert_eq!(layouts[0].attributes.len(), 2);
        assert_eq!(layouts[0].stride, geometry.stride);
        assert!(retained.state().blend.is_some());
    }

    #[test]
    fn forced_actor_keeps_all_materials_in_alternate_and_caster() {
        for blended in [false, true] {
            assert!(!Queue::Regular.accepts(blended, true));
            assert!(Queue::Alternate.accepts(blended, true));
            assert!(Queue::Caster.accepts(blended, true));
        }
        assert!(Queue::Regular.accepts(false, false));
        assert!(!Queue::Alternate.accepts(false, false));
    }

    #[test]
    fn shadow_alpha_state_can_admit_an_alternate_material() {
        use installed_content_core::pdxshader::BlendState;
        let opaque = BlendState::default();
        let cutout = BlendState {
            enable: false,
            alpha_test: true,
            ..opaque.clone()
        };
        let opaque = BlendState {
            enable: false,
            alpha_test: false,
            ..opaque
        };
        //A textured lit cutout never reaches the shadow-state alternate override.
        assert_eq!(material_flags(&cutout, &cutout, true), (true, false));
        // The same shadow state selects the alternate queue when the lit state had no textured test.
        assert_eq!(material_flags(&opaque, &cutout, true), (true, true));
        assert_eq!(material_flags(&cutout, &cutout, false), (false, true));
        assert_eq!(material_flags(&opaque, &opaque, true), (false, false));
    }

    #[test]
    fn object_upload_retains_pixel_snapshot_and_cpu_userdata_independently() {
        let mut object = ObjectConstants::default();
        let mut scratch = plans::Scratch::default();
        scratch.begin(&object);
        scratch.update(&[[2.; 4]; 4], None);
        assert!(scratch.pixel().is_none());
        scratch.update(&[[3.; 4]; 4], Some([5.; 8]));
        scratch.update(&[[7.; 4]; 4], None);
        let (vertex, pixel) = (scratch.vertex(), scratch.pixel());
        let pixel = pixel.unwrap();
        assert_eq!(&pixel[..64], bytemuck::bytes_of(&[[3_f32; 4]; 4]));
        assert_eq!(&pixel[64..96], bytemuck::bytes_of(&[5_f32; 8]));
        let retained = pixel.to_vec();
        assert_eq!(&vertex[..64], bytemuck::bytes_of(&[[7_f32; 4]; 4]));
        assert_eq!(&vertex[64..96], bytemuck::bytes_of(&[5_f32; 8]));
        // Starting another native invocation clears CPU scratch, not PS b1.
        scratch.sync(&mut object);
        object.begin();
        scratch.begin(&object);
        scratch.update(&[[11.; 4]; 4], None);
        let (vertex, pixel) = (scratch.vertex(), scratch.pixel());
        assert_eq!(pixel.unwrap(), retained);
        assert_eq!(&vertex[..64], bytemuck::bytes_of(&[[11_f32; 4]; 4]));
        assert!(vertex[64..].iter().all(|byte| *byte == 0));
        scratch.update(&[[13.; 4]; 4], Some([17.; 8]));
        let pixel = scratch.pixel().unwrap();
        assert_eq!(&pixel[..64], bytemuck::bytes_of(&[[13_f32; 4]; 4]));
        assert_eq!(&pixel[64..96], bytemuck::bytes_of(&[17_f32; 8]));
    }

    #[test]
    fn caller_basis_survives_roll_and_nonorthogonal_interpolation() {
        let basis = [[0.5, 0.25, 0.], [-0.75, 1., 0.125], [0., -0.5, 0.875]];
        let mut instance = ModelInstance {
            native_flags: 4,
            native_basis: basis,
            position: [11., 12., 13.],
            rotation: f32::NAN,
            pitch: f32::NAN,
            roll: 0.4,
            scale: 2.,
            ..Default::default()
        };
        let world = instance.native_transform().unwrap();
        assert_eq!(world.translation, [11., 12., 13.]);
        assert_eq!(
            world.basis,
            [[1., 0.5, 0.], [-1.5, 2., 0.25], [0., -1., 1.75]]
        );
        instance.native_flags = 0;
        assert!(instance.native_transform().is_err());
    }

    #[test]
    fn descriptor_lods_keep_source_order_and_append_unnumbered_records() {
        assert_eq!(
            lods([
                (Some(2), 'a'),
                (None, 'b'),
                (Some(0), 'c'),
                (Some(-1), 'd'),
                (Some(2), 'e')
            ])
            .unwrap(),
            vec![vec!['c'], vec![], vec!['a', 'e'], vec!['b', 'd']]
        );
        assert_eq!(
            lods([(None, 'a'), (None, 'b')]).unwrap(),
            vec![vec!['a', 'b']]
        );
        assert!(lods::<char>([]).unwrap().is_empty());
        assert!(
            lods([(Some(-2), 'a')])
                .unwrap_err()
                .to_string()
                .contains("negative LOD -2")
        );
    }

    #[test]
    fn lod_uses_actor_distance_squared_and_native_last_level_fallback() {
        let mut descriptor = Descriptor {
            name: "synthetic_lods".into(),
            node: 0,
            distances: vec![4., 100.],
            lods: vec![vec![], vec![], vec![]],
        };
        // Two actors in one old cull group can select different levels.
        assert_eq!(descriptor.lod([1., 0., 0.], [0.; 3]).unwrap(), Some(2));
        assert_eq!(descriptor.lod([2., 0., 0.], [0.; 3]).unwrap(), Some(0));
        assert_eq!(descriptor.lod([6., 8., 0.], [0.; 3]).unwrap(), Some(1));
        // Walk backwards without sorting or clamping the authored ladder.
        descriptor.distances = vec![0., 100., 25.];
        assert_eq!(descriptor.lod([6., 0., 0.], [0.; 3]).unwrap(), Some(2));
        descriptor.distances.clear();
        assert_eq!(descriptor.lod([0.; 3], [0.; 3]).unwrap(), Some(2));
        descriptor.distances = vec![0.; 4];
        assert!(
            descriptor
                .lod([0.; 3], [0.; 3])
                .unwrap_err()
                .to_string()
                .contains("LOD 3")
        );
        descriptor.lods.clear();
        assert_eq!(descriptor.lod([0.; 3], [0.; 3]).unwrap(), None);
    }

    #[test]
    fn native_userdata_is_caller_specific_and_land_motion_is_independent_of_clip() {
        let source = mesh_pose::Source {
            pdxmesh: true,
            entity_locators: Vec::new(),
            binary_locators: Vec::new(),
            skeleton: Vec::new(),
            mesh_scale: 0.5,
            entity_scale: 1.,
            frozen: None,
            game_data: Some(Ok(mesh::EntityGameData {
                atlas_size: [8, 3],
                texture_anim_speed: 0.75,
            })),
        };
        let mut instance = ModelInstance {
            native_flags: 1,
            ..Default::default()
        };
        assert_eq!(
            plan_tail::Caller::select("city sprawl").evaluate(&source, instance, 1.).unwrap(),
            None
        );
        assert_eq!(
            plan_tail::Caller::select("unit models").evaluate(&source, instance, 1.).unwrap(),
            Some([3., 0., 8., 3., 0., 0., 0., 0.])
        );
        instance.native_flags = 3;
        assert_eq!(
            plan_tail::Caller::select("unit models").evaluate(&source, instance, 1.)
                .unwrap()
                .unwrap()[4],
            0.75
        );
        assert_eq!(
            plan_tail::Caller::select("map buildings").evaluate(&source, instance, 1.).unwrap(),
            Some([0., 0., 1., 1., 1., 0., 0., 0.])
        );
        instance.train_clip = [2., 3., 5., 7.];
        assert_eq!(
            plan_tail::Caller::select("supply trains").evaluate(&source, instance, 0.25).unwrap(),
            Some([1., 1., 1., 0.25, 2., 3., 5., 7.])
        );
    }

    #[test]
    fn actor_matrix_keeps_affine_w_and_separate_import_scale() {
        // The game scales the twelve basis floats, never the homogeneous row.
        let actor = mesh::Transform {
            basis: [[0., 0., 2.], [0., 2., 0.], [-2., 0., 0.]],
            translation: [7., 11., 13.],
        };
        let packed = matrix(&actor);
        let imported = [0.5, 1., 1.5, 1.];
        let transformed: [f32; 4] = std::array::from_fn(|row| {
            (0..4)
                .map(|column| packed[column][row] * imported[column])
                .sum()
        });
        assert_eq!(transformed, [4., 13., 14., 1.]);
        assert_eq!(actor.transform_point([0.5, 1., 1.5]), [4., 13., 14.]);
        assert_eq!(packed[3], [7., 11., 13., 1.]);
    }
}

#[cfg(test)]
mod dump_tests;
