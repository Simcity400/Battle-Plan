
use shader_pipeline::gpu::stats::CountedPasses as _;
use crate::gpu_contract::DEPTH_FORMAT as gpu_contract_DEPTH_FORMAT;
use crate::gpu_contract::OUTPUT_FORMAT as gpu_contract_OUTPUT_FORMAT;
use crate::gpu_contract::SCENE_FORMAT as gpu_contract_SCENE_FORMAT;
use eframe::wgpu;
use std::time::Duration;
use std::{io, sync::mpsc};

pub const MAP_SAMPLED_TEXTURES: u32 = 19;
pub const REFRACTION_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Bgra8Unorm;

pub const DEPTH_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Depth24PlusStencil8;
pub const OUTPUT_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;
pub const SCENE_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;

pub struct Targets {
    scene: wgpu::Texture,
    scene_ms: wgpu::TextureView,
    depth_ms: wgpu::TextureView,
    scene_view: wgpu::TextureView,
    color: wgpu::Texture,
    color_view: wgpu::TextureView,
    pub refraction_view: wgpu::TextureView,
    refraction_depth: wgpu::TextureView,
    refraction_content: crate::textures::Content,
    sea_bed: SeaBed,
    size: [u32; 2],
    world_size: [u32; 2],
    samples: u32,
}

impl Targets {
    pub fn new(
        device: &wgpu::Device,
        size: [u32; 2],
        world_size: [u32; 2],
        samples: u32,
        sea_bed: SeaBed,
    ) -> Self {
        let attachment = |label, size, format, samples, usage| {
            device.create_texture(&wgpu::TextureDescriptor {
                label: Some(label),
                size: extent(size),
                mip_level_count: 1,
                sample_count: samples,
                dimension: wgpu::TextureDimension::D2,
                format,
                usage,
                view_formats: &[],
            })
        };
        let scene_ms = (samples > 1).then(|| {
            attachment(
                "map scene (multisampled float)",
                world_size,
                SCENE_FORMAT,
                samples,
                wgpu::TextureUsages::RENDER_ATTACHMENT,
            )
        });
        let depth_ms = attachment(
            "map depth (multisampled)",
            world_size,
            DEPTH_FORMAT,
            samples,
            wgpu::TextureUsages::RENDER_ATTACHMENT,
        );
        let scene = attachment(
            "map scene (float)",
            world_size,
            SCENE_FORMAT,
            1,
            wgpu::TextureUsages::RENDER_ATTACHMENT
                | wgpu::TextureUsages::TEXTURE_BINDING
                | wgpu::TextureUsages::COPY_SRC,
        );
        let color = attachment(
            "map output",
            size,
            OUTPUT_FORMAT,
            1,
            wgpu::TextureUsages::RENDER_ATTACHMENT
                | wgpu::TextureUsages::TEXTURE_BINDING
                | wgpu::TextureUsages::COPY_SRC,
        );
        let refraction = attachment(
            "water refraction",
            sea_bed.size(size),
            REFRACTION_FORMAT,
            1,
            wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::RENDER_ATTACHMENT,
        );
        let refraction_depth = attachment(
            "water refraction depth",
            sea_bed.size(size),
            DEPTH_FORMAT,
            1,
            wgpu::TextureUsages::RENDER_ATTACHMENT,
        );
        Self {
            scene_ms: scene_ms
                .as_ref()
                .unwrap_or(&scene)
                .create_view(&Default::default()),
            depth_ms: depth_ms.create_view(&Default::default()),
            scene_view: scene.create_view(&Default::default()),
            scene,
            color_view: color.create_view(&Default::default()),
            color,
            refraction_view: refraction.create_view(&Default::default()),
            refraction_depth: refraction_depth.create_view(&Default::default()),
            refraction_content: crate::textures::Content::next(),
            sea_bed,
            size,
            world_size,
            samples,
        }
    }

    pub fn scene_attachment(
        &self,
        load: wgpu::LoadOp<wgpu::Color>,
    ) -> wgpu::RenderPassColorAttachment<'_> {
        wgpu::RenderPassColorAttachment {
            view: if self.samples == 1 {
                &self.scene_view
            } else {
                &self.scene_ms
            },
            depth_slice: None,
            resolve_target: None,
            ops: wgpu::Operations {
                load,
                store: wgpu::StoreOp::Store,
            },
        }
    }

    pub fn output_attachment(
        &self,
        load: wgpu::LoadOp<wgpu::Color>,
    ) -> wgpu::RenderPassColorAttachment<'_> {
        wgpu::RenderPassColorAttachment {
            view: &self.color_view,
            depth_slice: None,
            resolve_target: None,
            ops: wgpu::Operations {
                load,
                store: wgpu::StoreOp::Store,
            },
        }
    }

    pub fn resolve_scene(&self, encoder: &mut wgpu::CommandEncoder) {
        if self.samples == 1 {
            return;
        }
        drop(encoder.counted_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("map scene resolve"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &self.scene_ms,
                depth_slice: None,
                resolve_target: Some(&self.scene_view),
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Load,
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        }));
    }

    pub fn depth_attachment(
        &self,
        load: wgpu::LoadOp<f32>,
    ) -> wgpu::RenderPassDepthStencilAttachment<'_> {
        wgpu::RenderPassDepthStencilAttachment {
            view: &self.depth_ms,
            depth_ops: Some(wgpu::Operations {
                load,
                store: wgpu::StoreOp::Store,
            }),
            stencil_ops: Some(wgpu::Operations {
                load: match load {
                    wgpu::LoadOp::Clear(_) => wgpu::LoadOp::Clear(0),
                    wgpu::LoadOp::Load => wgpu::LoadOp::Load,
                    wgpu::LoadOp::DontCare(value) => wgpu::LoadOp::DontCare(value),
                },
                store: wgpu::StoreOp::Store,
            }),
        }
    }

    pub fn size(&self) -> [u32; 2] {
        self.size
    }

    pub fn samples(&self) -> u32 {
        self.samples
    }

    pub fn world_size(&self) -> [u32; 2] {
        self.world_size
    }

    pub fn sea_bed(&self) -> SeaBed {
        self.sea_bed
    }

    pub fn refraction_content(&self) -> crate::textures::Content {
        self.refraction_content
    }

    pub fn refraction_attachment(&self) -> wgpu::RenderPassColorAttachment<'_> {
        wgpu::RenderPassColorAttachment {
            view: &self.refraction_view,
            depth_slice: None,
            resolve_target: None,
            ops: wgpu::Operations {
                load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                store: wgpu::StoreOp::Store,
            },
        }
    }

    pub fn refraction_depth_attachment(&self) -> wgpu::RenderPassDepthStencilAttachment<'_> {
        wgpu::RenderPassDepthStencilAttachment {
            view: &self.refraction_depth,
            depth_ops: Some(wgpu::Operations {
                load: wgpu::LoadOp::Clear(1.0),
                store: wgpu::StoreOp::Discard,
            }),
            stencil_ops: None,
        }
    }

    pub fn scene_view(&self) -> &wgpu::TextureView {
        &self.scene_view
    }

    pub fn scene_texture(&self) -> &wgpu::Texture {
        &self.scene
    }

    pub fn output(&self) -> &wgpu::Texture {
        &self.color
    }

    pub fn output_view(&self) -> &wgpu::TextureView {
        &self.color_view
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SeaBed {
    Half,
    Full,
}

impl SeaBed {
    pub fn size(self, output: [u32; 2]) -> [u32; 2] {
        match self {
            Self::Half => output.map(|side| side.div_ceil(2)),
            Self::Full => output,
        }
    }
}

pub fn extent(size: [u32; 2]) -> wgpu::Extent3d {
    wgpu::Extent3d {
        width: size[0],
        height: size[1],
        depth_or_array_layers: 1,
    }
}

pub fn texture_entry(
    binding: u32,
    visibility: wgpu::ShaderStages,
    dimension: wgpu::TextureViewDimension,
) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility,
        ty: wgpu::BindingType::Texture {
            sample_type: wgpu::TextureSampleType::Float { filterable: true },
            view_dimension: dimension,
            multisampled: false,
        },
        count: None,
    }
}

pub fn sampler_entry(binding: u32) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::FRAGMENT,
        ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
        count: None,
    }
}

pub fn buffer(
    device: &wgpu::Device,
    label: &str,
    usage: wgpu::BufferUsages,
    data: &[u8],
) -> io::Result<wgpu::Buffer> {
    let size = data.len().max(wgpu::COPY_BUFFER_ALIGNMENT as usize) as u64;
    let size = size.next_multiple_of(wgpu::COPY_BUFFER_ALIGNMENT);
    if size > device.limits().max_buffer_size {
        return Err(invalid(format!(
            "{label}: {size}-byte buffer exceeds GPU limit {}",
            device.limits().max_buffer_size
        )));
    }
    let buffer = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some(label),
        size,
        usage,
        mapped_at_creation: true,
    });
    let mut view = buffer
        .slice(..)
        .get_mapped_range_mut()
        .map_err(|error| invalid(format!("{label} could not be filled: {error}")))?;
    view.slice(..data.len()).copy_from_slice(data);
    drop(view);
    buffer.unmap();
    Ok(buffer)
}

pub fn validate_size(size: [u32; 2], limit: u32) -> io::Result<()> {
    if size
        .into_iter()
        .any(|dimension| dimension == 0 || dimension > limit)
    {
        Err(invalid(format!(
            "render dimensions {}x{} must be nonzero and no larger than the GPU limit {limit}",
            size[0], size[1]
        )))
    } else {
        Ok(())
    }
}

pub fn invalid(message: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, message.into())
}

pub fn unsupported(message: impl std::fmt::Display) -> io::Error {
    io::Error::new(
        io::ErrorKind::Unsupported,
        format!("UnsupportedPoliticalCompute: {message}"),
    )
}

pub fn compute_limits(limits: &wgpu::Limits) -> bool {
    limits.max_compute_workgroups_per_dimension != 0
        && limits.max_storage_buffers_per_shader_stage >= 8
        && limits.max_compute_invocations_per_workgroup >= 64
}

pub const SOFTWARE_RENDERER_NOTICE: &str = "Software renderer: no graphics card that can run the \
     map was found, so it is drawn on the CPU and will be slow.";

#[derive(Clone, Debug)]
pub struct AdapterCandidate {
    pub name: String,
    pub device_type: wgpu::DeviceType,
    pub backend: wgpu::Backend,
    pub compute: bool,
    pub textures: bool,
    pub presents: bool,
}

impl AdapterCandidate {
    pub fn of(adapter: &wgpu::Adapter, surface: Option<&wgpu::Surface<'_>>) -> Self {
        let info = adapter.get_info();
        let limits = adapter.limits();
        Self {
            name: info.name,
            device_type: info.device_type,
            backend: info.backend,
            compute: adapter
                .get_downlevel_capabilities()
                .flags
                .contains(wgpu::DownlevelFlags::COMPUTE_SHADERS)
                && compute_limits(&limits),
            textures: adapter
                .features()
                .contains(wgpu::Features::TEXTURE_COMPRESSION_BC)
                && limits.max_sampled_textures_per_shader_stage >= MAP_SAMPLED_TEXTURES,
            presents: surface.is_none_or(|surface| adapter.is_surface_supported(surface)),
        }
    }

    fn usable(&self) -> bool {
        self.compute && self.textures && self.presents
    }
}

pub fn choose_adapter(
    candidates: &[AdapterCandidate],
    power: wgpu::PowerPreference,
) -> io::Result<usize> {
    let rank = |device_type| match (power, device_type) {
        (wgpu::PowerPreference::None, _) => 0,
        (wgpu::PowerPreference::LowPower, wgpu::DeviceType::IntegratedGpu) => 0,
        (wgpu::PowerPreference::LowPower, wgpu::DeviceType::DiscreteGpu) => 1,
        (_, wgpu::DeviceType::DiscreteGpu) => 0,
        (_, wgpu::DeviceType::IntegratedGpu) => 1,
        (_, wgpu::DeviceType::Other) => 2,
        (_, wgpu::DeviceType::VirtualGpu) => 3,
        (_, wgpu::DeviceType::Cpu) => 4,
    };
    let usable = || candidates.iter().enumerate().filter(|(_, c)| c.usable());
    usable()
        .filter(|(_, candidate)| candidate.device_type != wgpu::DeviceType::Cpu)
        .min_by_key(|&(index, candidate)| (rank(candidate.device_type), index))
        .or_else(|| usable().find(|(_, c)| c.device_type == wgpu::DeviceType::Cpu))
        .map(|(index, _)| index)
        .ok_or_else(|| {
            let seen = candidates
                .iter()
                .map(|c| {
                    let reason = if !c.compute {
                        "no compute shaders".to_string()
                    } else if !c.textures {
                        format!(
                            "no BC texture compression or fewer than {MAP_SAMPLED_TEXTURES} \
                             sampled textures per stage"
                        )
                    } else if !c.presents {
                        "cannot present to the window".to_string()
                    } else {
                        "usable".to_string()
                    };
                    format!(
                        "{} ({:?}, {:?}): {reason}",
                        c.name, c.backend, c.device_type
                    )
                })
                .collect::<Vec<_>>();
            unsupported(format!(
                "no graphics adapter that can run the map was found, not even the platform's \
                 software renderer (adapters: {}). A DirectX 12 or Vulkan graphics device is \
                 required.",
                if seen.is_empty() {
                    "none".into()
                } else {
                    seen.join("; ")
                }
            ))
        })
}

pub fn select_adapter(
    adapters: &[wgpu::Adapter],
    surface: Option<&wgpu::Surface<'_>>,
) -> io::Result<wgpu::Adapter> {
    let candidates = adapters
        .iter()
        .map(|adapter| AdapterCandidate::of(adapter, surface))
        .collect::<Vec<_>>();
    let power = wgpu::PowerPreference::from_env().unwrap_or(wgpu::PowerPreference::HighPerformance);
    let index = choose_adapter(&candidates, power)?;
    let chosen = &candidates[index];
    if chosen.device_type == wgpu::DeviceType::Cpu {
        eprintln!(
            "{SOFTWARE_RENDERER_NOTICE} Adapter: {} ({:?}).",
            chosen.name, chosen.backend
        );
    }
    Ok(adapters[index].clone())
}

pub fn is_software(adapter: &wgpu::Adapter) -> bool {
    adapter.get_info().device_type == wgpu::DeviceType::Cpu
}

pub fn device_requirements(features: wgpu::Features) -> Vec<String> {
    let mut missing = Vec::new();
    if !features.contains(wgpu::Features::ADDRESS_MODE_CLAMP_TO_BORDER) {
        missing.push(
            "the adapter lacks ADDRESS_MODE_CLAMP_TO_BORDER: compiled effects whose samplers \
             address with Border are not drawn on this device (a viewer hardware requirement, \
             not missing replay data)"
                .to_owned(),
        );
    }
    if !features.contains(wgpu::Features::DUAL_SOURCE_BLENDING) {
        missing.push(
            "the adapter lacks DUAL_SOURCE_BLENDING: compiled effects whose blend state reads \
             the second source colour are not drawn on this device (a viewer hardware \
             requirement, not missing replay data)"
                .to_owned(),
        );
    }
    missing
}

pub fn device_descriptor(adapter: &wgpu::Adapter) -> wgpu::DeviceDescriptor<'static> {
    let limits = adapter.limits();
    let compute = adapter
        .get_downlevel_capabilities()
        .flags
        .contains(wgpu::DownlevelFlags::COMPUTE_SHADERS)
        && limits.max_storage_buffers_per_shader_stage >= 8
        && limits.max_compute_invocations_per_workgroup >= 64;
    let base_limits = if compute {
        wgpu::Limits::default()
    } else {
        wgpu::Limits::downlevel_webgl2_defaults()
    };
    crate::atlas_pages::record_device_limit(limits.max_texture_dimension_2d);
    static NAMED: std::sync::Once = std::sync::Once::new();
    NAMED.call_once(|| {
        for requirement in device_requirements(adapter.features()) {
            eprintln!("Warning: {requirement}");
        }
    });
    let mut wanted = wgpu::Features::TEXTURE_COMPRESSION_BC
        | wgpu::Features::TEXTURE_ADAPTER_SPECIFIC_FORMAT_FEATURES
        | wgpu::Features::ADDRESS_MODE_CLAMP_TO_BORDER
        | wgpu::Features::DUAL_SOURCE_BLENDING
        | wgpu::Features::FLOAT32_FILTERABLE;
    if crate::gpu_passes::requested() || crate::gpu_passes::recorded() {
        wanted |= crate::gpu_passes::features();
    }
    if crate::gpu_frame_time::requested() {
        wanted |= crate::gpu_frame_time::features();
    }
    let features = adapter.features() & wanted;
    wgpu::DeviceDescriptor {
        label: Some("BattlePlan viewer"),
        required_features: features,
        required_limits: required_limits(&limits, base_limits),
        ..Default::default()
    }
}

fn required_limits(limits: &wgpu::Limits, base: wgpu::Limits) -> wgpu::Limits {
    wgpu::Limits {
        max_buffer_size: limits.max_buffer_size,
        max_storage_buffer_binding_size: limits.max_storage_buffer_binding_size,
        max_texture_dimension_1d: limits.max_texture_dimension_1d,
        max_texture_dimension_2d: limits.max_texture_dimension_2d,
        max_sampled_textures_per_shader_stage: limits.max_sampled_textures_per_shader_stage,
        max_samplers_per_shader_stage: limits.max_samplers_per_shader_stage,
        max_dynamic_uniform_buffers_per_pipeline_layout: limits.max_dynamic_uniform_buffers_per_pipeline_layout,
        max_dynamic_storage_buffers_per_pipeline_layout: limits.max_dynamic_storage_buffers_per_pipeline_layout,
        max_storage_buffers_per_shader_stage: limits.max_storage_buffers_per_shader_stage,
        ..base
    }
}

pub fn map_device_descriptor(adapter: &wgpu::Adapter) -> wgpu::DeviceDescriptor<'static> {
    let descriptor = device_descriptor(adapter);
    dashboard_core::map_settings::set_sample_counts(sample_counts(
        adapter,
        descriptor.required_features,
    ));
    descriptor
}

pub fn sample_counts(adapter: &wgpu::Adapter, features: wgpu::Features) -> Vec<u32> {
    [1, 2, 4, 8]
        .into_iter()
        .filter(|&count| {
            [
                gpu_contract_SCENE_FORMAT,
                gpu_contract_OUTPUT_FORMAT,
                gpu_contract_DEPTH_FORMAT,
            ]
            .into_iter()
            .all(|format| {
                if format == gpu_contract_OUTPUT_FORMAT && count > 1 {
                    return true;
                }
                let flags = if features
                    .contains(wgpu::Features::TEXTURE_ADAPTER_SPECIFIC_FORMAT_FEATURES)
                {
                    adapter.get_texture_format_features(format).flags
                } else {
                    format.guaranteed_format_features(features).flags
                };
                flags.sample_count_supported(count)
                    && (count == 1
                        || format == gpu_contract_DEPTH_FORMAT
                        || flags.contains(wgpu::TextureFormatFeatureFlags::MULTISAMPLE_RESOLVE))
            })
        })
        .collect()
}

pub fn scoped<T>(
    device: &wgpu::Device,
    what: impl FnOnce() -> String,
    work: impl FnOnce() -> io::Result<T>,
) -> io::Result<T> {
    let memory = device.push_error_scope(wgpu::ErrorFilter::OutOfMemory);
    let validation = device.push_error_scope(wgpu::ErrorFilter::Validation);
    let result = work();
    match pollster::block_on(validation.pop()).or(pollster::block_on(memory.pop())) {
        Some(error) => Err(invalid(format!("{}: {error}", what()))),
        None => result,
    }
}

pub fn read_texture(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    texture: &wgpu::Texture,
    texel_bytes: u32,
) -> io::Result<Vec<u8>> {
    scoped(
        device,
        || "The graphics device could not read this map back".to_owned(),
        || read_texture_unscoped(device, queue, texture, texel_bytes),
    )
}

fn read_texture_unscoped(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    texture: &wgpu::Texture,
    texel_bytes: u32,
) -> io::Result<Vec<u8>> {
    let [width, height] = [texture.width(), texture.height()];
    let row_bytes = width * texel_bytes;
    let padded_row_bytes =
        row_bytes.div_ceil(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT) * wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
    let readback_bytes = u64::from(padded_row_bytes) * u64::from(height);
    if readback_bytes > device.limits().max_buffer_size {
        return Err(invalid(format!(
            "Map readback needs {readback_bytes} bytes; GPU device buffer limit is {}",
            device.limits().max_buffer_size
        )));
    }
    let buffer = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("map texture readback"),
        size: readback_bytes,
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
        label: Some("map texture copy"),
    });
    encoder.copy_texture_to_buffer(
        texture.as_image_copy(),
        wgpu::TexelCopyBufferInfo {
            buffer: &buffer,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(padded_row_bytes),
                rows_per_image: Some(height),
            },
        },
        extent([width, height]),
    );
    let submission = queue.submit([encoder.finish()]);
    let slice = buffer.slice(..);
    let (send, receive) = mpsc::sync_channel(1);
    slice.map_async(wgpu::MapMode::Read, move |result| {
        let _ = send.send(result);
    });
    device
        .poll(wgpu::PollType::Wait {
            submission_index: Some(submission),
            timeout: Some(Duration::from_secs(60)),
        })
        .map_err(io::Error::other)?;
    receive
        .recv_timeout(Duration::from_secs(60))
        .map_err(io::Error::other)?
        .map_err(io::Error::other)?;
    let mapped = slice.get_mapped_range().map_err(io::Error::other)?;
    let mut pixels = Vec::with_capacity(row_bytes as usize * height as usize);
    for row in mapped.chunks_exact(padded_row_bytes as usize) {
        pixels.extend_from_slice(&row[..row_bytes as usize]);
    }
    drop(mapped);
    buffer.unmap();
    Ok(pixels)
}

pub fn compiled<T>(slot: &Result<T, String>) -> io::Result<&T> {
    slot.as_ref()
        .map_err(|reason| io::Error::other(reason.clone()))
}

#[cfg(test)]
mod limit_tests {
    use super::*;

    #[test]
    fn a_device_asks_the_adapter_s_own_storage_buffer_limits() {
        let adapter = wgpu::Limits {
            max_dynamic_storage_buffers_per_pipeline_layout: 16,
            max_storage_buffers_per_shader_stage: 64,
            max_dynamic_uniform_buffers_per_pipeline_layout: 15,
            ..wgpu::Limits::default()
        };
        let asked = required_limits(&adapter, wgpu::Limits::default());
        assert_eq!(asked.max_dynamic_storage_buffers_per_pipeline_layout, 16);
        assert_eq!(asked.max_storage_buffers_per_shader_stage, 64);
        assert_eq!(asked.max_dynamic_uniform_buffers_per_pipeline_layout, 15);
        assert!(wgpu::Limits::default().max_dynamic_storage_buffers_per_pipeline_layout < 8, "the default refused them");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use wgpu::{Backend, DeviceType};

    #[test]
    fn the_sea_bed_target_is_half_the_output_rounded_up_unless_full() {
        assert_eq!(SeaBed::Half.size([1920, 1080]), [960, 540]);
        assert_eq!(SeaBed::Half.size([2049, 1153]), [1025, 577]);
        assert_eq!(SeaBed::Half.size([1, 1]), [1, 1]);
        assert_eq!(SeaBed::Half.size([3, 2]), [2, 1]);
        assert_eq!(SeaBed::Full.size([2049, 1153]), [2049, 1153]);
    }

    #[test]
    fn missing_compiled_effect_states_are_named_as_device_requirements() {
        let all = wgpu::Features::FLOAT32_FILTERABLE
            | wgpu::Features::ADDRESS_MODE_CLAMP_TO_BORDER
            | wgpu::Features::DUAL_SOURCE_BLENDING;
        assert!(device_requirements(all - wgpu::Features::FLOAT32_FILTERABLE).is_empty());
        for (feature, name) in [
            (
                wgpu::Features::ADDRESS_MODE_CLAMP_TO_BORDER,
                "ADDRESS_MODE_CLAMP_TO_BORDER",
            ),
            (wgpu::Features::DUAL_SOURCE_BLENDING, "DUAL_SOURCE_BLENDING"),
        ] {
            let named = device_requirements(all - feature);
            assert_eq!(named.len(), 1, "{named:?}");
            assert!(named[0].contains(name) && named[0].contains("hardware requirement"));
        }
        assert_eq!(device_requirements(wgpu::Features::empty()).len(), 2);
    }

    fn adapter(name: &str, device_type: DeviceType, backend: Backend) -> AdapterCandidate {
        AdapterCandidate {
            name: name.into(),
            device_type,
            backend,
            compute: true,
            textures: true,
            presents: true,
        }
    }

    fn chosen(candidates: &[AdapterCandidate]) -> &str {
        chosen_at(candidates, wgpu::PowerPreference::HighPerformance)
    }

    fn chosen_at(candidates: &[AdapterCandidate], power: wgpu::PowerPreference) -> &str {
        &candidates[choose_adapter(candidates, power).unwrap()].name
    }

    #[test]
    fn the_power_preference_orders_hardware_as_request_adapter_does() {
        let candidates = [
            adapter("warp", DeviceType::Cpu, Backend::Dx12),
            adapter("virtual", DeviceType::VirtualGpu, Backend::Vulkan),
            adapter("discrete", DeviceType::DiscreteGpu, Backend::Vulkan),
            adapter("integrated", DeviceType::IntegratedGpu, Backend::Vulkan),
        ];
        let low = wgpu::PowerPreference::LowPower;
        assert_eq!(chosen_at(&candidates, low), "integrated");
        assert_eq!(chosen_at(&candidates[..3], low), "discrete");
        assert_eq!(
            chosen_at(&candidates, wgpu::PowerPreference::None),
            "virtual"
        );
    }

    #[test]
    fn hardware_follows_high_performance_order_then_enumeration_order() {
        let candidates = [
            adapter("warp", DeviceType::Cpu, Backend::Dx12),
            adapter("integrated", DeviceType::IntegratedGpu, Backend::Vulkan),
            adapter("discrete vulkan", DeviceType::DiscreteGpu, Backend::Vulkan),
            adapter("discrete dx12", DeviceType::DiscreteGpu, Backend::Dx12),
        ];
        assert_eq!(chosen(&candidates), "discrete vulkan");
        assert_eq!(chosen(&candidates[..2]), "integrated");
        let virtual_other = [
            adapter("virtual", DeviceType::VirtualGpu, Backend::Vulkan),
            adapter("other", DeviceType::Other, Backend::Gl),
        ];
        assert_eq!(chosen(&virtual_other), "other");
    }

    #[test]
    fn software_is_used_only_when_no_hardware_adapter_runs_the_map() {
        let mut no_compute = adapter("old card", DeviceType::DiscreteGpu, Backend::Gl);
        no_compute.compute = false;
        let mut elsewhere = adapter("other display", DeviceType::DiscreteGpu, Backend::Vulkan);
        elsewhere.presents = false;
        let mut no_bc = adapter("no bc", DeviceType::DiscreteGpu, Backend::Vulkan);
        no_bc.textures = false;
        let warp = adapter("warp", DeviceType::Cpu, Backend::Dx12);
        assert_eq!(chosen(&[no_compute.clone(), warp.clone()]), "warp");
        assert_eq!(chosen(&[elsewhere.clone(), warp.clone()]), "warp");
        assert_eq!(chosen(&[no_bc, warp.clone()]), "warp");
        assert_eq!(chosen(std::slice::from_ref(&warp)), "warp");
        let lavapipe = adapter("lavapipe", DeviceType::Cpu, Backend::Vulkan);
        assert_eq!(chosen(&[warp.clone(), lavapipe]), "warp");
        let card = adapter("card", DeviceType::IntegratedGpu, Backend::Dx12);
        assert_eq!(chosen(&[warp, no_compute, card]), "card");
    }

    #[test]
    fn no_usable_adapter_is_a_named_refusal() {
        let mut warp = adapter("warp", DeviceType::Cpu, Backend::Dx12);
        warp.compute = false;
        let mut card = adapter("card", DeviceType::DiscreteGpu, Backend::Gl);
        card.compute = false;
        for candidates in [vec![], vec![card, warp]] {
            let error = choose_adapter(&candidates, wgpu::PowerPreference::HighPerformance)
                .unwrap_err()
                .to_string();
            assert!(error.starts_with("UnsupportedPoliticalCompute:"), "{error}");
            assert!(
                error.contains("A DirectX 12 or Vulkan graphics device is required"),
                "{error}"
            );
        }
    }
}
