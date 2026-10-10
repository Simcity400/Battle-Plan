//! GPU resources for compiled game techniques. Inputs are native register bytes and vertex
//! semantics, so changing an effect's uniform names never changes what the engine uploads.

use super::{Buffer, Compiled};
use installed_content::pdxshader::{
    Address, DepthStencilState, RasterizerState,
    program::{Filter, Program, Sampler, Stage, Technique},
};
use std::{
    collections::{BTreeMap, HashMap},
    sync::{Arc, Mutex},
};
use wgpu;

#[path = "gpu/arena.rs"]
mod arena;
pub use arena::{Arena, Slice};

pub struct Pipeline {
    pub pipeline: wgpu::RenderPipeline,
    label: String,
    null_cube: Option<wgpu::TextureView>,
    /// Constant and texture layouts; the samplers are fixed per pipeline.
    layouts: [wgpu::BindGroupLayout; 2],
    samplers: wgpu::BindGroup,
    /// Ascending binding order, the order of the constant group's dynamic offsets.
    buffers: Vec<Buffer>,
    textures: Vec<(u32, super::TextureKind)>,
    /// Texture bindings with a filtering image/sampler pair.
    filtered: Vec<u32>,
    fallback: Option<Fallback>,
    inputs: Vec<super::VertexInput>,
    groups: Mutex<Groups>,
}

/// Float formats are known only when views are bound; keep variants by native binding mask.
struct Fallback {
    compiled: Compiled,
    effect: String,
    technique: Technique,
    state: State,
    pass: Pass,
    variants: Mutex<HashMap<u32, Cached>>,
}

/// Bind groups by resource identity: constants per arena buffer, textures per view list.
/// An entry unused for a whole frame is dropped, releasing the resources it holds.
#[derive(Default)]
struct Groups {
    frame: u64,
    constants: Vec<(wgpu::Buffer, wgpu::BindGroup, u64)>,
    /// By the hash of the views, so a hit clones none of them.
    textures: HashMap<u64, Vec<(Vec<wgpu::TextureView>, wgpu::BindGroup, u64)>>,
    /// The last texture entry used: consecutive draws usually share a material.
    last: Option<(u64, usize)>,
}

pub struct Draw {
    bindings: Bindings,
    vertices: Slice,
    count: u32,
}

/// Owned vertex layouts allow any pass ABI, including separate and instanced buffers.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct VertexLayout {
    pub stride: u64,
    pub step: wgpu::VertexStepMode,
    pub attributes: Vec<wgpu::VertexAttribute>,
}

/// Complete immutable pass state, part of the device cache key. Dynamic viewport/scissor,
/// stencil reference and blend constants remain the caller's render-pass commands.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Pass {
    pub vertices: Vec<VertexLayout>,
    pub primitive: wgpu::PrimitiveState,
    pub depth_stencil: Option<wgpu::DepthStencilState>,
    pub multisample: wgpu::MultisampleState,
    pub targets: Vec<Option<wgpu::ColorTargetState>>,
}

impl Pass {
    /// Tightly packed 32-bit input helper. Map callers can provide their native layouts directly.
    pub fn packed(
        compiled: &Compiled,
        state: &State,
        format: wgpu::TextureFormat,
        samples: u32,
    ) -> Self {
        let mut stride = 0;
        let attributes = compiled
            .vertex
            .inputs
            .iter()
            .map(|input| {
                use super::ScalarKind;
                let format = match (input.kind, input.components) {
                    (ScalarKind::Float, 1) => wgpu::VertexFormat::Float32,
                    (ScalarKind::Float, 2) => wgpu::VertexFormat::Float32x2,
                    (ScalarKind::Float, 3) => wgpu::VertexFormat::Float32x3,
                    (ScalarKind::Float, _) => wgpu::VertexFormat::Float32x4,
                    (ScalarKind::Sint, 1) => wgpu::VertexFormat::Sint32,
                    (ScalarKind::Sint, 2) => wgpu::VertexFormat::Sint32x2,
                    (ScalarKind::Sint, 3) => wgpu::VertexFormat::Sint32x3,
                    (ScalarKind::Sint, _) => wgpu::VertexFormat::Sint32x4,
                    (ScalarKind::Uint, 1) => wgpu::VertexFormat::Uint32,
                    (ScalarKind::Uint, 2) => wgpu::VertexFormat::Uint32x2,
                    (ScalarKind::Uint, 3) => wgpu::VertexFormat::Uint32x3,
                    (ScalarKind::Uint, _) => wgpu::VertexFormat::Uint32x4,
                };
                let attribute = wgpu::VertexAttribute {
                    format,
                    offset: stride,
                    shader_location: input.location,
                };
                stride += u64::from(input.components) * 4;
                attribute
            })
            .collect();
        Self {
            vertices: vec![VertexLayout {
                stride,
                step: wgpu::VertexStepMode::Vertex,
                attributes,
            }],
            primitive: Default::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState {
                count: samples,
                ..Default::default()
            },
            targets: vec![Some(wgpu::ColorTargetState {
                format,
                blend: state.blend,
                write_mask: state.write_mask,
            })],
        }
    }
}

/// Every `binding(slot, stage)` of a constant buffer or texture is below these: D3D11's
/// registers over both stages, which compilation enforces.
pub const CONSTANT_BINDINGS: usize = 2 * super::BUFFER_SLOTS as usize;
pub const TEXTURE_BINDINGS: usize = 2 * super::SAMPLER_SLOTS as usize;
const BUFFERS: usize = CONSTANT_BINDINGS;
const TEXTURES: usize = TEXTURE_BINDINGS;

pub struct Bindings {
    pipeline: wgpu::RenderPipeline,
    groups: [wgpu::BindGroup; 3],
    offsets: [u32; BUFFERS],
    count: usize,
}
impl Bindings {
    pub fn bind(&self, pass: &mut wgpu::RenderPass<'_>) {
        self.bind_after(pass, None);
    }

    /// Bind after `previous` of the same pipeline in the same pass, setting only the
    /// groups that differ. Each set merges all of a group's resources into the pass.
    pub fn bind_after(&self, pass: &mut wgpu::RenderPass<'_>, previous: Option<&Self>) {
        // The bound views may select a shader-filtered variant of the caller's pipeline.
        if previous.is_none_or(|previous| previous.pipeline != self.pipeline) {
            pass.set_pipeline(&self.pipeline);
        }
        let offsets = &self.offsets[..self.count];
        if previous.is_none_or(|previous| {
            previous.groups[0] != self.groups[0] || previous.offsets[..previous.count] != *offsets
        }) {
            pass.set_bind_group(0, &self.groups[0], offsets);
        }
        for index in 1..3 {
            if previous.is_none_or(|previous| previous.groups[index] != self.groups[index]) {
                pass.set_bind_group(index as u32, &self.groups[index], &[]);
            }
        }
    }
}

/// Resolved native state after applying this draw's binds. Program selection, blend selection
/// and each sampler bind are independent, especially for the null-fallback technique.
#[derive(Clone, Debug, PartialEq)]
pub struct State {
    pub samplers: BTreeMap<u32, Sampler>,
    pub blend: Option<wgpu::BlendState>,
    pub write_mask: wgpu::ColorWrites,
}

impl State {
    pub fn authored(program: &Program, technique: &Technique) -> Result<Self, String> {
        let (blend, write_mask) = blend(program, technique)?;
        let mut samplers = BTreeMap::new();
        for sampler in program
            .common
            .samplers
            .iter()
            .chain(&program.pixel.samplers)
        {
            samplers.insert(super::binding(sampler.index, Stage::Pixel), sampler.clone());
        }
        for sampler in &program.vertex.samplers {
            samplers.insert(
                super::binding(sampler.index, Stage::Vertex),
                sampler.clone(),
            );
        }
        Ok(Self {
            samplers,
            blend,
            write_mask,
        })
    }
}

struct PipelineKey {
    effect: uuid::Uuid,
    technique: usize,
    samplers: Vec<(u32, [u32; 10])>,
    blend: Option<wgpu::BlendState>,
    write_mask: wgpu::ColorWrites,
    pass: Pass,
}

fn sampler_key(sampler: &Sampler) -> [u32; 10] {
    [
        sampler.mag as u32,
        sampler.min as u32,
        sampler.mip as u32,
        sampler.u as u32,
        sampler.v as u32,
        sampler.w as u32,
        sampler.lod_bias.to_bits(),
        sampler.min_lod as u32,
        sampler.max_lod as u32,
        u32::from(sampler.cube),
    ]
}

impl PipelineKey {
    /// Hash of the key a lookup would build, from borrowed parts.
    fn hash(effect: uuid::Uuid, technique: usize, state: &State, pass: &Pass) -> u64 {
        use std::hash::{Hash, Hasher};
        let mut hasher = std::hash::DefaultHasher::new();
        (effect, technique, state.blend, state.write_mask, pass).hash(&mut hasher);
        for (binding, sampler) in &state.samplers {
            (binding, sampler_key(sampler)).hash(&mut hasher);
        }
        hasher.finish()
    }

    fn matches(&self, effect: uuid::Uuid, technique: usize, state: &State, pass: &Pass) -> bool {
        self.effect == effect
            && self.technique == technique
            && self.blend == state.blend
            && self.write_mask == state.write_mask
            && self.pass == *pass
            && self.samplers.iter().copied().eq(state
                .samplers
                .iter()
                .map(|(binding, sampler)| (*binding, sampler_key(sampler))))
    }
}

type Cached = Result<Arc<Pipeline>, String>;

/// Device-scoped pipelines, including validation failures, keyed by effect, technique, sampler
/// state and the complete pass. One cache belongs to one device. A hit clones no key.
#[derive(Default)]
pub struct Cache {
    entries: HashMap<u64, Vec<(PipelineKey, Cached)>>,
    pub refused: BTreeMap<String, u64>,
    pub created: usize,
}

impl Cache {
    /// The packed pipeline of `selection` drawing the caller's `topology`; each topology
    /// is its own pipeline.
    pub fn get(
        &mut self,
        device: &wgpu::Device,
        selection: &super::library::Selection,
        state: &State,
        format: wgpu::TextureFormat,
        samples: u32,
        topology: wgpu::PrimitiveTopology,
    ) -> Result<Arc<Pipeline>, String> {
        let compiled = selection.effect.compiled(selection.index)?;
        let mut pass = Pass::packed(&compiled, state, format, samples);
        pass.primitive.topology = topology;
        self.get_pass(device, selection, state, &pass)
    }

    pub fn get_pass(
        &mut self,
        device: &wgpu::Device,
        selection: &super::library::Selection,
        state: &State,
        pass: &Pass,
    ) -> Result<Arc<Pipeline>, String> {
        let effect = &selection.effect;
        let bucket = self
            .entries
            .entry(PipelineKey::hash(effect.id, selection.index, state, pass))
            .or_default();
        let result = match bucket
            .iter()
            .find(|(key, _)| key.matches(effect.id, selection.index, state, pass))
        {
            Some((_, result)) => result.clone(),
            None => {
                let result = (|| {
                    let compiled = effect.compiled(selection.index)?;
                    let pipeline = Pipeline::for_pass(
                        device,
                        &compiled,
                        &effect.source,
                        &effect.source.techniques[selection.index],
                        state,
                        pass,
                    )?;
                    self.created += 1;
                    Ok(Arc::new(pipeline))
                })();
                let key = PipelineKey {
                    effect: effect.id,
                    technique: selection.index,
                    samplers: state
                        .samplers
                        .iter()
                        .map(|(binding, sampler)| (*binding, sampler_key(sampler)))
                        .collect(),
                    blend: state.blend,
                    write_mask: state.write_mask,
                    pass: pass.clone(),
                };
                bucket.push((key, result.clone()));
                result
            }
        };
        if let Err(reason) = &result {
            *self.refused.entry(reason.clone()).or_default() += 1;
        }
        result
    }
}

impl Pipeline {
    pub fn new(
        device: &wgpu::Device,
        compiled: &Compiled,
        program: &Program,
        technique: &Technique,
        format: wgpu::TextureFormat,
        samples: u32,
    ) -> Result<Self, String> {
        Self::with_state(
            device,
            compiled,
            program,
            technique,
            format,
            samples,
            &State::authored(program, technique)?,
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub fn with_state(
        device: &wgpu::Device,
        compiled: &Compiled,
        program: &Program,
        technique: &Technique,
        format: wgpu::TextureFormat,
        samples: u32,
        state: &State,
    ) -> Result<Self, String> {
        Self::for_pass(
            device,
            compiled,
            program,
            technique,
            state,
            &Pass::packed(compiled, state, format, samples),
        )
    }

    pub fn for_pass(
        device: &wgpu::Device,
        compiled: &Compiled,
        program: &Program,
        technique: &Technique,
        state: &State,
        pass: &Pass,
    ) -> Result<Self, String> {
        let memory = device.push_error_scope(wgpu::ErrorFilter::OutOfMemory);
        let validation = device.push_error_scope(wgpu::ErrorFilter::Validation);
        let result = Self::create(device, compiled, &program.effect, technique, state, pass, 0);
        let error = pollster::block_on(validation.pop()).or(pollster::block_on(memory.pop()));
        match error {
            Some(error) => Err(error.to_string()),
            None => result,
        }
        .map_err(|reason| {
            format!(
                "effect {:?}, technique {:?}: {reason}",
                program.effect, technique.name
            )
        })
        .map(|mut pipeline| {
            if !pipeline.filtered.is_empty()
                && !device
                    .features()
                    .contains(wgpu::Features::FLOAT32_FILTERABLE)
            {
                pipeline.fallback = Some(Fallback {
                    compiled: compiled.clone(),
                    effect: program.effect.clone(),
                    technique: technique.clone(),
                    state: state.clone(),
                    pass: pass.clone(),
                    variants: Mutex::default(),
                });
            }
            pipeline
        })
    }

    #[allow(clippy::too_many_arguments)]
    fn create(
        device: &wgpu::Device,
        compiled: &Compiled,
        effect: &str,
        technique: &Technique,
        state: &State,
        pass: &Pass,
        unfilterable: u32,
    ) -> Result<Self, String> {
        let mut buffers = compiled.vertex.buffers.clone();
        buffers.extend_from_slice(&compiled.pixel.buffers);
        buffers.sort_by_key(|buffer| buffer.binding);
        let dynamic = device
            .limits()
            .max_dynamic_uniform_buffers_per_pipeline_layout;
        if buffers.len() > dynamic as usize {
            return Err(format!(
                "{} constant buffers exceed this device's {dynamic} dynamically offset buffers",
                buffers.len()
            ));
        }
        let mut textures = compiled.vertex.textures.clone();
        textures.extend_from_slice(&compiled.pixel.textures);
        let mut samplers = Vec::new();
        for binding in compiled
            .vertex
            .samplers
            .iter()
            .chain(&compiled.pixel.samplers)
        {
            let fact = state
                .samplers
                .get(binding)
                .ok_or_else(|| format!("sampler binding {binding} has no native bound state"))?;
            let address = |mode| match mode {
                Address::Wrap => Ok(wgpu::AddressMode::Repeat),
                Address::Mirror => Ok(wgpu::AddressMode::MirrorRepeat),
                Address::Clamp | Address::MirrorOnce => Ok(wgpu::AddressMode::ClampToEdge),
                Address::Border
                    if device
                        .features()
                        .contains(wgpu::Features::ADDRESS_MODE_CLAMP_TO_BORDER) =>
                {
                    Ok(wgpu::AddressMode::ClampToBorder)
                }
                Address::Border => {
                    Err("adapter has no clamp-to-border sampler support".to_string())
                }
            };
            let anisotropic = [fact.mag, fact.min, fact.mip].contains(&Filter::Anisotropic);
            let filter = |filter| {
                if anisotropic || matches!(filter, Filter::Linear) {
                    wgpu::FilterMode::Linear
                } else {
                    wgpu::FilterMode::Nearest
                }
            };
            let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
                label: Some(&fact.name),
                address_mode_u: address(fact.u)?,
                address_mode_v: address(fact.v)?,
                address_mode_w: address(fact.w)?,
                mag_filter: filter(fact.mag),
                min_filter: filter(fact.min),
                mipmap_filter: if anisotropic || fact.mip == Filter::Linear {
                    wgpu::MipmapFilterMode::Linear
                } else {
                    wgpu::MipmapFilterMode::Nearest
                },
                anisotropy_clamp: if anisotropic { 16 } else { 1 }, // Authored MaxAnisotropy is ignored; anisotropic filtering uses 16.
                lod_min_clamp: fact.min_lod.max(0) as f32,
                lod_max_clamp: fact.max_lod.max(0) as f32,
                border_color: Some(wgpu::SamplerBorderColor::TransparentBlack),
                ..Default::default()
            });
            samplers.push((*binding, sampler));
        }
        let visibility = |binding: u32| {
            if binding.is_multiple_of(2) {
                wgpu::ShaderStages::VERTEX
            } else {
                wgpu::ShaderStages::FRAGMENT
            }
        };
        let filtering = |binding: &u32| {
            state.samplers.get(binding).is_some_and(|sampler| {
                [sampler.mag, sampler.min, sampler.mip]
                    .iter()
                    .any(|filter| matches!(filter, Filter::Linear | Filter::Anisotropic))
            })
        };
        let filterable = |binding: &u32| {
            compiled
                .vertex
                .sampling_pairs
                .iter()
                .chain(&compiled.pixel.sampling_pairs)
                .any(|(image, sampler)| image == binding && filtering(sampler))
        };
        let [constants, textures_layout, samplers_layout] = [
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("native constants"),
                entries: &buffers
                    .iter()
                    .map(|b| wgpu::BindGroupLayoutEntry {
                        binding: b.binding,
                        visibility: visibility(b.binding),
                        ty: wgpu::BindingType::Buffer {
                            ty: wgpu::BufferBindingType::Uniform,
                            has_dynamic_offset: true,
                            min_binding_size: std::num::NonZeroU64::new(u64::from(b.size)),
                        },
                        count: None,
                    })
                    .collect::<Vec<_>>(),
            }),
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("native textures"),
                entries: &textures
                    .iter()
                    .map(|(binding, cube)| wgpu::BindGroupLayoutEntry {
                        binding: *binding,
                        visibility: visibility(*binding),
                        ty: wgpu::BindingType::Texture {
                            sample_type: wgpu::TextureSampleType::Float {
                                filterable: filterable(binding)
                                    && unfilterable & (1 << binding) == 0,
                            },
                            view_dimension: cube.view(),
                            multisampled: false,
                        },
                        count: None,
                    })
                    .collect::<Vec<_>>(),
            }),
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("native samplers"),
                entries: &samplers
                    .iter()
                    .map(|(binding, _)| wgpu::BindGroupLayoutEntry {
                        binding: *binding,
                        visibility: visibility(*binding),
                        ty: wgpu::BindingType::Sampler(if filtering(binding) {
                            wgpu::SamplerBindingType::Filtering
                        } else {
                            wgpu::SamplerBindingType::NonFiltering
                        }),
                        count: None,
                    })
                    .collect::<Vec<_>>(),
            }),
        ];
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some(effect),
            bind_group_layouts: &[
                Some(&constants),
                Some(&textures_layout),
                Some(&samplers_layout),
            ],
            immediate_size: 0,
        });
        let samplers = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("native samplers"),
            layout: &samplers_layout,
            entries: &samplers
                .iter()
                .map(|(binding, sampler)| wgpu::BindGroupEntry {
                    binding: *binding,
                    resource: wgpu::BindingResource::Sampler(sampler),
                })
                .collect::<Vec<_>>(),
        });
        let vertex = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some(&technique.vertex),
            source: wgpu::ShaderSource::Wgsl(
                super::sampling::lower(
                    &compiled.vertex.wgsl,
                    &state.samplers,
                    &compiled.vertex.textures,
                    unfilterable,
                )?
                .into(),
            ),
        });
        let pixel = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some(&technique.pixel),
            source: wgpu::ShaderSource::Wgsl(
                super::sampling::lower(
                    &compiled.pixel.wgsl,
                    &state.samplers,
                    &compiled.pixel.textures,
                    unfilterable,
                )?
                .into(),
            ),
        });
        let vertex_layouts: Vec<_> = pass
            .vertices
            .iter()
            .map(|layout| {
                Some(wgpu::VertexBufferLayout {
                    array_stride: layout.stride,
                    step_mode: layout.step,
                    attributes: &layout.attributes,
                })
            })
            .collect();
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some(&format!("{effect}:{}", technique.name)),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &vertex,
                entry_point: Some("main"),
                compilation_options: Default::default(),
                buffers: &vertex_layouts,
            },
            primitive: pass.primitive,
            depth_stencil: pass.depth_stencil.clone(),
            multisample: pass.multisample,
            fragment: Some(wgpu::FragmentState {
                module: &pixel,
                entry_point: Some("main"),
                compilation_options: Default::default(),
                targets: &pass.targets,
            }),
            multiview_mask: None,
            cache: None,
        });
        Ok(Self {
            pipeline,
            label: format!("effect {effect:?}, technique {:?}", technique.name),
            null_cube: textures
                .iter()
                .any(|(_, kind)| *kind == super::TextureKind::Cube)
                .then(|| null_texture(device, super::TextureKind::Cube)),
            layouts: [constants, textures_layout],
            samplers,
            buffers,
            filtered: textures
                .iter()
                .map(|(binding, _)| *binding)
                .filter(filterable)
                .collect(),
            textures,
            inputs: compiled.vertex.inputs.clone(),
            groups: Mutex::default(),
            fallback: None,
        })
    }

    /// Resources are keyed by `binding(slot, stage)`: native vertex/pixel slots are independent.
    /// Slots without an upload must be supplied by the caller's retained bind state.
    pub fn prepare(
        &self,
        arena: &Arena,
        constants: &BTreeMap<u32, impl AsRef<[u8]>>,
        textures: &BTreeMap<u32, wgpu::TextureView>,
        vertices: &super::vertices::Vertices<'_>,
    ) -> Result<Draw, String> {
        let bindings = self.bindings(arena, constants, textures)?;
        let data = vertices.pack(&self.inputs)?;
        Ok(Draw {
            bindings,
            vertices: arena.vertices(bytemuck::cast_slice(&data))?,
            count: vertices.count as u32,
        })
    }

    /// Bind arbitrary native register bytes and texture views. Caller supplies vertex/index
    /// buffers and draw commands; no GUI geometry, buffer names or matrix ABI is imposed.
    /// The bytes are staged in `arena`; the bindings are valid until its next reset.
    pub fn bindings(
        &self,
        arena: &Arena,
        constants: &BTreeMap<u32, impl AsRef<[u8]>>,
        textures: &BTreeMap<u32, wgpu::TextureView>,
    ) -> Result<Bindings, String> {
        self.bind_with(
            arena,
            |binding| constants.get(&binding).map(AsRef::as_ref),
            |binding| textures.get(&binding),
        )
    }

    /// `bindings` with the caller's lookups by `binding(slot, stage)`.
    pub fn bind_with<'a>(
        &'a self,
        arena: &Arena,
        constants: impl Fn(u32) -> Option<&'a [u8]>,
        textures: impl Fn(u32) -> Option<&'a wgpu::TextureView>,
    ) -> Result<Bindings, String> {
        let sampled = |key| {
            let view = textures(key)?;
            // The DDS cube flag alone determines the texture view; binding does not reshape it.
            // The native hardware GPU samples a 2D SRV through TextureCube as zero.
            // Preserve the caller's view: a later Texture2D draw still reads its texels.
            if let Some(zero) = &self.null_cube
                && self
                    .textures
                    .iter()
                    .any(|(slot, kind)| *slot == key && *kind == super::TextureKind::Cube)
                && view.texture().dimension() == wgpu::TextureDimension::D2
                && view.texture().depth_or_array_layers() == 1
            {
                Some(zero)
            } else {
                Some(view)
            }
        };
        self.bind(arena, constants, sampled)
            .map_err(|reason| format!("{}: {reason}", self.label))
    }

    fn bind<'a>(
        &self,
        arena: &Arena,
        constants: impl Fn(u32) -> Option<&'a [u8]>,
        textures: impl Fn(u32) -> Option<&'a wgpu::TextureView>,
    ) -> Result<Bindings, String> {
        if let Some(fallback) = &self.fallback {
            let device = arena.device();
            let mask = self.filtered.iter().fold(0, |mask, binding| {
                let manual = textures(*binding).is_some_and(|view| {
                    view.texture()
                        .format()
                        .sample_type(None, Some(device.features()))
                        == Some(wgpu::TextureSampleType::Float { filterable: false })
                });
                mask | if manual { 1 << binding } else { 0 }
            });
            if mask != 0 {
                let variant = fallback
                    .variants
                    .lock()
                    .unwrap_or_else(|error| error.into_inner())
                    .entry(mask)
                    .or_insert_with(|| {
                        scoped(device, || {
                            Self::create(
                                device,
                                &fallback.compiled,
                                &fallback.effect,
                                &fallback.technique,
                                &fallback.state,
                                &fallback.pass,
                                mask,
                            )
                        })
                        .and_then(|result| result)
                        .map(Arc::new)
                        .map_err(|reason| {
                            format!(
                                "effect {:?}, technique {:?}: {reason}",
                                fallback.effect, fallback.technique.name
                            )
                        })
                    })
                    .clone()?;
                return variant.bind(arena, constants, textures);
            }
        }
        // Compilation refuses registers past D3D11's, so both lists fit their arrays.
        let mut blocks: [(&[u8], u64); BUFFERS] = [(&[], 0); BUFFERS];
        for (block, buffer) in blocks.iter_mut().zip(&self.buffers) {
            let data = constants(buffer.binding).ok_or_else(|| {
                format!(
                    "native constant buffer {} has not been bound",
                    buffer.binding / 2
                )
            })?;
            *block = (data, u64::from(buffer.size));
        }
        let mut found: [Option<&wgpu::TextureView>; TEXTURES] = [None; TEXTURES];
        for (view, (binding, _)) in found.iter_mut().zip(&self.textures) {
            *view = Some(textures(*binding).ok_or_else(|| {
                format!("native texture slot {} has not been bound", binding / 2)
            })?);
        }
        let views = &found[..self.textures.len()];
        let count = self.buffers.len();
        let mut offsets = [0; BUFFERS];
        let buffer = arena.constants(&blocks[..count], &mut offsets[..count])?;
        let frame = arena.frame();
        let device = arena.device();
        let mut groups = self
            .groups
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let groups = &mut *groups;
        if groups.frame != frame {
            groups.frame = frame;
            let live = |used: u64| used + 1 >= frame;
            groups.constants.retain(|(_, _, used)| live(*used));
            for entries in groups.textures.values_mut() {
                entries.retain(|(_, _, used)| live(*used));
            }
            groups.textures.retain(|_, entries| !entries.is_empty());
            groups.last = None;
        }
        let constants = match groups
            .constants
            .iter_mut()
            .find(|(cached, _, _)| *cached == buffer)
        {
            Some((_, group, used)) => {
                *used = frame;
                group.clone()
            }
            None => {
                let group = scoped(device, || {
                    device.create_bind_group(&wgpu::BindGroupDescriptor {
                        label: Some("native constants"),
                        layout: &self.layouts[0],
                        entries: &self
                            .buffers
                            .iter()
                            .map(|spec| wgpu::BindGroupEntry {
                                binding: spec.binding,
                                resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                                    buffer: &buffer,
                                    offset: 0,
                                    size: std::num::NonZeroU64::new(u64::from(spec.size)),
                                }),
                            })
                            .collect::<Vec<_>>(),
                    })
                })?;
                groups.constants.push((buffer, group.clone(), frame));
                group
            }
        };
        let same = |cached: &[wgpu::TextureView]| cached.iter().map(Some).eq(views.iter().copied());
        let last = groups
            .last
            .filter(|(hash, index)| same(&groups.textures[hash][*index].0));
        let (hash, found) = match last {
            Some((hash, index)) => (hash, Some(index)),
            None => {
                use std::hash::{Hash, Hasher};
                let mut hasher = std::hash::DefaultHasher::new();
                views.hash(&mut hasher);
                let hash = hasher.finish();
                let found = groups
                    .textures
                    .get(&hash)
                    .and_then(|entries| entries.iter().position(|(cached, _, _)| same(cached)));
                (hash, found)
            }
        };
        let entries = groups.textures.entry(hash).or_default();
        let textures = match found {
            Some(index) => {
                let (_, group, used) = &mut entries[index];
                *used = frame;
                groups.last = Some((hash, index));
                group.clone()
            }
            None => {
                let group = scoped(device, || {
                    device.create_bind_group(&wgpu::BindGroupDescriptor {
                        label: Some("native textures"),
                        layout: &self.layouts[1],
                        entries: &self
                            .textures
                            .iter()
                            .zip(views.iter().flatten())
                            .map(|((binding, _), view)| wgpu::BindGroupEntry {
                                binding: *binding,
                                resource: wgpu::BindingResource::TextureView(view),
                            })
                            .collect::<Vec<_>>(),
                    })
                })?;
                let views = views.iter().flatten().map(|view| (*view).clone()).collect();
                entries.push((views, group.clone(), frame));
                groups.last = Some((hash, entries.len() - 1));
                group
            }
        };
        Ok(Bindings {
            pipeline: self.pipeline.clone(),
            groups: [constants, textures, self.samplers.clone()],
            offsets,
            count,
        })
    }

    /// Check a physical native allocation whose initialized bytes are only partly known.
    /// Consumers call this before `bindings`; slots absent from the program need no bytes.
    pub fn check_initialized(
        &self,
        binding: u32,
        physical: u32,
        known: &[std::ops::Range<u32>],
    ) -> Result<(), String> {
        for buffer in self
            .buffers
            .iter()
            .filter(|buffer| buffer.binding == binding)
        {
            buffer.check_initialized(physical, known)?;
        }
        Ok(())
    }

    pub fn draw(&self, pass: &mut wgpu::RenderPass<'_>, draw: &Draw) {
        // A null blend-factor pointer gives D3D11's all-ones blend constant.
        pass.set_blend_constant(wgpu::Color::WHITE);
        draw.bindings.bind(pass);
        pass.set_vertex_buffer(0, draw.vertices.slice());
        pass.draw(0..draw.count, 0..1);
    }
}

/// DX11 reads explicit null SRVs as zero; WebGPU zero-initialises resources.
pub(crate) fn null_texture(device: &wgpu::Device, kind: super::TextureKind) -> wgpu::TextureView {
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("native null texture"),
        size: wgpu::Extent3d {
            width: 1,
            height: 1,
            depth_or_array_layers: if kind == super::TextureKind::Cube {
                6
            } else {
                1
            },
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: if kind == super::TextureKind::D3 {
            wgpu::TextureDimension::D3
        } else {
            wgpu::TextureDimension::D2
        },
        format: wgpu::TextureFormat::Rgba8Unorm,
        usage: wgpu::TextureUsages::TEXTURE_BINDING,
        view_formats: &[],
    });
    texture.create_view(&wgpu::TextureViewDescriptor {
        dimension: Some(kind.view()),
        ..Default::default()
    })
}

/// Validation and out-of-memory errors while creating a GPU object, as a named failure.
fn scoped<T>(device: &wgpu::Device, create: impl FnOnce() -> T) -> Result<T, String> {
    let memory = device.push_error_scope(wgpu::ErrorFilter::OutOfMemory);
    let validation = device.push_error_scope(wgpu::ErrorFilter::Validation);
    let result = create();
    match pollster::block_on(validation.pop()).or(pollster::block_on(memory.pop())) {
        Some(error) => Err(error.to_string()),
        None => Ok(result),
    }
}

/// Native blend descriptor defaults and lowering.
pub fn blend(
    program: &Program,
    technique: &Technique,
) -> Result<(Option<wgpu::BlendState>, wgpu::ColorWrites), String> {
    let state = program.blend(technique);
    let factor = |code: u32| {
        [
            wgpu::BlendFactor::Zero,
            wgpu::BlendFactor::One,
            wgpu::BlendFactor::Src,
            wgpu::BlendFactor::OneMinusSrc,
            wgpu::BlendFactor::SrcAlpha,
            wgpu::BlendFactor::OneMinusSrcAlpha,
            wgpu::BlendFactor::DstAlpha,
            wgpu::BlendFactor::OneMinusDstAlpha,
            wgpu::BlendFactor::Dst,
            wgpu::BlendFactor::OneMinusDst,
            wgpu::BlendFactor::SrcAlphaSaturated,
            wgpu::BlendFactor::Constant,
            wgpu::BlendFactor::OneMinusConstant,
            wgpu::BlendFactor::Src1,
            wgpu::BlendFactor::OneMinusSrc1,
            wgpu::BlendFactor::Src1Alpha,
            wgpu::BlendFactor::OneMinusSrc1Alpha,
        ]
        .get(code as usize)
        .copied()
        .ok_or_else(|| format!("unknown native blend factor {code}"))
    };
    let operation = |code: u32| {
        [
            wgpu::BlendOperation::Add,
            wgpu::BlendOperation::Subtract,
            wgpu::BlendOperation::ReverseSubtract,
            wgpu::BlendOperation::Min,
            wgpu::BlendOperation::Max,
        ]
        .get(code as usize)
        .copied()
        .ok_or_else(|| format!("unknown native blend operation {code}"))
    };
    let mut blend = wgpu::BlendState {
        color: wgpu::BlendComponent {
            src_factor: factor(state.source)?,
            dst_factor: factor(state.dest)?,
            operation: operation(state.op)?,
        },
        alpha: wgpu::BlendComponent {
            src_factor: factor(state.source_alpha)?,
            dst_factor: factor(state.dest_alpha)?,
            operation: operation(state.op_alpha)?,
        },
    };
    let enabled = state.enable;
    let mask = wgpu::ColorWrites::from_bits_truncate(state.write_mask);
    // Native defaults ONE/ZERO/ADD form a sentinel: even an explicitly authored sentinel
    // inherits the colour equation. This includes SRC_ALPHA on the alpha channel.
    if blend.alpha == wgpu::BlendComponent::REPLACE {
        blend.alpha = blend.color;
    }
    // D3D ignores factors for MIN/MAX; wgpu requires One explicitly.
    for component in [&mut blend.color, &mut blend.alpha] {
        if matches!(
            component.operation,
            wgpu::BlendOperation::Min | wgpu::BlendOperation::Max
        ) {
            component.src_factor = wgpu::BlendFactor::One;
            component.dst_factor = wgpu::BlendFactor::One;
        }
    }
    Ok((enabled.then_some(blend), mask))
}

/// Native depth-stencil lowering. Comparisons and stencil operations both use the authored
/// index plus one in D3D11 order, anything else the first. The front-face stencil state applies
/// to both faces and the masks are their low bytes. D3D11's depth bias is rasterizer state but
/// wgpu's is depth state, so it comes from
/// `rasterizer`. The stencil reference is the caller's dynamic `set_stencil_reference`.
pub fn depth_stencil(
    state: &DepthStencilState,
    rasterizer: &RasterizerState,
    format: wgpu::TextureFormat,
) -> (wgpu::DepthStencilState, u32) {
    use wgpu::{CompareFunction as C, StencilOperation as O};
    let compare = |code: u32| {
        [
            C::Never,
            C::Less,
            C::Equal,
            C::LessEqual,
            C::Greater,
            C::NotEqual,
            C::GreaterEqual,
            C::Always,
        ]
        .get(code as usize)
        .copied()
        .unwrap_or(C::Never)
    };
    let operation = |code: u32| {
        [
            O::Keep,
            O::Zero,
            O::Replace,
            O::IncrementClamp,
            O::DecrementClamp,
            O::Invert,
            O::IncrementWrap,
            O::DecrementWrap,
        ]
        .get(code as usize)
        .copied()
        .unwrap_or(O::Keep)
    };
    let stencil = if state.stencil_enable {
        let face = wgpu::StencilFaceState {
            compare: compare(state.front_function),
            fail_op: operation(state.front_fail),
            depth_fail_op: operation(state.front_depth_fail),
            pass_op: operation(state.front_pass),
        };
        wgpu::StencilState {
            front: face,
            back: face,
            read_mask: state.stencil_read_mask & 0xFF,
            write_mask: state.stencil_write_mask & 0xFF,
        }
    } else {
        wgpu::StencilState::default()
    };
    let descriptor = wgpu::DepthStencilState {
        format,
        // A disabled depth test neither tests nor writes.
        depth_write_enabled: Some(state.depth_enable && state.depth_write_mask == 1),
        depth_compare: Some(if state.depth_enable {
            compare(state.depth_function)
        } else {
            C::Always
        }),
        stencil,
        bias: wgpu::DepthBiasState {
            constant: rasterizer.depth_bias,
            slope_scale: rasterizer.slope_scaled_depth_bias,
            clamp: rasterizer.depth_bias_clamp,
        },
    };
    (descriptor, state.stencil_ref)
}

/// Native rasterizer lowering: only `fill_solid` fills, cull mode follows the authored index,
/// winding is authored, and depth clipping is always on. Multisample
/// and line antialiasing come from the renderer, not the authored value. The second value is
/// the authored scissor enable; the rectangle is the caller's dynamic `set_scissor_rect`.
pub fn primitive(
    state: &RasterizerState,
    topology: wgpu::PrimitiveTopology,
) -> (wgpu::PrimitiveState, bool) {
    let primitive = wgpu::PrimitiveState {
        topology,
        front_face: if state.front_ccw {
            wgpu::FrontFace::Ccw
        } else {
            wgpu::FrontFace::Cw
        },
        cull_mode: match state.cull_mode {
            1 => Some(wgpu::Face::Front),
            2 => Some(wgpu::Face::Back),
            _ => None,
        },
        polygon_mode: if state.fill_mode == 2 {
            wgpu::PolygonMode::Fill
        } else {
            wgpu::PolygonMode::Line
        },
        ..Default::default()
    };
    (primitive, state.scissor)
}
