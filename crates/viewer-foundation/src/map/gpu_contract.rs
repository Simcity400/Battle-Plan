//! Shared map render targets, formats, bindings and geometry upload.

use crate::map::gpu_contract::DEPTH_FORMAT as gpu_contract_DEPTH_FORMAT;
use crate::map::gpu_contract::OUTPUT_FORMAT as gpu_contract_OUTPUT_FORMAT;
use crate::map::gpu_contract::SCENE_FORMAT as gpu_contract_SCENE_FORMAT;
use eframe::wgpu;
use std::time::Duration;
use std::{io, sync::mpsc};

pub const MAP_SAMPLED_TEXTURES: u32 = 19;
pub const REFRACTION_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Bgra8Unorm;

/// Depth buffer format; the engine's exact format does not reach a pixel.
// Scene allocation includes D24S8 depth/stencil.
// The stencil plane is shared by meshes, map arrows and world-space names.
pub const DEPTH_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Depth24PlusStencil8;
// Output colour is gamma-encoded RGB in an UNORM target, matching egui and the game framebuffer. The
// view applies no additional sRGB conversion.
pub const OUTPUT_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;
// colour: the scene the world passes draw into and the `post` stage grades. It
// is high dynamic range because the engine's is, and because the grade is the
// only stage entitled to decide what "white" is.
//
// HDR scene targets use RGBA16_FLOAT; luminance targets use R16_FLOAT. Only disabling the HDR chain
// selects an eight-bit scene.
//
// The floating-point target retains highlights above one until post-processing applies tone
// mapping, including water glints, city lights and snow highlights.
pub const SCENE_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;

/// The two multisampled colour chains, their resolve targets and the
/// separate sea-bed refraction target.
///
/// colour: there are two because the grade sits between them. The world passes
/// draw into the float scene chain ([`SCENE_FORMAT`]); `post` reads the
/// resolved scene and writes the graded frame into the 8-bit output chain
/// ([`OUTPUT_FORMAT`]), which screen-space text and icons then draw over and which
/// egui and the PNG readback take. Both chains use the selected sample count
/// and resolve into their own single-sample texture; the depth buffer belongs
/// to the world passes and is shared by neither overlay nor grade.
/// `draw_map_full_res=no` halves only the scene/depth chain;
/// restore samples it linearly into the full-size output before grading.
pub struct Targets {
    scene: wgpu::Texture,
    scene_ms: wgpu::TextureView,
    depth_ms: wgpu::TextureView,
    scene_view: wgpu::TextureView,
    color_ms: wgpu::TextureView,
    color: wgpu::Texture,
    color_view: wgpu::TextureView,
    pub refraction_view: wgpu::TextureView,
    refraction_depth: wgpu::TextureView,
    size: [u32; 2],
    world_size: [u32; 2],
    samples: u32,
}

impl Targets {
    pub fn new(device: &wgpu::Device, size: [u32; 2], world_size: [u32; 2], samples: u32) -> Self {
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
        let color_ms = (samples > 1).then(|| {
            attachment(
                "map colour (multisampled)",
                size,
                OUTPUT_FORMAT,
                samples,
                wgpu::TextureUsages::RENDER_ATTACHMENT,
            )
        });
        let color = attachment(
            "map output",
            size,
            OUTPUT_FORMAT,
            1,
            wgpu::TextureUsages::RENDER_ATTACHMENT
                | wgpu::TextureUsages::TEXTURE_BINDING
                | wgpu::TextureUsages::COPY_SRC,
        );
        // Water t5 retains the renderer's unscaled dimensions even with a half-size HDR scene.
        let refraction = attachment(
            "water refraction",
            size,
            REFRACTION_FORMAT,
            1,
            wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::RENDER_ATTACHMENT,
        );
        let refraction_depth = attachment(
            "water refraction depth",
            size,
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
            color_ms: color_ms
                .as_ref()
                .unwrap_or(&color)
                .create_view(&Default::default()),
            color_view: color.create_view(&Default::default()),
            color,
            refraction_view: refraction.create_view(&Default::default()),
            refraction_depth: refraction_depth.create_view(&Default::default()),
            size,
            world_size,
            samples,
        }
    }

    /// The multisampled float scene attachment resolving into the scene
    /// texture. Every world pass, that is everything the grade reads, draws
    /// here.
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
            resolve_target: (self.samples > 1).then_some(&self.scene_view),
            ops: wgpu::Operations {
                load,
                store: wgpu::StoreOp::Store,
            },
        }
    }

    /// The multisampled 8-bit output attachment resolving into the output
    /// texture. The grade writes it first and screen-space text and icons draw
    /// over it; each of them resolves, so the output texture holds the frame
    /// whichever of them ran last.
    pub fn output_attachment(
        &self,
        load: wgpu::LoadOp<wgpu::Color>,
    ) -> wgpu::RenderPassColorAttachment<'_> {
        wgpu::RenderPassColorAttachment {
            view: if self.samples == 1 {
                &self.color_view
            } else {
                &self.color_ms
            },
            depth_slice: None,
            resolve_target: (self.samples > 1).then_some(&self.color_view),
            ops: wgpu::Operations {
                load,
                store: wgpu::StoreOp::Store,
            },
        }
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
                // Clear colour, depth and stencil before the map. Stencil starts at zero and subsequent passes
                // retain it.
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

    /// The scene and depth attachments' sample count, which every world pipeline matches.
    pub fn samples(&self) -> u32 {
        self.samples
    }

    /// HDR scene and depth dimensions; output, GUI and refraction keep [`Self::size`].
    pub fn world_size(&self) -> [u32; 2] {
        self.world_size
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

    /// The resolved float scene, which is what `post` grades. Egui
    /// and the PNG readback take [`Targets::output`].
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

pub fn extent(size: [u32; 2]) -> wgpu::Extent3d {
    wgpu::Extent3d {
        width: size[0],
        height: size[1],
        depth_or_array_layers: 1,
    }
}

/// A sampled-texture entry for a bind group layout.
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

/// A filtering-sampler entry for a bind group layout.
pub fn sampler_entry(binding: u32) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::FRAGMENT,
        ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
        count: None,
    }
}

/// Creates a buffer already holding `data`.
///
/// Shared by every pass that uploads geometry once and never rewrites it.
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

/// Whether these limits run the kernel: eight storage buffers and 64-thread workgroups.
pub fn compute_limits(limits: &wgpu::Limits) -> bool {
    limits.max_compute_workgroups_per_dimension != 0
        && limits.max_storage_buffers_per_shader_stage >= 8
        && limits.max_compute_invocations_per_workgroup >= 64
}

/// Shown over every map drawn on the platform's software adapter.
pub const SOFTWARE_RENDERER_NOTICE: &str = "Software renderer: no graphics card that can run the \
     map was found, so it is drawn on the CPU and will be slow.";

/// What adapter selection reads of one adapter.
#[derive(Clone, Debug)]
pub struct AdapterCandidate {
    pub name: String,
    pub device_type: wgpu::DeviceType,
    pub backend: wgpu::Backend,
    /// Compute shaders with the political kernel's storage buffers and workgroups.
    pub compute: bool,
    /// BC texture compression and [`MAP_SAMPLED_TEXTURES`] per stage, which the world scene
    /// needs.
    pub textures: bool,
    /// Presents to the window's surface; always `true` headless.
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

/// The adapter order the window, the dashboard maps and `--dump` share. First the hardware
/// adapter `request_adapter` picks at `power` among those that run the map: at high
/// performance discrete, integrated, other, virtual; at low power integrated first; with no
/// preference, enumeration order. Enumeration order breaks ties. Then the platform's software
/// adapter, the one `force_fallback_adapter` selects: WARP on DirectX 12, lavapipe on Vulkan.
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

/// Choose from enumerated adapters by [`choose_adapter`], naming a software choice on stderr.
pub fn select_adapter(
    adapters: &[wgpu::Adapter],
    surface: Option<&wgpu::Surface<'_>>,
) -> io::Result<wgpu::Adapter> {
    let candidates = adapters
        .iter()
        .map(|adapter| AdapterCandidate::of(adapter, surface))
        .collect::<Vec<_>>();
    // `WGPU_POWER_PREF` as egui-wgpu and `request_adapter` honour it.
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

/// Whether a map on this adapter carries [`SOFTWARE_RENDERER_NOTICE`].
pub fn is_software(adapter: &wgpu::Adapter) -> bool {
    adapter.get_info().device_type == wgpu::DeviceType::Cpu
}

/// The optional device features the viewer draws some content exactly with that a device with
/// `features` lacks, each named with what it costs: viewer hardware requirements, not native
/// failures or missing replay data. Named once at start-up ([`device_descriptor`]) and in the
/// diagnostics (the App's warnings, the details output and the headless dump's).
pub fn device_requirements(features: wgpu::Features) -> Vec<String> {
    let mut missing = Vec::new();
    // The compiled GUI and map effects' own states (`shader_pipeline::gpu`): a program the
    // device cannot build is refused by name where it is drawn.
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

/// The device the map pipeline needs.
///
/// `TEXTURE_COMPRESSION_BC` is required because the game's atlases, water and
/// border textures are authored as DXT and are uploaded still compressed;
/// [`Scene::new`] reports a named error if the adapter lacks it.
///
/// `max_sampled_textures_per_shader_stage` is the adapter's rather than wgpu's
/// default of 16, because the terrain fragment stage now needs 19: the fifteen
/// the lit pass samples, `ShadowMap`, and the three `fow.fxh` reads —
/// `IntelMap`, `FOWNoise` and `FOWHeight`. The game's own terrain pixel shader
/// declares sixteen samplers on a D3D11 feature-level 11 device, whose
/// guaranteed minimum is 128, so an adapter that can run the map at all has the
/// headroom several times over.
pub fn device_descriptor(adapter: &wgpu::Adapter) -> wgpu::DeviceDescriptor<'static> {
    let limits = adapter.limits();
    // Let Scene issue UnsupportedPoliticalCompute on downlevel adapters, before creating passes.
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
    // Atlases built on the CPU page to this side ([`crate::map::atlas_pages`]).
    crate::map::atlas_pages::record_device_limit(limits.max_texture_dimension_2d);
    // Float32 filtering uses hardware when available, otherwise shader-pipeline lowers it.
    // Clamp-to-border and dual-source blending remain optional device requirements.
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
    // Profiling only: per-pass GPU timestamps (`map::gpu_passes`).
    if crate::map::gpu_passes::requested() {
        wanted |= crate::map::gpu_passes::features();
    }
    let features = adapter.features() & wanted;
    wgpu::DeviceDescriptor {
        label: Some("Battle Plan viewer"),
        required_features: features,
        required_limits: wgpu::Limits {
            max_buffer_size: limits.max_buffer_size,
            max_storage_buffer_binding_size: limits.max_storage_buffer_binding_size,
            max_texture_dimension_1d: limits.max_texture_dimension_1d,
            max_texture_dimension_2d: limits.max_texture_dimension_2d,
            max_sampled_textures_per_shader_stage: limits.max_sampled_textures_per_shader_stage,
            max_samplers_per_shader_stage: limits.max_samplers_per_shader_stage,
            // Compiled programs bind every constant buffer at a dynamic offset into the
            // frame's upload arena; D3D11 allows fourteen per stage.
            max_dynamic_uniform_buffers_per_pipeline_layout: limits
                .max_dynamic_uniform_buffers_per_pipeline_layout,
            ..base_limits
        },
        ..Default::default()
    }
}

/// [`device_descriptor`] for the map renderer's own device: also records the anti-aliasing its
/// adapter supports, which its scenes check and its host advertises. Other devices, such as the
/// game windows' on the host, leave that record alone.
pub fn map_device_descriptor(adapter: &wgpu::Adapter) -> wgpu::DeviceDescriptor<'static> {
    let descriptor = device_descriptor(adapter);
    dashboard_core::map_settings::set_sample_counts(sample_counts(
        adapter,
        descriptor.required_features,
    ));
    descriptor
}

/// The sample counts every map target format supports on `adapter` with `features`, resolve
/// included for the colour targets.
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

pub fn read_texture(
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
mod tests {
    use super::*;
    use wgpu::{Backend, DeviceType};

    /// The compiled effects' optional device states, clamp-to-border sampling and dual-source
    /// blending, are named the same way when the device lacks them: effects that author them
    /// are refused where they are drawn.
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
        // No preference: the first hardware adapter enumerated, never the software one.
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
        // A card lacking the map's texture needs falls back rather than being refused.
        assert_eq!(chosen(&[no_bc, warp.clone()]), "warp");
        assert_eq!(chosen(std::slice::from_ref(&warp)), "warp");
        let lavapipe = adapter("lavapipe", DeviceType::Cpu, Backend::Vulkan);
        assert_eq!(chosen(&[warp.clone(), lavapipe]), "warp");
        // A usable hardware adapter wins wherever it is listed.
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
