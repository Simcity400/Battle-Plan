//! Device setup and the outputs of the map pipeline.
//!
//! The window and the headless `--dump` drive the same [`Scene`]; this module
//! only owns the device requirements, the PNG readback, the unverified-content
//! banner and the headless entry point.

use crate::map_icons::MapIcons;
use std::collections::BTreeMap;
use std::time::Instant;
use viewer_foundation::map::camera::Camera;
use viewer_foundation::map::icon_windows::DumpFrames;
use viewer_foundation::map::layers::Omissions;
use viewer_foundation::map::presentation::Continuity;
use viewer_foundation::profile_writer::profile_print;
use viewer_foundation::profile_writer::profiling;
use viewer_world::map::scene_content::MapAssets;

use viewer_foundation::map::gpu_contract::{
    device_requirements, map_device_descriptor, read_texture, select_adapter,
};

use std::{fs::File, io, io::BufWriter, path::Path, sync::Arc};

use eframe::wgpu;

use {
    map_display_data::buildings::Buildings as PreparedBuildings,
    viewer_foundation::map::gpu_contract::OUTPUT_FORMAT,
    viewer_foundation::map::gpu_contract::invalid,
    viewer_foundation::map::ribbon_content::OrderAssets,
    viewer_foundation::map::ribbon_content::OrderField, viewer_foundation::map::textures::IntelMap,
    viewer_world::map::border_pass::BorderAssets, viewer_world::map::border_pass::StraitVertex,
    viewer_world::map::borders::BorderMeshes, viewer_world::map::demilitarized::Zone,
    viewer_world::map::map_buildings::MapBuildingAssets, viewer_world::map::mapframe::FrameAssets,
    viewer_world::map::political::PoliticalLayers, viewer_world::map::railways::Bridges,
    viewer_world::map::railways::RailAssets, viewer_world::map::railways::RailMesh,
    viewer_world::map::rivers::RiverAssets, viewer_world::map::rivers::RiverMesh,
    viewer_world::map::scene::Request, viewer_world::map::scene::Scene,
    viewer_world::map::sprawl::Sprawl, viewer_world::map::trees::TreeAssets,
    viewer_world::map::trees::TreeField, viewer_world::map::trees::TreeGeometry,
    viewer_world::map::unit_models::UnitModelAssets, viewer_world::map::weather::SnowMud,
};

// political fill + borders
/// Everything a headless render needs beyond the terrain and water assets.
pub struct MapLayers<'a> {
    pub content: &'a crate::document::Content,
    pub graphics: dashboard::protocol::MapGraphics,
    /// The hour drawn, and its map events for the viewing country.
    pub at: (replay_data::BranchId, i32),
    pub map_text: &'a viewer_world::map::animated_map_text::events::Prepared,
    pub omitted: Omissions,
    /// The hour's compact political region inputs.
    pub political: &'a PoliticalLayers,
    pub border_assets: &'a BorderAssets,
    pub border_meshes: &'a BorderMeshes,
    pub straits: &'a [StraitVertex],
    /// This hour's demilitarised zone; `viewer_world::map::demilitarized`. Empty unless `--demilitarized` is
    /// given, because no recorded table carries the flag.
    pub demilitarized: &'a Zone,
    // rivers
    pub river_assets: &'a RiverAssets,
    pub river_mesh: &'a RiverMesh,
    // weather
    /// The hour's `SnowMudData` grid.
    pub snow_mud: &'a Arc<SnowMud>,
    // fog of war
    /// The hour's `IntelMap` for the perspective the frame is drawn for.
    pub intel_map: &'a IntelMap,
    /// What the map mode shows of the layers: the fog of war, province borders, land models and
    /// unit arrows ([`viewer_world::map::map_mode::visibility`]).
    pub visibility: viewer_foundation::map::mode_visibility::Visibility,
    // the map's table frame
    pub frame_assets: &'a FrameAssets,
    /// This hour's nuclear impacts, whose province effects start at presentation time zero.
    pub nuclear_impacts: &'a viewer_world::map::strike_effects::Impacts,
    // close zoom
    pub sprawl: &'a Sprawl,
    pub map_buildings: &'a MapBuildingAssets,
    /// This hour's placed buildings, from [`MapBuildingAssets::field`].
    pub map_building_field: &'a PreparedBuildings,
    /// Cameras visited before this frame; see [`Scene::visit_map_buildings`].
    pub building_cameras: &'a [Camera],
    pub rail_assets: &'a RailAssets,
    pub rail_mesh: &'a RailMesh,
    pub aircraft: &'a Arc<viewer_world::map::aircraft::AircraftAssets>,
    pub aircraft_plan: &'a viewer_world::map::aircraft::AircraftPlan,
    pub trains: &'a Arc<viewer_world::map::trains::TrainAssets>,
    pub train_plan: &'a viewer_world::map::trains::TrainPlan,
    pub bridges: &'a Bridges,
    pub unit_models: &'a UnitModelAssets,
    /// This hour's placed figures, from [`super::map::unit_models::PreparedModels::field`].
    pub unit_model_field: &'a viewer_world::map::unit_history::Field,
    // orders: the recorded player's battle plan
    pub order_assets: &'a OrderAssets,
    /// This hour's ribbons, from [`crate::document::Document::order_field`].
    pub order_field: &'a OrderField,
    pub weather_assets: Arc<viewer_world::map::weather_effects::Assets>,
    pub weather_field: viewer_world::map::weather_effects::Field,
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
        setup.device_descriptor = Arc::new(map_device_descriptor);
        // The window and the dashboard maps it hosts choose like `--dump`.
        setup.native_adapter_selector = Some(Arc::new(|adapters, surface| {
            select_adapter(adapters, surface).map_err(|error| error.to_string())
        }));
    }
    configuration
}

/// Asks, before a headless dump loads its document, for the device descriptor of the adapter
/// the dump will render on ([`dump_png`] selects it the same way), checking adapter support
/// before loading the document.
pub fn probe_dump_device() -> io::Result<()> {
    pollster::block_on(async {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
        let adapters = instance.enumerate_adapters(wgpu::Backends::all()).await;
        let adapter = select_adapter(&adapters, None)?;
        map_device_descriptor(&adapter);
        Ok(())
    })
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

/// What one headless render reports back.
pub struct DumpReport {
    pub pass_warnings: Vec<String>,
    pub shader_refusals: BTreeMap<String, u64>,
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
    /// `DRAW_SHADOWS_CUTOFF` and not above it (`viewer_world::map::shadow`).
    pub shadows: bool,
    /// The post grade, when the frame was graded.
    pub grading: Option<String>,
}

/// Renders one frame headlessly through the same [`Scene`] the window uses.
#[allow(clippy::too_many_arguments)] // The window's scene inputs plus the dump's request and output.
pub fn dump_png(
    assets: &MapAssets,
    // political fill + borders
    layers: MapLayers<'_>,
    // names
    icons: &MapIcons,
    settle: f32,
    overlay: &mut crate::document::DumpOverlay<'_>,
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
            .request_device(&map_device_descriptor(&adapter))
            .await
            .map_err(io::Error::other)?;
        let mut scene = Scene::new(
            &device,
            &queue,
            assets,
            &icons.images,
            &icons.lazy,
            layers.graphics,
        )?;
        // A dump is a capture: its models draw complete, built in place.
        scene.settle_builds(true);
        // A dump installs no hours: its names build where they draw.
        scene.build_names_in_upload(true);
        // political fill + borders
        if !layers.omitted.gradient {
            scene.warm_political(
                &device,
                &queue,
                &layers.content.map,
                &layers.content.strategic_regions,
                &layers.content.map_defines.gradient_field,
                &layers.content.political_gpu,
            )?;
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
            scene.set_city_damage(layers.map_building_field, true);
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
                Continuity::Reset,
            )?;
            scene.set_bridges(
                &device,
                &queue,
                layers.bridges,
                layers.bridges.field(layers.rail_mesh),
            )?;
        }
        if !layers.omitted.models {
            let _ = scene.set_aircraft(
                layers.aircraft.clone(),
                layers.aircraft_plan.clone(),
                Continuity::Reset,
            )?;
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
            scene.set_orders(
                &device,
                &queue,
                layers.order_assets,
                layers.order_field.clone(),
            )?;
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
        scene.set_snow_mud(&device, &queue, snow_mud, Continuity::Reset)?;
        // fog of war
        scene.set_fog_of_war(&queue, &layers.intel_map.texels, layers.intel_map.fog)?;
        scene.set_visibility(layers.visibility);
        scene.settle_models();
        scene.install_animated_text(
            &layers.content.icons.animated_text,
            &layers.content.icons.counters.gui,
        );
        scene.share_province_schedule(overlay.schedule());
        scene.present_hours(
            layers.at,
            true,
            [(layers.nuclear_impacts, layers.map_text)],
            Continuity::Reset,
            |province| layers.content.counters.anchor(province, 0),
            viewer_foundation::map::province_map::HeightField {
                width: assets.heightmap.width,
                height: assets.heightmap.height,
                values: assets.heightmap_texels(),
            },
            &layers.content.icons.counters.gui,
        );
        // `--settle`: play the preceding seconds, one frame a second, so entity
        // events and particles are under way when the frame renders.
        let steps = settle.ceil() as u32;
        for step in 0..steps {
            let at = request.animation_time + step as f32 * settle / steps as f32;
            dump_frame(
                &mut scene,
                &device,
                &queue,
                overlay,
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
        dump_frame(&mut scene, &device, &queue, overlay, request)?;
        let mut frames = DumpFrames::default();
        while frames.next(&overlay.retries()) {
            dump_frame(&mut scene, &device, &queue, overlay, request)?;
        }
        device
            .poll(wgpu::PollType::Wait {
                submission_index: None,
                timeout: None,
            })
            .map_err(io::Error::other)?;
        let start = Instant::now();
        // Allocation retry frames have zero elapsed time. Scene and GUI clocks
        // stay together at the requested timestamp; the recorded hour stays fixed.
        dump_frame(&mut scene, &device, &queue, overlay, request)?;
        let cpu_seconds = start.elapsed().as_secs_f64();
        overlay.refuse_settlement(overlay.retries());
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
        let request = Request {
            overlay: viewer_world::map::scene::Overlay {
                sprites: request.overlay.interface.then_some(overlay.shown()),
                ..request.overlay
            },
            ..request
        };
        bench_frames(&mut scene, &device, &queue, request)?;
        model_frames(&mut scene, &device, &queue, request, path, banner)?;
        Ok(report)
    })
}

fn dump_frame(
    scene: &mut Scene,
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    overlay: &mut crate::document::DumpOverlay<'_>,
    request: Request<'_>,
) -> io::Result<()> {
    // Province owners, icon manager, then draw; a hidden interface still steps.
    overlay.present(request.animation_time)?;
    scene.set_air_assignment(queue, overlay.prepared().province_stripes())?;
    let sprites = overlay.shown();
    scene.render(
        device,
        queue,
        Request {
            overlay: viewer_world::map::scene::Overlay {
                sprites: request.overlay.interface.then_some(sprites),
                ..request.overlay
            },
            ..request
        },
    )
}

/// Diagnostic only: after the settled PNG, restart the model histories and
/// export viewer frames at 120 Hz, each with the dump's banner. No game
/// process or replay clock is advanced.
fn model_frames(
    scene: &mut Scene,
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    request: Request<'_>,
    path: &Path,
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
        let start = Instant::now();
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
        let laps = if profiling() {
            let [animate, overlay, encode, submit] = scene.render_laps();
            format!(
                " laps animate={animate:.3} overlay_upload={overlay:.3} encode={encode:.3} submit={submit:.3} detail {}",
                scene.detail_laps()
            )
        } else {
            String::new()
        };
        profile_print(format!(
            "BENCH frame {frame} cpu {cpu:.3} drained {drained:.3}{laps}"
        ));
    }
    Ok(())
}
