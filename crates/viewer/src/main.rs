mod app;
mod blocks;
mod combat;
mod compare;
mod dashboard_maps;
mod document;
mod embedded;
mod engaged_index;
mod hours;
mod map;
mod parallel;
mod playback;
mod profiling;
mod raid_lane;
mod render;
mod state_resources;
mod table_access;
#[cfg(test)]
mod unit_scale_tests;
mod worker_priority;

use dashboard::Config;

use map::{camera::Camera, frame::DayNight, political};
// names
use map::{
    name_pass::LayerFade,
    scene::{Overlay, Request},
};
use replay::{BranchId, Hour};
use std::{
    collections::BTreeSet,
    ffi::OsString,
    fs, io,
    path::PathBuf,
    sync::{Arc, Mutex},
};

const USAGE: &str = "viewer [replay.battleplan] [--game-dir <directory>] [--user-dir <directory>]\nviewer --dump <replay.battleplan> --hour <n>[.<fraction>] --out <frame.png>\nviewer --window-dump <replay.battleplan> --hour <n> --out <frame.png>\nviewer --compare <game.png> <viewer.png> [--mask <mask.png>] [--diff <out.png>]\nOptions: --branch <id> --hour-fraction <0..1>, how far past --hour the frame is; a moving
         division's counter stands at its settled route point all hour, as the game draws it
         --allow-unverified-content\n         --width <pixels> --height <pixels> --camera <x,z,height> --postfx on|off\n         --perspective <TAG>|recorded, the viewing country (default recorded)\n         --fog on|off, the game's fog-of-war switch (default on)\n         --combat <full-handle> | --combat-at <province>, open a recorded combat or retained naval result\nWindow: --diagnostics shows technical stats and data limitations for this run; F3 or the\n        header's Diagnostics checkbox toggles and saves them (off by default)\n        --out names where F12 saves the map target at its native size; --window-dump opens\n        the window, renders one frame through the windowed path at --width/--height and\n        --camera, writes it and exits\n        --animation-time <seconds> --animation-step <seconds> --window-frames <1..1000>\n        sample a deterministic sequence; default is one frame at time zero\nDump only: --presentation-time <seconds>, the engine's vGlobalTime and FoWTime, zero in a dump:\n             the fog-of-war noise and the entity animation\n           --map-time <seconds>, the map scene's own second clock: the selection\n             pulse, the river flow and the waves\n           --settle <seconds>, play that many seconds before the frame so entity events\n             (damaged fire and smoke) and particles are under way; renders at the clock plus it\n           --building-camera <x,z,height>, repeatable: earlier views whose map buildings the frame keeps\n           --demilitarized <state-id,...>, a verification aid that adds the save's demilitarized=yes states\n           --debug-borders, the game's console toggle: no gradient fill and no border strips\n           --no-water, a verification aid that skips the water plane as the game's debug_water does\n           --no-trees, a verification aid that skips the canopy as the game's debug_trees does\n           --layers <names>, comma-separated passes to OMIT: counters,labels,trees,borders,gradient,\n             rivers,weather,orders,models,sprawl,buildings,rails\n             labels include VP/capital sprites; counters include bases/combat; borders include straits;\n             rails include trains/bridges; weather omits particles only, preserving terrain snow/mud\n           --counters-json <out.json>, submitted plates with interface rectangles\n             (device pixels at --ui-scale 1), sprite frames and text\n           --ui-scale <0.5..3>, --dump and --window-dump: device pixels per map-interface pixel\n             (counters, icons, labels, combat window); default 1.0, the game's frame at gui_scale 1\nCompare: --exclude-game-ui --exclude <x,y,w,h> --regions <cols>x<rows> --json <out.json>
         --upscale <factor> --dim <factor>";

fn invalid(message: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message.into())
}

fn parse_placement(value: &str) -> io::Result<map::camera::Placement> {
    let values = value
        .split(',')
        .map(str::parse::<f32>)
        .collect::<Result<Vec<_>, _>>()
        .map_err(|_| invalid("camera requires world-x,world-z,height"))?;
    if values.len() != 3 || !values.iter().all(|value| value.is_finite()) {
        return Err(invalid("camera requires finite world-x,world-z,height"));
    }
    Ok(map::camera::Placement {
        center: [values[0], values[1]],
        height: values[2],
    })
}

// counters: the glide between recorded hours
/// `--hour`'s value: a recorded engine hour, optionally with a fraction of an
/// hour past it.
///
/// `--hour 60832633` is the recorded hour and renders it exactly.
/// `--hour 60832633.5` is half an hour later, a frame the recording does not
/// hold: the hour's own tables are drawn and its moving divisions advanced
/// along their recorded routes by their recorded ETA. The fraction is parsed
/// off the text rather than through `f64` so a large engine hour cannot lose
/// its last digits on the way.
fn parse_hour(value: &str) -> io::Result<(Hour, Option<f32>)> {
    let Some((whole, rest)) = value.split_once('.') else {
        return Ok((
            value.parse().map_err(|_| invalid("invalid engine hour"))?,
            None,
        ));
    };
    let hour: Hour = whole.parse().map_err(|_| invalid("invalid engine hour"))?;
    if rest.is_empty() {
        return Err(invalid("invalid engine hour"));
    }
    Ok((hour, Some(parse_fraction(&format!("0.{rest}"))?)))
}

/// A fraction of an hour past the recorded one, which has to stay inside it.
///
/// One whole hour is the next recorded hour, which is its own render: allowing
/// it here would draw hour `h`'s tables where hour `h + 1`'s belong.
fn parse_fraction(value: &str) -> io::Result<f32> {
    let fraction: f32 = value
        .parse()
        .map_err(|_| invalid("invalid hour fraction"))?;
    if !(0.0..1.0).contains(&fraction) {
        return Err(invalid(
            "the hour fraction is how far past the recorded hour the frame is and lies in [0, 1);              one whole hour is the next recorded hour, which renders on its own",
        ));
    }
    Ok(fraction)
}

#[derive(Clone, Default)]
struct Options {
    combat: Option<combat::Selector>,
    replay: Option<PathBuf>,
    /// Where a rendered frame is written: `--dump`'s and `--window-dump`'s
    /// output, and the destination of the window's own F12 capture.
    out: Option<PathBuf>,
    /// `--dump`: render one frame without opening a window.
    headless_dump: bool,
    // capture
    /// `--window-dump`: open the window and render one frame through the
    /// windowed path at `size` and `camera` into `dump`, then close.
    ///
    /// The window and `--dump` are only comparable if the frame comes out of the
    /// window's own path, so this drives [`app::Viewer`] rather than
    /// [`render::dump_png`]; the readback is the dump's own.
    window_dump: bool,
    animation_time: f32,
    animation_step: f32,
    window_frames: u32,
    /// `--dump --presentation-time <seconds>`: the engine's global presentation
    /// clock, `vGlobalTime`, which a dump otherwise renders at zero.
    ///
    /// It is not a recorded quantity and cannot become one — the clock runs
    /// while the game is paused, so no value sampled at an hour equals the clock
    /// at a screenshot taken later. What it is for is proving that the port's
    /// pattern is the game's *at the same clock*: fit it against a reference
    /// frame, then score that frame with it. Every consumer of `vGlobalTime`
    /// reads it, which is the fog-of-war noise and the entity animation.
    presentation_time: f32,
    /// `--dump --settle <seconds>`: frames played on the presentation clock
    /// before the dump, one second apart, so entity events such as a damaged
    /// building's fire and smoke and weather particles are under way as in a
    /// running viewer. The dump then renders at `presentation_time + settle`.
    settle: f32,
    /// `--dump --map-time <seconds>`: the map scene's own clock, which frame33840
    /// shows is a different one — `vGlobalTime` 547.981 and `FoWTime` 547.978
    /// against the border pass's `vTime_Transparency.x` 206.792, the river's
    /// `vTimeDirectionSeasonLerp.x` 206.822 and the water's 206.826. It drives
    /// the border selection pulse, the river flow and the waves.
    map_time: f32,
    hour: Option<Hour>,
    // counters: the glide between recorded hours
    /// How far past `hour` the frame is, in hours, in `[0, 1)`.
    ///
    /// `--hour 60832633.5` and `--hour-fraction 0.5` are two spellings of the
    /// same number, and it is [`document::Document::hour_fraction`]: the
    /// recorded hour is the whole frame's state and this only advances the
    /// moving divisions along their recorded routes. Zero renders the recorded
    /// hour exactly.
    hour_fraction: f32,
    branch: Option<BranchId>,
    allow_unverified: bool,
    // counters
    /// `--perspective` and `--fog`: the game's two map controls. `recorded`
    /// (the default) views the map as the recording client's viewpoint at that hour and a tag as
    /// that country; `--fog off` is the game's `fow` console switch. See
    /// `map::perspective`.
    perspective: map::perspective::Choice,
    camera: Option<map::camera::Placement>,
    /// `--dump --building-camera <x,z,height>`, repeatable: cameras the map
    /// visited before this frame, in order. Map buildings keep what those views
    /// created, as a live session does.
    building_cameras: Vec<map::camera::Placement>,
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
    /// `--dump --no-water` skips the water plane, which is what the game's
    /// `debug_water` console toggle does; see `map::scene::Request::no_water`.
    no_water: bool,
    /// `--dump --no-shadows` skips the cast-shadow chain, which is what the
    /// game's `Draw.Shadows` console toggle does; see
    /// `map::scene::Request::no_shadows`.
    no_shadows: bool,
    // trees
    /// `--dump --no-trees` skips the canopy, which is what the game's
    /// `debug_trees` console toggle does; see `map::scene::Request::no_trees`.
    no_trees: bool,
    /// Dump-only comma-separated pass omissions, for residual attribution.
    layers: map::layers::Omissions,
    /// Write the actual rendered counter quads, grouped into plates.
    counters_json: Option<PathBuf>,
    /// `--ui-scale`: device pixels per interface pixel of the screen-space map
    /// interface. A dump renders at the game's frame size, where the verified
    /// comparison pipeline uses 1.0, so that is the default; the window
    /// otherwise uses its setting ([`map::ui_scale`]).
    ui_scale: Option<f32>,
    /// `--diagnostics`: show the window's technical stats and data
    /// limitations for this run without changing the saved choice.
    diagnostics: bool,
    // borders
    /// `--dump --demilitarized <ids>`: state ids to treat as demilitarised **in addition to** the
    /// hour's recorded `StateFlags`, so an hour whose flags are short can still be scored against
    /// a reference pair. Verification aid, `--dump` only; the window has no equivalent and draws
    /// the recording.
    demilitarized: BTreeSet<u32>,
    /// `--dump --debug-borders`: the game's own console toggle, which removes the gradient
    /// political fill and the border strips and leaves bare terrain. Pairing a frame with and
    /// without it is how the fill law is measured against the game's `pairE_*_{fill,nofill}`
    /// captures; `map::gradient` records what those measurements settled.
    debug_borders: bool,
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
            no_water: false,
            no_shadows: false,
            no_trees: false,
            ..Default::default()
        };
        let mut args = args.peekable();
        while let Some(arg) = args.next() {
            match arg.to_str() {
                Some("--combat" | "--combat-at") => {
                    if options.combat.is_some() {
                        return Err(invalid("choose exactly one --combat or --combat-at"));
                    }
                    let value = args
                        .next()
                        .and_then(|s| s.into_string().ok())
                        .ok_or_else(|| invalid("combat selection requires an integer"))?;
                    options.combat = Some(if arg == "--combat-at" {
                        combat::Selector::Province(
                            value
                                .parse()
                                .map_err(|_| invalid("combat province must be a u32"))?,
                        )
                    } else {
                        combat::Selector::Id(
                            value
                                .strip_prefix("0x")
                                .map(|v| u64::from_str_radix(v, 16))
                                .unwrap_or_else(|| value.parse())
                                .map_err(|_| invalid("combat id must be a full u64 handle"))?,
                        )
                    });
                }
                Some("--dump") if options.replay.is_none() => {
                    options.replay =
                        Some(PathBuf::from(args.next().ok_or_else(|| invalid(USAGE))?));
                    options.headless_dump = true;
                }
                // capture
                Some("--window-dump") if options.replay.is_none() => {
                    options.replay =
                        Some(PathBuf::from(args.next().ok_or_else(|| invalid(USAGE))?));
                    options.window_dump = true;
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
                Some("--counters-json") => {
                    options.counters_json =
                        Some(PathBuf::from(args.next().ok_or_else(|| invalid(USAGE))?))
                }
                // colour
                Some("--no-water") => options.no_water = true,
                Some("--no-shadows") => options.no_shadows = true,
                // trees
                Some("--no-trees") => options.no_trees = true,
                Some("--layers") => {
                    let value = args
                        .next()
                        .and_then(|value| value.into_string().ok())
                        .ok_or_else(|| invalid(USAGE))?;
                    options.layers = map::layers::Omissions::parse(&value)?;
                }
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
                // borders
                Some("--demilitarized") => {
                    let value = args
                        .next()
                        .and_then(|value| value.into_string().ok())
                        .ok_or_else(|| invalid(USAGE))?;
                    options.demilitarized = map::demilitarized::parse_states(&value)?;
                }
                Some("--debug-borders") => options.debug_borders = true,
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
                Some("--ui-scale") => {
                    let value = args
                        .next()
                        .and_then(|value| value.into_string().ok())
                        .ok_or_else(|| invalid(USAGE))?;
                    let scale = value
                        .parse::<f32>()
                        .ok()
                        .filter(|scale| map::ui_scale::valid(*scale))
                        .ok_or_else(|| {
                            let range = dashboard::protocol::MAP_UI_SCALE_RANGE;
                            invalid(format!(
                                "the UI scale must be a number from {} to {}",
                                range.start(),
                                range.end()
                            ))
                        })?;
                    options.ui_scale = Some(scale);
                }
                Some("--allow-unverified-content") => options.allow_unverified = true,
                Some("--diagnostics") => options.diagnostics = true,
                Some("--out") => {
                    options.out = Some(PathBuf::from(args.next().ok_or_else(|| invalid(USAGE))?))
                }
                Some("--game-dir") => {
                    options.game_dir =
                        Some(PathBuf::from(args.next().ok_or_else(|| invalid(USAGE))?))
                }
                Some("--user-dir") => {
                    options.user_dir =
                        Some(PathBuf::from(args.next().ok_or_else(|| invalid(USAGE))?))
                }
                Some(
                    "--hour"
                    | "--hour-fraction"
                    | "--branch"
                    | "--width"
                    | "--height"
                    | "--camera"
                    | "--building-camera"
                    | "--perspective"
                    | "--fog"
                    | "--animation-time"
                    | "--animation-step"
                    | "--window-frames"
                    | "--presentation-time"
                    | "--settle"
                    | "--map-time",
                ) => {
                    let value = args
                        .next()
                        .and_then(|value| value.into_string().ok())
                        .ok_or_else(|| invalid(USAGE))?;
                    match arg.to_str().unwrap() {
                        "--hour" => {
                            let (hour, fraction) = parse_hour(&value)?;
                            options.hour = Some(hour);
                            if let Some(fraction) = fraction {
                                options.hour_fraction = fraction;
                            }
                        }
                        "--hour-fraction" => options.hour_fraction = parse_fraction(&value)?,
                        "--animation-time" | "--animation-step" => {
                            let seconds = value.parse::<f32>().map_err(|_| {
                                invalid("animation time must be finite nonnegative seconds")
                            })?;
                            if !seconds.is_finite() || seconds < 0.0 {
                                return Err(invalid(
                                    "animation time must be finite nonnegative seconds",
                                ));
                            }
                            if arg == "--animation-time" {
                                options.animation_time = seconds;
                            } else {
                                options.animation_step = seconds;
                            }
                        }
                        "--presentation-time" | "--map-time" | "--settle" => {
                            let seconds = value
                                .parse::<f32>()
                                .ok()
                                .filter(|seconds: &f32| seconds.is_finite() && *seconds >= 0.0);
                            let seconds = seconds.ok_or_else(|| {
                                invalid("a presentation clock must be finite nonnegative seconds")
                            })?;
                            match arg.to_str() {
                                Some("--presentation-time") => options.presentation_time = seconds,
                                Some("--settle") => options.settle = seconds,
                                _ => options.map_time = seconds,
                            }
                        }
                        "--window-frames" => {
                            options.window_frames = value
                                .parse()
                                .map_err(|_| invalid("invalid window frame count"))?;
                            if !(1..=1000).contains(&options.window_frames) {
                                return Err(invalid("window frames must be 1..1000"));
                            }
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
                        "--camera" => options.camera = Some(parse_placement(&value)?),
                        "--building-camera" => {
                            options.building_cameras.push(parse_placement(&value)?)
                        }
                        // counters
                        "--perspective" => {
                            options.perspective.country = map::perspective::Country::parse(&value)?
                        }
                        "--fog" => {
                            options.perspective.fog = map::perspective::Choice::parse_fog(&value)?
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
        if options.layers != map::layers::Omissions::default() && !options.headless_dump {
            return Err(invalid("--layers omits named passes and requires --dump"));
        }
        if !options.building_cameras.is_empty() && !options.headless_dump {
            return Err(invalid("--building-camera requires --dump"));
        }
        if options.counters_json.is_some() && !options.headless_dump {
            return Err(invalid("--counters-json requires --dump"));
        }
        if options.ui_scale.is_some() && !(options.headless_dump || options.window_dump) {
            return Err(invalid(
                "--ui-scale sets a dump's map interface scale and requires --dump or --window-dump; \
                 the window uses its UI scale setting",
            ));
        }
        if options.compare.is_some() {
            if options.combat.is_some() {
                return Err(invalid("--combat requires a replay"));
            }
            if options.headless_dump
                || options.window_dump
                || options.replay.is_some()
                || options.out.is_some()
                || options.hour.is_some()
                || options.hour_fraction != 0.0
                || options.branch.is_some()
                || options.camera.is_some()
            {
                return Err(invalid(
                    "--compare scores two images and takes no replay, --dump, --out, --hour, --hour-fraction, --branch or --camera",
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
        if options.combat.is_some() && options.replay.is_none() {
            return Err(invalid("--combat and --combat-at require a replay"));
        }
        if options.headless_dump && (options.out.is_none() || options.hour.is_none()) {
            return Err(invalid(USAGE));
        }
        // counters: the fraction is measured from a named recorded hour, so it
        // is meaningless without one.
        if options.hour_fraction != 0.0 && options.hour.is_none() {
            return Err(invalid(
                "the hour fraction is how far past a recorded hour the frame is and needs --hour",
            ));
        }
        // capture: `--out` is the destination of the window's F12 capture too,
        // so it names a file for all three paths and selects none of them. Only
        // `--dump` renders without a window; `--window-dump` needs a destination
        // because it closes the window as soon as it has written one.
        if options.window_dump && options.out.is_none() {
            return Err(invalid("--window-dump writes one frame and needs --out"));
        }
        if !options.window_dump
            && (options.animation_time != 0.0
                || options.animation_step != 0.0
                || options.window_frames != 0)
        {
            return Err(invalid("animation sampling options require --window-dump"));
        }
        if (options.presentation_time != 0.0 || options.map_time != 0.0 || options.settle != 0.0)
            && !options.headless_dump
        {
            return Err(invalid(
                "--presentation-time, --map-time and --settle set the clocks a dump otherwise renders \
                 at zero; the window runs its own, and --window-dump samples it with --animation-time",
            ));
        }
        if options.out.is_some() && options.replay.is_none() {
            return Err(invalid(USAGE));
        }
        if options.no_water && !options.headless_dump {
            return Err(invalid(
                "--no-water skips the water plane so the underwater pass can be scored on its own \
                 and is a --dump verification aid",
            ));
        }
        // shadows: the same reasoning — the game has this switch too, and the
        // pair it makes is how the cast-shadow chain is measured.
        if options.no_shadows && !options.headless_dump {
            return Err(invalid(
                "--no-shadows skips the cast-shadow chain so its effect can be measured on its \
                 own and is a --dump verification aid",
            ));
        }
        // trees: the same reasoning again — `debug_trees` is what showed that the game's dark
        // forest masses live in the terrain atlas and not in the canopy, so the pair it makes is
        // how the terrain albedo is measured per terrain type.
        if options.no_trees && !options.headless_dump {
            return Err(invalid(
                "--no-trees skips the canopy so the bare terrain can be measured on its own and                  is a --dump verification aid",
            ));
        }
        // borders: the override adds a save's flag to an hour whose recorded flags are short, so
        // it is a verification aid for scoring one frame, not a viewer setting.
        if !options.demilitarized.is_empty() && !options.headless_dump {
            return Err(invalid(
                "--demilitarized adds a save's demilitarized=yes states to the hour's recorded \
                 StateFlags and requires --dump",
            ));
        }
        // borders: the same reasoning as above — this one reproduces a game console toggle so a
        // layer can be scored on its own, and it is not a viewer setting either.
        if options.debug_borders && !options.headless_dump {
            return Err(invalid(
                "--debug-borders reproduces the game console's debug_borders for scoring one \
                 frame and requires --dump",
            ));
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

fn run() -> Result<(), Box<dyn std::error::Error>> {
    let args = std::env::args_os().skip(1).collect::<Vec<_>>();
    if args.iter().any(|arg| arg == "--help" || arg == "-h") {
        println!("{USAGE}");
        return Ok(());
    }
    let options = Options::parse(args.into_iter())?;
    let parent = embedded::Parent::from_environment()?;
    if std::env::var_os("BATTLE_PLAN_VIEWER_PARENT").or_else(|| std::env::var_os("HOI4_VIEWER_PARENT")).is_some()
        && (options.compare.is_some() || options.headless_dump || options.window_dump)
    {
        return Err(invalid(
            "Desktop embedding requires the interactive viewer, without dump or compare options",
        )
        .into());
    }
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
    if let (Some(output), true) = (&options.out, options.headless_dump) {
        if let Some(error) = config_error {
            return Err(invalid(format!("viewer.json: {error}")).into());
        }
        let mut document = document::Document::load(
            options.replay.as_ref().unwrap(),
            &config.resolve_options()?,
            options.allow_unverified,
            &document::InitialHour {
                branch: options.branch,
                hour: options.hour,
                hour_fraction: options.hour_fraction,
            },
            None,
            0,
        )?;
        // Each frame settles its own flag and portrait decodes, in request
        // order, so a dump lays out the same cells on every run.
        document.icons.lazy.set_background(false);
        let session = document.playback.session();
        if let Some(session) = document
            .playback
            .replay()
            .sessions
            .iter()
            .find(|candidate| candidate.id == session)
        {
            println!(
                "Sample boundary: {} (session {})",
                session.sample_boundary, session.id
            );
        }
        // counters: a dump names its hour, so a viewing country that hour does not hold is an
        // error here rather than the window's stated fallback to the recorded player.
        if let map::perspective::Resolution::Absent(tag) =
            document.vision(&options.perspective)?.resolution()
        {
            return Err(Box::new(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!(
                    "--perspective {tag}: hour {} records no country {tag}",
                    document.playback.hour()
                ),
            )));
        }
        // borders: the verification override, before the limitations are printed, so an hour
        // rendered with it does not also claim the layer was not drawn.
        document.demilitarized_override = options.demilitarized.clone();
        // An hour the collector had not finished filling still renders; what it
        // could not draw is stated here, one line per degraded table, before
        // anything that could fail on its own.
        for line in document.limitations(&options.perspective)?.lines() {
            eprintln!("Warning: {line}");
        }
        // political fill
        let layers = political::build(&document.map, document.frame()?)?;
        let mut omitted = options.layers;
        omitted.borders |= options.debug_borders;
        omitted.gradient |= options.debug_borders;
        omitted.trees |= options.no_trees;
        println!("Omitted dump layers: {omitted:?}");
        // weather
        let snow_mud = document.snow_mud()?;
        // close zoom
        let (map_building_field, building_summary) =
            document.map_building_field(&options.perspective)?;
        let aircraft_plan = document.aircraft_plan(&options.perspective)?;
        for line in aircraft_plan.report(&document.aircraft) {
            println!("{line}");
        }
        let mut train_plan = document.train_plan(&options.perspective)?;
        println!(
            "Supply trains (recorded simulation; UNVERIFIED presentation phase): {} routes, {} mesh objects at time 0, seed 0x54524149",
            train_plan.route_count(),
            train_plan.field(&document.trains, 0.0).instances.len()
        );
        for line in train_plan.diagnostics(&document.trains) {
            println!("{line}");
        }
        // orders: the recorded player's battle plan
        let order_field = document.order_field(&options.perspective)?;
        for warning in document.warnings.iter().chain(layers.warnings()) {
            eprintln!("Warning: {warning}");
        }
        let map_size = map::camera::MapSize::of(&document.map);
        let defines = document.map_defines;
        let place = |placement: map::camera::Placement| {
            let [min, max] = defines.heights;
            if !(min..=max).contains(&placement.height) {
                return Err(invalid(format!(
                    "camera height must be between the content's CAMERA_MIN_HEIGHT {min} \
                     and CAMERA_MAX_HEIGHT {max}"
                )));
            }
            Ok(placement.on(map_size, defines))
        };
        let camera = match options.camera {
            None => Camera::fit(map_size, defines, options.size),
            Some(placement) => place(placement)?,
        };
        let building_cameras = options
            .building_cameras
            .iter()
            .map(|placement| place(*placement))
            .collect::<io::Result<Vec<_>>>()?;
        let (unit_model_field, unit_report) = document.unit_model_field(&options.perspective)?;
        // fog of war: the hour's `IntelMap` for the viewing country and the fog
        // switch. The camera decides `FOWFadeFactor`, so the report needs it.
        let intel_map = document.intel_map(&options.perspective)?;
        println!(
            "{}",
            intel_map.line(&camera.defines, camera.height, options.perspective.fog)
        );
        let unverified = document
            .warnings
            .iter()
            .filter(|warning| warning.starts_with("UNVERIFIED CONTENT"))
            .cloned()
            .collect::<Vec<_>>()
            .join("; ");
        let hour = document.playback.hour();
        // names: a headless render has no frame history, so the fade jumps
        // straight to its settled value (R2 section 4.5).
        let fade = LayerFade::instant(camera);
        let placements = document.place_names()?;
        for warning in &placements.warnings {
            eprintln!("Warning: {warning}");
        }
        // counters: bases, labels, counters and combats in priority order.
        let ui_scale = options.ui_scale.unwrap_or(1.0);
        let (mut label_batch, label_report, counter_report, raid_report) = document.overlay_batch(
            camera,
            options.size,
            ui_scale,
            &options.perspective,
            omitted,
        )?;
        for warning in label_report.warnings() {
            eprintln!("Warning: {warning}");
        }
        if let Some(path) = &options.counters_json {
            let mut census =
                document
                    .icons
                    .counters
                    .census(&label_batch, &document.icons.fonts, ui_scale);
            // The same combat lines the dump prints, so the export names the
            // combats whose icons were undecided or drawn with a limited value.
            census["combat_diagnostics"] =
                serde_json::json!(counter_report.combat_gap_lines().collect::<Vec<_>>());
            std::fs::write(path, serde_json::to_vec_pretty(&census)?)?;
        }
        if let Some(selector) = options.combat {
            let mut window = combat::Window::default();
            let selection = selector.resolve(&document)?;
            window.open(selection);
            // Draw until every flag and portrait the window asked for is decoded.
            let drawn = loop {
                let before = document.icons.lazy.version();
                let drawn = window.draw(
                    &document,
                    map::ui_scale::extent(options.size, ui_scale),
                    &options.perspective,
                    None,
                    false,
                )?;
                document.icons.lazy.settle();
                if document.icons.lazy.version() == before {
                    break drawn;
                }
            };
            // The window's warnings say why an undecided flag showed nothing, so they print
            // whether or not it drew.
            for warning in window.warnings(&document) {
                eprintln!("Warning: {warning}");
            }
            if let Some(drawn) = drawn {
                println!("Combat window: {selection:?}, rect {:?}", drawn.rect);
                label_batch.quads.extend(drawn.batch.quads);
            } else {
                println!(
                    "Combat window: {selection:?} not shown: its combat is not engaged for the viewing country at this hour, or the flag is undecided (see the warnings)"
                );
            }
        }
        // borders: the hour's recorded StateFlags, plus whatever --demilitarized added.
        let rail_mesh = document.rail_mesh()?;
        let demilitarized = document.demilitarized_zone(&layers.regions)?;
        if !demilitarized.is_empty() {
            println!(
                "Demilitarised states{}: {}",
                if options.demilitarized.is_empty() {
                    " (recorded)"
                } else {
                    " (recorded, plus the --demilitarized override)"
                },
                demilitarized
                    .states()
                    .iter()
                    .map(u32::to_string)
                    .collect::<Vec<_>>()
                    .join(", ")
            );
        }
        let report = render::dump_png(
            &document.assets,
            // political fill + borders
            render::MapLayers {
                content: &document.content,
                omitted,
                political: &layers,
                border_assets: &document.border_assets,
                border_meshes: &document.border_meshes,
                straits: &document.straits,
                // borders
                demilitarized: &demilitarized,
                // rivers
                river_assets: &document.river_assets,
                river_mesh: &document.river_mesh,
                // weather
                snow_mud: &snow_mud,
                // fog of war
                intel_map: &intel_map,
                // the map's table frame
                frame_assets: &document.frame_assets,
                // close zoom
                sprawl: &document.sprawl,
                map_buildings: &document.map_buildings,
                map_building_field: &map_building_field,
                building_cameras: &building_cameras,
                rail_assets: &document.rail_assets,
                rail_mesh: &rail_mesh,
                aircraft: &document.aircraft,
                aircraft_plan: &aircraft_plan,
                trains: &document.trains,
                train_plan: &train_plan,
                bridges: &document.bridges,
                unit_models: &document.unit_models,
                unit_model_field: &unit_model_field,
                // orders: the recorded player's battle plan
                order_assets: &document.orders,
                order_field: &order_field,
                weather_assets: document.weather_effects.clone(),
                weather_field: crate::map::weather_effects::field(document.frame()?)?,
                // trees
                tree_assets: &document.tree_assets,
                tree_geometry: &document.tree_geometry,
                tree_field: &document.tree_field,
            },
            // names
            &document.icons,
            options.settle,
            Request {
                size: options.size,
                camera,
                day_night: DayNight::at_hour(hour),
                // `vGlobalTime`: zero unless `--presentation-time` names one, so
                // a headless dump is deterministic either way.
                animation_time: options.presentation_time,
                map_time: options.map_time,
                // colour
                postfx: options.postfx,
                no_water: options.no_water,
                // shadows
                no_shadows: options.no_shadows,
                // trees
                no_trees: options.no_trees,
                overlay: Overlay {
                    names: (!omitted.labels).then_some(&placements),
                    layer_alpha: fade.0,
                    sprites: Some(&label_batch),
                    ui_scale,
                },
                // trees
            },
            output,
            (!unverified.is_empty()).then_some(unverified.as_str()),
        )?;
        println!(
            "Rendered branch {}, hour {hour}{} to {} using {}",
            document.playback.branch(),
            if document.hour_fraction > 0.0 {
                format!(" + {:.3} h", document.hour_fraction)
            } else {
                String::new()
            },
            output.display(),
            report.adapter
        );
        println!(
            "Camera: {},{},{}",
            camera.center[0], camera.center[1], camera.height
        );
        println!(
            "Presentation clocks: vGlobalTime {} s (after {} s settled), map scene {} s",
            options.presentation_time + options.settle,
            options.settle,
            options.map_time
        );
        println!(
            "Terrain grid: {} quads at step {}; render {:.1} ms (cpu {:.1}, gpu {:.1})",
            report.quads,
            report.step,
            report.render_seconds * 1000.0,
            report.cpu_seconds * 1000.0,
            (report.render_seconds - report.cpu_seconds) * 1000.0
        );
        // borders
        if !demilitarized.is_empty() {
            println!(
                "Demilitarised-zone border: {} province boundaries highlighted",
                report.demilitarized_edges
            );
        }
        // political fill + borders
        let (filled, distinct) = layers.summary();
        println!(
            "Political layer: {filled} provinces filled in {distinct} country colors (controller); region tables derived in {:.2} ms",
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
        // the map's table frame
        println!(
            "Map frame: {} ambient actors, {} resolved meshes through the compiled content shader",
            document.frame_assets.field.instances.len(),
            document.frame_assets.library.meshes.len(),
        );
        // shadows
        println!(
            "Shadows: cast-shadow chain {} at height {:.3} (cutoff {}), ShadowFadeFactor {:.4}",
            if report.shadows { "drawn" } else { "skipped" },
            camera.height,
            camera.defines.shadows[0],
            map::frame::shadow_fade_factor(&camera.defines, camera.height),
        );
        // colour
        if let Some(grading) = &report.grading {
            println!("Grading: {grading}");
        }
        // trees
        println!(
            "Trees: {} instances in {} buckets over {} kinds, {} canopy vertices;              fade {:.4} at height {:.3}; {} instances drawn in {} draw calls;              generated in {:.0} ms",
            document.tree_field.instances.len(),
            document.tree_field.buckets.len(),
            document.tree_assets.meshes.len(),
            document.tree_geometry.vertices.len(),
            map::trees::tree_fade(&camera.defines, camera.height),
            camera.height,
            report.tree_instances,
            report.tree_draw_calls,
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
                tree.lods.len(),
                tree.scale
            );
        }
        // orders: the recorded player's battle plan
        println!(
            "Battle plan: {} recorded army orders over every country ({}), {} for this perspective, {} drawn ({}), {} with no arrow (garrison areas, invasions without a route, fronts with an empty or destroyed span), {} with fewer than two placeable controls; province centroids and border chains for {} provinces in {:.0} ms",
            order_field.report.recorded,
            order_field
                .report
                .per_kind_recorded
                .iter()
                .map(|(kind, count)| format!("{count} {kind}"))
                .collect::<Vec<_>>()
                .join(", "),
            order_field.report.ours,
            order_field.report.drawn,
            if order_field.report.per_kind.is_empty() {
                "none".to_owned()
            } else {
                order_field
                    .report
                    .per_kind
                    .iter()
                    .map(|(kind, count)| format!("{count} {kind}"))
                    .collect::<Vec<_>>()
                    .join(", ")
            },
            order_field.report.no_arrow,
            order_field.report.too_short,
            document.province_geometry.provinces(),
            document.province_geometry_build.as_secs_f64() * 1000.0
        );
        let plan = &order_field.report;
        println!(
            "  {} snapped offensive lines, {} offset fronts, {} labels placed, {} labels fitting no variant, {} labels without their transport count{}{}",
            plan.snapped,
            plan.offset_fronts,
            plan.labels,
            plan.labels_unfitted,
            plan.transports_omitted,
            plan.refused
                .iter()
                .map(|(what, count)| format!("; refused {count} {what}"))
                .collect::<String>(),
            plan.unavailable
                .as_ref()
                .map(|why| format!("; not drawn: {why}"))
                .unwrap_or_default()
        );
        for (why, count) in &plan.invasion_refused {
            println!("  invasion routes: {count} refused, {why}");
        }
        if order_field.report.drawn > 0 {
            println!(
                "  the drawn curve lands on the recorded province chain: a path province's own centroid is at most {:.3} and on average {:.3} world units from its order's curve; {} ribbon vertices",
                order_field.report.control_gap_max_milli as f64 / 1000.0,
                order_field.report.control_gap_mean_milli as f64 / 1000.0,
                order_field.vertices.len()
            );
        }
        // raids: the viewing country's raid arrows, drawn with the plan
        let raid_arrows = &order_field.report.raid_arrows;
        println!(
            "Raid arrows: {} non-ended raids of the viewing country, {} drawn, {} hidden by the raid filter, {} with no path or controls",
            raid_arrows.raids, raid_arrows.drawn, raid_arrows.filtered, raid_arrows.empty
        );
        for warning in raid_arrows.warnings() {
            println!("  {warning}");
        }
        for unit in &order_field.raid_units {
            println!(
                "  Raid unit {} ({}): at ({:.3}, {:.3}, {:.3}), arrow progress {:.5}, scale {:.3}",
                unit.seed,
                unit.entity.as_deref().unwrap_or("no entity"),
                unit.position[0],
                unit.position[1],
                unit.position[2],
                unit.progress,
                unit.scale
            );
        }
        // close zoom
        let summary = document.sprawl.plan_summary;
        println!(
            "City sprawl: {} urban pixels, {} samples, {} houses over {} entities and {} meshes;              drawn at height {:.3}: {}; {} drawn; generated in {:.0} ms",
            summary.urban_pixels,
            summary.samples,
            summary.placed,
            summary.entities,
            summary.meshes,
            camera.height,
            map::models::draws_city_sprawl(&camera),
            report.sprawl_drawn,
            document.sprawl_build.as_secs_f64() * 1000.0
        );
        println!(
            "Map buildings: {} admitted slots, {} built rows, {} entities selected, {} drawn; \
             {} damaged, {} destroyed, {} in the hysteresis band; \
             {} rows with an unnamed type token, {} with no slot at their location; \
             {} slots hidden by fog of war, {} hub slots the port or city graphics carry \
             ({} kept, naval bases incomplete), {} undetected foreign need_detection slots hidden, \
             {} kept with detection unrecorded, {} floating harbours undecided",
            building_summary.slots,
            building_summary.built,
            building_summary.placed,
            report.map_buildings_drawn,
            building_summary.damaged,
            building_summary.destroyed,
            building_summary.uncertain_damage,
            building_summary.unnamed_tokens,
            building_summary.missing_slots,
            building_summary.fogged,
            building_summary.hubs_left_to_another_layer,
            building_summary.unknown_hub_membership,
            building_summary.undetected,
            building_summary.unknown_detection,
            building_summary
                .unknown_floating_harbors
                .values()
                .sum::<usize>()
        );
        for (name, count) in &building_summary.missing_by_type {
            println!("  no authored slot: {name} x{count}");
        }
        for (name, count) in &building_summary.unresolved_by_type {
            println!("  no entity: {name} x{count}");
        }
        // The refused tokens and the floating harbours by reason, as the window, a capture and
        // map details list them.
        for line in building_summary.warnings() {
            println!("  {line}");
        }
        println!(
            "Railways: {} recorded graph curves, {} dropped for a missing anchor, {} curve samples, \
             {} strip vertices; alpha {:.0} at height {:.3}; traced in {:.0} ms",
            rail_mesh.segments,
            rail_mesh.unanchored,
            rail_mesh.samples,
            rail_mesh.native_vertices.len(),
            map::railways::RailFade::default().update(&camera, 0.0),
            camera.height,
            document.rail_build.as_secs_f64() * 1000.0
        );
        let levels = unit_report
            .by_level
            .iter()
            .map(|(level, count)| format!("{level:?} {count}"))
            .collect::<Vec<_>>()
            .join(", ");
        let gates = document.unit_models.gates;
        println!(
            "Unit models: {} stacks by ladder level ({levels}); {} took the sub-unit's sprite key, {} ride a transport, {} stand along a route, {} have no recorded template, {} have no known country operands, {} have no anchor, {} are hidden from this perspective, {} carry attached entities and {} of those are an attachment rig with no geometry of their own; {} drawn at height {:.3} (land cutoff {}, ship cutoff {}); {} at-sea hulls prepared",
            unit_report.stacks,
            unit_report.sprite_key,
            unit_report.transports,
            unit_report.moving,
            unit_report.untemplated,
            unit_report.unknown_identity,
            unit_report.unanchored,
            unit_report.unseen,
            unit_report.attached,
            unit_report.rigs,
            report.unit_models_drawn,
            camera.height,
            gates.units,
            gates.ships,
            unit_report.ship_hulls,
        );
        if let Some(reason) = &unit_report.disabled {
            println!("  the unit-model layer is off: {reason}");
        }
        if unit_report.label_ordering > 0 {
            println!(
                "  no model for {}: the no-pair fallback's order needs unrecorded variant labels",
                unit_report.label_ordering
            );
        }
        for (reason, count) in &unit_report.selection_errors {
            println!("  no model for {count}: {reason}");
        }
        println!(
            "Aircraft: {} models submitted in this camera",
            report.aircraft_drawn
        );
        println!(
            " ladder inputs: {} sub-units from common/units, {} entities baked as hours placed them",
            document.unit_models.sub_units.len(),
            document.unit_models.library().meshes.len(),
        );
        for (entity, count) in unit_report.by_entity.iter().rev().take(12) {
            println!("  {entity} x{count}");
        }
        println!(
            "Railway bridges: {} placed on river adjacencies, {} meshes",
            rail_mesh.bridges.len(),
            document.bridges.library.meshes.len()
        );
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
        println!("Localisation: {} keys", document.names.localisation.len());
        println!(
            "Names: {} country areas, {} glyph vertices at layer alpha {}; province graph {} edges; {} icon pages",
            placements.country.len(),
            report.overlay.0,
            fade.0,
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
            "Labels: {} icons, {} texts ({} names refused by the localiser) in {} quads; allegiance measured against {}",
            label_report.icons,
            label_report.texts,
            label_report.refused_names,
            report.overlay.1,
            label_report
                .perspective
                .as_deref()
                .unwrap_or("no human country")
        );
        println!(
            "Raid icons: {} ({} own, {} incoming, {} offers); {} offers with an unverified tint, {} withheld, {} with no anchor, {} targets with no province, {} byte counters wrapped, {} without their undeclared glyph, {} incoming with the unknown glyph",
            raid_report.icons,
            raid_report.own,
            raid_report.incoming,
            raid_report.offers,
            raid_report.unverified_tint,
            raid_report.withheld,
            raid_report.unanchored,
            raid_report.unresolved,
            raid_report.wrapped,
            raid_report.missing_glyphs,
            raid_report.unknown_glyphs,
        );
        for (reason, count) in &raid_report.reasons {
            println!("  raid icons: {reason} ({count})");
        }
        // counters
        println!(
            "Counters: {} land ({} between provinces), {} naval ({} task forces not drawn, {} standing for an unplaced route); {} air bases ({} showing their plane count, {} hidden by fog of war), {} naval bases, {} naval HQs, {} facilities ({} unresolved sprites; undecided: {} bases, {} HQs, {} facilities); {} land combats, {} naval combats ({} undecided), {} naval result icons ({} undecided); {} stacks with no anchor",
            counter_report.land_counters,
            counter_report.moving,
            counter_report.naval_counters,
            counter_report
                .unplaced_task_forces
                .values()
                .map(BTreeSet::len)
                .sum::<usize>(),
            counter_report
                .unplaced_task_force_routes
                .values()
                .map(BTreeSet::len)
                .sum::<usize>(),
            counter_report.air_bases,
            counter_report.air_base_counts,
            counter_report.hidden_air_bases,
            counter_report.naval_bases,
            counter_report.naval_headquarters,
            counter_report.facilities,
            counter_report
                .unresolved_facility_sprites
                .values()
                .sum::<usize>(),
            counter_report.unknown_bases.totals()[0],
            counter_report.unknown_bases.totals()[1],
            counter_report.unknown_bases.totals()[2],
            counter_report.land_combats,
            counter_report.naval_combats,
            counter_report.unavailable_naval_combats,
            counter_report.naval_result_icons,
            counter_report.unavailable_naval_results,
            counter_report.unanchored
        );
        for line in counter_report.combat_gap_lines() {
            println!("{line}");
        }
        // The refused tokens, the task forces and base icons not drawn and the undecided naval
        // icons, as the window lists them.
        for warning in counter_report.warnings() {
            println!("{warning}");
        }
        // The recorded naval engaged bytes against the flag derived for the recording's
        // player, over every recorded hour: the count offline runs read.
        let started = std::time::Instant::now();
        let check = engaged_index::check_recorded_bytes(document.playback.replay());
        let lines = check.lines();
        if lines.is_empty() {
            println!("Naval engaged check: the recording holds no hour to fold.");
        }
        for line in lines {
            println!("{line}");
        }
        println!(
            "Naval engaged check folded the recording in {:.2} s.",
            started.elapsed().as_secs_f64()
        );
        // counters: faction-theatre pins
        let pings = &counter_report.faction_ping_limits;
        println!(
            "Faction-theatre pins: {} drawn ({} without a resolved emblem); {} theatres refused for a shared first region, {} without a region position, {} unresolved hidden-for links{}",
            counter_report.faction_pings,
            counter_report.unresolved_ping_emblems,
            pings.ambiguous,
            pings.unplaced,
            pings.unresolved_hidden,
            pings
                .undecided
                .as_deref()
                .map(|reason| format!("; undecided: {reason}"))
                .unwrap_or_default()
        );
        // counters: railway guns and the ability grid, and what they could not draw
        println!(
            "Railway guns: {} plates drawn, {} guns not drawn (incomplete census or unrecorded definition name), {} moving guns standing because content cannot place their route; {} active abilities without a resolved sprite",
            counter_report.railway_guns,
            counter_report.unavailable_railway_guns,
            counter_report.unplaced_railway_gun_routes,
            counter_report.unavailable_ability_icons,
        );
        // counters: ETA derivation coverage for this hour
        if let Some(line) = counter_report.eta.line() {
            println!("{line}");
        }
        // counters: the viewing country and the fog switch, and what they hid
        println!(
            "Counter perspective: viewing country {} ({}), fog of war {}; {} of {} counters carry the game's question mark, {} of {} combat icons do",
            document.viewing_tag(&counter_report.perspective)?,
            options.perspective.country,
            if counter_report.perspective.fog() {
                "on"
            } else {
                "off"
            },
            counter_report.no_intel_counters,
            counter_report.land_counters + counter_report.naval_counters,
            counter_report.no_intel_combats,
            counter_report.land_combats + counter_report.naval_combats,
        );
        // counters: the province term of the engine's land branch
        println!(
            "Province visibility: {} provinces revealed to this perspective{}",
            counter_report.perspective.revealed_provinces(),
            match counter_report.perspective.visibility_gap() {
                Some(input) => format!("; fog not rebuilt: {input} unavailable"),
                None => String::new(),
            }
        );
        // counters: the combat icon's direction wedge follows the same intelligence gate
        println!(
            "Combat direction wedges: {} drawn of {} icons; the engine hides the wedge for a combat              the viewer has no intelligence on",
            (counter_report.land_combats).saturating_sub(counter_report.no_intel_combats),
            counter_report.land_combats,
        );
        // counters
        if let Some(level) = counter_report.grouping {
            println!(
                "Counter grouping: {level:?} level, {} groups",
                counter_report.groups,
            );
        }
        println!(
            "Counter assets: {} sprite sheets, {} small flags indexed ({} decoded, {} refused), {} lazy texture bytes resident and {} unit-type frames packed{}",
            document.icons.counters.sheet_count(),
            document.icons.counters.flags.len(),
            document.icons.counters.flags.loaded(),
            document.icons.counters.flags.rejected(),
            document.icons.lazy.resident_bytes(),
            document.icons.counters.types.len(),
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
    let viewport = eframe::egui::ViewportBuilder::default()
        .with_inner_size([1400.0, 900.0])
        .with_min_inner_size([800.0, 500.0]);
    let viewport = if parent.is_some() {
        viewport
            .with_visible(false)
            .with_active(false)
            .with_decorations(false)
            .with_resizable(false)
            .with_taskbar(false)
            .with_min_inner_size([1.0, 1.0])
    } else {
        viewport
    };
    let native = eframe::NativeOptions {
        renderer: eframe::Renderer::Wgpu,
        viewport,
        wgpu_options: render::configuration(),
        ..Default::default()
    };
    // capture: a `--window-dump` failure has to leave the process, and the only
    // thing the window can hand back through `run_native` is what it wrote here.
    let failure: Arc<Mutex<Option<String>>> = Arc::new(Mutex::new(None));
    let reported = Arc::clone(&failure);
    eframe::run_native(
        "Battle Plan",
        native,
        Box::new(move |cc| {
            let embedding = parent.map(|parent| parent.attach(cc)).transpose()?;
            let viewer = app::Viewer::new(cc, config, options, config_error, reported, embedding);
            Ok(Box::new(viewer))
        }),
    )?;
    map::counter_move::profile_flush();
    let failure = failure.lock().map_err(|error| error.to_string())?.take();
    match failure {
        Some(error) => Err(invalid(error).into()),
        None => Ok(()),
    }
}

fn main() -> std::process::ExitCode {
    let result = run();
    // Profiling lines are written by a thread of their own; every mode waits
    // for them, a headless dump included.
    map::counter_move::profile_flush();
    match result {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("{error}");
            embedded::startup_error(error.to_string());
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
    fn combat_selection_retains_the_complete_native_handle() {
        let options = parse(&["battle.battleplan", "--combat", "0xffffffffffffffff"]).unwrap();
        assert_eq!(options.combat, Some(combat::Selector::Id(u64::MAX)));
        assert_eq!(
            parse(&["battle.battleplan", "--combat-at", "362"])
                .unwrap()
                .combat,
            Some(combat::Selector::Province(362))
        );
        assert!(parse(&["--combat", "1"]).is_err());
        assert!(parse(&["battle.battleplan", "--combat", "1", "--combat-at", "362"]).is_err());
        assert!(parse(&["battle.battleplan", "--combat-at", "4294967296"]).is_err());
    }

    /// The game's two map controls, and no third.
    #[test]
    fn perspective_and_fog_are_the_two_view_options() {
        let default = parse(&["campaign.battleplan"]).unwrap();
        assert_eq!(default.perspective, map::perspective::Choice::default());
        assert!(default.perspective.fog);
        let options = parse(&[
            "campaign.battleplan",
            "--perspective",
            "sov",
            "--fog",
            "off",
        ])
        .unwrap();
        assert_eq!(
            options.perspective,
            map::perspective::Choice {
                country: map::perspective::Country::Tag("SOV".into()),
                fog: false,
            }
        );
        assert!(parse(&["campaign.battleplan", "--perspective", "omniscient"]).is_err());
        assert!(parse(&["campaign.battleplan", "--fog", "maybe"]).is_err());
        assert!(parse(&["campaign.battleplan", "--fog"]).is_err());
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

    // counters: the glide between recorded hours
    #[test]
    fn a_fractional_hour_names_a_frame_between_two_recorded_ones() {
        let dump = parse(&[
            "--dump",
            "campaign.battleplan",
            "--hour",
            "60832633.5",
            "--out",
            "frame.png",
        ])
        .unwrap();
        assert_eq!(dump.hour, Some(60832633));
        assert!((dump.hour_fraction - 0.5).abs() < 1e-6);

        // The two spellings are the same number.
        let split = parse(&[
            "--dump",
            "campaign.battleplan",
            "--hour",
            "60832633",
            "--hour-fraction",
            "0.5",
            "--out",
            "frame.png",
        ])
        .unwrap();
        assert_eq!(
            (split.hour, split.hour_fraction),
            (dump.hour, dump.hour_fraction)
        );

        // A recorded hour renders exactly, which is a zero fraction.
        let whole = parse(&[
            "--dump",
            "campaign.battleplan",
            "--hour",
            "60832633",
            "--out",
            "frame.png",
        ])
        .unwrap();
        assert_eq!(whole.hour_fraction, 0.0);
    }

    #[test]
    fn an_hour_fraction_outside_its_own_hour_or_without_one_is_refused() {
        for bad in ["60832633.", "60832633.x", "60832633.5.5", "x.5"] {
            assert!(parse_hour(bad).is_err(), "`{bad}` should be refused");
        }
        // One whole hour is the next recorded hour, which renders on its own.
        assert!(parse_fraction("1").is_err());
        assert!(parse_fraction("1.5").is_err());
        assert!(parse_fraction("-0.5").is_err());
        assert!((parse_fraction("0.75").unwrap() - 0.75).abs() < 1e-6);
        // The fraction is measured from a named recorded hour.
        assert!(parse(&["campaign.battleplan", "--hour-fraction", "0.5"]).is_err());
        assert!(parse(&["--compare", "a.png", "b.png", "--hour-fraction", "0.5"]).is_err());
    }

    #[test]
    fn a_dump_takes_an_optional_ui_scale_and_the_window_refuses_one() {
        let dump = |scale: &str| {
            parse(&[
                "--dump",
                "c.battleplan",
                "--hour",
                "60832633",
                "--out",
                "frame.png",
                "--ui-scale",
                scale,
            ])
        };
        assert_eq!(dump("1.25").unwrap().ui_scale, Some(1.25));
        for bad in ["0", "0.4", "3.5", "x", "NaN"] {
            assert!(dump(bad).is_err(), "`{bad}` should be refused");
        }
        let plain = parse(&[
            "--dump",
            "c.battleplan",
            "--hour",
            "60832633",
            "--out",
            "frame.png",
        ])
        .unwrap();
        assert_eq!(plain.ui_scale, None);
        assert!(parse(&["campaign.battleplan", "--ui-scale", "1.25"]).is_err());
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
        ])
        .unwrap();
        assert_eq!(dump.replay, Some(PathBuf::from("campaign.battleplan")));
        assert_eq!(dump.out, Some(PathBuf::from("frame.png")));
        assert!(dump.headless_dump);
        assert_eq!(dump.hour, Some(60826315));
        assert!(dump.compare.is_none());
        assert_eq!(dump.camera.unwrap().center, [100.0, 200.0]);
        assert_eq!(dump.camera.unwrap().height, 300.0);
        assert_eq!(dump.size, [2048, 1152]);

        let window = parse(&["campaign.battleplan", "--branch", "2"]).unwrap();
        assert_eq!(window.replay, Some(PathBuf::from("campaign.battleplan")));
        assert!(window.out.is_none() && window.compare.is_none());
        assert!(!window.headless_dump && !window.window_dump);

        assert!(parse(&["--dump", "campaign.battleplan", "--hour", "1"]).is_err());
        assert!(parse(&["--dump", "campaign.battleplan", "--out", "f.png"]).is_err());
    }

    // borders
    #[test]
    fn the_layer_toggle_is_a_dump_only_verification_aid() {
        let dump = parse(&[
            "--dump",
            "campaign.battleplan",
            "--hour",
            "60826315",
            "--out",
            "frame.png",
            "--debug-borders",
        ])
        .unwrap();
        assert!(dump.debug_borders);
        assert!(
            !parse(&["--dump", "c.battleplan", "--hour", "1", "--out", "f.png"])
                .unwrap()
                .debug_borders
        );
        assert!(parse(&["c.battleplan", "--debug-borders"]).is_err());
        let dump = parse(&[
            "--dump",
            "c.battleplan",
            "--hour",
            "1",
            "--out",
            "f.png",
            "--layers",
            "labels,counters,gradient",
        ])
        .unwrap();
        assert!(dump.layers.labels && dump.layers.counters && dump.layers.gradient);
        assert!(!dump.layers.borders);
        for value in ["unknown", "labels,", ""] {
            assert!(map::layers::Omissions::parse(value).is_err());
        }
        assert!(parse(&["c.battleplan", "--layers", "trees"]).is_err());
        assert!(parse(&["--compare", "g.png", "v.png", "--layers", "trees"]).is_err());
    }

    // trees
    #[test]
    fn the_canopy_toggle_is_a_dump_only_verification_aid() {
        let dump = parse(&[
            "--dump",
            "campaign.battleplan",
            "--hour",
            "60828490",
            "--out",
            "frame.png",
            "--no-trees",
        ])
        .unwrap();
        assert!(dump.no_trees);
        assert!(
            !parse(&["--dump", "c.battleplan", "--hour", "1", "--out", "f.png"])
                .unwrap()
                .no_trees
        );
        assert!(parse(&["c.battleplan", "--no-trees"]).is_err());
    }

    /// The presentation clock is a dump option: the window runs its own, and a
    /// dump renders zero unless a fitted time is named.
    #[test]
    fn the_presentation_clock_is_a_dump_option_and_defaults_to_zero() {
        let dump = |seconds: &str| {
            parse(&[
                "--dump",
                "campaign.battleplan",
                "--hour",
                "60835691",
                "--out",
                "frame.png",
                "--presentation-time",
                seconds,
            ])
        };
        assert_eq!(dump("547.981").unwrap().presentation_time, 547.981);
        assert_eq!(dump("0").unwrap().presentation_time, 0.0);
        let settled = parse(&[
            "--dump",
            "c.battleplan",
            "--hour",
            "1",
            "--out",
            "f.png",
            "--settle",
            "20",
        ])
        .unwrap();
        assert_eq!(settled.settle, 20.0);
        assert!(parse(&["c.battleplan", "--settle", "20"]).is_err());
        assert!(dump("-1").is_err());
        assert!(dump("nan").is_err());
        assert!(dump("inf").is_err());
        // The map scene runs a second clock and takes its own option.
        let both = parse(&[
            "--dump",
            "campaign.battleplan",
            "--hour",
            "60835691",
            "--out",
            "frame.png",
            "--presentation-time",
            "547.981",
            "--map-time",
            "206.792",
        ])
        .unwrap();
        assert_eq!(both.presentation_time, 547.981);
        assert_eq!(both.map_time, 206.792);
        assert_eq!(dump("1").unwrap().map_time, 0.0);
        assert_eq!(
            parse(&["--dump", "c.battleplan", "--hour", "1", "--out", "f.png"])
                .unwrap()
                .presentation_time,
            0.0
        );
        assert!(
            parse(&[
                "--window-dump",
                "c.battleplan",
                "--hour",
                "1",
                "--out",
                "f.png",
                "--presentation-time",
                "1",
            ])
            .is_err()
        );
    }

    // capture
    #[test]
    fn window_dump_takes_the_dump_options_and_drives_the_window() {
        let options = parse(&[
            "--window-dump",
            "campaign.battleplan",
            "--hour",
            "60826315",
            "--branch",
            "1",
            "--out",
            "frame.png",
            "--camera",
            "100,200,300",
            "--width",
            "1279",
            "--height",
            "723",
            "--postfx",
            "off",
        ])
        .unwrap();
        assert!(options.window_dump);
        assert_eq!(options.replay, Some(PathBuf::from("campaign.battleplan")));
        assert_eq!(options.out, Some(PathBuf::from("frame.png")));
        assert!(!options.headless_dump);
        assert_eq!(options.size, [1279, 723]);
        assert!(!options.postfx);
        // It writes one frame and closes, so it needs somewhere to write it.
        assert!(parse(&["--window-dump", "campaign.battleplan", "--hour", "1"]).is_err());
        // Only one of the two paths can own the replay.
        assert!(
            parse(&[
                "--dump",
                "a.battleplan",
                "--hour",
                "1",
                "--out",
                "f.png",
                "--window-dump",
                "b.battleplan"
            ])
            .is_err()
        );
    }

    // capture
    #[test]
    fn the_window_takes_out_as_the_destination_of_its_own_capture() {
        let options = parse(&["campaign.battleplan", "--out", "capture.png"]).unwrap();
        assert!(!options.window_dump);
        // `--out` alone still opens a window; it only names where F12 writes.
        assert!(!options.headless_dump);
        assert_eq!(options.out, Some(PathBuf::from("capture.png")));
        // A destination with nothing to render into it is still a mistake.
        assert!(parse(&["--out", "capture.png"]).is_err());
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
        let visited = parse(&[
            "--dump",
            "c.battleplan",
            "--hour",
            "1",
            "--out",
            "f.png",
            "--building-camera",
            "1,2,100",
            "--building-camera",
            "3,4,90",
        ])
        .unwrap()
        .building_cameras;
        assert_eq!(visited.len(), 2);
        assert_eq!(visited[1].center, [3.0, 4.0]);
        assert!(parse(&["c.battleplan", "--building-camera", "1,2,100"]).is_err());
        for bad in ["3010,1568", "3010,1568,inf", "a,b,c"] {
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
