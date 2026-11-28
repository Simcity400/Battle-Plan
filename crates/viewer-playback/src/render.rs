
use shader_pipeline::gpu::stats::CountedPasses as _;
use crate::map_icons::MapIcons;
use std::collections::BTreeMap;
use std::time::Instant;
use viewer_gpu::camera::Camera;
use viewer_native_gui::icon_windows::DumpFrames;
use viewer_gpu::layers::Omissions;
use viewer_runtime::presentation::Continuity;
use viewer_runtime::profile_writer::profile_print;
use viewer_runtime::profile_writer::profiling;
use viewer_world::scene_content::MapAssets;

use viewer_gpu::gpu_contract::{
    device_requirements,
    map_device_descriptor,
    read_texture,
    select_adapter,
};

use std::{fs::File, io, path::Path, sync::Arc};

use eframe::wgpu;

use map_display_data::buildings::Buildings as PreparedBuildings;
use viewer_gpu::{
    gpu_contract::{OUTPUT_FORMAT, invalid},
    ribbon_content::{OrderAssets, OrderField},
    textures::IntelMap,
};
use viewer_borders::{
    border_pass::BorderAssets,
    strait_shader::StraitVertex,
    borders::BorderMeshes,
    demilitarized::Zone,
};
use viewer_buildings::{map_buildings::MapBuildingAssets, sprawl::Sprawl};
use viewer_world::{mapframe::FrameAssets, scene::{Request, Scene}};
use viewer_map_modes::political::PoliticalLayers;
use viewer_rail::railways::{Bridges, RailAssets, RailMesh};
use viewer_terrain::{rivers::{RiverAssets, RiverMesh}, trees::{TreeAssets, TreeField, TreeGeometry}};
use viewer_unit_models::unit_models::UnitModelAssets;
use map_display_data::SnowMud;

pub struct MapLayers<'a> {
    pub content: &'a crate::document::Content,
    pub graphics: dashboard_core::protocol::MapGraphics,
    pub at: (replay_data::BranchId, i32),
    pub map_text: &'a viewer_map_text::animated_map_text::events::Prepared,
    pub omitted: Omissions,
    pub political: &'a PoliticalLayers,
    pub border_assets: &'a BorderAssets,
    pub border_meshes: &'a BorderMeshes,
    pub straits: &'a [StraitVertex],
    pub demilitarized: &'a Zone,
    pub river_assets: &'a RiverAssets,
    pub river_mesh: &'a RiverMesh,
    pub snow_mud: &'a Arc<SnowMud>,
    pub intel_map: &'a IntelMap,
    pub visibility: viewer_foundation::mode_visibility::Visibility,
    pub frame_assets: &'a FrameAssets,
    pub nuclear_impacts: &'a viewer_models::strike_effects::Impacts,
    pub sprawl: &'a Sprawl,
    pub map_buildings: &'a MapBuildingAssets,
    pub map_building_field: &'a PreparedBuildings,
    pub building_cameras: &'a [Camera],
    pub rail_assets: &'a RailAssets,
    pub rail_mesh: &'a RailMesh,
    pub aircraft: &'a Arc<viewer_aircraft::aircraft::AircraftAssets>,
    pub aircraft_plan: &'a viewer_aircraft::aircraft::AircraftPlan,
    pub trains: &'a Arc<viewer_rail::trains::TrainAssets>,
    pub train_plan: &'a viewer_rail::trains::TrainPlan,
    pub bridges: &'a Bridges,
    pub unit_models: &'a UnitModelAssets,
    pub unit_model_field: &'a viewer_unit_history::unit_history::Field,
    pub order_assets: &'a OrderAssets,
    pub order_field: &'a OrderField,
    pub weather_assets: Arc<viewer_weather_effects::weather_effects::Assets>,
    pub weather_field: map_display_data::WeatherField,
    pub tree_assets: &'a TreeAssets,
    pub tree_geometry: &'a TreeGeometry,
    pub tree_field: &'a TreeField,
}

pub fn configuration() -> eframe::egui_wgpu::WgpuConfiguration {
    let mut configuration = eframe::egui_wgpu::WgpuConfiguration::default();
    configuration.surface = eframe::egui_wgpu::SurfaceConfig::LOW_LATENCY;
    if std::env::var_os("BATTLEPLAN_VIEWER_NO_VSYNC").or_else(|| std::env::var_os("HOI4_VIEWER_NO_VSYNC")).is_some() {
        configuration.surface.present_mode = wgpu::PresentMode::AutoNoVsync;
    }
    if let eframe::egui_wgpu::WgpuSetup::CreateNew(setup) = &mut configuration.wgpu_setup {
        setup.device_descriptor = Arc::new(map_device_descriptor);
        setup.native_adapter_selector = Some(Arc::new(|adapters, surface| {
            select_adapter(adapters, surface).map_err(|error| error.to_string())
        }));
    }
    configuration
}

pub fn probe_dump_device() -> io::Result<()> {
    pollster::block_on(async {
        let instance = shader_pipeline::gpu::instance();
        let adapters = instance.enumerate_adapters(wgpu::Backends::all()).await;
        let adapter = select_adapter(&adapters, None)?;
        map_device_descriptor(&adapter);
        Ok(())
    })
}

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
            .counted_render_pass(&wgpu::RenderPassDescriptor {
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

pub fn write_png(
    scene: &Scene,
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    path: &Path,
) -> io::Result<()> {
    warn(path, write_png_under(&replay_settings::exit::EXIT.admit(replay_settings::exit::What::Export)?, scene, device, queue, path)?);
    Ok(())
}

fn warn(path: &Path, warning: Option<String>) {
    if let Some(warning) = warning {
        eprintln!("Saved {}, but {warning}", path.display());
    }
}

pub fn write_png_under(
    receipt: &replay_settings::exit::Receipt<'_>,
    scene: &Scene,
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    path: &Path,
) -> io::Result<Option<String>> {
    let targets = scene
        .targets()
        .ok_or_else(|| invalid("render a map before saving PNG"))?;
    write_texture_under(receipt, targets.output(), device, queue, path)
}

pub fn write_texture_png(
    texture: &wgpu::Texture,
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    path: &Path,
) -> io::Result<()> {
    warn(path, write_texture_under(&replay_settings::exit::EXIT.admit(replay_settings::exit::What::Export)?, texture, device, queue, path)?);
    Ok(())
}

fn write_texture_under(
    receipt: &replay_settings::exit::Receipt<'_>,
    texture: &wgpu::Texture,
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    path: &Path,
) -> io::Result<Option<String>> {
    let (width, height) = (texture.width(), texture.height());
    let pixels = read_texture(device, queue, texture, 4)?;
    receipt.replace(path, |out| {
        let mut encoder = png::Encoder::new(out, width, height);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        encoder.set_source_srgb(png::SrgbRenderingIntent::Perceptual);
        let mut writer = encoder.write_header().map_err(io::Error::other)?;
        writer.write_image_data(&pixels).map_err(io::Error::other)?;
        writer.finish().map_err(io::Error::other)
    })
    .map(|applied| applied.warning)
}

pub struct DumpReport {
    pub pass_warnings: Vec<String>,
    pub shader_refusals: BTreeMap<String, u64>,
    pub adapter: String,
    pub device_requirements: Vec<String>,
    pub render_seconds: f64,
    pub cpu_seconds: f64,
    pub blocks: usize,
    pub quads: u64,
    pub overlay: (u32, u32),
    pub icon_pages: usize,
    pub tree_instances: u32,
    pub tree_draw_calls: u32,
    pub sprawl_drawn: u32,
    pub map_buildings_drawn: u32,
    pub unit_models_drawn: u32,
    pub aircraft_drawn: u32,
    pub demilitarized_edges: usize,
    pub shadows: bool,
    pub grading: Option<String>,
}

#[expect(clippy::too_many_arguments, reason = "The window's scene inputs plus the dump's request and output.")]
#[expect(clippy::await_holding_lock, reason = "block_on polls this future to completion on this thread, and wgpu's native adapter and device futures resolve without another thread taking the lock, which must span device creation")]
pub fn dump_png(
    assets: &MapAssets,
    layers: MapLayers<'_>,
    icons: &MapIcons,
    settle: f32,
    overlay: &mut crate::document::DumpOverlay<'_>,
    request: Request<'_>,
    path: &Path,
    banner: Option<&str>,
) -> io::Result<DumpReport> {
    request.camera.validate()?;
    pollster::block_on(async {
        let lifecycle = shader_pipeline::gpu::device_lifecycle();
        let instance = shader_pipeline::gpu::instance();
        let adapters = instance.enumerate_adapters(wgpu::Backends::all()).await;
        let adapter = select_adapter(&adapters, None)?;
        let info = adapter.get_info();
        let (device, queue) = adapter
            .request_device(&map_device_descriptor(&adapter))
            .await
            .map_err(io::Error::other)?;
        drop(lifecycle);
        let textures = layers.content.scene_textures.warm(&device, &queue, assets)?;
        let mut scene = Scene::with_textures(
            &device,
            &queue,
            textures,
            assets,
            &icons.lazy,
            layers.graphics,
            &viewer_runtime::parallel::Workers::default(),
        )?;
        scene.settle_builds(true);
        scene.settle_poses(true);
        scene.build_names_in_upload(true);
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
        if !layers.omitted.rivers {
            scene.set_rivers(&device, &queue, layers.river_assets, layers.river_mesh)?;
        }
        scene.set_map_frame(layers.frame_assets)?;
        if !layers.omitted.trees {
            scene.set_trees(
                &device,
                &queue,
                layers.tree_assets,
                layers.tree_geometry.clone(),
                layers.tree_field,
            )?;
        }
        if !layers.omitted.sprawl {
            scene.set_sprawl(layers.sprawl)?;
            scene.set_city_damage(layers.map_building_field, true);
        }
        if !layers.omitted.buildings {
            scene.set_map_buildings(
                layers.map_buildings,
                layers.map_building_field.clone(),
                None,
            )?;
            scene.visit_map_buildings(layers.building_cameras, request.size);
        }
        if !layers.omitted.rails {
            scene.set_railways(&device, &queue, layers.rail_assets, layers.rail_mesh)?;
            let _ = scene.set_trains(
                layers.trains.clone(),
                layers.train_plan.clone(),
                Continuity::Reset,
            )?;
            scene.set_bridges(
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
                layers.unit_models,
                layers.unit_model_field.clone(),
                true,
            )?;
        }
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
        let snow_mud = layers.snow_mud;
        scene.set_snow_mud(&device, &queue, snow_mud, Continuity::Reset)?;
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
            viewer_gpu::province_map::HeightField {
                width: assets.heightmap.width,
                height: assets.heightmap.height,
                values: assets.heightmap_texels(),
            },
            &layers.content.icons.counters.gui,
        );
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
        dump_frame(&mut scene, &device, &queue, overlay, request)?;
        let cpu_seconds = start.elapsed().as_secs_f64();
        overlay.refuse_settlement(overlay.retries());
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
        if let Some(path) = std::env::var_os("BATTLEPLAN_MAP_SCENE_RGBA16F").or_else(|| std::env::var_os("HOI4_MAP_SCENE_RGBA16F")) {
            use std::io::Write;
            let targets = scene
                .targets()
                .ok_or_else(|| invalid("scene readback has no target"))?;
            let bytes = read_texture(&device, &queue, targets.scene_texture(), 8)?;
            File::create_new(path)?.write_all(&bytes)?;
        }
        let terrain = scene.terrain_drawn();
        let report = DumpReport {
            pass_warnings: scene.pass_warnings(),
            shader_refusals: scene.shader_refusals().clone(),
            adapter: format!("{} ({:?})", info.name, info.backend),
            device_requirements: device_requirements(device.features()),
            render_seconds,
            cpu_seconds,
            blocks: terrain.0,
            quads: terrain.1,
            overlay: scene.overlay_counts(),
            icon_pages: scene.icon_page_count(),
            tree_instances: scene.tree_instances(),
            tree_draw_calls: scene.tree_draw_calls(),
            sprawl_drawn: scene.sprawl_drawn(),
            map_buildings_drawn: scene.map_buildings_drawn(),
            unit_models_drawn: scene.unit_models_drawn(),
            aircraft_drawn: scene.aircraft_drawn(),
            demilitarized_edges: scene.demilitarized_edges(),
            shadows: scene.shadows(),
            grading: scene.grading(),
        };
        let request = Request {
            overlay: viewer_world::scene::Overlay {
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
    overlay.present(request.animation_time)?;
    scene.set_air_assignment(device, queue, overlay.prepared().province_stripes())?;
    let sprites = overlay.shown();
    scene.render(
        device,
        queue,
        Request {
            overlay: viewer_world::scene::Overlay {
                sprites: request.overlay.interface.then_some(sprites),
                ..request.overlay
            },
            ..request
        },
    )
}

fn model_frames(
    scene: &mut Scene,
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    request: Request<'_>,
    path: &Path,
    banner: Option<&str>,
) -> io::Result<()> {
    let Some(value) = std::env::var_os("BATTLEPLAN_VIEWER_MODEL_FRAMES").or_else(|| std::env::var_os("HOI4_VIEWER_MODEL_FRAMES")) else {
        return Ok(());
    };
    let count: u32 = value
        .to_str()
        .and_then(|value| value.parse().ok())
        .ok_or_else(|| {
            io::Error::other("BATTLEPLAN_VIEWER_MODEL_FRAMES must be an unsigned frame count")
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

fn bench_frames(
    scene: &mut Scene,
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    request: Request<'_>,
) -> io::Result<()> {
    let Some(frames) = std::env::var("BATTLEPLAN_VIEWER_BENCH_FRAMES").or_else(|_| std::env::var("HOI4_VIEWER_BENCH_FRAMES"))
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
    if std::env::var_os("BATTLEPLAN_VIEWER_BENCH_MODELS").or_else(|| std::env::var_os("HOI4_VIEWER_BENCH_MODELS")).is_some() {
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
