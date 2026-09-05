use crate::*;
use std::env;
use std::path::PathBuf;

pub fn run_cli() -> Result<()> {
    let mut args = env::args().skip(1).collect::<Vec<_>>();
    if args.is_empty() {
        print_help();
        return Ok(());
    }

    let command = args.remove(0);
    match command.as_str() {
        "bundle" => {
            let options = BundleOptions::parse(&args)?;
            build_bundle(&options)?;
        }
        "render-terrain" => {
            let resolver = &optional_runtime_game_file_resolver(&args)?.ok_or("terrain render requires --game and effective content selection")?;
            let bundle = value_after(&args, "--bundle")
                .map(PathBuf::from)
                .unwrap_or_else(default_bundle_dir);
            if let Some(map_frame_bundle) = value_after(&args, "--map-frame-bundle") {
                let map_frame_bundle = PathBuf::from(map_frame_bundle);
                let frame = validate_map_frame_runtime_bundle_with_resolver(&map_frame_bundle, resolver)?;
                let bundle_dir = map_frame_bundle_dir(&map_frame_bundle)?;
                if args.iter().any(|arg| arg == "--require-post-diffuse") {
                    validate_map_frame_post_diffuse_inputs(resolver, &frame, &bundle, &bundle_dir)?;
                }
                let output = value_after(&args, "--out")
                    .map(PathBuf::from)
                    .unwrap_or_else(|| bundle_dir.join("map_frame_terrain_render.rgbx"));
                let summary = render_map_frame_terrain(resolver, &frame, &bundle_dir, &bundle, &output)?;
                eprintln!(
                    "map-frame terrain rendered: {} draws, {} indices, {} triangles, {} covered pixels of {}, SnowMudData loaded {}, output {}",
                    summary.draw_count,
                    summary.index_count,
                    summary.triangle_count,
                    summary.covered_pixel_count,
                    summary.output_width * summary.output_height,
                    summary.snow_mud_data_loaded,
                    output.display()
                );
                return Ok(());
            }
            let output = value_after(&args, "--out")
                .map(PathBuf::from)
                .unwrap_or_else(|| bundle.join("terrain_render.rgbx"));
            let summary = render_production_terrain(resolver, &bundle, &output)?;
            eprintln!(
                "production terrain rendered: {} inputs, TerrainIDMap {}x{}, MAP_POW2 {}, {}, FOW_POW2 {}, {}, output {}",
                summary.validated_input_count,
                summary.terrain_id_map_width,
                summary.terrain_id_map_height,
                summary.map_pow2_x,
                summary.map_pow2_y,
                summary.fow_pow2_x,
                summary.fow_pow2_y,
                output.display()
            );
        }
        "render-terrain-d3d11" => {
            let map_frame_bundle =
                PathBuf::from(required_value_after(&args, "--map-frame-bundle")?);
            let output = value_after(&args, "--out").map(PathBuf::from);
            let parity_report = value_after(&args, "--parity-report").map(PathBuf::from);
            let repeat = optional_positive_usize_arg(&args, "--repeat")?.unwrap_or(1);
            let compare_each_draw = args.iter().any(|arg| arg == "--compare-each-draw");
            let resolver = optional_runtime_game_file_resolver(&args)?
                .ok_or("D3D11 rendering requires --game and effective content selection")?;
            let bundle = NativeD3d11TerrainBundle::load_with_resolver(&map_frame_bundle, &resolver)?;
            let renderer = NativeD3d11TerrainRenderer::new()?
                .with_reflection_resolver(resolver);
            let prepared = renderer.prepare_production_map(&bundle, None)?;
            let mut summary = None;
            for iteration in 0..repeat {
                let final_iteration = iteration + 1 == repeat;
                let options = NativeD3d11RenderOptions {
                    output: final_iteration.then(|| output.clone()).flatten(),
                    readback: false,
                    compare_each_draw,
                    parity_report: final_iteration.then(|| parity_report.clone()).flatten(),
                    ..NativeD3d11RenderOptions::default()
                };
                summary = Some(
                    renderer
                        .render_prepared_production_map(&prepared, &options)?
                        .summary,
                );
            }
            let summary = summary.ok_or("native D3D11 terrain replay did not execute")?;
            eprintln!(
                "production native D3D11 terrain pass completed: {} repeat(s), {} draws, {} vertex buffers, {} index buffers, {} constant buffers, {} SRVs, {} samplers, target {}x{} format {}, output {}",
                repeat,
                summary.draw_count,
                summary.vertex_buffer_count,
                summary.index_buffer_count,
                summary.constant_buffer_count,
                summary.shader_resource_count,
                summary.sampler_count,
                summary.target_width,
                summary.target_height,
                summary.target_format,
                output
                    .as_ref()
                    .map(|path| path.display().to_string())
                    .unwrap_or_else(|| "not requested".to_string())
            );
        }
        "validate-terrain-chunk-synthesis" => {
            let library = PathBuf::from(required_value_after(&args, "--terrain-chunk-library")?);
            let report_path = value_after(&args, "--report").map(PathBuf::from);
            let report = validate_terrain_chunk_synthesis(&optional_runtime_game_file_resolver(&args)?.ok_or("chunk validation requires --game and effective content selection")?, &library, report_path.as_deref())?;
            eprintln!(
                "terrain chunk synthesis byte-exact validation: status {}, {} compared records",
                report.status,
                report.compared_records.len()
            );
            for comparison in &report.compared_records {
                eprintln!(
                    "record {} tile_size {} cb1 [{:.3}, {:.3}, {:.3}, {:.3}] captured {} synthesized {} identical {} first_mismatch_byte {} first_mismatch_texel {}",
                    comparison.record_chunk_id,
                    comparison.tile_size,
                    comparison.quad_offset_scale_is_detail[0],
                    comparison.quad_offset_scale_is_detail[1],
                    comparison.quad_offset_scale_is_detail[2],
                    comparison.quad_offset_scale_is_detail[3],
                    comparison.captured_fnv1a64,
                    comparison.synthesized_fnv1a64,
                    comparison.identical,
                    comparison
                        .first_mismatch_byte
                        .map(|value| value.to_string())
                        .unwrap_or_else(|| "none".to_string()),
                    comparison
                        .first_mismatch_texel
                        .map(|value| format!("{},{}", value[0], value[1]))
                        .unwrap_or_else(|| "none".to_string())
                );
            }
            if report.status != "identical" {
                return Err(
                    "terrain chunk synthesis byte-exact validation found mismatches".into(),
                );
            }
        }
        "smoke-terrain-chunk-synthesis" => {
            let library = PathBuf::from(required_value_after(&args, "--terrain-chunk-library")?);
            let target_tile_size = optional_u32_arg(&args, "--target-tile-size")?.unwrap_or(128);
            let output = value_after(&args, "--out").map(PathBuf::from);
            let report_path = value_after(&args, "--report").map(PathBuf::from);
            let cull_rect = match value_after(&args, "--cull-rect") {
                Some(spec) => {
                    let parts: Vec<f32> = spec
                        .split(',')
                        .map(|part| part.trim().parse::<f32>())
                        .collect::<std::result::Result<_, _>>()
                        .map_err(|err| format!("--cull-rect: {err}"))?;
                    if parts.len() != 4 {
                        return Err("--cull-rect expects x0,z0,x1,z1".into());
                    }
                    Some([parts[0], parts[1], parts[2], parts[3]])
                }
                None => None,
            };
            let scene_frame_bundle = value_after(&args, "--scene-frame-bundle")
                .map(PathBuf::from)
                .or_else(|| {
                    let capture_dir = if library.is_dir() {
                        Some(library.as_path())
                    } else {
                        library.parent()
                    };
                    let sibling = capture_dir?.join("map_frame_bundle.json");
                    sibling.is_file().then_some(sibling)
                });
            let report = smoke_terrain_chunk_synthesis(&optional_runtime_game_file_resolver(&args)?.ok_or("chunk validation requires --game and effective content selection")?, 
                &library,
                target_tile_size,
                output.as_deref(),
                cull_rect,
                scene_frame_bundle.as_deref(),
            )?;
            if let Some(report_path) = report_path.as_ref() {
                write_json(report_path, &report)?;
            }
            eprintln!(
                "terrain chunk synthesis smoke: status {}, tile_size {}, expected_tile_count {}, rendered_tile_count {}, target {}x{} format {}, elapsed_ms {:.2}, scene_frame_bundle {}, output {}",
                report.status,
                report.target_tile_size,
                report.expected_tile_count,
                report.rendered_tile_count,
                report.target_width,
                report.target_height,
                report.target_format,
                report.elapsed_ms,
                report
                    .scene_frame_bundle
                    .as_deref()
                    .unwrap_or("none (template frame state)"),
                output
                    .as_ref()
                    .map(|path| path.display().to_string())
                    .unwrap_or_else(|| "not requested".to_string())
            );
            if cull_rect.is_none() && report.expected_tile_count != report.rendered_tile_count {
                return Err("terrain chunk synthesis smoke rendered unexpected tile count".into());
            }
        }
        "render-production-map-d3d11" => {
            let terrain_map_frame_bundle =
                PathBuf::from(required_value_after(&args, "--terrain-map-frame-bundle")?);
            let water_map_frame_bundle =
                value_after(&args, "--water-map-frame-bundle").map(PathBuf::from);
            let rivers_map_frame_bundle =
                value_after(&args, "--rivers-map-frame-bundle").map(PathBuf::from);
            let borders_map_frame_bundle =
                value_after(&args, "--borders-map-frame-bundle").map(PathBuf::from);
            let trees_map_frame_bundle =
                value_after(&args, "--trees-map-frame-bundle").map(PathBuf::from);
            let output = value_after(&args, "--out").map(PathBuf::from);
            let parity_report = value_after(&args, "--parity-report").map(PathBuf::from);
            let repeat = optional_positive_usize_arg(&args, "--repeat")?.unwrap_or(1);
            let compare_each_draw = args.iter().any(|arg| arg == "--compare-each-draw");
            let compare_final_reference = !args.iter().any(|arg| arg == "--skip-reference-compare");
            let map_clock_time = optional_nonnegative_f32_arg(&args, "--map-clock-time")?;
            let water_full = args.iter().any(|arg| arg == "--water-full");
            let resolver = optional_runtime_game_file_resolver(&args)?
                .ok_or("D3D11 rendering requires --game and effective content selection")?;
            let terrain_bundle = NativeD3d11TerrainBundle::load_with_resolver(
                &terrain_map_frame_bundle,
                &resolver,
            )?;
            let camera = if let Some(camera_path) = value_after(&args, "--camera-json") {
                production_map_camera_from_json(&PathBuf::from(camera_path))?
            } else {
                terrain_bundle.production_camera_from_capture(&resolver)?
            };
            let mut renderer = NativeD3d11TerrainRenderer::new()?
                .with_water_full(water_full)
                .with_reflection_resolver(resolver.clone());
            if water_full {
                renderer = renderer.with_underwater_resolver(resolver.clone());
            }
            let water_bundle = water_map_frame_bundle
                .as_ref()
                .map(|path| NativeD3d11TerrainBundle::load_water_with_resolver(path, &resolver))
                .transpose()?;
            let rivers_bundle = rivers_map_frame_bundle
                .as_ref()
                .map(|path| NativeD3d11TerrainBundle::load_rivers_with_resolver(path, &resolver))
                .transpose()?;
            let borders_bundle = borders_map_frame_bundle
                .as_ref()
                .map(|path| NativeD3d11TerrainBundle::load_borders_with_resolver(path, &resolver))
                .transpose()?;
            let trees_bundle = trees_map_frame_bundle
                .as_ref()
                .map(|path| NativeD3d11TerrainBundle::load_trees_with_resolver(path, &resolver))
                .transpose()?;
            let mut prepared = if map_clock_time.is_some() {
                renderer.prepare_production_map_with_layers(
                    &terrain_bundle,
                    water_bundle.as_ref(),
                    rivers_bundle.as_ref(),
                    borders_bundle.as_ref(),
                    trees_bundle.as_ref(),
                    camera,
                )?
            } else {
                renderer.prepare_reference_production_map_with_layers(
                    &terrain_bundle,
                    water_bundle.as_ref(),
                    rivers_bundle.as_ref(),
                    borders_bundle.as_ref(),
                    trees_bundle.as_ref(),
                    camera,
                )?
            };
            if let Some(clock_time) = map_clock_time {
                renderer.update_prepared_production_map_scene(
                    &mut prepared,
                    camera,
                    None,
                    Some(clock_time),
                    None,
                )?;
            }
            let ready_layers = prepared.ready_dependent_pass_labels();
            let mut result = None;
            for iteration in 0..repeat {
                let final_iteration = iteration + 1 == repeat;
                let options = NativeD3d11RenderOptions {
                    output: final_iteration.then(|| output.clone()).flatten(),
                    readback: false,
                    compare_each_draw,
                    compare_final_reference,
                    parity_report: final_iteration.then(|| parity_report.clone()).flatten(),
                    map_clock_time,
                    ..NativeD3d11RenderOptions::default()
                };
                result = Some(renderer.render_prepared_production_map(&prepared, &options)?);
            }
            let result = result.ok_or("production native D3D11 map graph did not execute")?;
            let final_hash = result
                .reference_comparison
                .as_ref()
                .map(|comparison| comparison.actual_fnv1a64.clone())
                .unwrap_or_else(|| "not compared".to_string());
            eprintln!(
                "production native D3D11 map graph completed: {} repeat(s), ready dependent passes [{}], final pass {} draws, target {}x{} format {}, final fnv1a64 {}, output {}",
                repeat,
                if ready_layers.is_empty() {
                    "none".to_string()
                } else {
                    ready_layers.join(", ")
                },
                result.summary.draw_count,
                result.summary.target_width,
                result.summary.target_height,
                result.summary.target_format,
                final_hash,
                output
                    .as_ref()
                    .map(|path| path.display().to_string())
                    .unwrap_or_else(|| "not requested".to_string())
            );
        }
        "accept-terrain-d3d11" => {
            let map_frame_bundle =
                PathBuf::from(required_value_after(&args, "--map-frame-bundle")?);
            let output = value_after(&args, "--out").map(PathBuf::from);
            let report = value_after(&args, "--report").map(PathBuf::from);
            let repeat = optional_positive_usize_arg(&args, "--repeat")?.unwrap_or(1);
            let request = TerrainD3d11AcceptanceRequest {
                map_frame_bundle,
                output,
                report,
                repeat,
            };
            let resolver = optional_runtime_game_file_resolver(&args)?
                .ok_or("D3D11 rendering requires --game and effective content selection")?;
            let report = accept_terrain_d3d11_with_resolver(&request, &resolver)?;
            eprintln!(
                "terrain D3D11 acceptance completed: {} repeat(s), {} draws, target {}x{} format {}, final fnv1a64 {}, report {}",
                report.repeat,
                report.draw_count,
                report.target_width,
                report.target_height,
                report.target_format,
                report.final_actual_fnv1a64,
                request
                    .report
                    .as_ref()
                    .map(|path| path.display().to_string())
                    .unwrap_or_else(|| "terrain_d3d11_acceptance.json".to_string())
            );
        }
        "validate-water-map-frame" => {
            let map_frame_bundle =
                PathBuf::from(required_value_after(&args, "--map-frame-bundle")?);
            validate_water_map_frame_bundle(&map_frame_bundle)?;
            eprintln!(
                "water map-frame bundle validated: {}",
                map_frame_bundle.display()
            );
        }
        "validate-rivers-map-frame" => {
            let map_frame_bundle =
                PathBuf::from(required_value_after(&args, "--map-frame-bundle")?);
            validate_rivers_map_frame_bundle(&map_frame_bundle)?;
            eprintln!(
                "rivers map-frame bundle validated: {}",
                map_frame_bundle.display()
            );
        }
        "validate-borders-map-frame" => {
            let map_frame_bundle =
                PathBuf::from(required_value_after(&args, "--map-frame-bundle")?);
            validate_borders_map_frame_bundle(&map_frame_bundle)?;
            eprintln!(
                "borders map-frame bundle validated: {}",
                map_frame_bundle.display()
            );
        }
        "validate-trees-map-frame" => {
            let map_frame_bundle =
                PathBuf::from(required_value_after(&args, "--map-frame-bundle")?);
            validate_trees_map_frame_bundle(&map_frame_bundle)?;
            eprintln!(
                "trees map-frame bundle validated: {}",
                map_frame_bundle.display()
            );
        }
        "import-terrain-runtime" => {
            let options = ImportTerrainRuntimeOptions::parse(&args)?;
            import_terrain_runtime_inputs(&optional_runtime_game_file_resolver(&args)?.ok_or("game source validation requires --game and effective content selection")?, &options)?;
        }
        "import-runtime-texture" => {
            let options = ImportRuntimeTextureOptions::parse(&args)?;
            import_runtime_texture_input(&optional_runtime_game_file_resolver(&args)?.ok_or("runtime texture import requires --game and effective content selection")?, &options)?;
        }
        "import-snow-mud-runtime" => {
            let options = ImportRuntimeTextureOptions::parse_snow_mud(SNOW_MUD_DATA_SYMBOL, &args)?;
            import_runtime_texture_input(&optional_runtime_game_file_resolver(&args)?.ok_or("runtime texture import requires --game and effective content selection")?, &options)?;
        }
        "import-runtime-constant-buffer" => {
            let options = ImportRuntimeConstantBufferOptions::parse(&args)?;
            import_runtime_constant_buffer_input(&optional_runtime_game_file_resolver(&args)?.ok_or("constant buffer import requires --game and effective content selection")?, &options)?;
        }
        "import-terrain-scene-cb0-runtime" => {
            let options = ImportRuntimeConstantBufferOptions::parse_scene_cb0(
                TERRAIN_SCENE_CB0_SYMBOL,
                &args,
            )?;
            import_runtime_constant_buffer_input(&optional_runtime_game_file_resolver(&args)?.ok_or("constant buffer import requires --game and effective content selection")?, &options)?;
        }
        "import-terrain-vertex-buffer-runtime" => {
            let options = ImportRuntimeVertexBufferOptions::parse_terrain_vertex_buffer(
                TERRAIN_VERTEX_BUFFER_SYMBOL,
                &args,
            )?;
            import_runtime_vertex_buffer_input(&optional_runtime_game_file_resolver(&args)?.ok_or("game source validation requires --game and effective content selection")?, &options)?;
        }
        "import-terrain-input-layout-runtime" => {
            let options = ImportRuntimeInputLayoutOptions::parse_terrain_input_layout(
                TERRAIN_INPUT_LAYOUT_SYMBOL,
                &args,
            )?;
            import_runtime_input_layout(&optional_runtime_game_file_resolver(&args)?.ok_or("game source validation requires --game and effective content selection")?, &options)?;
        }
        "scan-shader-cache" => {
            let shader_cache = PathBuf::from(required_value_after(&args, "--shader-cache")?);
            let terrain_id_slot = optional_u32_arg(&args, "--terrain-id-slot")?;
            let constants = find_shader_cache_map_constants(&shader_cache, terrain_id_slot)?;
            println!(
                "shader cache map constants: {} MAP_SIZE {}, {} MAP_POW2 {}, {} FOW_POW2 {}, {} TerrainIDMap t{}",
                constants.cache_file,
                constants.map_size_x,
                constants.map_size_y,
                constants.map_pow2_x,
                constants.map_pow2_y,
                constants.fow_pow2_x,
                constants.fow_pow2_y,
                constants.terrain_id_srv_slot
            );
        }
        "help" | "--help" | "-h" => print_help(),
        _ => {
            return Err(format!("unknown command: {command}").into());
        }
    }
    Ok(())
}

pub(crate) fn print_help() {
    eprintln!(
        "map_viewer\n\n\
         Commands:\n\
           bundle --game <the game install> [--mod <content-dir> ...] [--use-dlc-load --userdata <dir>] [--out <bundle dir>] [--force]\n\
           render-terrain [--bundle <bundle dir>] [--map-frame-bundle <map_frame_bundle.json|capture dir>] [--require-post-diffuse] [--out <terrain_render.rgbx>]\n\
render-terrain-d3d11 --map-frame-bundle <map_frame_bundle.json|capture dir> --game <the game install> [--mod <content-dir> ...] [--use-dlc-load --userdata <dir>] [--out <terrain_native.r16g16b16a16_float>] [--repeat <n>] [--compare-each-draw] [--parity-report <dir>]\n\
           validate-terrain-chunk-synthesis --terrain-chunk-library <terrain_chunk_library.json|capture dir> [--report <terrain_chunk_synthesis_validation.json>]\n\
           smoke-terrain-chunk-synthesis --terrain-chunk-library <terrain_chunk_library.json|capture dir> [--target-tile-size <world units>] [--scene-frame-bundle <map_frame_bundle.json>] [--out <terrain_chunk_synthesis.raw>] [--report <terrain_chunk_synthesis_smoke.json>]\n\
render-production-map-d3d11 --terrain-map-frame-bundle <map_frame_bundle.json|capture dir> --game <the game install> [--mod <content-dir> ...] [--use-dlc-load --userdata <dir>] [--camera-json <production_camera.json>] [--water-map-frame-bundle <map_frame_water_pass_bundle.json|capture dir>] [--out <map_native.raw>] [--repeat <n>] [--compare-each-draw] [--skip-reference-compare] [--parity-report <dir>]\n\
accept-terrain-d3d11 --map-frame-bundle <map_frame_bundle.json|capture dir> --game <the game install> [--mod <content-dir> ...] [--use-dlc-load --userdata <dir>] [--out <terrain_native.r16g16b16a16_float>] [--report <terrain_d3d11_acceptance.json>] [--repeat <n>]\n\
           validate-water-map-frame --map-frame-bundle <map_frame_water_pass_bundle.json|capture dir>\n\
           validate-rivers-map-frame --map-frame-bundle <map_frame_rivers_pass_bundle.json|capture dir>\n\
           validate-borders-map-frame --map-frame-bundle <map_frame_borders_pass_bundle.json|capture dir>\n\
           validate-trees-map-frame --map-frame-bundle <map_frame_trees_pass_bundle.json|capture dir>\n\
           import-terrain-runtime --bundle <bundle dir> --terrain-id-map-rgba <raw rgba> --width <w> --height <h> (--map-pow2-x <x> --map-pow2-y <y> --fow-pow2-x <x> --fow-pow2-y <y> | --shader-cache <dir>) [--provenance <label>] [--force]\n\
           import-runtime-texture --bundle <bundle dir> --symbol <shader sampler> --rgba <raw rgba> --width <w> --height <h> [--provenance <label>] [--force]\n\
           import-snow-mud-runtime --bundle <bundle dir> --snow-mud-rgba <raw rgba> --width <w> --height <h> [--provenance <label>] [--force]\n\
           import-runtime-constant-buffer --bundle <bundle dir> --symbol <shader cbuffer> --bin <raw bytes> [--provenance <label>] [--force]\n\
           import-terrain-scene-cb0-runtime --bundle <bundle dir> --cb0-bin <raw bytes> [--provenance <label>] [--force]\n\
           import-terrain-vertex-buffer-runtime --bundle <bundle dir> --capture-json <terrain_vertex_buffer_capture.json> [--provenance <label>] [--force]\n\
           import-terrain-input-layout-runtime --bundle <bundle dir> --capture-json <terrain_input_layout_capture.json> [--provenance <label>] [--force]\n\
           scan-shader-cache --shader-cache <dir> [--terrain-id-slot <n>]\n\n\
         Environment:\n\
           BATTLE_PLAN_GAME_DIR can supply --game.\n"
    );
}

fn optional_runtime_game_file_resolver(args: &[String]) -> Result<Option<GameFileResolver>> {
    let game = value_after(args, "--game").or_else(|| env::var("BATTLE_PLAN_GAME_DIR").or_else(|_| env::var("HOI4_GAME_DIR")).ok());
    let mod_paths = values_after(args, "--mod")
        .into_iter()
        .map(PathBuf::from)
        .collect::<Vec<_>>();
    let use_dlc_load = args.iter().any(|arg| arg == "--use-dlc-load");
    let userdata = value_after(args, "--userdata").map(PathBuf::from);
    let wants_resolver =
        game.is_some() || !mod_paths.is_empty() || use_dlc_load || userdata.is_some();
    if !wants_resolver {
        return Ok(None);
    }
    let game =
        game.ok_or("pass --game <the game install> or set BATTLE_PLAN_GAME_DIR to synthesize TerrainIDMap")?;
    if use_dlc_load && !mod_paths.is_empty() {
        return Err("pass either repeated --mod content dirs or --use-dlc-load, not both".into());
    }
    if use_dlc_load && userdata.is_none() {
        return Err("pass --userdata <the game user-data dir> with --use-dlc-load".into());
    }
    Ok(Some(GameFileResolver::from_options(
        PathBuf::from(game),
        GameFileResolverOptions {
            mod_paths,
            use_dlc_load,
            userdata,
        },
    )?))
}

#[derive(Clone)]
pub(crate) struct BundleOptions {
    pub(crate) game: PathBuf,
    pub(crate) out: PathBuf,
    pub(crate) force: bool,
    pub(crate) mod_selection: BundleModSelectionOptions,
}

impl BundleOptions {
    pub(crate) fn parse(args: &[String]) -> Result<Self> {
        let game = value_after(args, "--game")
            .or_else(|| env::var("BATTLE_PLAN_GAME_DIR").or_else(|_| env::var("HOI4_GAME_DIR")).ok())
            .ok_or("pass --game <the game install> or set BATTLE_PLAN_GAME_DIR")?;
        let out = value_after(args, "--out")
            .map(PathBuf::from)
            .unwrap_or_else(default_bundle_dir);
        let force = args.iter().any(|a| a == "--force");
        let mods = values_after(args, "--mod")
            .into_iter()
            .map(PathBuf::from)
            .collect::<Vec<_>>();
        let use_dlc_load = args.iter().any(|a| a == "--use-dlc-load");
        if use_dlc_load && !mods.is_empty() {
            return Err(
                "pass either repeated --mod content dirs or --use-dlc-load, not both".into(),
            );
        }
        let userdata = value_after(args, "--userdata").map(PathBuf::from);
        if use_dlc_load && userdata.is_none() {
            return Err("pass --userdata <the game user-data dir> with --use-dlc-load".into());
        }
        Ok(Self {
            game: PathBuf::from(game),
            out,
            force,
            mod_selection: BundleModSelectionOptions {
                mods,
                use_dlc_load,
                userdata,
            },
        })
    }
}

impl ImportTerrainRuntimeOptions {
    pub(crate) fn parse(args: &[String]) -> Result<Self> {
        let bundle = value_after(args, "--bundle")
            .map(PathBuf::from)
            .unwrap_or_else(default_bundle_dir);
        let terrain_id_map_rgba =
            PathBuf::from(required_value_after(args, "--terrain-id-map-rgba")?);
        let width = parse_positive_usize_arg(args, "--width")?;
        let height = parse_positive_usize_arg(args, "--height")?;
        let pow2 = parse_pow2_source(args)?;
        let provenance = value_after(args, "--provenance")
            .unwrap_or_else(|| "runtime_extraction_unlabeled".to_string());
        let force = args.iter().any(|a| a == "--force");
        Ok(Self {
            bundle,
            terrain_id_map_rgba,
            width,
            height,
            map_pow2_x: pow2.map_pow2_x,
            map_pow2_y: pow2.map_pow2_y,
            map_pow2_extraction: pow2.map_pow2_extraction,
            fow_pow2_x: pow2.fow_pow2_x,
            fow_pow2_y: pow2.fow_pow2_y,
            fow_pow2_extraction: pow2.fow_pow2_extraction,
            provenance,
            force,
        })
    }
}

impl ImportRuntimeTextureOptions {
    pub(crate) fn parse(args: &[String]) -> Result<Self> {
        let bundle = value_after(args, "--bundle")
            .map(PathBuf::from)
            .unwrap_or_else(default_bundle_dir);
        let source_rgba = PathBuf::from(required_value_after(args, "--rgba")?);
        let width = parse_positive_usize_arg(args, "--width")?;
        let height = parse_positive_usize_arg(args, "--height")?;
        let shader_symbol = required_value_after(args, "--symbol")?;
        let provenance = value_after(args, "--provenance")
            .unwrap_or_else(|| "runtime_extraction_unlabeled".to_string());
        let force = args.iter().any(|a| a == "--force");
        Ok(Self {
            bundle,
            source_rgba,
            width,
            height,
            shader_symbol,
            provenance,
            force,
        })
    }

    pub(crate) fn parse_snow_mud(shader_symbol: &str, args: &[String]) -> Result<Self> {
        let bundle = value_after(args, "--bundle")
            .map(PathBuf::from)
            .unwrap_or_else(default_bundle_dir);
        let source_rgba = PathBuf::from(required_value_after(args, "--snow-mud-rgba")?);
        let width = parse_positive_usize_arg(args, "--width")?;
        let height = parse_positive_usize_arg(args, "--height")?;
        let provenance = value_after(args, "--provenance")
            .unwrap_or_else(|| "runtime_extraction_unlabeled".to_string());
        let force = args.iter().any(|a| a == "--force");
        Ok(Self {
            bundle,
            source_rgba,
            width,
            height,
            shader_symbol: shader_symbol.to_string(),
            provenance,
            force,
        })
    }
}

impl ImportRuntimeConstantBufferOptions {
    pub(crate) fn parse(args: &[String]) -> Result<Self> {
        let bundle = value_after(args, "--bundle")
            .map(PathBuf::from)
            .unwrap_or_else(default_bundle_dir);
        let source_bin = PathBuf::from(required_value_after(args, "--bin")?);
        let shader_symbol = required_value_after(args, "--symbol")?;
        let provenance = value_after(args, "--provenance")
            .unwrap_or_else(|| "runtime_extraction_unlabeled".to_string());
        let force = args.iter().any(|a| a == "--force");
        Ok(Self {
            bundle,
            source_bin,
            shader_symbol,
            provenance,
            force,
        })
    }

    pub(crate) fn parse_scene_cb0(shader_symbol: &str, args: &[String]) -> Result<Self> {
        let bundle = value_after(args, "--bundle")
            .map(PathBuf::from)
            .unwrap_or_else(default_bundle_dir);
        let source_bin = PathBuf::from(required_value_after(args, "--cb0-bin")?);
        let provenance = value_after(args, "--provenance")
            .unwrap_or_else(|| "runtime_extraction_unlabeled".to_string());
        let force = args.iter().any(|a| a == "--force");
        Ok(Self {
            bundle,
            source_bin,
            shader_symbol: shader_symbol.to_string(),
            provenance,
            force,
        })
    }
}

impl ImportRuntimeVertexBufferOptions {
    pub(crate) fn parse_terrain_vertex_buffer(
        shader_symbol: &str,
        args: &[String],
    ) -> Result<Self> {
        let bundle = value_after(args, "--bundle")
            .map(PathBuf::from)
            .unwrap_or_else(default_bundle_dir);
        let capture_json = PathBuf::from(required_value_after(args, "--capture-json")?);
        let provenance = value_after(args, "--provenance")
            .unwrap_or_else(|| "runtime_extraction_unlabeled".to_string());
        let force = args.iter().any(|a| a == "--force");
        Ok(Self {
            bundle,
            capture_json,
            shader_symbol: shader_symbol.to_string(),
            provenance,
            force,
        })
    }
}

impl ImportRuntimeInputLayoutOptions {
    pub(crate) fn parse_terrain_input_layout(shader_symbol: &str, args: &[String]) -> Result<Self> {
        let bundle = value_after(args, "--bundle")
            .map(PathBuf::from)
            .unwrap_or_else(default_bundle_dir);
        let capture_json = PathBuf::from(required_value_after(args, "--capture-json")?);
        let provenance = value_after(args, "--provenance")
            .unwrap_or_else(|| "runtime_extraction_unlabeled".to_string());
        let force = args.iter().any(|a| a == "--force");
        Ok(Self {
            bundle,
            capture_json,
            shader_symbol: shader_symbol.to_string(),
            provenance,
            force,
        })
    }
}

pub(crate) fn default_height_normal_file() -> String {
    "height_normal.rgbx".to_string()
}

pub(crate) fn default_terrain_id_map_file() -> String {
    "terrain_id_map.rgba".to_string()
}

pub(crate) fn default_terrain_runtime_file() -> String {
    "terrain_runtime.json".to_string()
}

pub(crate) fn default_snow_mud_data_file() -> String {
    "snow_mud_data.rgba".to_string()
}

pub(crate) fn default_terrain_scene_cb0_file() -> String {
    "terrain_scene_cb0.bin".to_string()
}

pub(crate) fn default_province_secondary_color_map_file() -> String {
    "province_secondary_color_map.rgba".to_string()
}

pub(crate) fn default_gradient_border_channel1_file() -> String {
    "gradient_border_channel1.rgba".to_string()
}

pub(crate) fn default_gradient_border_channel2_file() -> String {
    "gradient_border_channel2.rgba".to_string()
}

pub(crate) fn default_gradient_border_channel3_file() -> String {
    "gradient_border_channel3.rgba".to_string()
}

pub(crate) fn default_light_index_map_file() -> String {
    "light_index_map.rgba".to_string()
}

pub(crate) fn default_light_data_map_file() -> String {
    "light_data_map.rgba".to_string()
}

pub(crate) fn default_shadow_map_file() -> String {
    "shadow_map.rgba".to_string()
}

pub(crate) fn default_cities_file() -> String {
    "cities.r8".to_string()
}

pub(crate) fn default_trees_file() -> String {
    "trees.r8".to_string()
}

pub(crate) fn default_tree_tint_file() -> String {
    "tree_tint.rgbx".to_string()
}

pub(crate) fn default_tree_season_file() -> String {
    "tree_season.rgbx".to_string()
}

pub(crate) fn default_map_object_meta_file() -> String {
    "map_object_meta.json".to_string()
}

pub(crate) fn default_map_material_contract_file() -> String {
    "map_material_contract.json".to_string()
}

pub(crate) fn default_terrain_indices_file() -> String {
    "terrain_indices.r8".to_string()
}

pub(crate) fn default_terrain_texture_ids_file() -> String {
    "terrain_texture_ids.r8".to_string()
}

pub(crate) fn default_heightmap_file() -> String {
    "heightmap.r8".to_string()
}

pub(crate) fn default_rivers_file() -> String {
    "rivers.r8".to_string()
}

pub(crate) fn default_terrain_diffuse_atlas_file() -> String {
    "terrain_diffuse_atlas0.rgbx".to_string()
}

pub(crate) fn default_terrain_normal_atlas_file() -> String {
    "terrain_normal_atlas0.rgbx".to_string()
}

pub(crate) fn default_terrain_diffuse_atlas_mips_file() -> String {
    "terrain_diffuse_atlas0_mips.rgba".to_string()
}

pub(crate) fn default_terrain_normal_atlas_mips_file() -> String {
    "terrain_normal_atlas0_mips.rgba".to_string()
}

pub(crate) fn value_after(args: &[String], flag: &str) -> Option<String> {
    args.windows(2)
        .find(|pair| pair[0] == flag)
        .map(|pair| pair[1].clone())
}

pub(crate) fn values_after(args: &[String], flag: &str) -> Vec<String> {
    args.windows(2)
        .filter(|pair| pair[0] == flag)
        .map(|pair| pair[1].clone())
        .collect()
}

pub(crate) fn required_value_after(args: &[String], flag: &str) -> Result<String> {
    value_after(args, flag).ok_or_else(|| format!("missing required {flag}").into())
}

pub(crate) fn parse_positive_usize_arg(args: &[String], flag: &str) -> Result<usize> {
    let value = required_value_after(args, flag)?;
    let parsed = value
        .parse::<usize>()
        .map_err(|_| format!("{flag} must be a positive integer, got {value}"))?;
    if parsed == 0 {
        return Err(format!("{flag} must be greater than zero").into());
    }
    Ok(parsed)
}

pub(crate) fn optional_positive_usize_arg(args: &[String], flag: &str) -> Result<Option<usize>> {
    value_after(args, flag)
        .map(|value| {
            let parsed = value
                .parse::<usize>()
                .map_err(|_| format!("{flag} must be a positive integer, got {value}"))?;
            if parsed == 0 {
                return Err(format!("{flag} must be greater than zero").into());
            }
            Ok(parsed)
        })
        .transpose()
}

pub(crate) fn optional_u32_arg(args: &[String], flag: &str) -> Result<Option<u32>> {
    value_after(args, flag)
        .map(|value| {
            value
                .parse::<u32>()
                .map_err(|_| format!("{flag} must be a non-negative integer, got {value}").into())
        })
        .transpose()
}

pub(crate) fn optional_nonnegative_f32_arg(args: &[String], flag: &str) -> Result<Option<f32>> {
    value_after(args, flag)
        .map(|value| {
            let parsed = value
                .parse::<f32>()
                .map_err(|_| format!("{flag} must be a finite non-negative number, got {value}"))?;
            if !parsed.is_finite() || parsed < 0.0 {
                return Err(format!("{flag} must be a finite non-negative number").into());
            }
            Ok(parsed)
        })
        .transpose()
}

pub(crate) fn parse_positive_f32_arg(args: &[String], flag: &str) -> Result<f32> {
    let value = required_value_after(args, flag)?;
    let parsed = value
        .parse::<f32>()
        .map_err(|_| format!("{flag} must be a finite positive number, got {value}"))?;
    if !parsed.is_finite() || parsed <= 0.0 {
        return Err(format!("{flag} must be a finite positive number").into());
    }
    Ok(parsed)
}

pub(crate) fn parse_pow2_source(args: &[String]) -> Result<RuntimePow2Source> {
    match (
        value_after(args, "--map-pow2-x"),
        value_after(args, "--map-pow2-y"),
        value_after(args, "--fow-pow2-x"),
        value_after(args, "--fow-pow2-y"),
        value_after(args, "--shader-cache"),
    ) {
        (Some(_), Some(_), Some(_), Some(_), Some(_)) => Err(
            "pass either explicit --map-pow2/--fow-pow2 values or --shader-cache, not both".into(),
        ),
        (Some(_), Some(_), Some(_), Some(_), None) => Ok(RuntimePow2Source {
            map_pow2_x: parse_positive_f32_arg(args, "--map-pow2-x")?,
            map_pow2_y: parse_positive_f32_arg(args, "--map-pow2-y")?,
            map_pow2_extraction: "runtime_extracted_shader_constant_no_static_fallback"
                .to_string(),
            fow_pow2_x: parse_positive_f32_arg(args, "--fow-pow2-x")?,
            fow_pow2_y: parse_positive_f32_arg(args, "--fow-pow2-y")?,
            fow_pow2_extraction: "runtime_extracted_shader_constant_no_static_fallback"
                .to_string(),
        }),
        (None, None, None, None, Some(shader_cache)) => {
            let terrain_id_slot = optional_u32_arg(args, "--terrain-id-slot")?;
            let constants =
                find_shader_cache_map_constants(&PathBuf::from(shader_cache), terrain_id_slot)?;
            Ok(RuntimePow2Source {
                map_pow2_x: constants.map_pow2_x,
                map_pow2_y: constants.map_pow2_y,
                map_pow2_extraction: format!(
                    "shader_cache_define:{}:TerrainIDMap_t{}",
                    constants.cache_file, constants.terrain_id_srv_slot
                ),
                fow_pow2_x: constants.fow_pow2_x,
                fow_pow2_y: constants.fow_pow2_y,
                fow_pow2_extraction: format!(
                    "shader_cache_define:{}:FOW_POW2",
                    constants.cache_file
                ),
            })
        }
        (None, None, None, None, None) => {
            Err("pass --map-pow2-x/--map-pow2-y plus --fow-pow2-x/--fow-pow2-y, or --shader-cache".into())
        }
        _ => Err("pass the complete explicit pow2 set (--map-pow2-x, --map-pow2-y, --fow-pow2-x, --fow-pow2-y), or pass only --shader-cache".into()),
    }
}

pub(crate) fn default_bundle_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("out")
        .join("map_bundle")
}
