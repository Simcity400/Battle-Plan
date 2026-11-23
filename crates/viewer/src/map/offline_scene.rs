use viewer_playback::map_icons::MapIcons;
use eframe::wgpu;
use content_reader_layers::ResolvedLayers;
use serde::Deserialize;
use std::sync::Arc;
use std::{fs, io, path::PathBuf};
use viewer_gpu::gpu_contract::map_device_descriptor;
use viewer_gpu::gpu_contract::select_adapter;
use viewer_gpu::{camera, frame, province_map::HeightField};
use viewer_world::{mapframe, scene};
use viewer_terrain::{rivers, trees};

#[derive(Deserialize)]
struct Layer {
    root: PathBuf,
    #[serde(default)]
    replace_paths: Vec<String>,
}

#[derive(Deserialize)]
struct Input {
    layers: Vec<Layer>,
    output: PathBuf,
    cameras: Vec<[f32; 3]>,
    size: [u32; 2],
    hour: i32,
    language: String,
    camera_tilt: f32,
    #[serde(default)]
    force_pow2_textures: bool,
    #[serde(default = "default_steps")]
    presentation_steps: u32,
    #[serde(default)]
    presentation_dt: f32,
    #[serde(default)]
    impacts: Vec<SyntheticImpact>,
}

#[derive(Deserialize)]
struct SyntheticImpact {
    province: u32,
    #[serde(default)]
    effect: SyntheticEffect,
}

#[derive(Clone, Copy, Default, Deserialize)]
#[serde(rename_all = "snake_case")]
enum SyntheticEffect {
    #[default]
    Nuclear,
    Thermonuclear,
    CriticalBombardment,
}

impl From<SyntheticEffect> for viewer_models::strike_effects::Effect {
    fn from(effect: SyntheticEffect) -> Self {
        match effect {
            SyntheticEffect::Nuclear => Self::Nuclear,
            SyntheticEffect::Thermonuclear => Self::Thermonuclear,
            SyntheticEffect::CriticalBombardment => Self::CriticalBombardment,
        }
    }
}

fn default_steps() -> u32 {
    2
}

#[test]
#[ignore = "offline content-reader integration; requires BATTLEPLAN_CONTENT_SCENE_INPUT JSON"]
fn content_stack_scene() -> io::Result<()> {
    content_reader_script::test_game::enter();
    let path = std::env::var_os("BATTLEPLAN_CONTENT_SCENE_INPUT").or_else(|| std::env::var_os("HOI4_CONTENT_SCENE_INPUT"))
        .ok_or_else(|| io::Error::other("BATTLEPLAN_CONTENT_SCENE_INPUT is required"))?;
    let input: Input = serde_json::from_slice(&fs::read(path)?).map_err(io::Error::other)?;
    if input.output.exists() {
        return Err(io::Error::other("content-scene output already exists"));
    }
    fs::create_dir_all(&input.output)?;
    let paths: Vec<Vec<&str>> = input
        .layers
        .iter()
        .map(|l| l.replace_paths.iter().map(String::as_str).collect())
        .collect();
    let folders: Vec<_> = input
        .layers
        .iter()
        .zip(&paths)
        .map(|(l, p)| (l.root.as_path(), p.as_slice()))
        .collect();
    eprintln!("Content scene: resolve and verify ordered folders");
    let layers = ResolvedLayers::from_folders(&folders)?;
    eprintln!("Content scene: resolved");
    let mut warnings = Vec::new();
    let result = render(&layers, &input, &mut warnings);
    for line in &warnings {
        eprintln!("{line}");
    }
    fs::write(input.output.join("report.json"), serde_json::to_vec_pretty(&serde_json::json!({
        "scope": "offline content scene, static geography and ambient placements; no recorded political state, units or historical particle populations",
        "layers": input.layers.iter().map(|l| serde_json::json!({"root":l.root,"replace_paths":l.replace_paths})).collect::<Vec<_>>(),
        "warnings":warnings,
        "error": result.as_ref().err().map(ToString::to_string),
    })).map_err(io::Error::other)?)?;
    result
}

fn render(layers: &ResolvedLayers, input: &Input, warnings: &mut Vec<String>) -> io::Result<()> {
    eprintln!("Content scene: static map assets");
    let definitions = content_reader_key_names::terrain::read_definitions(layers)?;
    let default_map = content_reader_key_names::terrain::read_default_map(layers)?;
    let province_image =
        content_reader_image::bmp::decode_rgb(&layers.read(&format!("map/{}", default_map.provinces))?)?;
    let map = camera::MapSize {
        width: province_image.width,
        height: province_image.height,
    };
    let defines = content_reader_defines::defines::read(layers)?;
    let decode = content_reader_image::native::Decode {
        force_pow2_textures: input.force_pow2_textures,
    };
    let assets = viewer_world::map_assets::load_with_products(
        None,
        layers,
        &defines,
        &defines,
        map,
        &definitions,
        &default_map,
        warnings,
        0,
        decode,
    )?;
    eprintln!("Content scene: interface assets");
    let map_data = content_reader_map::map::load(layers)?;
    let regions = content_reader_regions::regions::read(layers)?.regions;
    let viewer_playback::map_icons::LoadedIcons {
        combat,
        raids,
        images,
        fonts,
        counters,
        animated_text,
        lazy,
    } = MapIcons::load_with_products(
        None,
        layers,
        (&defines, &defines),
        &map_data,
        &regions,
        &input.language,
        [map_data.width, map_data.height],
        [1280, 720],
        decode,
    )?;
    let icons = MapIcons {
        combat: Arc::new(combat),
        raids: Arc::new(raids),
        intel: Arc::new(viewer_raids::map::intel_icons::IntelAssets::load(
            layers, &defines,
        )),
        occupation: Err("unused".into()),
        images,
        fonts,
        counters: Arc::new(counters),
        animated_text: Arc::new(animated_text),
        lazy,
    };
    let camera_defines = camera::MapDefines::load(&defines, input.camera_tilt)?;
    let heights = HeightField {
        width: map.width,
        height: map.height,
        values: assets.heightmap_texels(),
    };
    let tree_field = trees::generate(
        &trees::TreeImage {
            width: assets.trees.width,
            height: assets.trees.height,
            indices: &assets.trees.indices,
        },
        &heights,
    )?;
    eprintln!("Content scene: tree assets");
    let tree_assets = trees::TreeAssets::load(
        layers,
        decode,
        &tree_field.palettes,
        &assets.rivers,
        warnings,
    )?;
    eprintln!("Content scene: ambient assets");
    let frame_assets = mapframe::FrameAssets::load(
        layers,
        decode,
        icons.raids.trigger_content.facts(),
        warnings,
    )?;
    let river_assets = rivers::RiverAssets::load(layers, [map.width, map.height], 0, decode)?;
    let river_mesh = match rivers::RiverDefines::load(&defines) {
        Ok(river_defines) => {
            rivers::build(
                &rivers::RiverImage {
                    width: map.width,
                    height: map.height,
                    indices: &assets.rivers.indices,
                },
                &heights,
                river_defines,
            )?
            .0
        }
        Err(error) => {
            warnings.push(format!("Rivers are withheld and draw nothing: {error}"));
            rivers::RiverMesh::default()
        }
    };
    eprintln!("Content scene: particle assets");
    let weather = viewer_weather_effects::weather_effects::Assets::load(
        layers,
        &defines,
        [map.width, map.height],
        viewer_weather_effects::weather_effects::RegionProvinces::new(&regions, |province| {
            map_data.definitions.contains_key(&province)
        }),
        decode,
    )?;
    warnings.extend(
        weather
            .withheld
            .iter()
            .map(|why| format!("Withheld, draws nothing: {why}")),
    );
    pollster::block_on(async {
        let lifecycle = shader_pipeline::gpu::device_lifecycle();
        let instance = shader_pipeline::gpu::instance();
        let adapters = instance.enumerate_adapters(wgpu::Backends::all()).await;
        let adapter = select_adapter(&adapters, None)?;
        let (device, queue) = adapter
            .request_device(&map_device_descriptor(&adapter))
            .await
            .map_err(io::Error::other)?;
        drop(lifecycle);
        let mut scene = scene::Scene::new(
            &device,
            &queue,
            &assets,
            &icons.images,
            &icons.lazy,
            Default::default(),
        )?;
        scene.settle_builds(true);
        scene.build_names_in_upload(true);
        scene.set_trees(
            &device,
            &queue,
            &tree_assets,
            trees::geometry(&tree_assets)?,
            &tree_field,
        )?;
        scene.set_map_frame(&device, &queue, &frame_assets)?;
        scene.settle_poses(true);
        scene.warm_weather(&device, &queue, weather)?;
        scene.set_rivers(&device, &queue, &river_assets, &river_mesh)?;
        let impacts = synthetic_impacts(layers, &input.impacts)?;
        for (index, &[x, z, height]) in input.cameras.iter().enumerate() {
            scene.set_nuclear_impacts(
                [&impacts],
                viewer_runtime::presentation::Continuity::Reset,
            );
            eprintln!("Content scene: view {index}");
            let request = scene::Request {
                size: input.size,
                camera: camera::Placement {
                    center: [x, z],
                    height,
                    pose: viewer_gpu::camera::Pose::Map,
                }
                .on(map, camera_defines),
                day_night: frame::DayNight::at_hour(input.hour),
                animation_time: 0.,
                map_time: 0.,
                postfx: true,
                no_water: false,
                no_shadows: false,
                overlay: Default::default(),
                no_trees: false,
            };
            let mut submissions = Vec::new();
            for step in 0..input.presentation_steps.max(2) {
                scene.render(
                    &device,
                    &queue,
                    scene::Request {
                        animation_time: step as f32 * input.presentation_dt,
                        map_time: step as f32 * input.presentation_dt,
                        ..request
                    },
                )?;
                submissions.push(
                    serde_json::json!({"step":step, "particles":scene.offline_particle_packets()}),
                );
                if step == 1 {
                    crate::render::write_png(
                        &scene,
                        &device,
                        &queue,
                        &input.output.join(format!("view-{index}-initial.png")),
                    )?;
                }
            }
            fs::write(
                input.output.join(format!("view-{index}-submissions.json")),
                serde_json::to_vec_pretty(&submissions).map_err(io::Error::other)?,
            )?;
            crate::render::write_png(
                &scene,
                &device,
                &queue,
                &input.output.join(format!("view-{index}.png")),
            )?;
        }
        Ok(())
    })
}

fn synthetic_impacts(
    layers: &ResolvedLayers,
    impacts: &[SyntheticImpact],
) -> io::Result<viewer_models::strike_effects::Impacts> {
    let stacks = content_reader_map::positions::read_unit_stacks(layers)?;
    let map = content_reader_map::map::load(layers)?;
    let boxes = viewer_gpu::province_admission::province_boxes(
        &viewer_gpu::province_map::ProvinceMap {
            width: map.width,
            height: map.height,
            ids: &map.province_ids,
        },
    );
    let mut out = viewer_models::strike_effects::Impacts::default();
    for (id, impact) in impacts.iter().enumerate() {
        let stack = stacks
            .iter()
            .find(|stack| {
                stack.province == impact.province
                    && stack.kind == viewer_foundation::unit_rules::STANDSTILL
            })
            .ok_or_else(|| {
                io::Error::other(format!(
                    "province {} has no authored standstill anchor",
                    impact.province
                ))
            })?;
        let bounds = *boxes.get(&impact.province).ok_or_else(|| {
            io::Error::other(format!("province {} is not painted", impact.province))
        })?;
        out.impacts.push(viewer_models::strike_effects::Impact {
            id: id as u64,
            province: impact.province,
            effect: impact.effect.into(),
            position: [stack.x, stack.y, stack.z],
            bounds,
        });
    }
    Ok(out)
}
