//! Opt-in content-stack integration check; never launches or attaches to the game.
use super::{
    borders::HeightField, camera, frame, mapframe, rivers, scene, sprites, textures, trees,
};
use eframe::wgpu;
use installed_content::ResolvedLayers;
use serde::Deserialize;
use std::{fs, io, path::PathBuf};

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
    /// The client's `settings.txt` `force_pow2_textures`, which the font pages decode under.
    #[serde(default)]
    force_pow2_textures: bool,
    #[serde(default = "default_steps")]
    presentation_steps: u32,
    #[serde(default)]
    presentation_dt: f32,
}

fn default_steps() -> u32 {
    2
}

#[test]
#[ignore = "offline installed-content integration; requires BATTLE_PLAN_CONTENT_SCENE_INPUT JSON"]
fn content_stack_scene() -> io::Result<()> {
    let path = std::env::var_os("BATTLE_PLAN_CONTENT_SCENE_INPUT").or_else(|| std::env::var_os("HOI4_CONTENT_SCENE_INPUT"))
        .ok_or_else(|| io::Error::other("BATTLE_PLAN_CONTENT_SCENE_INPUT is required"))?;
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
    let definitions = installed_content::terrain::read_definitions(layers)?;
    let default_map = installed_content::terrain::read_default_map(layers)?;
    let province_image = installed_content::image::bmp::decode_rgb(
        &layers.read(&format!("map/{}", default_map.provinces))?,
    )?;
    let map = camera::MapSize {
        width: province_image.width,
        height: province_image.height,
    };
    let defines = installed_content::defines::read(layers)?;
    let assets = textures::MapAssets::load(
        layers,
        &defines,
        map,
        &definitions,
        &default_map,
        warnings,
        0,
    )?;
    eprintln!("Content scene: interface assets");
    let map_data = installed_content::map::load(layers)?;
    // No settings.txt here: the constructor's 1280 x 720 frame (`142212BD0`), which decides
    // only which resources the map-icon windows load.
    let icons = sprites::MapIcons::load(
        layers,
        &defines,
        &map_data,
        &input.language,
        [map_data.width, map_data.height],
        [1280, 720],
        installed_content::image::native::Decode {
            force_pow2_textures: input.force_pow2_textures,
        },
    )?;
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
    let tree_assets =
        trees::TreeAssets::load(layers, &tree_field.palettes, &assets.rivers, warnings)?;
    eprintln!("Content scene: ambient assets");
    let frame_assets = mapframe::FrameAssets::load(layers, warnings)?;
    let river_assets = rivers::RiverAssets::load(layers, [map.width, map.height], 0)?;
    let (river_mesh, _) = rivers::build(
        &rivers::RiverImage {
            width: map.width,
            height: map.height,
            indices: &assets.rivers.indices,
        },
        &heights,
    )?;
    eprintln!("Content scene: particle assets");
    let weather = super::weather_effects::Assets::load(layers, &defines, [map.width, map.height])?;
    if !weather.unresolved.is_empty() {
        warnings.push(format!(
            "{} unresolved particle declarations: {}",
            weather.unresolved.len(),
            weather.unresolved.join(", ")
        ));
    }
    pollster::block_on(async {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
        let adapters = instance.enumerate_adapters(wgpu::Backends::all()).await;
        let adapter = crate::render::select_adapter(&adapters, None)?;
        let (device, queue) = adapter
            .request_device(&crate::render::device_descriptor(&adapter))
            .await
            .map_err(io::Error::other)?;
        let mut scene = scene::Scene::new(&device, &queue, &assets, &icons, Default::default())?;
        scene.set_trees(
            &device,
            &queue,
            &tree_assets,
            trees::geometry(&tree_assets),
            &tree_field,
        )?;
        scene.set_map_frame(&device, &queue, &frame_assets)?;
        scene.warm_weather(&device, &queue, weather)?;
        scene.set_rivers(&device, &queue, &river_assets, &river_mesh)?;
        for (index, &[x, z, height]) in input.cameras.iter().enumerate() {
            eprintln!("Content scene: view {index}");
            let request = scene::Request {
                size: input.size,
                camera: camera::Placement {
                    center: [x, z],
                    height,
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
            //The underwater shader consumes the preceding shadow frame.
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
