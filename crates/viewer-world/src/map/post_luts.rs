
use std::io;

use eframe::wgpu;

use super::{LutProgram, LutSource, PostAssets, invalid};

const LUT_SIZE: [u32; 2] = super::compiled_luts::SIZE;

pub(super) struct LutRenderer {
    compiled: super::compiled_luts::Blends,
    sources: Vec<wgpu::TextureView>,
    intermediate: [wgpu::TextureView; 3],
    last: Option<(LutProgram, LutSource)>,
}

impl LutRenderer {
    pub(super) fn invalidate(&mut self) {
        self.last = None;
    }

    pub(super) fn new(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        assets: &PostAssets,
    ) -> io::Result<Self> {
        if let Some(lut) = assets
            .luts
            .iter()
            .find(|lut| [lut.width, lut.height] != LUT_SIZE)
        {
            return Err(invalid(format!(
                "a post-effect colour cube is {}x{}, not 1024x32",
                lut.width, lut.height
            )));
        }
        let extent = |layers| wgpu::Extent3d {
            width: LUT_SIZE[0],
            height: LUT_SIZE[1],
            depth_or_array_layers: layers,
        };
        let source = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("post source colour cubes"),
            size: extent(assets.luts.len() as u32),
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        let sources = (0..assets.luts.len() as u32)
            .zip(&assets.luts)
            .map(|(layer, lut)| {
                shader_pipeline::gpu::stats::write_texture(
                    &queue,
                    wgpu::TexelCopyTextureInfo {
                        texture: &source,
                        mip_level: 0,
                        origin: wgpu::Origin3d {
                            x: 0,
                            y: 0,
                            z: layer,
                        },
                        aspect: wgpu::TextureAspect::All,
                    },
                    &lut.pixels,
                    wgpu::TexelCopyBufferLayout {
                        offset: 0,
                        bytes_per_row: Some(LUT_SIZE[0] * 4),
                        rows_per_image: Some(LUT_SIZE[1]),
                    },
                    extent(1),
                );
                source.create_view(&wgpu::TextureViewDescriptor {
                    label: Some("post source colour cube"),
                    dimension: Some(wgpu::TextureViewDimension::D2),
                    base_array_layer: layer,
                    array_layer_count: Some(1),
                    ..Default::default()
                })
            })
            .collect();
        let target = || {
            device
                .create_texture(&wgpu::TextureDescriptor {
                    label: Some("post blended colour cube"),
                    size: extent(1),
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: wgpu::TextureDimension::D2,
                    format: wgpu::TextureFormat::Bgra8Unorm,
                    usage: wgpu::TextureUsages::TEXTURE_BINDING
                        | wgpu::TextureUsages::RENDER_ATTACHMENT,
                    view_formats: &[],
                })
                .create_view(&Default::default())
        };
        Ok(Self {
            compiled: super::compiled_luts::Blends::new(device, &assets.lut_shader)?,
            sources,
            intermediate: std::array::from_fn(|_| target()),
            last: None,
        })
    }

    pub(super) fn output(&self) -> &wgpu::TextureView {
        self.view(self.last.expect("LUT program submitted before restore").1)
    }

    fn view(&self, source: LutSource) -> &wgpu::TextureView {
        match source {
            LutSource::Asset(index) => &self.sources[index],
            LutSource::Intermediate(index) => &self.intermediate[index],
        }
    }

    pub(super) fn draw(
        &mut self,
        arena: &shader_pipeline::gpu::Arena,
        encoder: &mut wgpu::CommandEncoder,
        program: LutProgram,
        result: LutSource,
        mark: &mut impl FnMut(&mut wgpu::CommandEncoder, &'static str),
    ) -> io::Result<()> {
        if self.last == Some((program, result)) {
            return Ok(());
        }
        for (index, operation) in program.operations[..program.len].iter().enumerate() {
            self.compiled.draw(
                arena,
                encoder,
                &self.intermediate[index],
                operation.sources.map(|source| self.view(source)),
                operation.factors,
                operation.double,
            )?;
            mark(
                encoder,
                if operation.double {
                    "post LUT double"
                } else {
                    "post LUT single"
                },
            );
        }
        self.last = Some((program, result));
        Ok(())
    }
}
