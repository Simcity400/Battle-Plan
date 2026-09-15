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
    map_buildings::MapBuildingAssets,
    // the map's table frame
    mapframe::{FrameAssets, FrameGeometry},
    models::ModelField,
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
    /// The hour's gradient-border textures and the region keys behind them.
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
    pub frame_geometry: &'a FrameGeometry,
    // close zoom
    pub sprawl: &'a Sprawl,
    pub map_buildings: &'a MapBuildingAssets,
    /// This hour's placed buildings, from [`MapBuildingAssets::field`].
    pub map_building_field: &'a ModelField,
    pub rail_assets: &'a RailAssets,
    pub rail_mesh: &'a RailMesh,
    pub aircraft: &'a std::sync::Arc<crate::map::aircraft::AircraftAssets>,
    pub aircraft_plan: &'a crate::map::aircraft::AircraftPlan,
    pub trains: &'a std::sync::Arc<crate::map::trains::TrainAssets>,
    pub train_plan: &'a crate::map::trains::TrainPlan,
    pub bridges: &'a Bridges,
    pub unit_models: &'a UnitModelAssets,
    /// This hour's placed figures, from [`super::map::unit_models::PreparedModels::field`].
    pub unit_model_field: &'a ModelField,
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
    if let eframe::egui_wgpu::WgpuSetup::CreateNew(setup) = &mut configuration.wgpu_setup {
        setup.device_descriptor = Arc::new(device_descriptor);
    }
    configuration
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
    wgpu::DeviceDescriptor {
        label: Some("Battle Plan viewer"),
        required_features: adapter.features() & wgpu::Features::TEXTURE_COMPRESSION_BC,
        required_limits: wgpu::Limits {
            max_texture_dimension_1d: limits.max_texture_dimension_1d,
            max_texture_dimension_2d: limits.max_texture_dimension_2d,
            max_sampled_textures_per_shader_stage: limits.max_sampled_textures_per_shader_stage,
            max_samplers_per_shader_stage: limits.max_samplers_per_shader_stage,
            ..Default::default()
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
    let row_bytes = width * 4;
    let padded_row_bytes =
        row_bytes.div_ceil(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT) * wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
    let readback_bytes = u64::from(padded_row_bytes) * u64::from(height);
    if readback_bytes > device.limits().max_buffer_size {
        return Err(invalid(format!(
            "PNG readback needs {readback_bytes} bytes; GPU device buffer limit is {}",
            device.limits().max_buffer_size
        )));
    }
    let buffer = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("map PNG readback"),
        size: readback_bytes,
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
        label: Some("map PNG copy"),
    });
    encoder.copy_texture_to_buffer(
        targets.output().as_image_copy(),
        wgpu::TexelCopyBufferInfo {
            buffer: &buffer,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(padded_row_bytes),
                rows_per_image: Some(height),
            },
        },
        extent(targets.size()),
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
    let mut encoder = png::Encoder::new(BufWriter::new(File::create(path)?), width, height);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    encoder.set_source_srgb(png::SrgbRenderingIntent::Perceptual);
    let mut writer = encoder.write_header().map_err(io::Error::other)?;
    writer.write_image_data(&pixels).map_err(io::Error::other)?;
    writer.finish().map_err(io::Error::other)
}

/// What one headless render reports back.
pub struct DumpReport {
    /// Adapter name and backend, so a dump identifies the GPU it used.
    pub adapter: String,
    /// Wall time of the render call alone, excluding device and texture setup.
    pub render_seconds: f64,
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
}

/// Renders one frame headlessly through the same [`Scene`] the window uses.
pub fn dump_png(
    assets: &MapAssets,
    // political fill + borders
    layers: MapLayers<'_>,
    // names
    icons: &MapIcons,
    request: Request<'_>,
    path: &Path,
    banner: Option<&str>,
) -> io::Result<DumpReport> {
    request.camera.validate()?;
    pollster::block_on(async {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
        let adapter = instance
            .request_adapter(&wgpu::RequestAdapterOptions {
                power_preference: wgpu::PowerPreference::HighPerformance,
                compatible_surface: None,
                force_fallback_adapter: false,
                ..Default::default()
            })
            .await
            .map_err(io::Error::other)?;
        let info = adapter.get_info();
        let (device, queue) = adapter
            .request_device(&device_descriptor(&adapter))
            .await
            .map_err(io::Error::other)?;
        let mut scene = Scene::new(&device, &queue, assets, icons)?;
        // political fill + borders
        let textures = &layers.political.textures;
        scene.set_gradient_textures(
            &device,
            &queue,
            &textures.channel1,
            &textures.channel2,
            textures.width,
            textures.height,
        )?;
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
        // rivers
        scene.set_rivers(&device, &queue, layers.river_assets, layers.river_mesh)?;
        // the map's table frame
        scene.set_map_frame(
            &device,
            &queue,
            layers.frame_assets,
            layers.frame_geometry.clone(),
        )?;
        // trees
        scene.set_trees(
            &device,
            &queue,
            layers.tree_assets,
            layers.tree_geometry.clone(),
            layers.tree_field,
        )?;
        // close zoom
        scene.set_sprawl(&device, &queue, layers.sprawl)?;
        scene.set_map_buildings(
            &device,
            &queue,
            layers.map_buildings,
            layers.map_building_field.clone(),
        )?;
        scene.set_railways(&device, &queue, layers.rail_assets, layers.rail_mesh)?;
        scene.set_trains(
            &device,
            &queue,
            layers.trains.clone(),
            layers.train_plan.clone(),
        )?;
        scene.set_aircraft(
            &device,
            &queue,
            layers.aircraft.clone(),
            layers.aircraft_plan.clone(),
        )?;
        scene.set_bridges(&device, &queue, layers.bridges)?;
        scene.set_unit_models(
            &device,
            &queue,
            layers.unit_models,
            layers.unit_model_field.clone(),
        )?;
        // orders: the recorded player's battle plan
        scene.set_orders(&device, &queue, layers.order_assets, layers.order_field)?;
        scene.set_weather_effects(&device, &queue, layers.weather_assets, layers.weather_field)?;
        // weather
        let snow_mud = layers.snow_mud;
        scene.set_snow_mud(
            &device,
            &queue,
            &snow_mud.texels,
            snow_mud.width,
            snow_mud.height,
        )?;
        // fog of war
        scene.set_fog_of_war(&queue, &layers.intel_map.texels, layers.intel_map.fog)?;
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
        let grid = scene.grid();
        Ok(DumpReport {
            adapter: format!("{} ({:?})", info.name, info.backend),
            render_seconds,
            quads: grid.quads(),
            step: grid.step,
            // names
            overlay: scene.overlay_counts(),
            icon_pages: scene.icon_page_count(),
            // trees
            tree_instances: scene.tree_instances(),
            // close zoom
            sprawl_drawn: scene.sprawl_drawn(),
            map_buildings_drawn: scene.map_buildings_drawn(),
            unit_models_drawn: scene.unit_models_drawn(),
            aircraft_drawn: scene.aircraft_drawn(),
            // borders
            demilitarized_edges: scene.demilitarized_edges(),
            // shadows
            shadows: scene.shadows(),
        })
    })
}
