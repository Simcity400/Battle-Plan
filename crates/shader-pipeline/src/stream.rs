//! Apply native binds in submission order. The owner must feed this after its layer sorting:
//! a missing technique keeps the preceding program and its input declaration, not its uniforms.

use super::{
    binding, gpu,
    library::Selection,
    vertices::{Declaration, Vertices},
};
use installed_content::pdxshader::{
    Address,
    program::{Filter, Sampler, Stage},
};
use std::{collections::BTreeMap, sync::Arc};

pub struct Stream {
    program: Option<(Selection, Declaration)>,
    state: gpu::State,
    constants: BTreeMap<u32, Vec<u8>>,
    textures: BTreeMap<u32, Option<wgpu::TextureView>>,
    null_textures: BTreeMap<super::TextureKind, wgpu::TextureView>,
    pub cache: gpu::Cache,
    pub refused: BTreeMap<String, u64>,
}

impl Default for Stream {
    fn default() -> Self {
        Self {
            program: None,
            state: gpu::State {
                samplers: BTreeMap::new(),
                blend: None,
                write_mask: wgpu::ColorWrites::ALL,
            },
            constants: BTreeMap::new(),
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

impl Stream {
    /// Start a scoped render-to-texture pass with the caller's retained bindings.
    /// Keep this stream's device cache; later changes do not alter the caller.
    pub fn inherit_bindings(&mut self, caller: &Self) {
        self.program = caller.program.clone();
        self.state = caller.state.clone();
        self.constants = caller.constants.clone();
        self.textures = caller.textures.clone();
    }

    /// Independent renderer blend handle (142400090), after program selection. In
    /// particular 14228C8D0 disables background blending then uses alpha MAX for text.
    pub fn bind_blend(&mut self, blend: Option<wgpu::BlendState>, write_mask: wgpu::ColorWrites) {
        self.state.blend = blend;
        self.state.write_mask = write_mask;
    }

    /// A non-native pass (or a new frame) has replaced device state we cannot reconstruct.
    /// Keep compiled pipelines, but never guess that its program was the last native effect.
    pub fn invalidate_bindings(&mut self) {
        self.program = None;
        self.state.samplers.clear();
        self.constants.clear();
        self.textures.clear();
    }

    /// 142400520 retains program/layout on null; 142400090 selects the renderer default blend.
    pub fn bind_effect(
        &mut self,
        selection: Option<Selection>,
        declaration: Declaration,
    ) -> Result<(), String> {
        let (blend, mask) = if let Some(selection) = &selection {
            gpu::blend(
                &selection.effect.source,
                &selection.effect.source.techniques[selection.index],
            )?
        } else {
            (None, wgpu::ColorWrites::ALL)
        };
        self.state.blend = blend;
        self.state.write_mask = mask;
        if let Some(selection) = selection {
            self.program = Some((selection, declaration));
        }
        Ok(())
    }

    /// 1424000F0 does nothing for a null constant-buffer handle; it does not clear the slot.
    pub fn bind_constants(&mut self, slot: u32, stage: Stage, bytes: Option<&[u8]>) {
        if let Some(bytes) = bytes {
            self.constants.insert(binding(slot, stage), bytes.to_vec());
        }
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
            self.state.samplers.insert(key, sampler.clone());
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

    #[allow(clippy::too_many_arguments)]
    pub fn prepare(
        &mut self,
        device: &wgpu::Device,
        label: &str,
        bytes: &[u8],
        stride: usize,
        count: usize,
        format: wgpu::TextureFormat,
        samples: u32,
    ) -> Result<Submission, String> {
        let result = self
            .prepare_inner(device, bytes, stride, count, format, samples)
            .map_err(|reason| format!("native draw {label:?}: {reason}"));
        if let Err(reason) = &result {
            *self.refused.entry(reason.clone()).or_default() += 1;
        }
        result
    }

    fn prepare_inner(
        &mut self,
        device: &wgpu::Device,
        bytes: &[u8],
        stride: usize,
        count: usize,
        format: wgpu::TextureFormat,
        samples: u32,
    ) -> Result<Submission, String> {
        let (selection, declaration) = self.program.as_ref().ok_or(
            "null technique needs the preceding native program, which has not been supplied",
        )?;
        let pipeline = self
            .cache
            .get(device, selection, &self.state, format, samples)?;
        let compiled = selection.effect.compiled(selection.index)?;
        let mut textures = BTreeMap::new();
        for &(key, cube) in compiled
            .vertex
            .textures
            .iter()
            .chain(&compiled.pixel.textures)
        {
            let view = match self.textures.get(&key) {
                Some(Some(view)) => view.clone(),
                Some(None) => self
                    .null_textures
                    .entry(cube)
                    .or_insert_with(|| {
                        // DX11 reads an explicitly null SRV as zero. WebGPU zero-initialises resources.
                        let texture = device.create_texture(&wgpu::TextureDescriptor {
                            label: Some("native null texture"),
                            size: wgpu::Extent3d {
                                width: 1,
                                height: 1,
                                depth_or_array_layers: if cube == super::TextureKind::Cube {
                                    6
                                } else {
                                    1
                                },
                            },
                            mip_level_count: 1,
                            sample_count: 1,
                            dimension: if cube == super::TextureKind::D3 {
                                wgpu::TextureDimension::D3
                            } else {
                                wgpu::TextureDimension::D2
                            },
                            format: wgpu::TextureFormat::Rgba8Unorm,
                            usage: wgpu::TextureUsages::TEXTURE_BINDING,
                            view_formats: &[],
                        });
                        texture.create_view(&wgpu::TextureViewDescriptor {
                            dimension: Some(cube.view()),
                            ..Default::default()
                        })
                    })
                    .clone(),
                None => {
                    return Err(format!(
                        "effect {:?}: native texture binding {key} has not been supplied",
                        selection.effect.source.effect
                    ));
                }
            };
            textures.insert(key, view);
        }
        let draw = pipeline
            .prepare(
                device,
                &self.constants,
                &textures,
                &Vertices {
                    declaration: *declaration,
                    bytes,
                    stride,
                    count,
                },
            )
            .map_err(|reason| format!("effect {:?}: {reason}", selection.effect.source.effect))?;
        Ok(Submission { pipeline, draw })
    }
}
