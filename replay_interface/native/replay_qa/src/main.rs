use std::path::PathBuf;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match run(&args) {
        Ok(exit_code) => std::process::exit(exit_code),
        Err(err) => {
            eprintln!("replay_qa: {err}");
            std::process::exit(2);
        }
    }
}

fn run(args: &[String]) -> Result<i32, String> {
    let mut config = replay_qa::ReplayQaConfig::default();
    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "--config" => {
                index += 1;
                let path = args
                    .get(index)
                    .ok_or_else(|| "--config requires a path".to_string())?;
                let text = std::fs::read_to_string(path)
                    .map_err(|err| format!("read config {path}: {err}"))?;
                config.apply_config_text(&text)?;
            }
            "--out" => {
                index += 1;
                let path = args
                    .get(index)
                    .ok_or_else(|| "--out requires a path".to_string())?;
                config.out_dir = PathBuf::from(path);
            }
            "--terrain-map-frame-bundle" => {
                index += 1;
                let path = args
                    .get(index)
                    .ok_or_else(|| "--terrain-map-frame-bundle requires a path".to_string())?;
                config.set_terrain_map_frame_bundle(PathBuf::from(path));
                config.capture_dir = config
                    .terrain_map_frame_bundle
                    .parent()
                    .map(std::path::Path::to_path_buf)
                    .unwrap_or_else(|| config.capture_dir.clone());
            }
            "--water-map-frame-bundle" => {
                index += 1;
                let path = args
                    .get(index)
                    .ok_or_else(|| "--water-map-frame-bundle requires a path".to_string())?;
                config.set_water_map_frame_bundle(PathBuf::from(path));
            }
            "--terrain-chunk-library" => {
                index += 1;
                let path = args
                    .get(index)
                    .ok_or_else(|| "--terrain-chunk-library requires a path".to_string())?;
                config.terrain_chunk_library = PathBuf::from(path);
            }
            "--capture-dir" => {
                index += 1;
                let path = args
                    .get(index)
                    .ok_or_else(|| "--capture-dir requires a path".to_string())?;
                config.capture_dir = PathBuf::from(path);
            }
            "--game-root" => {
                index += 1;
                let path = args
                    .get(index)
                    .ok_or_else(|| "--game-root requires a path".to_string())?;
                config.game_root = Some(PathBuf::from(path));
            }
            "--mod-root" => {
                index += 1;
                let path = args
                    .get(index)
                    .ok_or_else(|| "--mod-root requires a path".to_string())?;
                config.mod_root = Some(PathBuf::from(path));
            }
            "--water-hard-pass" => {
                config.water_expected_fail = false;
            }
            "--help" | "-h" => {
                print_help();
                return Ok(0);
            }
            other => return Err(format!("unknown argument {other:?}; use --help")),
        }
        index += 1;
    }

    let report = replay_qa::run_harness(&config)?;
    let json_path = config.out_dir.join("replay_qa_report.json");
    let md_path = config.out_dir.join("replay_qa_report.md");
    std::fs::create_dir_all(&config.out_dir)
        .map_err(|err| format!("create {}: {err}", config.out_dir.display()))?;
    std::fs::write(&json_path, report.to_json())
        .map_err(|err| format!("write {}: {err}", json_path.display()))?;
    let markdown = report.to_markdown();
    std::fs::write(&md_path, &markdown)
        .map_err(|err| format!("write {}: {err}", md_path.display()))?;
    println!("{markdown}");
    Ok(if report.has_hard_failures() { 1 } else { 0 })
}

fn print_help() {
    println!(
        "usage: replay_qa [--config <file>] [--out <dir>] \
         [--terrain-map-frame-bundle <map_frame_bundle.json>] \
         [--water-map-frame-bundle <map_frame_water_pass_bundle.json>] \
         [--terrain-chunk-library <terrain_chunk_library.json>] \
         [--capture-dir <capture session dir>] [--game-root <the game dir>] \
         [--mod-root <mod dir>] [--water-hard-pass]\n\n\
         The optional config file is line-oriented key=value text. Supported keys include:\n\
         terrain_map_frame_bundle, water_map_frame_bundle, terrain_chunk_library,\n\
         capture_dir, bundle_dir, out_dir, game_root, mod_root, water.expected_fail,\n\
         frame_parity.<name>.<enabled|gate|pass|terrain_map_frame_bundle|compare_map_frame_bundle|water_map_frame_bundle|camera_json|hard>,\n\
         matrix.hours, camera.<full_map|europe_close|pacific_close>.zoom,\n\
         camera.<full_map|europe_close|pacific_close>.center, and threshold.* values."
    );
}
