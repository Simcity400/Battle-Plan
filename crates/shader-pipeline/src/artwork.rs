//! Isolated native sprite shader output for straight-alpha dashboard images.
//! GUI layout and compositing remain the caller's; programs and samplers come from content.

use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex, OnceLock},
};

use installed_content::{
    ResolvedLayers,
    image::{Rgba8Image, dds, native::Decode},
    pdxshader::program::{self, Stage},
};

use crate::{cache, gpu, library, stream, vertices};

/// Native register bytes and textures, including explicitly null texture binds.
pub struct Draw {
    pub effect: Vec<u8>,
    pub defines: Vec<String>,
    pub size: [u32; 2],
    pub vertices: [[f32; 5]; 4],
    pub constants: Vec<(u32, Vec<f32>, Vec<std::ops::Range<u32>>)>,
    pub textures: Vec<(u32, Option<dds::Texture>)>,
}

struct Renderer {
    device: wgpu::Device,
    queue: wgpu::Queue,
    arena: gpu::Arena,
    cache: Arc<cache::Cache>,
    effects: BTreeMap<String, Arc<library::Effect>>,
    stream: stream::Stream,
}

static RENDERER: OnceLock<Result<Mutex<Renderer>, String>> = OnceLock::new();

/// Executes the authored Up program with the native Sprite declaration and four-record strip.
/// Sprite draws bind b0 to both stages, textures to the pixel stage and a triangle strip.
pub fn render(layers: &ResolvedLayers, draw: Draw, decode: Decode) -> Result<Rgba8Image, String> {
    let source = Arc::new(program::read_reported(layers, &draw.effect)?);
    let technique = source.technique("Up").ok_or_else(|| format!(
        "effect {:?} has no Up technique; a preceding program is unavailable to isolated artwork", source.effect
    ))?;
    let mut defines = globals(layers, decode)?;
    defines.extend(draw.defines.iter().cloned());
    let key = cache::Cache::key(&source, technique, &defines)?;
    let renderer = RENDERER
        .get_or_init(|| Renderer::new().map(Mutex::new))
        .as_ref()
        .map_err(Clone::clone)?;
    let mut renderer = renderer.lock().unwrap_or_else(|error| error.into_inner());
    let effect = if let Some(effect) = renderer.effects.get(&key) {
        effect.clone()
    } else {
        let effect = library::Effect::new(source, defines, renderer.cache.clone());
        renderer.effects.insert(key, effect.clone());
        effect
    };
    let result = renderer.draw(draw, effect);
    for warning in renderer.cache.diagnostics().warnings.into_keys() {
        layers.note_shader_warning(warning);
    }
    result
}

fn globals(layers: &ResolvedLayers, decode: Decode) -> Result<Vec<String>, String> {
    // Map shader constants use the provinces bitmap named by default.map.
    let files = installed_content::terrain::read_default_map(layers).map_err(|e| e.to_string())?;
    let path = files.file("provinces").map_err(|e| e.to_string())?;
    let bytes = layers.read(&path).map_err(|e| e.to_string())?;
    let size = installed_content::image::bmp::rgb_dimensions(&bytes).map_err(|e| e.to_string())?;
    Ok(library::globals(size, decode))
}

impl Renderer {
    fn new() -> Result<Self, String> {
        let instance =
            wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle_from_env());
        let adapter = pollster::block_on(instance.request_adapter(&Default::default()))
            .map_err(|e| e.to_string())?;
        let limits = wgpu::Limits {
            max_texture_dimension_2d: adapter.limits().max_texture_dimension_2d,
            max_dynamic_uniform_buffers_per_pipeline_layout: adapter
                .limits()
                .max_dynamic_uniform_buffers_per_pipeline_layout,
            ..Default::default()
        };
        let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            required_features: adapter.features()
                & (wgpu::Features::ADDRESS_MODE_CLAMP_TO_BORDER
                    | wgpu::Features::DUAL_SOURCE_BLENDING
                    | wgpu::Features::TEXTURE_COMPRESSION_BC
                    | wgpu::Features::FLOAT32_FILTERABLE),
            required_limits: limits,
            ..Default::default()
        }))
        .map_err(|e| e.to_string())?;
        Ok(Self {
            arena: gpu::Arena::new(&device),
            device,
            queue,
            cache: Arc::new(cache::Cache::default()),
            effects: BTreeMap::new(),
            stream: stream::Stream::default(),
        })
    }

    fn draw(&mut self, draw: Draw, effect: Arc<library::Effect>) -> Result<Rgba8Image, String> {
        let memory = self.device.push_error_scope(wgpu::ErrorFilter::OutOfMemory);
        let validation = self.device.push_error_scope(wgpu::ErrorFilter::Validation);
        let result = self.draw_inner(draw, effect);
        match pollster::block_on(validation.pop()).or(pollster::block_on(memory.pop())) {
            Some(error) => Err(format!("artwork GPU: {error}")),
            None => result,
        }
    }

    fn draw_inner(
        &mut self,
        draw: Draw,
        effect: Arc<library::Effect>,
    ) -> Result<Rgba8Image, String> {
        let selection = effect.select("Up")?;
        let index = selection.as_ref().ok_or("missing Up technique")?.index;
        self.stream.invalidate_bindings();
        self.stream
            .bind_effect(selection, vertices::Declaration::Sprite)?;
        // A PNG carries the shader's straight-alpha output; the image's host composites it.
        self.stream.bind_blend(None, wgpu::ColorWrites::ALL);
        for (slot, values, known) in &draw.constants {
            for stage in [Stage::Vertex, Stage::Pixel] {
                self.stream.bind_partial_constants(
                    *slot,
                    stage,
                    Some(bytemuck::cast_slice(values)),
                    known,
                );
            }
        }
        let samplers =
            gpu::State::authored(&effect.source, &effect.source.techniques[index])?.samplers;
        for (slot, image) in draw.textures {
            let view = image.map(|image| self.upload(image)).transpose()?;
            self.stream.bind_texture(
                slot,
                Stage::Pixel,
                view,
                samplers.get(&crate::binding(slot, Stage::Pixel)),
            );
        }
        let submission = self.stream.prepare(
            &self.arena,
            &effect.source.effect,
            bytemuck::cast_slice(&draw.vertices),
            20,
            4,
            wgpu::PrimitiveTopology::TriangleStrip,
            wgpu::TextureFormat::Rgba8Unorm,
            1,
        )?;
        self.readback(draw.size, |pass| {
            submission.pipeline.draw(pass, &submission.draw)
        })
    }

    fn upload(&self, image: dds::Texture) -> Result<wgpu::TextureView, String> {
        use dds::TextureFormat as F;
        let format = match image.format {
            F::Rgba8 => wgpu::TextureFormat::Rgba8Unorm,
            F::Bc1 => wgpu::TextureFormat::Bc1RgbaUnorm,
            F::Bc2 => wgpu::TextureFormat::Bc2RgbaUnorm,
            F::Bc3 => wgpu::TextureFormat::Bc3RgbaUnorm,
            F::Bc7 { srgb: false } => wgpu::TextureFormat::Bc7RgbaUnorm,
            F::Bc7 { srgb: true } => wgpu::TextureFormat::Bc7RgbaUnormSrgb,
            F::B5G6R5 | F::B5G5R5A1 => wgpu::TextureFormat::Rgba32Float,
        };
        if format.is_compressed()
            && !self
                .device
                .features()
                .contains(wgpu::Features::TEXTURE_COMPRESSION_BC)
        {
            return Err("artwork adapter lacks native BC texture sampling".into());
        }
        let texture = self.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("native artwork texture"),
            size: wgpu::Extent3d {
                width: image.width,
                height: image.height,
                depth_or_array_layers: 1,
            },
            mip_level_count: image.mips[0].len() as u32,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        for (level, mip) in image.mips[0].iter().enumerate() {
            let (bw, bh) = format.block_dimensions();
            // The backend has no B5G6R5/B5G5R5A1 format. Float expansion keeps
            // native UNORM sample precision, rather than quantizing to RGBA8.
            let expanded: Vec<f32> = if image.format.sample_16(0).is_some() {
                mip.data
                    .chunks_exact(2)
                    .flat_map(|word| {
                        image
                            .format
                            .sample_16(u16::from_le_bytes([word[0], word[1]]))
                            .unwrap()
                    })
                    .collect()
            } else {
                Vec::new()
            };
            let bytes = if expanded.is_empty() {
                mip.data.as_slice()
            } else {
                bytemuck::cast_slice(&expanded)
            };
            self.queue.write_texture(
                wgpu::TexelCopyTextureInfo {
                    texture: &texture,
                    mip_level: level as u32,
                    origin: wgpu::Origin3d::ZERO,
                    aspect: wgpu::TextureAspect::All,
                },
                bytes,
                wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(
                        mip.width.div_ceil(bw) * format.block_copy_size(None).unwrap(),
                    ),
                    rows_per_image: None,
                },
                wgpu::Extent3d {
                    width: mip.width.div_ceil(bw) * bw,
                    height: mip.height.div_ceil(bh) * bh,
                    depth_or_array_layers: 1,
                },
            );
        }
        Ok(texture.create_view(&Default::default()))
    }

    fn readback(
        &self,
        [width, height]: [u32; 2],
        draw: impl FnOnce(&mut wgpu::RenderPass<'_>),
    ) -> Result<Rgba8Image, String> {
        let size = wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        };
        let target = self.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("artwork output"),
            size,
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let pitch = (width * 4).div_ceil(256) * 256;
        let read = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("artwork pixels"),
            size: u64::from(pitch) * u64::from(height),
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let mut encoder = self.device.create_command_encoder(&Default::default());
        {
            let view = target.create_view(&Default::default());
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    resolve_target: None,
                    depth_slice: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                ..Default::default()
            });
            draw(&mut pass);
        }
        encoder.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo {
                texture: &target,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::TexelCopyBufferInfo {
                buffer: &read,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(pitch),
                    rows_per_image: None,
                },
            },
            size,
        );
        self.arena.flush(&self.queue);
        self.queue.submit([encoder.finish()]);
        self.arena.reset();
        let slice = read.slice(..);
        let (sender, receiver) = std::sync::mpsc::channel();
        slice.map_async(wgpu::MapMode::Read, move |result| {
            let _ = sender.send(result);
        });
        self.device
            .poll(wgpu::PollType::wait_indefinitely())
            .map_err(|e| e.to_string())?;
        receiver
            .recv()
            .map_err(|e| e.to_string())?
            .map_err(|e| e.to_string())?;
        let mapped = slice.get_mapped_range().map_err(|e| e.to_string())?;
        let pixels = mapped
            .chunks_exact(pitch as usize)
            .flat_map(|row| row[..width as usize * 4].iter().copied())
            .collect();
        Ok(Rgba8Image {
            width,
            height,
            pixels,
        })
    }
}
