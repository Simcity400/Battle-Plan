//! GPU resources for compiled game techniques. Inputs are native register bytes and vertex
//! semantics, so changing an effect's uniform names never changes what the engine uploads.

use super::{Buffer, Compiled};
use installed_content::pdxshader::{
    Address, DepthStencilState, RasterizerState,
    program::{Filter, Program, Sampler, Stage, Technique},
};
use std::{
    collections::{BTreeMap, HashMap},
    sync::Arc,
};
use wgpu::{self, util::DeviceExt};

pub struct Pipeline {
    pub pipeline: wgpu::RenderPipeline,
    layouts: [wgpu::BindGroupLayout; 3],
    buffers: Vec<Buffer>,
    textures: Vec<(u32, super::TextureKind)>,
    samplers: Vec<(u32, wgpu::Sampler)>,
    inputs: Vec<super::VertexInput>,
}

pub struct Draw {
    groups: [wgpu::BindGroup; 3],
    vertices: wgpu::Buffer,
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

pub struct Bindings {
    groups: [wgpu::BindGroup; 3],
}
impl Bindings {
    pub fn bind(&self, pass: &mut wgpu::RenderPass<'_>) {
        for (index, group) in self.groups.iter().enumerate() {
            pass.set_bind_group(index as u32, group, &[]);
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

#[derive(PartialEq, Eq, Hash)]
struct PipelineKey {
    effect: uuid::Uuid,
    technique: usize,
    samplers: Vec<(u32, [u32; 10])>,
    blend: Option<wgpu::BlendState>,
    write_mask: wgpu::ColorWrites,
    pass: Pass,
}

/// Device-scoped pipelines, including validation failures, keyed by effect, technique, sampler
/// state and the complete pass. One cache belongs to one device.
#[derive(Default)]
pub struct Cache {
    entries: HashMap<PipelineKey, Result<Arc<Pipeline>, String>>,
    pub refused: BTreeMap<String, u64>,
    pub created: usize,
}

impl Cache {
    pub fn get(
        &mut self,
        device: &wgpu::Device,
        selection: &super::library::Selection,
        state: &State,
        format: wgpu::TextureFormat,
        samples: u32,
    ) -> Result<Arc<Pipeline>, String> {
        let compiled = selection.effect.compiled(selection.index)?;
        self.get_pass(
            device,
            selection,
            state,
            &Pass::packed(&compiled, state, format, samples),
        )
    }

    pub fn get_pass(
        &mut self,
        device: &wgpu::Device,
        selection: &super::library::Selection,
        state: &State,
        pass: &Pass,
    ) -> Result<Arc<Pipeline>, String> {
        let effect = &selection.effect;
        let key = PipelineKey {
            effect: effect.id,
            technique: selection.index,
            pass: pass.clone(),
            samplers: state
                .samplers
                .iter()
                .map(|(binding, sampler)| {
                    (
                        *binding,
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
                        ],
                    )
                })
                .collect(),
            blend: state.blend,
            write_mask: state.write_mask,
        };
        let result = self
            .entries
            .entry(key)
            .or_insert_with(|| {
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
            })
            .clone();
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
        let result = Self::create(device, compiled, program, technique, state, pass);
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
    }

    #[allow(clippy::too_many_arguments)]
    fn create(
        device: &wgpu::Device,
        compiled: &Compiled,
        program: &Program,
        technique: &Technique,
        state: &State,
        pass: &Pass,
    ) -> Result<Self, String> {
        let mut buffers = compiled.vertex.buffers.clone();
        buffers.extend_from_slice(&compiled.pixel.buffers);
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
                anisotropy_clamp: if anisotropic { 16 } else { 1 }, // 1423FD550 ignores authored MaxAnisotropy and supplies 16.
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
        let layouts = [
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("native constants"),
                entries: &buffers
                    .iter()
                    .map(|b| wgpu::BindGroupLayoutEntry {
                        binding: b.binding,
                        visibility: visibility(b.binding),
                        ty: wgpu::BindingType::Buffer {
                            ty: wgpu::BufferBindingType::Uniform,
                            has_dynamic_offset: false,
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
                            sample_type: wgpu::TextureSampleType::Float { filterable: true },
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
                        ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                        count: None,
                    })
                    .collect::<Vec<_>>(),
            }),
        ];
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some(&program.effect),
            bind_group_layouts: &layouts.iter().map(Some).collect::<Vec<_>>(),
            immediate_size: 0,
        });
        let vertex = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some(&technique.vertex),
            source: wgpu::ShaderSource::Wgsl(
                super::sampling::lower(
                    &compiled.vertex.wgsl,
                    &state.samplers,
                    &compiled.vertex.textures,
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
            label: Some(&format!("{}:{}", program.effect, technique.name)),
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
            layouts,
            buffers,
            textures,
            samplers,
            inputs: compiled.vertex.inputs.clone(),
        })
    }

    /// Resources are keyed by `binding(slot, stage)`: native vertex/pixel slots are independent.
    /// Slots without an upload must be supplied by the caller's retained bind state.
    pub fn prepare(
        &self,
        device: &wgpu::Device,
        constants: &BTreeMap<u32, Vec<u8>>,
        textures: &BTreeMap<u32, wgpu::TextureView>,
        vertices: &super::vertices::Vertices<'_>,
    ) -> Result<Draw, String> {
        let memory = device.push_error_scope(wgpu::ErrorFilter::OutOfMemory);
        let validation = device.push_error_scope(wgpu::ErrorFilter::Validation);
        let result = self.prepare_inner(device, constants, textures, vertices);
        let error = pollster::block_on(validation.pop()).or(pollster::block_on(memory.pop()));
        match error {
            Some(error) => Err(error.to_string()),
            None => result,
        }
    }

    fn prepare_inner(
        &self,
        device: &wgpu::Device,
        constants: &BTreeMap<u32, Vec<u8>>,
        textures: &BTreeMap<u32, wgpu::TextureView>,
        vertices: &super::vertices::Vertices<'_>,
    ) -> Result<Draw, String> {
        let bindings = self.bindings_inner(device, constants, textures)?;
        let data = vertices.pack(&self.inputs)?;
        Ok(Draw {
            groups: bindings.groups,
            vertices: device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("native vertices"),
                contents: bytemuck::cast_slice(&data),
                usage: wgpu::BufferUsages::VERTEX,
            }),
            count: vertices.count as u32,
        })
    }

    /// Bind arbitrary native register bytes and texture views. Caller supplies vertex/index
    /// buffers and draw commands; no GUI geometry, buffer names or matrix ABI is imposed.
    pub fn bindings(
        &self,
        device: &wgpu::Device,
        constants: &BTreeMap<u32, Vec<u8>>,
        textures: &BTreeMap<u32, wgpu::TextureView>,
    ) -> Result<Bindings, String> {
        let memory = device.push_error_scope(wgpu::ErrorFilter::OutOfMemory);
        let validation = device.push_error_scope(wgpu::ErrorFilter::Validation);
        let result = self.bindings_inner(device, constants, textures);
        match pollster::block_on(validation.pop()).or(pollster::block_on(memory.pop())) {
            Some(error) => Err(error.to_string()),
            None => result,
        }
    }

    fn bindings_inner(
        &self,
        device: &wgpu::Device,
        constants: &BTreeMap<u32, Vec<u8>>,
        textures: &BTreeMap<u32, wgpu::TextureView>,
    ) -> Result<Bindings, String> {
        let buffers = self
            .buffers
            .iter()
            .map(|buffer| {
                let data = constants.get(&buffer.binding).ok_or_else(|| {
                    format!(
                        "native constant buffer {} has not been bound",
                        buffer.binding / 2
                    )
                })?;
                // D3D11 returns zero for reads past the bound constant buffer's range.
                let mut data = data.clone();
                data.resize(data.len().max(buffer.size as usize), 0);
                Ok(
                    device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                        label: Some("native constants"),
                        contents: &data,
                        usage: wgpu::BufferUsages::UNIFORM,
                    }),
                )
            })
            .collect::<Result<Vec<_>, String>>()?;
        let buffer_entries: Vec<_> = self
            .buffers
            .iter()
            .zip(&buffers)
            .map(|(spec, data)| wgpu::BindGroupEntry {
                binding: spec.binding,
                resource: data.as_entire_binding(),
            })
            .collect();
        let texture_entries = self
            .textures
            .iter()
            .map(|(binding, _)| {
                let view = textures.get(binding).ok_or_else(|| {
                    format!("native texture slot {} has not been bound", binding / 2)
                })?;
                Ok(wgpu::BindGroupEntry {
                    binding: *binding,
                    resource: wgpu::BindingResource::TextureView(view),
                })
            })
            .collect::<Result<Vec<_>, String>>()?;
        let sampler_entries: Vec<_> = self
            .samplers
            .iter()
            .map(|(binding, sampler)| wgpu::BindGroupEntry {
                binding: *binding,
                resource: wgpu::BindingResource::Sampler(sampler),
            })
            .collect();
        let groups = [
            device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("native constants"),
                layout: &self.layouts[0],
                entries: &buffer_entries,
            }),
            device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("native textures"),
                layout: &self.layouts[1],
                entries: &texture_entries,
            }),
            device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("native samplers"),
                layout: &self.layouts[2],
                entries: &sampler_entries,
            }),
        ];
        Ok(Bindings { groups })
    }

    pub fn draw(&self, pass: &mut wgpu::RenderPass<'_>, draw: &Draw) {
        pass.set_pipeline(&self.pipeline);
        // 142400090 passes a null BlendFactor pointer, whose D3D11 value is all ones.
        pass.set_blend_constant(wgpu::Color::WHITE);
        for (index, group) in draw.groups.iter().enumerate() {
            pass.set_bind_group(index as u32, group, &[]);
        }
        pass.set_vertex_buffer(0, draw.vertices.slice(..));
        pass.draw(0..draw.count, 0..1);
    }
}

/// Native blend descriptor defaults and lowering: `1423F7110`, `142347E10`, `1423FCB30`.
pub fn blend(
    program: &Program,
    technique: &Technique,
) -> Result<(Option<wgpu::BlendState>, wgpu::ColorWrites), String> {
    let state = program.blend(technique);
    // Native numeric factor/operation codes: 1423F73B0 / 1423F75C0.
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
    // inherits the colour equation (1423FCB30). This includes SRC_ALPHA on the alpha channel.
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

/// Native depth-stencil lowering (`lower@1423FCFC0`). Comparisons and stencil operations both
/// pass through `lower_comparison@142411060` (authored index plus one, in D3D11 order, anything
/// else the first), the front-face stencil state applies to both faces and the masks are their
/// low bytes. D3D11's depth bias is rasterizer state but wgpu's is depth state, so it comes from
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

/// Native rasterizer lowering (`lower@1423FD2F0`): only `fill_solid` fills, cull mode through
/// `lower_cull_mode@1424110D0`, the authored winding, and depth clipping always on. Multisample
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
