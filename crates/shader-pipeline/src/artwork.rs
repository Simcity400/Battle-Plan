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
use sha2::{Digest, Sha256};

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
    textures: BTreeMap<[u8; 32], (wgpu::TextureView, usize, u64)>,
    texture_bytes: usize,
    texture_clock: u64,
}

static RENDERER: OnceLock<Result<Mutex<Renderer>, String>> = OnceLock::new();

#[derive(Default)]
struct Inputs {
    map_size: OnceLock<Result<[u32; 2], String>>,
    programs: Mutex<BTreeMap<Vec<u8>, Arc<ParsedProgram>>>,
}

type ParsedProgram = OnceLock<Result<Arc<program::Program>, String>>;

/// Parse each resolved artwork program once, retaining reporting reads and refusals.
pub fn program(layers: &ResolvedLayers, effect: &[u8]) -> Result<Arc<program::Program>, String> {
    let inputs = layers.derived::<Inputs>();
    let slot = {
        let mut programs = inputs
            .programs
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        Arc::clone(programs.entry(effect.to_vec()).or_default())
    };
    slot.get_or_init(|| program::read_reported(layers, effect).map(Arc::new))
        .clone()
}

/// Executes the authored Up program with the native Sprite declaration and four-record strip.
/// Sprite draws bind b0 to both stages, textures to the pixel stage and a triangle strip.
pub fn render(layers: &ResolvedLayers, draw: Draw, decode: Decode) -> Result<Rgba8Image, String> {
    render_batch(layers, vec![draw], decode)?
        .pop()
        .ok_or_else(|| "empty artwork submission".to_owned())
}

fn renderer() -> Result<&'static Mutex<Renderer>, String> {
    RENDERER
        .get_or_init(|| Renderer::new().map(Mutex::new))
        .as_ref()
        .map_err(Clone::clone)
}

fn variant(
    layers: &ResolvedLayers,
    effect: &[u8],
    caller: &[String],
    decode: Decode,
) -> Result<(String, Arc<program::Program>, Vec<String>), String> {
    let source = program(layers, effect)?;
    let technique = source.technique("Up").ok_or_else(|| format!("effect {:?} has no Up technique; a preceding program is unavailable to isolated artwork", source.effect))?;
    let mut defines = globals(layers, decode)?;
    defines.extend(caller.iter().cloned());
    Ok((
        cache::Cache::key(&source, technique, &defines)?,
        source,
        defines,
    ))
}

/// Several independent sprite targets share one submission and one mapped readback.
/// Only preparation/submission uses the renderer lock; waiting and copying do not.
pub fn render_batch(
    layers: &ResolvedLayers,
    draws: Vec<Draw>,
    decode: Decode,
) -> Result<Vec<Rgba8Image>, String> {
    if draws.is_empty() {
        return Ok(Vec::new());
    }
    let variants = draws
        .iter()
        .map(|draw| variant(layers, &draw.effect, &draw.defines, decode))
        .collect::<Result<Vec<_>, _>>()?;
    let job = {
        let mut renderer = renderer()?.lock().unwrap_or_else(|e| e.into_inner());
        let effects = variants
            .into_iter()
            .map(|variant| renderer.effect(variant))
            .collect::<Vec<_>>();
        let result = renderer.submit(draws.into_iter().zip(effects).collect());
        for warning in renderer.cache.diagnostics().warnings.into_keys() {
            layers.note_shader_warning(warning);
        }
        result?
    };
    job.finish()
}

fn globals(layers: &ResolvedLayers, decode: Decode) -> Result<Vec<String>, String> {
    // Map shader constants use the provinces bitmap named by default.map.
    let inputs = layers.derived::<Inputs>();
    let size = inputs
        .map_size
        .get_or_init(|| {
            let files = installed_content::terrain::read_default_map(layers).map_err(|e| e.to_string())?;
            let path = files.file("provinces").map_err(|e| e.to_string())?;
            let (prefix, length) = layers.read_prefix(&path, 138).map_err(|e| e.to_string())?;
            installed_content::image::bmp::rgb_dimensions_prefix(&prefix, length).map_err(|e| e.to_string())
        })
        .clone()?;
    Ok(library::globals(size, decode))
}

fn texture_key(image: &dds::Texture) -> [u8; 32] {
    use dds::TextureFormat as F;
    let mut hash = Sha256::new();
    hash.update([match image.format {
        F::Rgba8 => 0,
        F::Bc1 => 1,
        F::Bc2 => 2,
        F::Bc3 => 3,
        F::Bc7 { srgb: false } => 4,
        F::Bc7 { srgb: true } => 5,
        F::B5G6R5 => 6,
        F::B5G5R5A1 => 7,
    }]);
    hash.update(image.width.to_le_bytes());
    hash.update(image.height.to_le_bytes());
    for mip in &image.mips[0] {
        hash.update(mip.width.to_le_bytes());
        hash.update(mip.height.to_le_bytes());
        hash.update((mip.data.len() as u64).to_le_bytes());
        hash.update(&mip.data);
    }
    hash.finalize().into()
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
            textures: BTreeMap::new(),
            texture_bytes: 0,
            texture_clock: 0,
        })
    }

    fn effect(
        &mut self,
        (key, source, defines): (String, Arc<program::Program>, Vec<String>),
    ) -> Arc<library::Effect> {
        let cache = self.cache.clone();
        self.effects
            .entry(key)
            .or_insert_with(|| library::Effect::new(source, defines, cache))
            .clone()
    }

    fn prepare(
        &mut self,
        draw: Draw,
        effect: Arc<library::Effect>,
    ) -> Result<([u32; 2], stream::Submission), String> {
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
        Ok((draw.size, submission))
    }

    fn upload(&mut self, image: dds::Texture) -> Result<wgpu::TextureView, String> {
        use dds::TextureFormat as F;
        let key = texture_key(&image);
        self.texture_clock += 1;
        if let Some((view, _, last)) = self.textures.get_mut(&key) {
            *last = self.texture_clock;
            return Ok(view.clone());
        }
        let retained = std::mem::size_of::<([u8; 32], (wgpu::TextureView, usize, u64))>()
            + image.mips[0]
                .iter()
                .map(|mip| {
                    if image.format.sample_16(0).is_some() {
                        mip.data.len() * 8
                    } else {
                        mip.data.len()
                    }
                })
                .sum::<usize>();
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
        let view = texture.create_view(&Default::default());
        self.texture_bytes += retained;
        self.textures
            .insert(key, (view.clone(), retained, self.texture_clock));
        while self.texture_bytes > 64 * 1024 * 1024 {
            let oldest = self
                .textures
                .iter()
                .min_by_key(|(_, (_, _, last))| *last)
                .map(|(key, _)| *key)
                .unwrap();
            self.texture_bytes -= self.textures.remove(&oldest).unwrap().1;
        }
        Ok(view)
    }

    fn submit(&mut self, draws: Vec<(Draw, Arc<library::Effect>)>) -> Result<Readback, String> {
        let memory = self.device.push_error_scope(wgpu::ErrorFilter::OutOfMemory);
        let validation = self.device.push_error_scope(wgpu::ErrorFilter::Validation);
        let result = self.submit_inner(draws);
        self.arena.reset();
        self.stream.invalidate_bindings();
        self.stream.cache.release_bind_groups();
        match pollster::block_on(validation.pop()).or(pollster::block_on(memory.pop())) {
            Some(error) => {
                self.textures.clear();
                self.texture_bytes = 0;
                Err(format!("artwork GPU: {error}"))
            }
            None => result,
        }
    }

    fn submit_inner(
        &mut self,
        draws: Vec<(Draw, Arc<library::Effect>)>,
    ) -> Result<Readback, String> {
        let prepared = draws
            .into_iter()
            .map(|(draw, effect)| self.prepare(draw, effect))
            .collect::<Result<Vec<_>, _>>()?;
        let mut offset = 0_u64;
        let mut regions = Vec::new();
        for ([width, height], _) in &prepared {
            let pitch = width
                .checked_mul(4)
                .and_then(|n| n.checked_add(255))
                .ok_or("artwork row overflow")?
                / 256
                * 256;
            let length = u64::from(pitch) * u64::from(*height);
            regions.push(([*width, *height], pitch, offset));
            offset = offset.checked_add(length).ok_or("artwork batch overflow")?;
        }
        let read = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("artwork pixels"),
            size: offset,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let mut encoder = self.device.create_command_encoder(&Default::default());
        for (([width, height], submission), (_, pitch, offset)) in
            prepared.into_iter().zip(&regions)
        {
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
                submission.pipeline.draw(&mut pass, &submission.draw);
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
                        offset: *offset,
                        bytes_per_row: Some(*pitch),
                        rows_per_image: None,
                    },
                },
                size,
            );
        }
        self.arena.flush(&self.queue);
        self.queue.submit([encoder.finish()]);
        Ok(Readback {
            device: self.device.clone(),
            buffer: read,
            regions,
        })
    }
}

struct Readback {
    device: wgpu::Device,
    buffer: wgpu::Buffer,
    regions: Vec<([u32; 2], u32, u64)>,
}
impl Readback {
    fn finish(self) -> Result<Vec<Rgba8Image>, String> {
        let slice = self.buffer.slice(..);
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
        Ok(self
            .regions
            .into_iter()
            .map(|([width, height], pitch, offset)| {
                let pixels = mapped
                    [offset as usize..offset as usize + pitch as usize * height as usize]
                    .chunks_exact(pitch as usize)
                    .flat_map(|row| row[..width as usize * 4].iter().copied())
                    .collect();
                Rgba8Image {
                    width,
                    height,
                    pixels,
                }
            })
            .collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[ignore = "requires DXC and a GPU; synthetic content only"]
    fn batched_targets_match_independent_draws_and_reuse_uploaded_texels() {
        engine::executable_tables::initialize_for_tests()
            .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
        let scratch = crate::Scratch::new().unwrap();
        let root = &scratch.0;
        std::fs::create_dir_all(root.join("gfx/FX")).unwrap();
        std::fs::create_dir(root.join("map")).unwrap();
        std::fs::write(root.join("gfx/FX/synthetic.shader"), "VertexShader={MainCode V [[ float4 main(float3 p:POSITION):SV_POSITION {return float4(p,1);} ]]} PixelShader={Samplers={Source={Index=0 MagFilter=Point MinFilter=Point MipFilter=Point}} MainCode P [[ float4 main():SV_TARGET {return Source_Texture.SampleLevel(Source_Sampler,float2(0.5,0.5),0);} ]]} Effect Up {VertexShader=V PixelShader=P}").unwrap();
        std::fs::write(
            root.join("gfx/FX/requires_constants.shader"),
            r#"VertexShader = { MainCode V [[ float4 main(float3 p:POSITION):SV_POSITION {return float4(p,1);} ]] }
PixelShader = {
ConstantBuffer(0,0) { float4 value;
};
MainCode P [[ float4 main():SV_TARGET {return value;} ]]
}
Effect Up { VertexShader=V PixelShader=P }
"#,
        )
        .unwrap();
        std::fs::write(root.join("gfx/FX/defines_hlsl_dx11.fxh"), "").unwrap();
        std::fs::write(root.join("map/default.map"), "definitions=synthetic.csv provinces=synthetic.bmp terrain=terrain.bmp rivers=rivers.bmp heightmap=height.bmp tree_definition=trees.bmp").unwrap();
        let mut bitmap = vec![0; 58];
        bitmap[..2].copy_from_slice(b"BM");
        for (at, value) in [(10, 54_u32), (14, 40), (18, 1), (22, 1)] {
            bitmap[at..at + 4].copy_from_slice(&value.to_le_bytes());
        }
        bitmap[26..28].copy_from_slice(&1_u16.to_le_bytes());
        bitmap[28..30].copy_from_slice(&24_u16.to_le_bytes());
        std::fs::write(root.join("map/synthetic.bmp"), bitmap).unwrap();
        let layers = ResolvedLayers::from_folders(&[(root, &[])]).unwrap();
        let image = dds::Texture::from_rgba8(Rgba8Image {
            width: 1,
            height: 1,
            pixels: vec![64, 128, 192, 255],
        });
        let draw = |size| Draw {
            effect: b"gfx/FX/synthetic.shader".to_vec(),
            defines: Vec::new(),
            size,
            vertices: [
                [-1.0, -1.0, 0.0, 0.0, 0.0],
                [-1.0, 1.0, 0.0, 0.0, 0.0],
                [1.0, -1.0, 0.0, 0.0, 0.0],
                [1.0, 1.0, 0.0, 0.0, 0.0],
            ],
            constants: Vec::new(),
            textures: vec![(0, Some(image.clone()))],
        };
        let first = render(&layers, draw([2, 3]), Decode::default()).unwrap();
        assert_eq!(first.pixels, [64, 128, 192, 255].repeat(6));
        let (compiled, retained) = {
            let renderer = renderer().unwrap().lock().unwrap();
            (
                renderer.cache.diagnostics().compiled,
                renderer.texture_bytes,
            )
        };
        let second = render(&layers, draw([5, 2]), Decode::default()).unwrap();
        let batch =
            render_batch(&layers, vec![draw([2, 3]), draw([5, 2])], Decode::default()).unwrap();
        assert_eq!(batch, [first.clone(), second]);
        assert_eq!(
            renderer()
                .unwrap()
                .lock()
                .unwrap()
                .cache
                .diagnostics()
                .compiled,
            compiled
        );
        {
            let renderer = renderer().unwrap().lock().unwrap();
            assert_eq!(renderer.texture_bytes, retained);
            assert!(renderer.textures.contains_key(&texture_key(&image)));
            assert_eq!(renderer.stream.cache.retained_texture_bind_groups(), 0);
        }
        let mut refused = draw([2, 3]);
        refused.effect = b"gfx/FX/requires_constants.shader".to_vec();
        let refusal =
            render_batch(&layers, vec![draw([2, 3]), refused], Decode::default()).unwrap_err();
        assert!(refusal.contains("native constant buffer"), "{refusal}");
        assert_eq!(
            renderer()
                .unwrap()
                .lock()
                .unwrap()
                .stream
                .cache
                .retained_texture_bind_groups(),
            0
        );
        assert_eq!(
            render(&layers, draw([2, 3]), Decode::default()).unwrap(),
            first
        );
    }

    #[test]
    fn sprite_inputs_are_read_once_per_resolved_stack_and_keep_reporting_reads() {
        engine::executable_tables::initialize_for_tests()
            .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
        let scratch = crate::Scratch::new().unwrap();
        let root = &scratch.0;
        std::fs::create_dir_all(root.join("gfx/FX")).unwrap();
        std::fs::create_dir(root.join("map")).unwrap();
        std::fs::create_dir(root.join("outside")).unwrap();
        let effect = "outside/synthetic.shader";
        std::fs::write(root.join(effect), "VertexShader={MainCode V [[ float4 main(float3 p:POSITION):SV_POSITION {return float4(p,1);} ]]} PixelShader={MainCode P [[ float4 main():SV_TARGET {return 1;} ]]} Effect Up {VertexShader=V PixelShader=P}").unwrap();
        std::fs::write(root.join("gfx/FX/defines_hlsl_dx11.fxh"), "").unwrap();
        std::fs::write(
            root.join("map/default.map"),
            "definitions=synthetic.csv provinces=synthetic.bmp terrain=terrain.bmp rivers=rivers.bmp heightmap=height.bmp tree_definition=trees.bmp",
        )
        .unwrap();
        let mut bitmap = vec![0; 54 + 24 * 3];
        bitmap[..2].copy_from_slice(b"BM");
        bitmap[10..14].copy_from_slice(&54_u32.to_le_bytes());
        bitmap[14..18].copy_from_slice(&40_u32.to_le_bytes());
        bitmap[18..22].copy_from_slice(&7_i32.to_le_bytes());
        bitmap[22..26].copy_from_slice(&3_i32.to_le_bytes());
        bitmap[26..28].copy_from_slice(&1_u16.to_le_bytes());
        bitmap[28..30].copy_from_slice(&24_u16.to_le_bytes());
        std::fs::write(root.join("map/synthetic.bmp"), bitmap).unwrap();
        let layers = ResolvedLayers::from_folders(&[(root, &[][..])]).unwrap();
        let reads = layers.report_uncovered_reads();
        let first = program(&layers, effect.as_bytes()).unwrap();
        let globals = super::globals(&layers, Decode::default()).unwrap();
        std::fs::write(root.join(effect), "Effect Changed { }").unwrap();
        std::fs::remove_file(root.join("map/synthetic.bmp")).unwrap();
        assert!(Arc::ptr_eq(
            &first,
            &program(&layers.clone(), effect.as_bytes()).unwrap()
        ));
        assert_eq!(
            super::globals(&layers.clone(), Decode::default()).unwrap(),
            globals
        );
        assert!(
            reads
                .warning()
                .is_some_and(|warning| warning.contains(effect))
        );
        let fresh = ResolvedLayers::from_folders(&[(root, &[][..])]).unwrap();
        let _reads = fresh.report_uncovered_reads();
        let replacement = program(&fresh, effect.as_bytes()).unwrap();
        assert!(!Arc::ptr_eq(&first, &replacement));
        assert!(replacement.technique("Up").is_none());
        assert!(replacement.technique("Changed").is_some());
        assert!(super::globals(&fresh, Decode::default()).is_err());
    }
}
