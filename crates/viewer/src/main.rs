mod app;
mod compare;
mod document;
mod playback;
mod render;

use installed_content::ResolveOptions;
use replay::{BranchId, Hour, container::SeekResult};
use serde::{Deserialize, Serialize};
use std::{ffi::OsString, fs, io, path::PathBuf};

const USAGE: &str = "viewer [replay.battleplan] [--game-dir <directory>] [--user-dir <directory>]\nviewer --dump <replay.battleplan> --hour <n> --out <frame.png>\nviewer --compare <game.png> <viewer.png> [--mask <mask.png>] [--diff <out.png>]\nOptions: --branch <id> --allow-unverified-content --mode owner|controller\n         --width <pixels> --height <pixels> --camera <x,y,vertical-span>\nCompare: --exclude-game-ui --exclude <x,y,w,h> --regions <cols>x<rows> --json <out.json>";

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
    camera: Option<render::Camera>,
    size: [u32; 2],
    game_dir: Option<PathBuf>,
    user_dir: Option<PathBuf>,
    compare: Option<[PathBuf; 2]>,
    mask: Option<PathBuf>,
    diff: Option<PathBuf>,
    json: Option<PathBuf>,
    regions: Option<(u32, u32)>,
    excludes: Vec<compare::Rect>,
    exclude_game_ui: bool,
}

impl Options {
    fn parse(args: impl Iterator<Item = OsString>) -> io::Result<Self> {
        let mut options = Self {
            size: [1600, 900],
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
                                .map_err(|_| invalid("camera requires x,y,vertical-span"))?;
                            if values.len() != 3
                                || !values.iter().all(|value| value.is_finite())
                                || values[2] <= 0.0
                            {
                                return Err(invalid(
                                    "camera requires finite x,y and a positive vertical span",
                                ));
                            }
                            options.camera = Some(render::Camera {
                                center: [values[0], values[1]],
                                vertical_span: values[2],
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
            || !options.excludes.is_empty()
            || options.exclude_game_ui
        {
            return Err(invalid(
                "--mask, --diff, --json, --regions, --exclude and --exclude-game-ui require --compare",
            ));
        }
        if dump != options.dump.is_some() || (dump && options.hour.is_none()) {
            return Err(invalid(USAGE));
        }
        Ok(options)
    }
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
    let viewer = compare::read_png(&paths[1])?;
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
        let palette = document.palette(options.control)?;
        for warning in document.warnings.iter().chain(&palette.warnings) {
            eprintln!("Warning: {warning}");
        }
        let camera = options.camera.unwrap_or_else(|| {
            render::Camera::fit([document.map.width, document.map.height], options.size)
        });
        let unverified = document
            .warnings
            .iter()
            .filter(|warning| warning.starts_with("UNVERIFIED CONTENT"))
            .cloned()
            .collect::<Vec<_>>()
            .join("; ");
        let adapter = render::dump_png(
            render::MapImage {
                width: document.map.width,
                height: document.map.height,
                province_ids: &document.map.province_ids,
                colors: &palette.colors,
                owner_keys: &palette.owner_keys,
            },
            camera,
            options.size,
            output,
            (!unverified.is_empty()).then_some(unverified.as_str()),
        )?;
        println!(
            "Rendered branch {}, hour {} to {} using {adapter}",
            document.playback.branch(),
            document.playback.hour(),
            output.display()
        );
        println!(
            "Camera: {},{},{}; base country colors",
            camera.center[0], camera.center[1], camera.vertical_span
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
        assert!(options.exclude_game_ui);
        assert_eq!(options.excludes.len(), 2);
        assert_eq!(options.excludes[1].x, 5);
    }

    #[test]
    fn compare_defaults_to_the_whole_frame_and_a_four_by_four_grid() {
        let options = parse(&["--compare", "a.png", "b.png"]).unwrap();
        assert!(options.compare.is_some());
        assert_eq!(options.regions, None);
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
        assert_eq!(dump.size, [1600, 900]);

        let window = parse(&["campaign.battleplan", "--branch", "2"]).unwrap();
        assert_eq!(window.replay, Some(PathBuf::from("campaign.battleplan")));
        assert!(window.dump.is_none() && window.compare.is_none());

        assert!(parse(&["--dump", "campaign.battleplan", "--hour", "1"]).is_err());
        assert!(parse(&["--dump", "campaign.battleplan", "--out", "f.png"]).is_err());
    }
}
