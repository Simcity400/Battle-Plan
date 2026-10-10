//! Apply native binds in submission order. The owner must feed this after its layer sorting:
//! a missing technique keeps the preceding program and its input declaration. Each resource
//! changes only when its own binding command is supplied. A draw the owner refuses still makes
//! the binds the game makes for it; what the owner lacks of them (a program it could not
//! compile, a texture it does not hold) is forgotten ([`Stream::forget_program`],
//! [`Stream::forget_texture`]), so a later draw that reads it is refused instead of given an
//! earlier bind.

use super::{
    binding, gpu,
    library::Selection,
    vertices::{Declaration, Vertices},
};
use installed_content::pdxshader::{
    Address, DepthStencilState, RasterizerState,
    program::{Filter, Sampler, Stage},
};
use std::{collections::BTreeMap, sync::Arc};

pub struct Stream {
    program: Option<(Selection, Layout)>,
    vertex_strides: BTreeMap<u32, u64>,
    state: gpu::State,
    depth_stencil: Option<DepthStencilState>,
    rasterizer: Option<RasterizerState>,
    constants: BTreeMap<u32, Vec<u8>>,
    known_constants: BTreeMap<u32, Vec<std::ops::Range<u32>>>,
    textures: BTreeMap<u32, Option<wgpu::TextureView>>,
    null_textures: BTreeMap<super::TextureKind, wgpu::TextureView>,
    pub cache: gpu::Cache,
    pub refused: BTreeMap<String, u64>,
}

#[derive(Clone)]
enum Layout {
    Packed(Declaration),
    Native(Vec<gpu::VertexLayout>),
}

impl Default for Stream {
    fn default() -> Self {
        Self {
            program: None,
            vertex_strides: BTreeMap::new(),
            state: gpu::State {
                samplers: BTreeMap::new(),
                blend: None,
                write_mask: wgpu::ColorWrites::ALL,
            },
            depth_stencil: None,
            rasterizer: None,
            constants: BTreeMap::new(),
            known_constants: BTreeMap::new(),
            textures: BTreeMap::new(),
            null_textures: BTreeMap::new(),
            cache: gpu::Cache::default(),
            refused: BTreeMap::new(),
        }
    }
}

pub struct Submission {
    pub pipeline: Arc<gpu::Pipeline>,
    pub draw: gpu::Draw,
}

/// Geometry, viewport/scissor and blend constants remain caller-owned.
pub struct PassSubmission {
    pub pipeline: Arc<gpu::Pipeline>,
    pub bindings: gpu::Bindings,
    pub stencil_reference: Option<u32>,
    pub scissor_enabled: Option<bool>,
}

pub struct NativeSubmission {
    pub pass: PassSubmission,
    pub vertices: Vec<gpu::Slice>,
}

impl Stream {
    /// Start a scoped render-to-texture pass with the caller's retained bindings.
    /// Keep this stream's device cache; later changes do not alter the caller.
    pub fn inherit_bindings(&mut self, caller: &Self) {
        self.program = caller.program.clone();
        self.vertex_strides = caller.vertex_strides.clone();
        self.state = caller.state.clone();
        self.depth_stencil = caller.depth_stencil.clone();
        self.rasterizer = caller.rasterizer.clone();
        self.constants = caller.constants.clone();
        self.known_constants = caller.known_constants.clone();
        self.textures = caller.textures.clone();
    }

    /// Independent renderer blend handle (142400090), after program selection. In
    /// particular 14228C8D0 disables background blending then uses alpha MAX for text.
    pub fn bind_blend(&mut self, blend: Option<wgpu::BlendState>, write_mask: wgpu::ColorWrites) {
        self.state.blend = blend;
        self.state.write_mask = write_mask;
    }

    ///142400520 retains the program and its input layout on a null handle.
    ///Blend, depth, rasterizer and resource handles are independent commands.
    pub fn bind_program(&mut self, selection: Option<Selection>, vertices: Vec<gpu::VertexLayout>) {
        if let Some(selection) = selection {
            self.program = Some((selection, Layout::Native(vertices)));
        }
    }

    /// IA buffer strides are independent of the program/input declaration. For
    /// example1422805EB replaces trail slot0 even when142400520 retains a program.
    /// Handles and offsets remain caller-owned; an omitted slot keeps its stride.
    pub fn bind_vertex_stride(&mut self, slot: u32, stride: u64) {
        self.vertex_strides.insert(slot, stride);
    }

    /// Effective input declaration for binding the caller's retained buffers.
    /// Layout strides are defaults until an explicit buffer bind supplies one.
    pub fn vertex_layouts(&self) -> Result<Vec<gpu::VertexLayout>, String> {
        let (_, layout) = self.program.as_ref().ok_or(
            "null technique needs the preceding native program, which has not been supplied",
        )?;
        let Layout::Native(vertices) = layout else {
            return Err("packed vertex decoding requires prepare".into());
        };
        let mut vertices = vertices.clone();
        for (slot, vertex) in vertices.iter_mut().enumerate() {
            if let Some(stride) = self.vertex_strides.get(&(slot as u32)) {
                vertex.stride = *stride;
            }
        }
        Ok(vertices)
    }

    pub fn bind_depth_stencil(&mut self, state: DepthStencilState) {
        self.depth_stencil = Some(state);
    }

    pub fn bind_rasterizer(&mut self, state: RasterizerState) {
        self.rasterizer = Some(state);
    }

    /// A non-native pass (or a new frame) has replaced device state we cannot reconstruct.
    /// Keep compiled pipelines, but never guess that its program was the last native effect.
    pub fn invalidate_bindings(&mut self) {
        self.program = None;
        self.vertex_strides.clear();
        self.depth_stencil = None;
        self.rasterizer = None;
        self.state.samplers.clear();
        self.constants.clear();
        self.known_constants.clear();
        self.textures.clear();
    }

    /// 142400520 retains program/layout on null; 142400090 selects the renderer default blend.
    /// The program is bound even when its blend state is refused: the game binds both.
    pub fn bind_effect(
        &mut self,
        selection: Option<Selection>,
        declaration: Declaration,
    ) -> Result<(), String> {
        let Some(selection) = selection else {
            self.bind_blend(None, wgpu::ColorWrites::ALL);
            return Ok(());
        };
        let blend = gpu::blend(
            &selection.effect.source,
            &selection.effect.source.techniques[selection.index],
        );
        self.program = Some((selection, Layout::Packed(declaration)));
        let (blend, mask) = blend?;
        self.bind_blend(blend, mask);
        Ok(())
    }

    /// A declared technique the owner could not compile: the game binds its program record
    /// (142400520 with a non-null record), which the owner does not have.
    pub fn forget_program(&mut self) {
        self.program = None;
    }

    /// 1424000F0 does nothing for a null constant-buffer handle; it does not clear the slot.
    pub fn bind_constants(&mut self, slot: u32, stage: Stage, bytes: Option<&[u8]>) {
        if let Some(bytes) = bytes {
            self.write_constants(binding(slot, stage), bytes);
            self.known_constants.remove(&binding(slot, stage));
        }
    }

    fn write_constants(&mut self, key: u32, bytes: &[u8]) {
        let retained = self.constants.entry(key).or_default();
        retained.clear();
        retained.extend_from_slice(bytes);
    }

    /// `bytes` covers the physical allocation, including unknown discard padding.
    /// A null handle retains both the allocation and its known-byte ranges.
    pub fn bind_partial_constants(
        &mut self,
        slot: u32,
        stage: Stage,
        bytes: Option<&[u8]>,
        known: &[std::ops::Range<u32>],
    ) {
        if let Some(bytes) = bytes {
            self.write_constants(binding(slot, stage), bytes);
            let retained = self
                .known_constants
                .entry(binding(slot, stage))
                .or_default();
            retained.clear();
            retained.extend_from_slice(known);
        }
    }

    fn check_constants(&self, pipeline: &gpu::Pipeline) -> Result<(), String> {
        for (binding, known) in &self.known_constants {
            let physical = u32::try_from(self.constants[binding].len())
                .map_err(|_| "native constant allocation exceeds u32")?;
            pipeline.check_initialized(*binding, physical, known)?;
        }
        Ok(())
    }

    /// 1424005A0: a sampler overrides the caller's slot, for both texture and sampler.
    /// A null pixel sampler retains state, a null vertex sampler clears it; null texture unbinds.
    pub fn bind_texture(
        &mut self,
        slot: u32,
        stage: Stage,
        texture: Option<wgpu::TextureView>,
        sampler: Option<&Sampler>,
    ) {
        let slot = sampler.map_or(slot, |sampler| sampler.index);
        let key = binding(slot, stage);
        self.textures.insert(key, texture);
        if let Some(sampler) = sampler {
            if self.state.samplers.get(&key) != Some(sampler) {
                self.state.samplers.insert(key, sampler.clone());
            }
        } else if stage == Stage::Vertex {
            // 1424005A0 calls VSSetSamplers with null. D3D11's null sampler state is
            // MIN_MAG_MIP_LINEAR, CLAMP, bias 0, no LOD restriction (D3D11_SAMPLER_DESC).
            self.state.samplers.insert(
                key,
                Sampler {
                    name: "D3D11 null sampler".into(),
                    index: slot,
                    cube: false,
                    mag: Filter::Linear,
                    min: Filter::Linear,
                    mip: Filter::Linear,
                    u: Address::Clamp,
                    v: Address::Clamp,
                    w: Address::Clamp,
                    lod_bias: 0.,
                    min_lod: i32::MIN,
                    max_lod: i32::MAX,
                },
            );
        }
    }

    /// A texture the owner does not hold, at a slot a draw binds: the slot's sampler binds as
    /// [`Self::bind_texture`] binds it, and the texture is unknown to later draws.
    pub fn forget_texture(&mut self, slot: u32, stage: Stage, sampler: Option<&Sampler>) {
        self.bind_texture(slot, stage, None, sampler);
        let slot = sampler.map_or(slot, |sampler| sampler.index);
        self.textures.remove(&binding(slot, stage));
    }

    /// `count` records of `stride` bytes drawn as `topology` (the draw's own, as
    /// `GfxSetPrimitiveTopology@1424002E0` sets it): a native four-record strip stays four
    /// records, so `SV_VertexID` runs as it does natively.
    #[allow(clippy::too_many_arguments)]
    pub fn prepare(
        &mut self,
        arena: &gpu::Arena,
        label: &str,
        bytes: &[u8],
        stride: usize,
        count: usize,
        topology: wgpu::PrimitiveTopology,
        format: wgpu::TextureFormat,
        samples: u32,
    ) -> Result<Submission, String> {
        let result = self
            .prepare_inner(arena, bytes, stride, count, topology, format, samples)
            .map_err(|reason| format!("native draw {label:?}: {reason}"));
        if let Err(reason) = &result {
            *self.refused.entry(reason.clone()).or_default() += 1;
        }
        result
    }

    #[allow(clippy::too_many_arguments)]
    fn prepare_inner(
        &mut self,
        arena: &gpu::Arena,
        bytes: &[u8],
        stride: usize,
        count: usize,
        topology: wgpu::PrimitiveTopology,
        format: wgpu::TextureFormat,
        samples: u32,
    ) -> Result<Submission, String> {
        let (selection, declaration) = self.program.clone().ok_or(
            "null technique needs the preceding native program, which has not been supplied",
        )?;
        let Layout::Packed(declaration) = declaration else {
            return Err("native vertex buffers require prepare_pass".into());
        };
        let pipeline = self.cache.get(
            arena.device(),
            &selection,
            &self.state,
            format,
            samples,
            topology,
        )?;
        let textures = self.resolve_textures(arena.device(), &selection)?;
        self.check_constants(&pipeline)?;
        let draw = pipeline
            .prepare(
                arena,
                &self.constants,
                &textures,
                &Vertices {
                    declaration,
                    bytes,
                    stride,
                    count,
                },
            )
            .map_err(|reason| format!("effect {:?}: {reason}", selection.effect.source.effect))?;
        Ok(Submission { pipeline, draw })
    }

    ///Prepare arbitrary caller-owned vertex/index streams with retained native bindings.
    ///The template owns attachments, topology and multisampling. Bound program/layout,
    ///blend, rasterizer and depth handles override their corresponding template fields.
    pub fn prepare_pass(
        &mut self,
        arena: &gpu::Arena,
        label: &str,
        template: &gpu::Pass,
    ) -> Result<PassSubmission, String> {
        let result = self
            .prepare_pass_inner(arena, template, None)
            .map_err(|reason| format!("native draw {label:?}: {reason}"));
        if let Err(reason) = &result {
            *self.refused.entry(reason.clone()).or_default() += 1;
        }
        result
    }

    /// Decode the retained packed declaration, using the caller's attachments and
    /// retained native depth/raster state. Later texture groups can reuse these
    /// vertex slices with [`Self::prepare_pass`] without uploading the geometry again.
    pub fn prepare_packed_pass(
        &mut self,
        arena: &gpu::Arena,
        label: &str,
        template: &gpu::Pass,
        bytes: &[u8],
        stride: usize,
        count: usize,
    ) -> Result<NativeSubmission, String> {
        let result = (|| {
            let (selection, layout) = self.program.as_ref().ok_or(
                "null technique needs the preceding native program, which has not been supplied",
            )?;
            let Layout::Packed(declaration) = layout else {
                return Err("native vertex buffers require prepare_native_pass".into());
            };
            let compiled = selection.effect.compiled(selection.index)?;
            let packed = Vertices {
                declaration: *declaration,
                bytes,
                stride,
                count,
            }
            .pack(&compiled.vertex.inputs)?;
            let pass = self.prepare_pass_inner(arena, template, None)?;
            let vertices = vec![arena.vertices(bytemuck::cast_slice(&packed))?];
            Ok(NativeSubmission { pass, vertices })
        })()
        .map_err(|reason: String| format!("native draw {label:?}: {reason}"));
        if let Err(reason) = &result {
            *self.refused.entry(reason.clone()).or_default() += 1;
        }
        result
    }

    /// Decode complete native buffer allocations into WebGPU-compatible strides.
    /// This preserves overlapping declarations and D3D out-of-allocation reads;
    /// unknown bytes in an allocation are still a named refusal. The native
    /// retained declaration/strides remain unchanged by this temporary packing.
    pub fn prepare_native_pass(
        &mut self,
        arena: &gpu::Arena,
        label: &str,
        template: &gpu::Pass,
        buffers: &BTreeMap<u32, Option<super::vertices::NativeBuffer<'_>>>,
        vertices: u32,
        instances: u32,
    ) -> Result<NativeSubmission, String> {
        let result = (|| {
            let packed = super::vertices::pack_native(
                &self.vertex_layouts()?,
                buffers,
                vertices,
                instances,
            )?;
            let pass = self.prepare_pass_inner(arena, template, Some(packed.layouts))?;
            let vertices = packed
                .bytes
                .iter()
                .map(|bytes| arena.vertices(bytes))
                .collect::<Result<_, _>>()?;
            Ok(NativeSubmission { pass, vertices })
        })()
        .map_err(|reason: String| format!("native draw {label:?}: {reason}"));
        if let Err(reason) = &result {
            *self.refused.entry(reason.clone()).or_default() += 1;
        }
        result
    }

    fn prepare_pass_inner(
        &mut self,
        arena: &gpu::Arena,
        template: &gpu::Pass,
        vertices: Option<Vec<gpu::VertexLayout>>,
    ) -> Result<PassSubmission, String> {
        let (selection, layout) = self.program.as_ref().ok_or(
            "null technique needs the preceding native program, which has not been supplied",
        )?;
        let packed = matches!(layout, Layout::Packed(_));
        let selection = selection.clone();
        let (mut pass, stencil_reference, scissor_enabled) = self.pass_state(template);
        pass.vertices = match vertices {
            Some(vertices) => vertices,
            None if packed => {
                let compiled = selection.effect.compiled(selection.index)?;
                let format = template
                    .targets
                    .iter()
                    .flatten()
                    .next()
                    .ok_or("packed native pass has no colour attachment")?
                    .format;
                gpu::Pass::packed(&compiled, &self.state, format, template.multisample.count)
                    .vertices
            }
            None => self.vertex_layouts()?,
        };
        let pipeline = self
            .cache
            .get_pass(arena.device(), &selection, &self.state, &pass)?;
        let compiled = selection.effect.compiled(selection.index)?;
        let kinds = || {
            compiled
                .vertex
                .textures
                .iter()
                .chain(&compiled.pixel.textures)
        };
        self.null_textures(arena.device(), &selection, kinds())?;
        self.check_constants(&pipeline)?;
        let bindings = pipeline
            .bind_with(
                arena,
                |binding| self.constants.get(&binding).map(Vec::as_slice),
                |binding| match self.textures.get(&binding)? {
                    Some(view) => Some(view),
                    None => {
                        let (_, kind) = kinds().find(|(key, _)| *key == binding)?;
                        self.null_textures.get(kind)
                    }
                },
            )
            .map_err(|reason| format!("effect {:?}: {reason}", selection.effect.source.effect))?;
        Ok(PassSubmission {
            pipeline,
            bindings,
            stencil_reference,
            scissor_enabled,
        })
    }

    fn pass_state(&self, template: &gpu::Pass) -> (gpu::Pass, Option<u32>, Option<bool>) {
        let mut pass = template.clone();
        for target in pass.targets.iter_mut().flatten() {
            target.blend = self.state.blend;
            target.write_mask = self.state.write_mask;
        }
        let mut scissor = None;
        if let Some(raster) = &self.rasterizer {
            let (mut primitive, enabled) = gpu::primitive(raster, pass.primitive.topology);
            primitive.strip_index_format = pass.primitive.strip_index_format;
            pass.primitive = primitive;
            scissor = Some(enabled);
        }
        let mut reference = None;
        if let Some(depth) = &mut pass.depth_stencil {
            if let Some(raster) = &self.rasterizer {
                // Bias belongs to the native raster handle, independently of depth.
                depth.bias =
                    gpu::depth_stencil(&DepthStencilState::default(), raster, depth.format)
                        .0
                        .bias;
            }
            if let Some(state) = &self.depth_stencil {
                let bias = depth.bias;
                let (mut lowered, value) =
                    gpu::depth_stencil(state, &RasterizerState::default(), depth.format);
                lowered.bias = bias;
                *depth = lowered;
                reference = Some(value);
            }
        }
        (pass, reference, scissor)
    }

    fn resolve_textures(
        &mut self,
        device: &wgpu::Device,
        selection: &Selection,
    ) -> Result<BTreeMap<u32, wgpu::TextureView>, String> {
        let compiled = selection.effect.compiled(selection.index)?;
        let kinds = || {
            compiled
                .vertex
                .textures
                .iter()
                .chain(&compiled.pixel.textures)
        };
        self.null_textures(device, selection, kinds())?;
        Ok(kinds()
            .map(|(key, kind)| {
                let view = match &self.textures[key] {
                    Some(view) => view,
                    None => &self.null_textures[kind],
                };
                (*key, view.clone())
            })
            .collect())
    }

    /// Create the zero textures explicit null binds need; refuse slots never bound.
    fn null_textures<'a>(
        &mut self,
        device: &wgpu::Device,
        selection: &Selection,
        kinds: impl Iterator<Item = &'a (u32, super::TextureKind)>,
    ) -> Result<(), String> {
        for &(key, cube) in kinds {
            match self.textures.get(&key) {
                Some(Some(_)) => {}
                Some(None) => {
                    self.null_textures
                        .entry(cube)
                        .or_insert_with(|| gpu::null_texture(device, cube));
                }
                None => {
                    return Err(format!(
                        "effect {:?}: native texture binding {key} has not been supplied",
                        selection.effect.source.effect
                    ));
                }
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn template() -> gpu::Pass {
        gpu::Pass {
            vertices: Vec::new(),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleStrip,
                strip_index_format: Some(wgpu::IndexFormat::Uint16),
                ..Default::default()
            },
            depth_stencil: Some(
                gpu::depth_stencil(
                    &DepthStencilState::default(),
                    &RasterizerState::default(),
                    wgpu::TextureFormat::Depth24PlusStencil8,
                )
                .0,
            ),
            multisample: wgpu::MultisampleState {
                count: 4,
                ..Default::default()
            },
            targets: vec![Some(wgpu::ColorTargetState {
                format: wgpu::TextureFormat::Rgba16Float,
                blend: None,
                write_mask: wgpu::ColorWrites::ALL,
            })],
        }
    }

    #[test]
    fn independent_handles_survive_program_misses_and_scoped_passes() {
        let program = installed_content::pdxshader::program::read_with(b"states.shader", &|path| {
            Some(if path == b"states.shader" {
                b"Effect Draw { }".to_vec()
            } else {
                Vec::new()
            })
        })
        .unwrap();
        let selection = Selection {
            effect: super::super::library::Effect::from_compiled(
                Arc::new(program),
                Vec::new(),
                vec![Err("not compiled in a state test".into())],
            ),
            index: 0,
        };
        let layout = vec![gpu::VertexLayout {
            stride: 48,
            step: wgpu::VertexStepMode::Instance,
            attributes: vec![wgpu::VertexAttribute {
                format: wgpu::VertexFormat::Uint8x4,
                offset: 28,
                shader_location: 3,
            }],
        }];
        let mut stream = Stream::default();
        stream.bind_program(Some(selection), layout.clone());
        stream.bind_depth_stencil(DepthStencilState {
            depth_write_mask: 0,
            stencil_enable: true,
            stencil_ref: 37,
            ..Default::default()
        });
        stream.bind_rasterizer(RasterizerState {
            front_ccw: true,
            scissor: true,
            depth_bias: -7,
            slope_scaled_depth_bias: 2.,
            ..Default::default()
        });
        stream.bind_blend(
            Some(wgpu::BlendState::ALPHA_BLENDING),
            wgpu::ColorWrites::RED,
        );
        stream.bind_constants(1, Stage::Vertex, Some(&[1, 2, 3, 4]));
        stream.bind_constants(1, Stage::Pixel, Some(&[5, 6, 7, 8]));
        stream.bind_program(None, Vec::new());
        stream.bind_constants(1, Stage::Pixel, None);
        let (_, Layout::Native(retained)) = stream.program.as_ref().unwrap() else {
            panic!("native layout lost")
        };
        assert_eq!(retained, &layout);
        let template = template();
        let (pass, reference, scissor) = stream.pass_state(&template);
        assert_eq!((reference, scissor), (Some(37), Some(true)));
        assert_eq!(
            pass.primitive.strip_index_format,
            Some(wgpu::IndexFormat::Uint16)
        );
        assert_eq!(pass.primitive.front_face, wgpu::FrontFace::Ccw);
        assert_eq!(pass.multisample.count, 4);
        let depth = pass.depth_stencil.unwrap();
        assert_eq!(depth.depth_write_enabled, Some(false));
        assert_eq!(depth.bias.constant, -7);
        assert_eq!(depth.bias.slope_scale, 2.);
        let target = pass.targets[0].as_ref().unwrap();
        assert_eq!(target.format, wgpu::TextureFormat::Rgba16Float);
        assert_eq!(target.write_mask, wgpu::ColorWrites::RED);
        assert_eq!(target.blend, Some(wgpu::BlendState::ALPHA_BLENDING));
        let mut scoped = Stream::default();
        stream.bind_vertex_stride(0, 64);
        scoped.inherit_bindings(&stream);
        scoped.bind_vertex_stride(0, 80);
        assert_eq!(stream.vertex_layouts().unwrap()[0].stride, 64);
        assert_eq!(scoped.vertex_layouts().unwrap()[0].stride, 80);
        assert_eq!(scoped.pass_state(&template), stream.pass_state(&template));
        scoped.bind_constants(1, Stage::Pixel, Some(&[9; 4]));
        scoped.bind_rasterizer(RasterizerState::default());
        assert_eq!(stream.constants[&binding(1, Stage::Pixel)], [5, 6, 7, 8]);
        assert_eq!(scoped.constants[&binding(1, Stage::Vertex)], [1, 2, 3, 4]);
        assert_eq!(stream.rasterizer.as_ref().unwrap().depth_bias, -7);
        assert_eq!(scoped.rasterizer.as_ref().unwrap().depth_bias, 0);
    }

    #[test]
    fn packed_world_text_retains_raster_and_distinct_stage_allocations() {
        let source = installed_content::pdxshader::program::read_with(b"text.shader", &|path| {
            Some(if path == b"text.shader" {
                b"Effect Text3D { }".to_vec()
            } else {
                Vec::new()
            })
        })
        .unwrap();
        let selection = Selection {
            effect: super::super::library::Effect::from_compiled(
                Arc::new(source),
                Vec::new(),
                vec![],
            ),
            index: 0,
        };
        let mut stream = Stream::default();
        stream.bind_constants(0, Stage::Pixel, Some(&[17; 408]));
        stream.bind_rasterizer(RasterizerState {
            front_ccw: true,
            depth_bias: -3,
            scissor: true,
            ..Default::default()
        });
        stream.bind_depth_stencil(DepthStencilState {
            depth_enable: false,
            depth_write_mask: 0,
            ..Default::default()
        });
        stream
            .bind_effect(Some(selection), Declaration::Text)
            .unwrap();
        stream.bind_constants(0, Stage::Vertex, Some(&[29; 80]));
        let (pass, _, scissor) = stream.pass_state(&template());
        assert_eq!(stream.constants[&binding(0, Stage::Vertex)], [29; 80]);
        assert_eq!(stream.constants[&binding(0, Stage::Pixel)], [17; 408]);
        assert_eq!(pass.primitive.front_face, wgpu::FrontFace::Ccw);
        assert_eq!(scissor, Some(true));
        let depth = pass.depth_stencil.unwrap();
        assert_eq!(depth.depth_write_enabled, Some(false));
        assert_eq!(depth.depth_compare, Some(wgpu::CompareFunction::Always));
        assert_eq!(depth.bias.constant, -3);
        assert_eq!(pass.multisample.count, 4);
        // A null technique changes blend but keeps the native packed declaration.
        stream.bind_effect(None, Declaration::Sprite).unwrap();
        assert!(matches!(
            stream.program,
            Some((_, Layout::Packed(Declaration::Text)))
        ));
    }
}
