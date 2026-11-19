pub use super::batch_views::Views;
use super::{
    Arena, Bindings, Draw, GroupResources, GroupStamp, Pipeline, ResourceHandle, Slice,
    batch_shader,
};
use crate::{Compiled, vertices::Vertices};
use std::{collections::HashMap, sync::Arc};

pub(super) struct Program {
    pub(super) shader: batch_shader::Program,
    stride: usize,
    inputs: Vec<(u32, usize, usize)>,
}

pub enum Packet {
    Native { pipeline: Arc<Pipeline>, draw: Draw },
    Batched(Data),
}

impl Packet {
    /// The data of a batched packet.
    fn batched(&self) -> Option<&Data> {
        match self {
            Self::Native { .. } => None,
            Self::Batched(data) => Some(data),
        }
    }
}

/// A draw prepared for a batch. Its blocks are shared, so a caller that keeps the packet it
/// prepared and hands the batch a copy of it pays no copy for the keeping
/// ([`Data::rebound`]).
#[derive(Clone)]
pub struct Data {
    pipeline: Arc<Pipeline>,
    program: Arc<Program>,
    constants: Arc<Vec<(u32, Vec<u8>)>>,
    textures: Arc<Vec<super::TextureBinding>>,
    vertices: Arc<Vec<f32>>,
    count: usize,
    source_draws: u64,
}

/// The rows block a draw of instanced rows selects with a dynamic offset, and its bytes.
pub(super) const ROWS_BINDING: u32 = batch_shader::META_BINDING;
pub(super) const ROWS_BYTES: usize = batch_shader::ROWS_BYTES;
/// Where in a rows block the instanced stream's own row stands.
pub(super) const STREAM_ROW: usize = batch_shader::STREAM_ROW;
/// The instanced stream's words and each instance's first vertex in them.
pub(super) const STREAM_BINDINGS: [u32; 2] = [
    batch_shader::STREAM_BINDING,
    batch_shader::STREAM_BASE_BINDING,
];

/// The program of a pass that draws its constant rows per instance, or none with the reason the
/// pass draws as it is.
pub(super) fn instanced(
    device: &wgpu::Device,
    compiled: &Compiled,
    pass: &super::Pass,
    label: &str,
) -> Option<Arc<batch_shader::Instanced>> {
    let mask = pass.instanced;
    let retain = |reason: String| {
        eprintln!("native per-draw constants retained {label}: {reason}");
        None
    };
    let shader = match batch_shader::Instanced::new(compiled, mask, pass) {
        Ok(shader) => shader,
        Err(reason) => return retain(reason),
    };
    let limits = device.limits();
    let rows = |stage: &crate::Shader| {
        stage
            .buffers
            .iter()
            .filter(|buffer| buffer.binding < u32::BITS && mask & (1 << buffer.binding) != 0)
            .count()
    };
    let streams = usize::from(shader.stream.is_some()) * 2;
    if rows(&compiled.vertex) + streams > limits.max_storage_buffers_per_shader_stage as usize
        || rows(&compiled.pixel) > limits.max_storage_buffers_per_shader_stage as usize
    {
        return retain(
            "constant buffers read per instance exceed the device's storage limits".into(),
        );
    }
    if shader.pixel.is_some() && shader.varying >= limits.max_inter_stage_shader_variables {
        return retain("carrying the instance index exceeds the device's stage interface".into());
    }
    Some(Arc::new(shader))
}

/// The rows block an instanced program reads: the draw's base row for each of its per-instance
/// bindings, selected with a dynamic offset.
pub(super) fn rows_layout() -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding: batch_shader::META_BINDING,
        visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
        ty: wgpu::BindingType::Buffer {
            ty: wgpu::BufferBindingType::Uniform,
            has_dynamic_offset: true,
            min_binding_size: std::num::NonZeroU64::new(batch_shader::ROWS_BYTES as u64),
        },
        count: None,
    }
}

/// The instanced stream's own words and each instance's first vertex in them, both read whole.
pub(super) fn stream_layouts() -> [wgpu::BindGroupLayoutEntry; 2] {
    [
        batch_shader::STREAM_BINDING,
        batch_shader::STREAM_BASE_BINDING,
    ]
    .map(|binding| wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::VERTEX,
        ty: wgpu::BindingType::Buffer {
            ty: wgpu::BufferBindingType::Storage { read_only: true },
            has_dynamic_offset: false,
            min_binding_size: std::num::NonZeroU64::new(4),
        },
        count: None,
    })
}

pub(super) fn metadata_layout() -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding: batch_shader::META_BINDING,
        visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
        ty: wgpu::BindingType::Buffer {
            ty: wgpu::BufferBindingType::Storage { read_only: true },
            has_dynamic_offset: false,
            min_binding_size: std::num::NonZeroU64::new((batch_shader::META_WORDS * 4) as u64),
        },
        count: None,
    }
}

impl Program {
    pub(super) fn translate(
        device: &wgpu::Device,
        compiled: &Compiled,
        pass: &super::Pass,
        label: &str,
    ) -> Result<Option<Self>, String> {
        let retain = |reason: String| {
            eprintln!("GUI native draw retained {label}: {reason}");
            Ok(None)
        };
        match batch_shader::unsupported(compiled, pass) {
            Ok(None) => {}
            Ok(Some(reason)) | Err(reason) => return retain(reason),
        }
        let limits = device.limits();
        if [0, 1].into_iter().any(|stage| {
            [compiled.vertex.buffers.len(), compiled.pixel.buffers.len()][stage] + 1
                > limits.max_storage_buffers_per_shader_stage as usize
        }) {
            return retain("constant buffers exceed the device's batch storage limits".into());
        }
        let shader = match batch_shader::Program::new(compiled) {
            Ok(shader) => shader,
            Err(reason) => return retain(format!("GUI batch translation: {reason}")),
        };
        if shader.input >= limits.max_vertex_attributes
            || shader.varying >= limits.max_inter_stage_shader_variables
        {
            return retain("interfaces exceed the device's batch limits".into());
        }
        let mut offset = 0;
        let inputs = compiled
            .vertex
            .inputs
            .iter()
            .map(|input| {
                let row = (input.location, offset, input.components as usize);
                offset += input.components as usize;
                row
            })
            .collect();
        Ok(Some(Self {
            shader,
            stride: pass.vertices[0].stride as usize / 4 + 1,
            inputs,
        }))
    }
}

impl Pipeline {
    pub(crate) fn packet<'a>(
        self: &Arc<Self>,
        arena: &Arena,
        constants: impl Fn(u32) -> Option<&'a [u8]>,
        textures: impl Fn(u32) -> Option<&'a super::TextureBinding>,
        vertices: &Vertices<'_>,
    ) -> Result<Packet, String> {
        let views = |binding| textures(binding).map(|bound| self.sampled_texture(binding, bound));
        let mask = if self.fallback.is_some() {
            self.unfilterable(arena.device(), &views)
        } else {
            0
        };
        if mask != 0 {
            return self
                .variant(arena.device(), mask)?
                .packet(arena, constants, textures, vertices);
        }
        let Some(program) = self.batch.as_ref().cloned() else {
            return Ok(Packet::Native {
                pipeline: Arc::clone(self),
                draw: self.prepare_with(arena, |key| constants(key), |key| views(key), vertices)?,
            });
        };
        let constants = program
            .shader
            .buffers
            .iter()
            .map(|(binding, stride)| {
                constants(*binding)
                    .map(|bytes| {
                        (
                            *binding,
                            bytes[..bytes.len().min(*stride as usize)].to_vec(),
                        )
                    })
                    .ok_or_else(|| {
                        format!("native constant buffer {} has not been bound", binding / 2)
                    })
            })
            .collect::<Result<Vec<_>, _>>()?;
        let textures = self
            .textures
            .iter()
            .map(|(binding, _)| {
                textures(*binding).cloned().ok_or_else(|| {
                    format!("native texture slot {} has not been bound", binding / 2)
                })
            })
            .collect::<Result<Vec<_>, _>>()?;
        for ((binding, _), view) in self.textures.iter().zip(&textures) {
            let texture = view.texture();
            if texture.dimension() != wgpu::TextureDimension::D2
                || (texture.depth_or_array_layers() != 1 && view.array.is_none())
                || texture.sample_count() != 1
            {
                return Err(format!(
                    "{}: texture slot {} is not a single-sampled 2D image",
                    self.label,
                    binding / 2
                ));
            }
            if !matches!(
                view.format
                    .sample_type(None, Some(arena.device().features())),
                Some(wgpu::TextureSampleType::Float { .. })
            ) {
                return Err(format!(
                    "{}: texture slot {} does not supply floating-point samples",
                    self.label,
                    binding / 2
                ));
            }
        }
        let mut packed = Vec::new();
        self.packed_inputs[vertices.declaration as usize].pack_into(vertices, &mut packed)?;
        Ok(Packet::Batched(Data {
            pipeline: Arc::clone(self),
            program,
            constants: Arc::new(constants),
            textures: Arc::new(textures),
            vertices: Arc::new(packed),
            count: vertices.count,
            source_draws: 1,
        }))
    }
}

impl Data {
    /// Binds `bytes` at every binding of `at`, as the program reads them: a draw whose window
    /// moved over the map binds the same blocks with a new world transform, and nothing else
    /// about it changed ([`crate::gpu::Arena`] stages what the batch then holds).
    pub fn rebound(&mut self, at: &[u32], bytes: &[u8]) {
        for ((binding, held), (_, stride)) in Arc::make_mut(&mut self.constants)
            .iter_mut()
            .zip(&self.program.shader.buffers)
        {
            if at.contains(binding) {
                held.clear();
                held.extend_from_slice(&bytes[..bytes.len().min(*stride as usize)]);
            }
        }
    }

    /// What the packet uploads: each binding's constants, the packed vertices and their count.
    #[cfg(any(test, feature = "test-support"))]
    pub fn proof(&self) -> (Vec<(u32, Vec<u8>)>, Vec<f32>, usize) {
        ((*self.constants).clone(), (*self.vertices).clone(), self.count)
    }
}

impl Packet {
    /// The data of a batched packet.
    #[cfg(any(test, feature = "test-support"))]
    pub fn data(&self) -> Option<&Data> {
        self.batched()
    }

    pub fn source_draws(&mut self, count: u64) {
        match self {
            Self::Native { .. } => {}
            Self::Batched(data) => data.source_draws = count,
        }
    }
}

pub struct Item {
    pub packet: Packet,
    pub scissor: [u32; 4],
}

struct Batch {
    source: Arc<Pipeline>,
    program: Arc<Program>,
    textures: Vec<wgpu::TextureView>,
    constants: Vec<Vec<u8>>,
    metadata: Vec<u32>,
    vertices: Vec<f32>,
    rows: usize,
    source_draws: u64,
}

enum Ordered {
    Native(Arc<Pipeline>, Draw, [u32; 4]),
    Batch(Batch),
}

impl Batch {
    fn new(data: &Data, textures: Vec<wgpu::TextureView>) -> Self {
        Self {
            source: Arc::clone(&data.pipeline),
            program: Arc::clone(&data.program),
            textures,
            constants: vec![Vec::new(); data.constants.len()],
            metadata: Vec::new(),
            vertices: Vec::new(),
            rows: 0,
            source_draws: 0,
        }
    }

    fn fits(&self, data: &Data, limits: &wgpu::Limits) -> bool {
        let rows = self.rows + 1;
        let vertices = if data.pipeline.topology == wgpu::PrimitiveTopology::TriangleStrip {
            data.count.saturating_sub(2).saturating_mul(3)
        } else {
            data.count
        };
        self.program.shader.buffers.iter().all(|(_, stride)| {
            rows as u64 * u64::from(*stride) <= u64::from(limits.max_storage_buffer_binding_size)
        }) && rows as u64 * (batch_shader::META_WORDS * 4) as u64
            <= u64::from(limits.max_storage_buffer_binding_size)
            && (self.vertices.len() + vertices * self.program.stride) as u64 * 4
                <= limits.max_buffer_size
            && rows <= u32::MAX as usize
    }

    fn append(&mut self, data: &Data, layers: &[u32], scissor: [u32; 4]) {
        for ((target, (_, bytes)), (_, stride)) in self
            .constants
            .iter_mut()
            .zip(data.constants.iter())
            .zip(&self.program.shader.buffers)
        {
            let at = target.len();
            target.resize(at + *stride as usize, 0);
            let length = bytes.len().min(*stride as usize);
            target[at..at + length].copy_from_slice(&bytes[..length]);
        }
        let [x, y, width, height] = scissor;
        self.metadata
            .extend([x, y, x + width, y + height].map(|value| (value as f32).to_bits()));
        let at = self.metadata.len();
        self.metadata
            .resize(at + super::TEXTURE_BINDINGS + super::CONSTANT_BINDINGS, 0);
        for ((binding, _), layer) in data.pipeline.textures.iter().zip(layers) {
            self.metadata[at + *binding as usize] = *layer;
        }
        let native_stride = self.program.stride - 1;
        let mut append = |index: usize| {
            self.vertices.extend_from_slice(
                &data.vertices[index * native_stride..(index + 1) * native_stride],
            );
            self.vertices.push(f32::from_bits(self.rows as u32));
        };
        append_indices(data.pipeline.topology, data.count, &mut append);
        self.rows += 1;
        self.source_draws += data.source_draws;
    }

    fn finish(self, arena: &Arena) -> Result<crate::stream::Submission, String> {
        let device = arena.device();
        let mut slices: Vec<Slice> = self
            .constants
            .iter()
            .zip(&self.program.shader.buffers)
            .map(|(bytes, (_, stride))| arena.storage(bytes, *stride as usize))
            .collect::<Result<_, _>>()?;
        let mut metadata = self.metadata;
        for (row, values) in metadata
            .chunks_exact_mut(batch_shader::META_WORDS)
            .enumerate()
        {
            for ((binding, stride), slice) in self.program.shader.buffers.iter().zip(&slices) {
                values[4 + super::TEXTURE_BINDINGS + *binding as usize] =
                    u32::try_from(slice.offset / u64::from(*stride) + row as u64)
                        .map_err(|_| "batch constant row exceeds u32")?;
            }
        }
        let meta = arena.storage(
            bytemuck::cast_slice(&metadata),
            batch_shader::META_WORDS * 4,
        )?;
        let meta_base = u32::try_from(meta.offset / (batch_shader::META_WORDS * 4) as u64)
            .map_err(|_| "batch metadata row exceeds u32")?;
        let mut vertices = self.vertices;
        for vertex in vertices.chunks_exact_mut(self.program.stride) {
            let id = vertex.last_mut().unwrap();
            *id = f32::from_bits(
                id.to_bits()
                    .checked_add(meta_base)
                    .ok_or("batch row exceeds u32")?,
            );
        }
        slices.push(meta);
        let frame = arena.frame();
        let mut groups = self.source.groups.lock().unwrap_or_else(|e| e.into_inner());
        groups.advance(frame);
        let constants = if let Some((_, group, used)) = groups
            .batch_constants
            .iter_mut()
            .find(|(buffers, _, _)| buffers.iter().eq(slices.iter().map(|slice| &slice.buffer)))
        {
            used.set(frame);
            group.clone()
        } else {
            let limit = u64::from(device.limits().max_storage_buffer_binding_size);
            let group = super::scoped(device, || {
                super::stats::create_bind_group(
                    device,
                    &wgpu::BindGroupDescriptor {
                        label: Some("native GUI batch chunks"),
                        layout: &self.source.layouts[0],
                        entries: &self
                            .program
                            .shader
                            .buffers
                            .iter()
                            .map(|(binding, _)| *binding)
                            .chain(std::iter::once(batch_shader::META_BINDING))
                            .zip(&slices)
                            .map(|(binding, slice)| wgpu::BindGroupEntry {
                                binding,
                                resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                                    buffer: &slice.buffer,
                                    offset: 0,
                                    size: std::num::NonZeroU64::new(slice.buffer.size().min(limit)),
                                }),
                            })
                            .collect::<Vec<_>>(),
                    },
                )
            })?;
            groups.batch_constants.push((
                slices.iter().map(|slice| slice.buffer.clone()).collect(),
                group.clone(),
                GroupStamp::new(frame),
            ));
            group
        };
        use std::hash::{Hash, Hasher};
        let mut hasher = std::hash::DefaultHasher::new();
        self.textures.hash(&mut hasher);
        let entries = groups.textures.entry(hasher.finish()).or_default();
        let textures = if let Some((_, group, used)) = entries
            .iter_mut()
            .find(|(views, _, _)| views == &self.textures)
        {
            used.set(frame);
            group.clone()
        } else {
            let group = super::scoped(device, || {
                super::stats::create_bind_group(
                    device,
                    &wgpu::BindGroupDescriptor {
                        label: Some("native GUI source layers"),
                        layout: &self.source.layouts[1],
                        entries: &self
                            .source
                            .textures
                            .iter()
                            .zip(&self.textures)
                            .map(|((binding, _), view)| wgpu::BindGroupEntry {
                                binding: *binding,
                                resource: wgpu::BindingResource::TextureView(view),
                            })
                            .collect::<Vec<_>>(),
                    },
                )
            })?;
            entries.push((self.textures, group.clone(), GroupStamp::new(frame)));
            group
        };
        drop(groups);
        let offsets = [0; super::BUFFERS];
        let draw = Draw {
            bindings: Bindings {
                resources: ResourceHandle::Inline(GroupResources {
                    pipeline: self.source.pipeline.clone(),
                    read_only: self.source.read_only,
                    groups: [constants, textures, self.source.samplers.clone()],
                }),
                offsets,
                count: 0,
                #[cfg(any(test, feature = "test-support"))]
                proof: Vec::new(),
            },
            vertices: arena.vertices(bytemuck::cast_slice(&vertices))?,
            count: (vertices.len() / self.program.stride) as u32,
        };
        Ok(crate::stream::Submission {
            pipeline: self.source,
            draw,
        })
    }
}

struct Node {
    bounds: [f32; 4],
    parent: Option<usize>,
    children: Option<[usize; 2]>,
    latest: usize,
}

struct OverlapIndex {
    nodes: Vec<Node>,
    leaves: Vec<Option<usize>>,
    root: Option<usize>,
}

impl OverlapIndex {
    fn new(bounds: &[Option<[f32; 4]>]) -> Self {
        let mut index = Self {
            nodes: Vec::new(),
            leaves: vec![None; bounds.len()],
            root: None,
        };
        let mut active: Vec<_> = bounds
            .iter()
            .enumerate()
            .filter_map(|(i, bounds)| bounds.map(|bounds| (i, bounds)))
            .collect();
        if !active.is_empty() {
            index.root = Some(index.build(&mut active, None));
        }
        index
    }

    fn build(&mut self, entries: &mut [(usize, [f32; 4])], parent: Option<usize>) -> usize {
        let bounds = entries
            .iter()
            .map(|(_, bounds)| *bounds)
            .reduce(|a, b| {
                [
                    a[0].min(b[0]),
                    a[1].min(b[1]),
                    a[2].max(b[2]),
                    a[3].max(b[3]),
                ]
            })
            .unwrap();
        let at = self.nodes.len();
        self.nodes.push(Node {
            bounds,
            parent,
            children: None,
            latest: 0,
        });
        if entries.len() == 1 {
            self.leaves[entries[0].0] = Some(at);
        } else {
            let axis = usize::from(bounds[3] - bounds[1] > bounds[2] - bounds[0]);
            let mid = entries.len() / 2;
            entries.select_nth_unstable_by(mid, |(_, a), (_, b)| {
                (a[axis] + a[axis + 2]).total_cmp(&(b[axis] + b[axis + 2]))
            });
            let (a, b) = entries.split_at_mut(mid);
            let left = self.build(a, Some(at));
            let right = self.build(b, Some(at));
            self.nodes[at].children = Some([left, right]);
        }
        at
    }

    fn insert(&mut self, item: usize, batch: usize) {
        let mut at = self.leaves[item];
        while let Some(index) = at {
            let node = &mut self.nodes[index];
            if node.latest >= batch + 1 {
                break;
            }
            node.latest = batch + 1;
            at = node.parent;
        }
    }

    fn blocked(&self, candidate: usize, bounds: [f32; 4]) -> bool {
        fn visit(index: &OverlapIndex, at: usize, candidate: usize, bounds: [f32; 4]) -> bool {
            let node = &index.nodes[at];
            if node.latest <= candidate + 1 || !overlaps(node.bounds, bounds) {
                return false;
            }
            node.children.is_none_or(|[a, b]| {
                visit(index, a, candidate, bounds) || visit(index, b, candidate, bounds)
            })
        }
        self.root
            .is_some_and(|root| visit(self, root, candidate, bounds))
    }
}

fn bounds(item: &Item, size: [u32; 2], scratch: &mut Vec<(f64, f64)>) -> Option<[f32; 4]> {
    let data = item.packet.batched()?;
    let bounds = data
        .program
        .shader
        .position
        .as_ref()
        .and_then(|plan| {
            plan.bounds(
                &data.constants,
                &data.program.inputs,
                &data.vertices,
                data.program.stride - 1,
                size,
                scratch,
            )
        })
        .unwrap_or([0.0, 0.0, size[0] as f32, size[1] as f32]);
    let [x, y, width, height] = item.scissor;
    let bounds = [
        bounds[0].max(x as f32),
        bounds[1].max(y as f32),
        bounds[2].min((x + width) as f32),
        bounds[3].min((y + height) as f32),
    ];
    (bounds[2] > bounds[0] && bounds[3] > bounds[1]).then_some(bounds)
}

pub fn prepare(
    items: Vec<Item>,
    views: &mut Views,
    arena: &Arena,
    size: [u32; 2],
    mut refuse: impl FnMut(String, u64),
) -> Vec<(crate::stream::Submission, Option<[u32; 4]>)> {
    let limits = arena.device().limits();
    let mut scratch = Vec::new();
    let bounds: Vec<_> = items
        .iter()
        .map(|item| bounds(item, size, &mut scratch))
        .collect();
    let mut failed = vec![false; items.len()];
    for (index, (item, bounds)) in items.iter().zip(&bounds).enumerate() {
        if bounds.is_some()
            && let Some(data) = item.packet.batched()
        {
            for view in data.textures.iter() {
                let result = views.reserve(arena.device(), view);
                if let Err(reason) = result {
                    refuse(
                        format!("{}: {reason}", data.pipeline.label),
                        data.source_draws,
                    );
                    failed[index] = true;
                    break;
                }
            }
        }
    }
    let mut ordered = Vec::<Ordered>::new();
    let mut previous = HashMap::<u64, Vec<usize>>::new();
    let mut index = OverlapIndex::new(&bounds);
    for (item_index, (item, bounds)) in items.into_iter().zip(bounds).enumerate() {
        if failed[item_index] {
            continue;
        }
        match item.packet {
            Packet::Native { pipeline, draw } => {
                ordered.push(Ordered::Native(pipeline, draw, item.scissor));
                previous.clear();
            }
            packet => {
                let data = packet.batched().expect("a native packet is matched above");
                let Some(bounds) = bounds else {
                    continue;
                };
                let texture_count = data.textures.len();
                let mut textures = [None; super::TEXTURES];
                let mut layers = [0; super::TEXTURES];
                for (index, view) in data.textures.iter().enumerate() {
                    let (page, layer) = views.get(view);
                    textures[index] = Some(page);
                    layers[index] = layer;
                }
                let textures = &textures[..texture_count];
                use std::hash::{Hash, Hasher};
                let open = ordered.len().checked_sub(1).filter(|candidate| {
                    matches!(&ordered[*candidate], Ordered::Batch(batch)
                        if Arc::ptr_eq(&batch.source.core, &data.pipeline.core)
                        && batch.textures.iter().map(Some).eq(textures.iter().copied())
                        && batch.fits(&data, &limits))
                });
                let at = if let Some(at) = open {
                    at
                } else {
                    let mut hasher = std::hash::DefaultHasher::new();
                    (Arc::as_ptr(&data.pipeline.core) as usize, textures).hash(&mut hasher);
                    let entries = previous.entry(hasher.finish()).or_default();
                    entries
                        .iter()
                        .copied()
                        .rev()
                        .find(|candidate| {
                            matches!(&ordered[*candidate], Ordered::Batch(batch)
                        if Arc::ptr_eq(&batch.source.core, &data.pipeline.core)
                        && batch.textures.iter().map(Some).eq(textures.iter().copied())
                        && batch.fits(&data, &limits) && !index.blocked(*candidate, bounds))
                        })
                        .unwrap_or_else(|| {
                            let at = ordered.len();
                            ordered.push(Ordered::Batch(Batch::new(
                                &data,
                                textures
                                    .iter()
                                    .map(|view| (*view).unwrap().clone())
                                    .collect(),
                            )));
                            entries.push(at);
                            at
                        })
                };
                let Ordered::Batch(batch) = &mut ordered[at] else {
                    unreachable!()
                };
                if !batch.fits(&data, &limits) {
                    refuse(
                        format!(
                            "{}: one GUI draw exceeds the device's batch buffer limits",
                            data.pipeline.label
                        ),
                        data.source_draws,
                    );
                    continue;
                }
                batch.append(data, &layers[..texture_count], item.scissor);
                index.insert(item_index, at);
            }
        }
    }
    ordered
        .into_iter()
        .filter_map(|entry| match entry {
            Ordered::Native(pipeline, draw, scissor) => {
                Some((crate::stream::Submission { pipeline, draw }, Some(scissor)))
            }
            Ordered::Batch(batch) => {
                let count = batch.source_draws;
                let label = batch.source.label.clone();
                match batch.finish(arena) {
                    Ok(submission) => Some((submission, None)),
                    Err(reason) => {
                        refuse(format!("{label}: {reason}"), count);
                        None
                    }
                }
            }
        })
        .collect()
}

fn overlaps(a: [f32; 4], b: [f32; 4]) -> bool {
    a[0] < b[2] && b[0] < a[2] && a[1] < b[3] && b[1] < a[3]
}

fn append_indices(topology: wgpu::PrimitiveTopology, count: usize, mut append: impl FnMut(usize)) {
    if topology == wgpu::PrimitiveTopology::TriangleStrip {
        for index in 2..count {
            if index.is_multiple_of(2) {
                append(index - 2);
                append(index - 1);
                append(index);
            } else {
                append(index - 1);
                append(index - 2);
                append(index);
            }
        }
    } else {
        for index in 0..count - count % 3 {
            append(index);
        }
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn incomplete_lists_do_not_make_triangles_with_the_next_draw() {
        let mut indices = Vec::new();
        super::append_indices(wgpu::PrimitiveTopology::TriangleList, 4, |i| {
            indices.push(i)
        });
        super::append_indices(wgpu::PrimitiveTopology::TriangleList, 3, |i| {
            indices.push(i + 4)
        });
        assert_eq!(indices, [0, 1, 2, 4, 5, 6]);
    }
}
