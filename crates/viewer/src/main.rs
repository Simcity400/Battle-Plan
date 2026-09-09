mod app;
mod document;
mod inspector;
mod playback;
mod render;

use installed_content::ResolveOptions;
use replay::{BranchId, Hour, container::SeekResult};
use serde::{Deserialize, Serialize};
use std::{ffi::OsString, fs, io, path::PathBuf};

const USAGE: &str = "viewer [replay.battleplan] [--game-dir <directory>] [--user-dir <directory>]\nviewer --dump <replay.battleplan> --hour <n> --out <frame.png>\nOptions: --branch <id> --allow-unverified-content --mode owner|controller\n         --width <pixels> --height <pixels> --camera <x,y,vertical-span>";

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
        if dump != options.dump.is_some() || (dump && options.hour.is_none()) {
            return Err(invalid(USAGE));
        }
        Ok(options)
    }
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
