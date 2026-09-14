mod app;
mod compare;
mod document;
mod map;
mod playback;
mod render;

use installed_content::ResolveOptions;
use map::{camera::Camera, frame::DayNight, political};
// names
use map::{
    name_pass::LayerFade,
    scene::{Overlay, Request},
};
use replay::{BranchId, Hour, container::SeekResult};
use serde::{Deserialize, Serialize};
use std::{ffi::OsString, fs, io, path::PathBuf};

const USAGE: &str = "viewer [replay.battleplan] [--game-dir <directory>] [--user-dir <directory>]\nviewer --dump <replay.battleplan> --hour <n> --out <frame.png>\nviewer --compare <game.png> <viewer.png> [--mask <mask.png>] [--diff <out.png>]\nOptions: --branch <id> --allow-unverified-content --mode owner|controller\n         --width <pixels> --height <pixels> --camera <x,y,vertical-span> --postfx on|off\nCompare: --exclude-game-ui --exclude <x,y,w,h> --regions <cols>x<rows> --json <out.json>
         --upscale <factor> --dim <factor>";

fn invalid(message: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message.into())
}

#[derive(Clone, Default)]
struct Options {
    replay: Option<PathBuf>,
    dump: Option<PathBuf>,
    hour: Option<Hour>,
    branch: Option<BranchId>,
    allow_unverified: bool,
    control: bool,
    camera: Option<Camera>,
    size: [u32; 2],
    game_dir: Option<PathBuf>,
    user_dir: Option<PathBuf>,
    compare: Option<[PathBuf; 2]>,
    mask: Option<PathBuf>,
    diff: Option<PathBuf>,
    json: Option<PathBuf>,
    regions: Option<(u32, u32)>,
    upscale: Option<f64>,
    dim: Option<f64>,
    excludes: Vec<compare::Rect>,
    exclude_game_ui: bool,
    // colour
    /// `--postfx off` renders without the engine's colour grading.
    postfx: bool,
}

impl Options {
    fn parse(args: impl Iterator<Item = OsString>) -> io::Result<Self> {
        // The game renders the reference frames at 2048x1152 (R1 section 0), so
        // a dump matches that by default and `--upscale 1.25` scores it against
        // the screenshots.
        let mut options = Self {
            size: [2048, 1152],
            // colour: the reference session graded its frames, so the chain is
            // on unless `--postfx off` turns it off. The measurements behind
            // that are in `map::post`.
            postfx: true,
            ..Default::default()
        };
        let mut args = args.peekable();
        let mut dump = false;
        while let Some(arg) = args.next() {
            match arg.to_str() {
                Some("--dump") if options.replay.is_none() => {
                    options.replay =
                        Some(PathBuf::from(args.next().ok_or_else(|| invalid(USAGE))?));
                    dump = true;
                }
                Some("--compare") if options.compare.is_none() && options.replay.is_none() => {
                    let game = PathBuf::from(args.next().ok_or_else(|| invalid(USAGE))?);
                    let viewer = PathBuf::from(args.next().ok_or_else(|| invalid(USAGE))?);
                    options.compare = Some([game, viewer]);
                }
                Some("--mask") => {
                    options.mask = Some(PathBuf::from(args.next().ok_or_else(|| invalid(USAGE))?))
                }
                Some("--diff") => {
                    options.diff = Some(PathBuf::from(args.next().ok_or_else(|| invalid(USAGE))?))
                }
                Some("--json") => {
                    options.json = Some(PathBuf::from(args.next().ok_or_else(|| invalid(USAGE))?))
                }
                // colour
                Some("--postfx") => {
                    let value = args
                        .next()
                        .and_then(|value| value.into_string().ok())
                        .ok_or_else(|| invalid(USAGE))?;
                    options.postfx = match value.as_str() {
                        "on" => true,
                        "off" => false,
                        other => return Err(invalid(format!("`{other}` is not `on` or `off`"))),
                    };
                }
                Some("--exclude-game-ui") => options.exclude_game_ui = true,
                Some("--exclude") => {
                    let value = args
                        .next()
                        .and_then(|value| value.into_string().ok())
                        .ok_or_else(|| invalid(USAGE))?;
                    options.excludes.push(compare::Rect::parse(&value)?);
                }
                Some("--regions") => {
                    let value = args
                        .next()
                        .and_then(|value| value.into_string().ok())
                        .ok_or_else(|| invalid(USAGE))?;
                    options.regions = Some(parse_regions(&value)?);
                }
                Some("--dim") => {
                    let value = args
                        .next()
                        .and_then(|value| value.into_string().ok())
                        .ok_or_else(|| invalid(USAGE))?;
                    let factor = value
                        .parse::<f64>()
                        .map_err(|_| invalid("invalid dim factor"))?;
                    if !factor.is_finite() || !(0.0..=1.0).contains(&factor) {
                        return Err(invalid("the dim factor must be between 0 and 1"));
                    }
                    options.dim = Some(factor);
                }
                Some("--upscale") => {
                    let value = args
                        .next()
                        .and_then(|value| value.into_string().ok())
                        .ok_or_else(|| invalid(USAGE))?;
                    let factor = value
                        .parse::<f64>()
                        .map_err(|_| invalid("invalid upscale factor"))?;
                    if !factor.is_finite() || factor <= 0.0 {
                        return Err(invalid("the upscale factor must be finite and positive"));
                    }
                    options.upscale = Some(factor);
                }
                Some("--allow-unverified-content") => options.allow_unverified = true,
                Some("--out") => {
                    options.dump = Some(PathBuf::from(args.next().ok_or_else(|| invalid(USAGE))?))
                }
                Some("--game-dir") => {
                    options.game_dir =
                        Some(PathBuf::from(args.next().ok_or_else(|| invalid(USAGE))?))
                }
                Some("--user-dir") => {
                    options.user_dir =
                        Some(PathBuf::from(args.next().ok_or_else(|| invalid(USAGE))?))
                }
                Some("--hour" | "--branch" | "--width" | "--height" | "--camera" | "--mode") => {
                    let value = args
                        .next()
                        .and_then(|value| value.into_string().ok())
                        .ok_or_else(|| invalid(USAGE))?;
                    match arg.to_str().unwrap() {
                        "--hour" => {
                            options.hour =
                                Some(value.parse().map_err(|_| invalid("invalid engine hour"))?)
                        }
                        "--branch" => {
                            options.branch =
                                Some(value.parse().map_err(|_| invalid("invalid branch id"))?)
                        }
                        "--width" => {
                            options.size[0] = value.parse().map_err(|_| invalid("invalid width"))?
                        }
                        "--height" => {
                            options.size[1] =
                                value.parse().map_err(|_| invalid("invalid height"))?
                        }
                        "--camera" => {
                            let values = value
                                .split(',')
                                .map(str::parse::<f32>)
                                .collect::<Result<Vec<_>, _>>()
                                .map_err(|_| invalid("camera requires world-x,world-z,height"))?;
                            if values.len() != 3 || !values.iter().all(|value| value.is_finite()) {
                                return Err(invalid(
                                    "camera requires finite world-x,world-z,height",
                                ));
                            }
                            if !(map::camera::MIN_HEIGHT..=map::camera::MAX_HEIGHT)
                                .contains(&values[2])
                            {
                                return Err(invalid(format!(
                                    "camera height must be between {} and {}, the engine's CAMERA_MIN_HEIGHT and CAMERA_MAX_HEIGHT",
                                    map::camera::MIN_HEIGHT,
                                    map::camera::MAX_HEIGHT
                                )));
                            }
                            options.camera = Some(Camera {
                                center: [values[0], values[1]],
                                height: values[2],
                            });
                        }
                        "--mode" => {
                            options.control = match value.as_str() {
                                "owner" => false,
                                "controller" => true,
                                _ => return Err(invalid("mode must be owner or controller")),
                            }
                        }
                        _ => unreachable!(),
                    }
                }
                Some(value) if !value.starts_with('-') && options.replay.is_none() => {
                    options.replay = Some(PathBuf::from(arg))
                }
                _ => return Err(invalid(USAGE)),
            }
        }
        if options.size.iter().any(|size| !(1..=16384).contains(size)) {
            return Err(invalid("image dimensions must be 1 through 16384"));
        }
        if options.compare.is_some() {
            if dump
                || options.replay.is_some()
                || options.dump.is_some()
                || options.hour.is_some()
                || options.branch.is_some()
                || options.camera.is_some()
            {
                return Err(invalid(
                    "--compare scores two images and takes no replay, --dump, --out, --hour, --branch or --camera",
                ));
            }
            return Ok(options);
        }
        if options.mask.is_some()
            || options.diff.is_some()
            || options.json.is_some()
            || options.regions.is_some()
            || options.upscale.is_some()
            || options.dim.is_some()
            || !options.excludes.is_empty()
            || options.exclude_game_ui
        {
            return Err(invalid(
                "--mask, --diff, --json, --regions, --upscale, --dim, --exclude and --exclude-game-ui require --compare",
            ));
        }
        if dump != options.dump.is_some() || (dump && options.hour.is_none()) {
            return Err(invalid(USAGE));
        }
        Ok(options)
    }
}

/// Multiplies every channel of an image, for a screenshot taken behind the game's menu overlay.
fn dim(image: &compare::Image, factor: f64) -> io::Result<compare::Image> {
    let scaled = image
        .rgb()
        .iter()
        .map(|&channel| (f64::from(channel) * factor).round().clamp(0.0, 255.0) as u8)
        .collect();
    compare::Image::from_rgb(image.width(), image.height(), scaled)
}

/// Parses the `<cols>x<rows>` form taken by `--regions`.
fn parse_regions(value: &str) -> io::Result<(u32, u32)> {
    let malformed = || invalid(format!("`{value}` is not a <cols>x<rows> region grid"));
    let (columns, rows) = value.split_once(['x', 'X']).ok_or_else(malformed)?;
    let columns = columns.trim().parse::<u32>().map_err(|_| malformed())?;
    let rows = rows.trim().parse::<u32>().map_err(|_| malformed())?;
    if columns == 0 || rows == 0 {
        return Err(invalid(
            "a region grid needs at least one column and one row",
        ));
    }
    Ok((columns, rows))
}

/// Scores one reference pair: prints the metrics and writes the optional difference image and JSON.
fn run_compare(options: &Options, paths: &[PathBuf; 2]) -> io::Result<()> {
    let game = compare::read_png(&paths[0])?;
    let mut viewer = compare::read_png(&paths[1])?;
    if let Some(factor) = options.upscale {
        viewer = compare::upscale(&viewer, factor)?;
    }
    // The game dims the whole frame, interface included, while its escape and save menus are open:
    // every hour-independent interface pixel of `checkpoint2_save1_menu.png` is at exactly half the
    // value of the same pixel in `array_loaded.png` (median of 501 483 samples, quartiles 0.490 and
    // 0.500). A screenshot taken through that overlay can only be scored against a render put
    // through the same factor.
    if let Some(factor) = options.dim {
        viewer = dim(&viewer, factor)?;
    }
    if !game.same_size(&viewer) {
        return Err(invalid(format!(
            "image size mismatch: {} is {}x{} and {} is {}x{}",
            paths[0].display(),
            game.width(),
            game.height(),
            paths[1].display(),
            viewer.width(),
            viewer.height()
        )));
    }
    let mut mask = match &options.mask {
        Some(path) => {
            let image = compare::read_png(path)?;
            if !image.same_size(&game) {
                return Err(invalid(format!(
                    "mask size mismatch: {} is {}x{} and the images are {}x{}",
                    path.display(),
                    image.width(),
                    image.height(),
                    game.width(),
                    game.height()
                )));
            }
            compare::Mask::from_image(&image)
        }
        None => compare::Mask::full(game.width(), game.height())?,
    };
    if options.exclude_game_ui {
        if game.width() != compare::GAME_UI_WIDTH || game.height() != compare::GAME_UI_HEIGHT {
            return Err(invalid(format!(
                "--exclude-game-ui was measured on a {}x{} screenshot; these images are {}x{}",
                compare::GAME_UI_WIDTH,
                compare::GAME_UI_HEIGHT,
                game.width(),
                game.height()
            )));
        }
        for rect in compare::GAME_UI_EXCLUSIONS {
            mask.exclude_rect(rect);
        }
    }
    for rect in &options.excludes {
        mask.exclude_rect(*rect);
    }
    let (columns, rows) = options.regions.unwrap_or((4, 4));
    let report = compare::compare(&game, &viewer, &mask, columns, rows)?;
    print!("{}", report.to_text());
    if let Some(path) = &options.diff {
        compare::write_png(path, &compare::diff_image(&game, &viewer, &mask)?)?;
        println!("diff={}", path.display());
    }
    if let Some(path) = &options.json {
        fs::write(path, serde_json::to_vec_pretty(&report)?).map_err(|error| {
            io::Error::new(error.kind(), format!("{}: {error}", path.display()))
        })?;
        println!("json={}", path.display());
    }
    Ok(())
}

#[derive(Clone, Default, Deserialize, Serialize)]
struct Config {
    game_dir: PathBuf,
    user_dir: PathBuf,
}

impl Config {
    fn path() -> io::Result<PathBuf> {
        Ok(std::env::current_exe()?
            .parent()
            .ok_or_else(|| invalid("viewer executable has no parent directory"))?
            .join("viewer.json"))
    }
    fn load() -> io::Result<Self> {
        match fs::read(Self::path()?) {
            Ok(bytes) => Ok(serde_json::from_slice(&bytes)?),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(Self::default()),
            Err(error) => Err(error),
        }
    }
    fn save(&self) -> io::Result<()> {
        self.resolve_options()?;
        fs::write(Self::path()?, serde_json::to_vec_pretty(self)?)
    }
    fn resolve_options(&self) -> io::Result<ResolveOptions> {
        if !self.game_dir.join("hoi4.exe").is_file() || !self.user_dir.is_dir() {
            return Err(invalid(
                "Choose game folder and the Paradox user folder.",
            ));
        }
        Ok(ResolveOptions {
            game_dir: self.game_dir.clone(),
            user_dir: self.user_dir.clone(),
            cache_dir: PathBuf::from(
                std::env::var_os("LOCALAPPDATA")
                    .ok_or_else(|| invalid("LOCALAPPDATA is not set"))?,
            )
            .join("HOI4Replay")
            .join("viewer"),
        })
    }
}

fn choose_initial(document: &mut document::Document, options: &Options) -> io::Result<()> {
    if let Some(hour) = options.hour {
        let branch = if let Some(branch) = options.branch {
            branch
        } else {
            let mut candidates = Vec::new();
            for branch in &document.playback.replay().branches {
                if matches!(
                    document.playback.replay().seek(branch.id, hour)?,
                    SeekResult::Frame(_)
                ) {
                    candidates.push(branch.id);
                }
            }
            match candidates.as_slice() {
                [branch] => *branch,
                [] => {
                    return Err(invalid(format!(
                        "no recorded frame at hour {hour}; pass --branch to inspect a gap"
                    )));
                }
                _ => {
                    return Err(invalid(format!(
                        "hour {hour} occurs on branches {candidates:?}; pass --branch"
                    )));
                }
            }
        };
        document.playback.seek(branch, hour)?;
    } else if let Some(branch) = options.branch {
        document.playback.select_branch(branch)?;
    }
    Ok(())
}

fn run() -> Result<(), Box<dyn std::error::Error>> {
    let args = std::env::args_os().skip(1).collect::<Vec<_>>();
    if args.iter().any(|arg| arg == "--help" || arg == "-h") {
        println!("{USAGE}");
        return Ok(());
    }
    let options = Options::parse(args.into_iter())?;
    if let Some(paths) = &options.compare {
        run_compare(&options, paths)?;
        return Ok(());
    }
    let (mut config, config_error) = match Config::load() {
        Ok(config) => (config, None),
        Err(error) => (Config::default(), Some(error.to_string())),
    };
    if let Some(path) = &options.game_dir {
        config.game_dir = path.clone();
    }
    if let Some(path) = &options.user_dir {
        config.user_dir = path.clone();
    }
    if let Some(output) = &options.dump {
        if let Some(error) = config_error {
            return Err(invalid(format!("viewer.json: {error}")).into());
        }
        let mut document = document::Document::load(
            options.replay.as_ref().unwrap(),
            &config.resolve_options()?,
            options.allow_unverified,
        )?;
        choose_initial(&mut document, &options)?;
        // political fill
        let mode = political::Mode::from_control(options.control);
        let layers = political::build(&document.map, document.frame()?, mode)?;
        // weather
        let snow_mud = document.snow_mud()?;
        for warning in document.warnings.iter().chain(layers.warnings()) {
            eprintln!("Warning: {warning}");
        }
        let camera = options.camera.unwrap_or_else(|| {
            Camera::fit(
                [document.map.width as f32, document.map.height as f32],
                options.size,
            )
        });
        let unverified = document
            .warnings
            .iter()
            .filter(|warning| warning.starts_with("UNVERIFIED CONTENT"))
            .cloned()
            .collect::<Vec<_>>()
            .join("; ");
        let hour = document.playback.hour();
        // names: a headless render has no frame history, so the layer
        // cross-fade jumps straight to its settled value (R2 section 4.5) and
        // the state layer is only placed when that value is nonzero.
        let fade = LayerFade::instant(camera.height);
        let placements = document.place_names(LayerFade::wants_states(camera.height))?;
        for warning in &placements.warnings {
            eprintln!("Warning: {warning}");
        }
        // counters: bases, labels, counters and combats in priority order.
        let (label_batch, label_report, counter_report) =
            document.overlay_batch(camera, options.size)?;
        let report = render::dump_png(
            &document.assets,
            // political fill + borders
            render::MapLayers {
                political: &layers,
                border_assets: &document.border_assets,
                border_meshes: &document.border_meshes,
                straits: &document.straits,
                // rivers
                river_assets: &document.river_assets,
                river_mesh: &document.river_mesh,
                // weather
                snow_mud: &snow_mud,
                // trees
                tree_assets: &document.tree_assets,
                tree_geometry: &document.tree_geometry,
                tree_field: &document.tree_field,
            },
            // names
            &document.icons,
            Request {
                size: options.size,
                camera,
                day_night: DayNight::at_hour(hour),
                // The animation time is 0 so a headless dump is deterministic.
                animation_time: 0.0,
                // colour
                postfx: options.postfx,
                overlay: Overlay {
                    names: Some(&placements),
                    layer_alpha: fade.as_array(),
                    sprites: Some(&label_batch),
                },
                // trees
                season: document.tree_season(),
            },
            output,
            (!unverified.is_empty()).then_some(unverified.as_str()),
        )?;
        println!(
            "Rendered branch {}, hour {hour} to {} using {}",
            document.playback.branch(),
            output.display(),
            report.adapter
        );
        println!(
            "Camera: {},{},{}",
            camera.center[0], camera.center[1], camera.height
        );
        println!(
            "Terrain grid: {} quads at step {}; render {:.1} ms",
            report.quads,
            report.step,
            report.render_seconds * 1000.0
        );
        // political fill + borders
        let (filled, distinct) = layers.summary();
        println!(
            "Political layer: {filled} provinces filled in {distinct} country colors ({}); gradient fields rebuilt in {:.0} ms",
            mode.field(),
            layers.build.as_secs_f64() * 1000.0
        );
        let border_vertices: usize = document
            .border_meshes
            .meshes
            .iter()
            .map(|mesh| mesh.vertex_count())
            .sum();
        println!(
            "Borders: {border_vertices} vertices over {} classes and {} strait vertices; traced in {:.0} ms",
            document.border_meshes.meshes.len(),
            document.straits.len(),
            document.border_trace.as_secs_f64() * 1000.0
        );
        // rivers
        println!(
            "Rivers: {} ribbons from {} sources and {} of {} junction markers, {} vertices and {} indices; {} too small, {} circular, {} unreachable pixels; built in {:.0} ms",
            document.river_mesh.ribbons,
            document.river_trace.sources,
            document.river_trace.junctions_used,
            document.river_trace.junctions,
            document.river_mesh.vertices.len(),
            document.river_mesh.indices.len(),
            document.river_trace.too_small.len(),
            document.river_trace.circular.len(),
            document.river_trace.unreachable,
            document.river_build.as_secs_f64() * 1000.0
        );
        // trees
        let season = document.tree_season();
        println!(
            "Trees: {} instances in {} buckets over {} kinds, {} canopy vertices;              fade {:.4} at height {:.3}, season column {} lerp {:.2}; {} instances drawn;              generated in {:.0} ms",
            document.tree_field.instances.len(),
            document.tree_field.buckets.len(),
            document.tree_assets.meshes.len(),
            document.tree_geometry.vertices.len(),
            map::trees::tree_fade(camera.height),
            camera.height,
            season.column,
            season.lerp,
            report.tree_instances,
            document.tree_build.as_secs_f64() * 1000.0
        );
        for (palette, tree) in &document.tree_assets.meshes {
            let instances: u32 = document
                .tree_field
                .buckets
                .iter()
                .filter(|bucket| bucket.palette == *palette)
                .map(|bucket| bucket.range.end - bucket.range.start)
                .sum();
            println!(
                "  {}: {} ({} LODs, scale {}) {instances} instances",
                tree.name,
                tree.path,
                tree.mesh.shapes.len(),
                tree.scale
            );
        }
        // weather
        println!(
            "Snow and mud: {}; {}x{} grid rasterised in {:.0} ms",
            snow_mud.summary.line(),
            snow_mud.width,
            snow_mud.height,
            snow_mud.build.as_secs_f64() * 1000.0
        );
        // names
        for line in document.icons.fonts.summary() {
            println!("Font: {line}");
        }
        println!(
            "Localisation: {} keys, e.g. GER_fascism={:?} VICTORY_POINTS_6521={:?}",
            document.names.localisation.len(),
            document.names.localisation.get("GER_fascism"),
            document.names.localisation.get("VICTORY_POINTS_6521")
        );
        println!(
            "Names: {} country areas, {} state areas, {} glyph vertices at layer alpha {:?};              province graph {} edges; {} icon pages",
            placements.country.len(),
            placements.state.len(),
            report.overlay.0,
            fade.as_array(),
            document.names.graph.edge_count(),
            report.icon_pages
        );
        let mut placed: Vec<&map::name_pass::WorldName> = placements.country.iter().collect();
        placed.sort_by(|a, b| {
            b.font_size
                .partial_cmp(&a.font_size)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        for name in placed.iter().take(12) {
            println!("  {}", name.summary());
        }
        println!(
            "Labels: {} icons, {} texts in {} quads; allegiance measured against {}",
            label_report.icons,
            label_report.texts,
            report.overlay.1,
            label_report
                .perspective
                .as_deref()
                .unwrap_or("no human country")
        );
        // counters
        println!(
            "Counters: {} land, {} naval; {} air bases, {} naval bases; {} land combats, {} naval combats; {} stacks with no anchor",
            counter_report.land_counters,
            counter_report.naval_counters,
            counter_report.air_bases,
            counter_report.naval_bases,
            counter_report.land_combats,
            counter_report.naval_combats,
            counter_report.unanchored
        );
        // counters
        if let Some(pass) = counter_report.grouping {
            println!(
                "Counter grouping: {:?} level, {} groups, budget {}{}",
                pass.level,
                counter_report.groups,
                pass.budget,
                if pass.split { ", split" } else { "" }
            );
        }
        println!(
            "Counter assets: {} sprite sheets, {} small flags packed{}",
            document.icons.counters.sheet_count(),
            document.icons.counters.flags.len(),
            if counter_report.missing_flags.is_empty() {
                String::new()
            } else {
                format!(
                    "; no flag for {}",
                    counter_report
                        .missing_flags
                        .iter()
                        .cloned()
                        .collect::<Vec<_>>()
                        .join(", ")
                )
            }
        );
        return Ok(());
    }
    let native = eframe::NativeOptions {
        renderer: eframe::Renderer::Wgpu,
        viewport: eframe::egui::ViewportBuilder::default()
            .with_inner_size([1400.0, 900.0])
            .with_min_inner_size([800.0, 500.0]),
        wgpu_options: render::configuration(),
        ..Default::default()
    };
    eframe::run_native(
        "Battle Plan",
        native,
        Box::new(move |cc| {
            Ok(Box::new(app::Viewer::new(
                cc,
                config,
                options,
                config_error,
            )))
        }),
    )?;
    Ok(())
}

fn main() -> std::process::ExitCode {
    match run() {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("{error}");
            std::process::ExitCode::FAILURE
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(args: &[&str]) -> io::Result<Options> {
        Options::parse(args.iter().map(OsString::from))
    }

    #[test]
    fn compare_takes_two_images_and_its_own_options() {
        let options = parse(&[
            "--compare",
            "game.png",
            "viewer.png",
            "--mask",
            "mask.png",
            "--diff",
            "diff.png",
            "--json",
            "out.json",
            "--regions",
            "8x6",
            "--upscale",
            "1.25",
            "--dim",
            "0.5",
            "--exclude-game-ui",
            "--exclude",
            "1,2,3,4",
            "--exclude",
            "5,6,7,8",
        ])
        .unwrap();
        let paths = options.compare.as_ref().unwrap();
        assert_eq!(paths[0], PathBuf::from("game.png"));
        assert_eq!(paths[1], PathBuf::from("viewer.png"));
        assert_eq!(options.mask, Some(PathBuf::from("mask.png")));
        assert_eq!(options.diff, Some(PathBuf::from("diff.png")));
        assert_eq!(options.json, Some(PathBuf::from("out.json")));
        assert_eq!(options.regions, Some((8, 6)));
        assert_eq!(options.upscale, Some(1.25));
        assert_eq!(options.dim, Some(0.5));
        assert!(options.exclude_game_ui);
        assert_eq!(options.excludes.len(), 2);
        assert_eq!(options.excludes[1].x, 5);
    }

    #[test]
    fn compare_defaults_to_the_whole_frame_and_a_four_by_four_grid() {
        let options = parse(&["--compare", "a.png", "b.png"]).unwrap();
        assert!(options.compare.is_some());
        assert_eq!(options.regions, None);
        assert_eq!(options.upscale, None);
        assert!(options.mask.is_none() && options.diff.is_none() && options.json.is_none());
        assert!(options.excludes.is_empty() && !options.exclude_game_ui);
    }

    #[test]
    fn compare_refuses_missing_images_and_replay_options() {
        assert!(parse(&["--compare"]).is_err());
        assert!(parse(&["--compare", "only-one.png"]).is_err());
        assert!(parse(&["--compare", "a.png", "b.png", "campaign.battleplan"]).is_err());
        assert!(parse(&["--compare", "a.png", "b.png", "--hour", "5"]).is_err());
        assert!(parse(&["--compare", "a.png", "b.png", "--branch", "1"]).is_err());
        assert!(parse(&["--compare", "a.png", "b.png", "--camera", "1,2,3"]).is_err());
        assert!(parse(&["--dump", "c.battleplan", "--compare", "a.png", "b.png"]).is_err());
    }

    #[test]
    fn compare_options_are_refused_without_compare() {
        for args in [
            vec!["c.battleplan", "--mask", "m.png"],
            vec!["c.battleplan", "--diff", "d.png"],
            vec!["c.battleplan", "--json", "j.json"],
            vec!["c.battleplan", "--regions", "2x2"],
            vec!["c.battleplan", "--upscale", "1.25"],
            vec!["c.battleplan", "--dim", "0.5"],
            vec!["c.battleplan", "--exclude", "1,2,3,4"],
            vec!["c.battleplan", "--exclude-game-ui"],
        ] {
            assert!(parse(&args).is_err(), "{args:?} should be refused");
        }
    }

    #[test]
    fn region_grids_parse_and_reject_empty_or_malformed_values() {
        assert_eq!(parse_regions("4x4").unwrap(), (4, 4));
        assert_eq!(parse_regions("16X9").unwrap(), (16, 9));
        assert_eq!(parse_regions(" 3 x 2 ").unwrap(), (3, 2));
        for bad in ["4", "0x4", "4x0", "x4", "4x", "4x4x4", "-1x2", "axb", ""] {
            assert!(parse_regions(bad).is_err(), "`{bad}` should be refused");
        }
    }

    #[test]
    fn the_dump_and_window_paths_are_unchanged() {
        let dump = parse(&[
            "--dump",
            "campaign.battleplan",
            "--hour",
            "60826315",
            "--out",
            "frame.png",
            "--camera",
            "100,200,300",
            "--mode",
            "controller",
        ])
        .unwrap();
        assert_eq!(dump.replay, Some(PathBuf::from("campaign.battleplan")));
        assert_eq!(dump.dump, Some(PathBuf::from("frame.png")));
        assert_eq!(dump.hour, Some(60826315));
        assert!(dump.control);
        assert!(dump.compare.is_none());
        assert_eq!(dump.camera.unwrap().center, [100.0, 200.0]);
        assert_eq!(dump.camera.unwrap().height, 300.0);
        assert_eq!(dump.size, [2048, 1152]);

        let window = parse(&["campaign.battleplan", "--branch", "2"]).unwrap();
        assert_eq!(window.replay, Some(PathBuf::from("campaign.battleplan")));
        assert!(window.dump.is_none() && window.compare.is_none());

        assert!(parse(&["--dump", "campaign.battleplan", "--hour", "1"]).is_err());
        assert!(parse(&["--dump", "campaign.battleplan", "--out", "f.png"]).is_err());
    }

    #[test]
    fn the_camera_takes_world_coordinates_and_an_engine_legal_height() {
        let reference = parse(&[
            "--dump",
            "c.battleplan",
            "--hour",
            "1",
            "--out",
            "f.png",
            "--camera",
            "3010,1568,310.178",
        ])
        .unwrap()
        .camera
        .unwrap();
        assert_eq!(reference.center, [3010.0, 1568.0]);
        assert_eq!(reference.height, 310.178);
        for bad in ["3010,1568", "3010,1568,10", "3010,1568,4000", "a,b,c"] {
            assert!(
                parse(&[
                    "--dump",
                    "c.battleplan",
                    "--hour",
                    "1",
                    "--out",
                    "f.png",
                    "--camera",
                    bad
                ])
                .is_err(),
                "`{bad}` should be refused"
            );
        }
    }

    #[test]
    fn the_upscale_factor_must_be_positive() {
        assert!(parse(&["--compare", "a.png", "b.png", "--upscale", "0"]).is_err());
        assert!(parse(&["--compare", "a.png", "b.png", "--upscale", "-1"]).is_err());
        assert!(parse(&["--compare", "a.png", "b.png", "--upscale", "x"]).is_err());
    }

    #[test]
    fn the_dim_factor_is_a_fraction_and_halves_every_channel() {
        assert_eq!(
            parse(&["--compare", "a.png", "b.png", "--dim", "1"])
                .unwrap()
                .dim,
            Some(1.0)
        );
        for bad in ["-0.1", "1.1", "2", "x"] {
            assert!(
                parse(&["--compare", "a.png", "b.png", "--dim", bad]).is_err(),
                "`{bad}` should be refused"
            );
        }
        let image = compare::Image::from_rgb(2, 1, vec![255, 100, 0, 7, 8, 9]).unwrap();
        assert_eq!(dim(&image, 0.5).unwrap().rgb(), [128, 50, 0, 4, 4, 5]);
        assert_eq!(dim(&image, 1.0).unwrap().rgb(), image.rgb());
    }
}
