//! Device setup and the outputs of the map pipeline.
//!
//! The window and the headless `--dump` drive the same [`Scene`]; this module
//! only owns the device requirements, the PNG readback, the unverified-content
//! banner and the headless entry point.

use std::{
    fs::File,
    io,
    io::BufWriter,
    path::Path,
    sync::{Arc, mpsc},
};

use eframe::wgpu;

use crate::map::{
    // borders
    border_pass::{BorderAssets, StraitVertex},
    borders::BorderMeshes,
    demilitarized::Zone,
    // close zoom
    map_buildings::{MapBuildingAssets, PreparedBuildings},
    // the map's table frame
    mapframe::FrameAssets,
    // orders: the recorded player's battle plan
    orders::{OrderAssets, OrderField},
    // political fill
    political::PoliticalLayers,
    railways::{Bridges, RailAssets, RailMesh},
    // rivers
    rivers::{RiverAssets, RiverMesh},
    scene::{OUTPUT_FORMAT, Request, Scene, extent, invalid},
    sprawl::Sprawl,
    // names
    sprites::MapIcons,
    textures::{IntelMap, MapAssets},
    // trees
    trees::{TreeAssets, TreeField, TreeGeometry},
    unit_models::UnitModelAssets,
    // weather
    weather::SnowMud,
};

// political fill + borders
/// Everything a headless render needs beyond the terrain and water assets.
pub struct MapLayers<'a> {
    pub content: &'a crate::document::Content,
    pub omitted: crate::map::layers::Omissions,
    /// The hour's compact political region inputs.
    pub political: &'a PoliticalLayers,
    pub border_assets: &'a BorderAssets,
    pub border_meshes: &'a BorderMeshes,
    pub straits: &'a [StraitVertex],
    /// This hour's demilitarised zone; `map::demilitarized`. Empty unless `--demilitarized` is
    /// given, because no recorded table carries the flag.
    pub demilitarized: &'a Zone,
    // rivers
    pub river_assets: &'a RiverAssets,
    pub river_mesh: &'a RiverMesh,
    // weather
    /// The hour's `SnowMudData` grid.
    pub snow_mud: &'a SnowMud,
    // fog of war
    /// The hour's `IntelMap` for the perspective the frame is drawn for.
    pub intel_map: &'a IntelMap,
    // the map's table frame
    pub frame_assets: &'a FrameAssets,
    // close zoom
    pub sprawl: &'a Sprawl,
    pub map_buildings: &'a MapBuildingAssets,
    /// This hour's placed buildings, from [`MapBuildingAssets::field`].
    pub map_building_field: &'a PreparedBuildings,
    /// Cameras visited before this frame; see [`Scene::visit_map_buildings`].
    pub building_cameras: &'a [crate::map::camera::Camera],
    pub rail_assets: &'a RailAssets,
    pub rail_mesh: &'a RailMesh,
    pub aircraft: &'a std::sync::Arc<crate::map::aircraft::AircraftAssets>,
    pub aircraft_plan: &'a crate::map::aircraft::AircraftPlan,
    pub trains: &'a std::sync::Arc<crate::map::trains::TrainAssets>,
    pub train_plan: &'a crate::map::trains::TrainPlan,
    pub bridges: &'a Bridges,
    pub unit_models: &'a UnitModelAssets,
    /// This hour's placed figures, from [`super::map::unit_models::PreparedModels::field`].
    pub unit_model_field: &'a crate::map::unit_history::Field,
    // orders: the recorded player's battle plan
    pub order_assets: &'a OrderAssets,
    /// This hour's ribbons, from [`crate::document::Document::order_field`].
    pub order_field: &'a OrderField,
    pub weather_assets: std::sync::Arc<crate::map::weather_effects::Assets>,
    pub weather_field: crate::map::weather_effects::Field,
    // trees
    pub tree_assets: &'a TreeAssets,
    pub tree_geometry: &'a TreeGeometry,
    pub tree_field: &'a TreeField,
}

pub fn configuration() -> eframe::egui_wgpu::WgpuConfiguration {
    let mut configuration = eframe::egui_wgpu::WgpuConfiguration::default();
    // Profiling only: frame pacing without the display's vsync.
    if std::env::var_os("BATTLE_PLAN_VIEWER_NO_VSYNC").or_else(|| std::env::var_os("HOI4_VIEWER_NO_VSYNC")).is_some() {
        configuration.surface.present_mode = wgpu::PresentMode::AutoNoVsync;
    }
    if let eframe::egui_wgpu::WgpuSetup::CreateNew(setup) = &mut configuration.wgpu_setup {
        setup.device_descriptor = Arc::new(device_descriptor);
        // The window and the dashboard maps it hosts choose like `--dump`.
        setup.native_adapter_selector = Some(Arc::new(|adapters, surface| {
            select_adapter(adapters, surface).map_err(|error| error.to_string())
        }));
    }
    configuration
}

/// Shown over every map drawn on the platform's software adapter.
pub const SOFTWARE_RENDERER_NOTICE: &str = "Software renderer: no graphics card that can run the \
     map was found, so it is drawn on the CPU and will be slow.";

/// Sampled textures one shader stage binds: the terrain fragment stage's fifteen inputs,
/// `ShadowMap` and the three `fow.fxh` reads ([`device_descriptor`]). [`Scene::new`] refuses a
/// device below it, and adapter selection passes over an adapter below it.
pub const MAP_SAMPLED_TEXTURES: u32 = 19;

/// What adapter selection reads of one adapter.
#[derive(Clone, Debug)]
pub struct AdapterCandidate {
    pub name: String,
    pub device_type: wgpu::DeviceType,
    pub backend: wgpu::Backend,
    /// Compute shaders with the political kernel's storage buffers and workgroups.
    pub compute: bool,
    /// BC texture compression and [`MAP_SAMPLED_TEXTURES`] per stage, which [`Scene::new`]
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
                && crate::map::political_gpu::compute_limits(&limits),
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
            crate::map::political_gpu::unsupported(format!(
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

/// Asks, before a headless dump loads its document, for the device descriptor of the adapter
/// the dump will render on ([`dump_png`] selects it the same way), checking adapter support
/// before loading the document.
pub fn probe_dump_device() -> io::Result<()> {
    pollster::block_on(async {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
        let adapters = instance.enumerate_adapters(wgpu::Backends::all()).await;
        let adapter = select_adapter(&adapters, None)?;
        device_descriptor(&adapter);
        Ok(())
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

/// Whether a map on this adapter carries [`SOFTWARE_RENDERER_NOTICE`].
pub fn is_software(adapter: &wgpu::Adapter) -> bool {
    adapter.get_info().device_type == wgpu::DeviceType::Cpu
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
    let sample_mask = [1, 2, 4, 8]
        .into_iter()
        .filter(|&count| {
            [
                crate::map::scene::SCENE_FORMAT,
                crate::map::scene::OUTPUT_FORMAT,
                crate::map::scene::DEPTH_FORMAT,
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
                        || format == crate::map::scene::DEPTH_FORMAT
                        || flags.contains(wgpu::TextureFormatFeatureFlags::MULTISAMPLE_RESOLVE))
            })
        })
        .fold(0, |mask, count| mask | count);
    dashboard::map_settings::set_sample_counts(sample_mask);
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

/// Draws the orange banner over a rendered frame when content is unverified.
pub fn render_banner(
    scene: &Scene,
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    text: &str,
) -> io::Result<()> {
    use eframe::{egui, egui_wgpu};

    let targets = scene
        .targets()
        .ok_or_else(|| invalid("render a map before adding a banner"))?;
    let [width, height] = targets.size();
    let context = egui::Context::default();
    let mut output = context.run_ui(
        egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(width as f32, height as f32),
            )),
            ..Default::default()
        },
        |ui| {
            let painter = ui.painter();
            let galley = painter.layout(
                format!("UNVERIFIED CONTENT\n{text}"),
                egui::FontId::proportional(16.0),
                egui::Color32::BLACK,
                (width as f32 - 24.0).max(1.0),
            );
            painter.rect_filled(
                egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(width as f32, galley.size().y + 20.0),
                ),
                0.0,
                egui::Color32::from_rgb(238, 173, 49),
            );
            painter.galley(egui::pos2(12.0, 10.0), galley, egui::Color32::BLACK);
        },
    );
    let jobs = context.tessellate(output.shapes, output.pixels_per_point);
    let screen = egui_wgpu::ScreenDescriptor {
        size_in_pixels: targets.size(),
        pixels_per_point: output.pixels_per_point,
    };
    let mut renderer = egui_wgpu::Renderer::new(
        device,
        OUTPUT_FORMAT,
        egui_wgpu::RendererOptions {
            dithering: false,
            ..Default::default()
        },
    );
    for (id, deltas) in output.textures_delta.set.drain() {
        for delta in deltas {
            renderer.update_texture(device, queue, id, &delta);
        }
    }
    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
        label: Some("content verification banner"),
    });
    let commands = renderer.update_buffers(device, queue, &mut encoder, &jobs, &screen);
    {
        let mut pass = encoder
            .begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("content verification banner"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: targets.output_view(),
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Load,
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            })
            .forget_lifetime();
        renderer.render(&mut pass, &jobs, &screen);
    }
    queue.submit(commands.into_iter().chain([encoder.finish()]));
    for id in output.textures_delta.free.drain() {
        renderer.free_texture(&id);
    }
    Ok(())
}

/// Reads the rendered frame back and writes it as an 8-bit RGBA PNG.
pub fn write_png(
    scene: &Scene,
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    path: &Path,
) -> io::Result<()> {
    let targets = scene
        .targets()
        .ok_or_else(|| invalid("render a map before saving PNG"))?;
    let [width, height] = targets.size();
    let pixels = read_texture(device, queue, targets.output(), 4)?;
    let mut encoder = png::Encoder::new(BufWriter::new(File::create(path)?), width, height);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    encoder.set_source_srgb(png::SrgbRenderingIntent::Perceptual);
    let mut writer = encoder.write_header().map_err(io::Error::other)?;
    writer.write_image_data(&pixels).map_err(io::Error::other)?;
    writer.finish().map_err(io::Error::other)
}

fn read_texture(
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
            timeout: Some(std::time::Duration::from_secs(60)),
        })
        .map_err(io::Error::other)?;
    receive
        .recv_timeout(std::time::Duration::from_secs(60))
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

/// What one headless render reports back.
pub struct DumpReport {
    pub pass_warnings: Vec<String>,
    pub shader_refusals: std::collections::BTreeMap<String, u64>,
    /// Adapter name and backend, so a dump identifies the GPU it used.
    pub adapter: String,
    /// The device's missing feature requirements ([`device_requirements`]).
    pub device_requirements: Vec<String>,
    /// Wall time of the render call alone, excluding device and texture setup.
    pub render_seconds: f64,
    /// CPU seconds of the render call before the GPU drain wait.
    pub cpu_seconds: f64,
    /// Quads the visible grid held.
    pub quads: u32,
    /// Grid step in map pixels.
    pub step: f32,
    // names
    /// Name-layer vertices and icon quads the frame drew.
    pub overlay: (u32, u32),
    /// Font and sprite pages uploaded for the icon layer.
    pub icon_pages: usize,
    // trees
    /// Canopy instances the frame drew, over every wrap copy.
    pub tree_instances: u32,
    /// Canopy draw calls, one per visible tile.
    pub tree_draw_calls: u32,
    // close zoom
    /// City-sprawl houses the frame drew, over every wrap copy.
    pub sprawl_drawn: u32,
    /// Map buildings the frame drew, over every wrap copy.
    pub map_buildings_drawn: u32,
    /// Unit models the frame drew, over every wrap copy.
    pub unit_models_drawn: u32,
    pub aircraft_drawn: u32,
    // borders
    /// Province boundaries the demilitarised-zone pass drew.
    pub demilitarized_edges: usize,
    // shadows
    /// Whether the frame ran the cast-shadow chain, which it does below
    /// `DRAW_SHADOWS_CUTOFF` and not above it (`map::shadow`).
    pub shadows: bool,
    /// The post grade, when the frame was graded.
    pub grading: Option<String>,
}

/// Renders one frame headlessly through the same [`Scene`] the window uses.
pub fn dump_png(
    assets: &MapAssets,
    // political fill + borders
    layers: MapLayers<'_>,
    // names
    icons: &MapIcons,
    settle: f32,
    request: Request<'_>,
    path: &Path,
    banner: Option<&str>,
) -> io::Result<DumpReport> {
    request.camera.validate()?;
    pollster::block_on(async {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
        let adapters = instance.enumerate_adapters(wgpu::Backends::all()).await;
        let adapter = select_adapter(&adapters, None)?;
        let info = adapter.get_info();
        let (device, queue) = adapter
            .request_device(&device_descriptor(&adapter))
            .await
            .map_err(io::Error::other)?;
        let mut scene = Scene::new(&device, &queue, assets, icons, Default::default())?;
        // political fill + borders
        if !layers.omitted.gradient {
            scene.warm_political(&device, &queue, layers.content)?;
            scene.set_political(&device, &queue, &layers.political.regions)?;
        }
        if !layers.omitted.borders {
            scene.set_borders(
                &device,
                &queue,
                layers.border_assets,
                layers.border_meshes,
                layers.straits,
            )?;
            scene.set_classes(
                &device,
                layers.border_meshes,
                &layers.political.regions,
                layers.demilitarized,
            )?;
        }
        // rivers
        if !layers.omitted.rivers {
            scene.set_rivers(&device, &queue, layers.river_assets, layers.river_mesh)?;
        }
        // the map's table frame
        scene.set_map_frame(&device, &queue, layers.frame_assets)?;
        // trees
        if !layers.omitted.trees {
            scene.set_trees(
                &device,
                &queue,
                layers.tree_assets,
                layers.tree_geometry.clone(),
                layers.tree_field,
            )?;
        }
        // close zoom
        if !layers.omitted.sprawl {
            scene.set_sprawl(&device, &queue, layers.sprawl)?;
        }
        if !layers.omitted.buildings {
            scene.set_map_buildings(
                &device,
                &queue,
                layers.map_buildings,
                layers.map_building_field.clone(),
                None,
            )?;
            scene.visit_map_buildings(layers.building_cameras, request.size);
        }
        if !layers.omitted.rails {
            scene.set_railways(&device, &queue, layers.rail_assets, layers.rail_mesh)?;
            // A dump builds the pass, so it replaces no earlier plan.
            let _ = scene.set_trains(
                layers.trains.clone(),
                layers.train_plan.clone(),
                crate::map::presentation::Continuity::Reset,
            )?;
            scene.set_bridges(
                &device,
                &queue,
                layers.bridges,
                layers.bridges.field(layers.rail_mesh),
            )?;
        }
        if !layers.omitted.models {
            let _ = scene.set_aircraft(layers.aircraft.clone(), layers.aircraft_plan.clone())?;
            scene.set_unit_models(
                &device,
                &queue,
                layers.unit_models,
                layers.unit_model_field.clone(),
                true,
            )?;
        }
        // orders: the recorded player's battle plan
        if !layers.omitted.orders {
            scene.set_orders(&device, &queue, layers.order_assets, layers.order_field)?;
        }
        if !layers.omitted.weather {
            scene.set_weather_effects(
                &device,
                &queue,
                layers.weather_assets,
                layers.weather_field,
            )?;
        }
        // weather
        let snow_mud = layers.snow_mud;
        scene.set_snow_mud(
            &device,
            &queue,
            &snow_mud.texels,
            snow_mud.width,
            snow_mud.height,
            &snow_mud.province_snow,
        )?;
        // fog of war
        scene.set_fog_of_war(&queue, &layers.intel_map.texels, layers.intel_map.fog)?;
        scene.settle_models();
        // `--settle`: play the preceding seconds, one frame a second, so entity
        // events and particles are under way when the frame renders.
        let steps = settle.ceil() as u32;
        for step in 0..steps {
            let at = request.animation_time + step as f32 * settle / steps as f32;
            scene.render(
                &device,
                &queue,
                Request {
                    animation_time: at,
                    ..request
                },
            )?;
        }
        let request = Request {
            animation_time: request.animation_time + settle,
            ..request
        };
        // A first frame warms the pipeline caches so the timing below measures
        // the steady-state cost the window pays.
        scene.render(&device, &queue, request)?;
        device
            .poll(wgpu::PollType::Wait {
                submission_index: None,
                timeout: None,
            })
            .map_err(io::Error::other)?;
        let start = std::time::Instant::now();
        // The animation time is 0 and the overlay is the same one, so a
        // headless dump is deterministic.
        scene.render(&device, &queue, request)?;
        let cpu_seconds = start.elapsed().as_secs_f64();
        // `Wait` blocks until the GPU drains the submission, so this is the
        // frame's GPU execution time plus driver sync, the part no CPU lap
        // sees. There is no swap chain headlessly, so present/vsync needs the
        // interactive window.
        device
            .poll(wgpu::PollType::Wait {
                submission_index: None,
                timeout: None,
            })
            .map_err(io::Error::other)?;
        let render_seconds = start.elapsed().as_secs_f64();
        if let Some(text) = banner {
            render_banner(&scene, &device, &queue, text)?;
        }
        write_png(&scene, &device, &queue, path)?;
        // Offline shader parity evidence before grading/8-bit quantization.
        if let Some(path) = std::env::var_os("BATTLE_PLAN_MAP_SCENE_RGBA16F").or_else(|| std::env::var_os("HOI4_MAP_SCENE_RGBA16F")) {
            use std::io::Write;
            let targets = scene
                .targets()
                .ok_or_else(|| invalid("scene readback has no target"))?;
            let bytes = read_texture(&device, &queue, targets.scene_texture(), 8)?;
            File::create_new(path)?.write_all(&bytes)?;
        }
        let grid = scene.grid();
        let report = DumpReport {
            pass_warnings: scene.pass_warnings(),
            shader_refusals: scene.shader_refusals().clone(),
            adapter: format!("{} ({:?})", info.name, info.backend),
            device_requirements: device_requirements(device.features()),
            render_seconds,
            cpu_seconds,
            quads: grid.quads(),
            step: grid.step,
            // names
            overlay: scene.overlay_counts(),
            icon_pages: scene.icon_page_count(),
            // trees
            tree_instances: scene.tree_instances(),
            tree_draw_calls: scene.tree_draw_calls(),
            // close zoom
            sprawl_drawn: scene.sprawl_drawn(),
            map_buildings_drawn: scene.map_buildings_drawn(),
            unit_models_drawn: scene.unit_models_drawn(),
            aircraft_drawn: scene.aircraft_drawn(),
            // borders
            demilitarized_edges: scene.demilitarized_edges(),
            // shadows
            shadows: scene.shadows(),
            grading: scene.grading(),
        };
        // After the PNG and its report: diagnostics advance clocks a re-render
        // cannot rewind, such as the weather particles'.
        bench_frames(&mut scene, &device, &queue, request)?;
        model_frames(&mut scene, &device, &queue, request, path, banner)?;
        Ok(report)
    })
}

/// Diagnostic only: after the settled PNG, restart the model histories and
/// export viewer frames at 120 Hz, each with the dump's banner. No game
/// process or replay clock is advanced.
fn model_frames(
    scene: &mut Scene,
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    request: Request<'_>,
    path: &std::path::Path,
    banner: Option<&str>,
) -> io::Result<()> {
    let Some(value) = std::env::var_os("BATTLE_PLAN_VIEWER_MODEL_FRAMES").or_else(|| std::env::var_os("HOI4_VIEWER_MODEL_FRAMES")) else {
        return Ok(());
    };
    let count: u32 = value
        .to_str()
        .and_then(|value| value.parse().ok())
        .ok_or_else(|| {
            io::Error::other("BATTLE_PLAN_VIEWER_MODEL_FRAMES must be an unsigned frame count")
        })?;
    let stem = path.file_stem().unwrap_or_default().to_string_lossy();
    scene.restart_live_models();
    for frame in 0..count {
        scene.render(
            device,
            queue,
            Request {
                animation_time: request.animation_time + frame as f32 / 120.0,
                ..request
            },
        )?;
        if let Some(text) = banner {
            render_banner(scene, device, queue, text)?;
        }
        let output = path.with_file_name(format!("{stem}-model-frame-{frame:03}.png"));
        write_png(scene, device, queue, &output)?;
        eprintln!(
            "MODEL_FRAME frame={frame} units={} buildings={}",
            scene.unit_models_drawn(),
            scene.map_buildings_drawn()
        );
    }
    Ok(())
}

/// Profiling only: `BATTLE_PLAN_VIEWER_BENCH_FRAMES=<n>` renders `n` more frames of
/// the dumped hour, after its PNG is written, with both clocks advancing at
/// 120 Hz. Each prints `cpu`, the milliseconds `Scene::render` took to encode
/// and submit, and `drained`, the milliseconds from that call until the GPU
/// finished the frame. With `BATTLE_PLAN_VIEWER_PROFILE` it adds the render laps.
fn bench_frames(
    scene: &mut Scene,
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    request: Request<'_>,
) -> io::Result<()> {
    let Some(frames) = std::env::var("BATTLE_PLAN_VIEWER_BENCH_FRAMES").or_else(|_| std::env::var("HOI4_VIEWER_BENCH_FRAMES"))
        .ok()
        .and_then(|frames| frames.parse::<u32>().ok())
    else {
        return Ok(());
    };
    let wait = |device: &wgpu::Device| {
        device
            .poll(wgpu::PollType::Wait {
                submission_index: None,
                timeout: None,
            })
            .map(|_| ())
            .map_err(io::Error::other)
    };
    if std::env::var_os("BATTLE_PLAN_VIEWER_BENCH_MODELS").or_else(|| std::env::var_os("HOI4_VIEWER_BENCH_MODELS")).is_some() {
        scene.restart_live_models();
    }
    for frame in 0..frames {
        let step = (frame + 1) as f32 / 120.0;
        scene.label_frame(u64::from(frame));
        let start = std::time::Instant::now();
        scene.render(
            device,
            queue,
            Request {
                animation_time: request.animation_time + step,
                map_time: request.map_time + step,
                ..request
            },
        )?;
        let cpu = start.elapsed().as_secs_f64() * 1000.0;
        wait(device)?;
        let drained = start.elapsed().as_secs_f64() * 1000.0;
        let laps = if crate::map::counter_move::profiling() {
            let [animate, overlay, encode, submit] = scene.render_laps();
            format!(
                " laps animate={animate:.3} overlay_upload={overlay:.3} encode={encode:.3} submit={submit:.3} detail {}",
                scene.detail_laps()
            )
        } else {
            String::new()
        };
        crate::map::counter_move::profile_print(format!(
            "BENCH frame {frame} cpu {cpu:.3} drained {drained:.3}{laps}"
        ));
    }
    Ok(())
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
