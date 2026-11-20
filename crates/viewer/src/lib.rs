//! The map: the desktop app hosts it in its own process ([`host`]); the `viewer` binary is the
//! development harness and tooling around the same code.
use std::time::Instant;
use viewer_foundation::map::camera::MapSize;
use viewer_foundation::map::camera::Placement;
use viewer_foundation::map::camera::{Free, Lens, Pose};
use viewer_foundation::map::frame::shadow_fade_factor;
use viewer_foundation::map::layers::Omissions;
use viewer_foundation::map::perspective::Choice;
use viewer_foundation::map::perspective::Country;
use viewer_foundation::map::perspective::Resolution;
use viewer_foundation::map::ui_scale::extent;
use viewer_foundation::map::ui_scale::valid;
use viewer_foundation::profile_writer::profile_flush;
mod app;
use viewer_combat as combat;
#[cfg(test)]
mod combat_integration_tests;
mod compare;
mod controls;
mod corpus;
#[cfg(test)]
mod counter_integration_tests;
mod dashboard_maps;
use viewer_playback::document;
mod dump_head;
use viewer_foundation::errors::invalid;
mod embedded;
use viewer_playback::engaged_index;
pub mod host;
use viewer_playback::lane_rows;
use viewer_playback::layers;
mod map;
use viewer_playback::map_assets;
use viewer_playback::map_views;
mod page_clock;
use viewer_playback::perf_log;
use viewer_playback::playback;
mod playback_cost;
mod playhead_host {
    pub use viewer_playback::playhead_host::*;
    pub mod views {
        pub use viewer_playback::playhead_host::views::*;
        pub mod combat;
        pub mod proof;
        pub mod roster;
        pub mod window;
        #[cfg(test)]
        pub mod tests {
            pub use viewer_playback::playhead_host::views::tests::*;
            mod combat;
            mod hover;
        }
    }
}
use viewer_playback::render;
mod renderer_host;
mod screen_count;
mod suite_input;
#[cfg(test)]
mod unit_scale_tests;
use viewer_playback::visual_fallback;
use viewer_playback::worker_pool;

use dashboard::Config;

/// Tests run on the allocator the map ships with.
#[cfg(test)]
#[global_allocator]
static TEST_ALLOCATOR: engine_heap::Heap = engine_heap::Heap;

use {viewer_foundation::map::camera::Camera, viewer_foundation::map::frame::DayNight};
// names
use replay_data::{BranchId, Hour};
use std::{
    collections::BTreeSet,
    ffi::OsString,
    fs, io,
    path::PathBuf,
    sync::Arc,
};
use {
    viewer_world::map::name_pass::{LayerFade, StrategicNames},
    viewer_world::map::scene::Overlay,
    viewer_world::map::scene::Request,
};

/// The harness and tooling the viewer runs. Interactive maps belong to the desktop app, which
/// hosts the map in its own process.
const USAGE: &str = "viewer is the development harness and tooling around Battle Plan's map; the desktop
app (replay-desktop.exe) shows maps itself. It runs:
viewer --playback-cost <replay> --from-hour H --hours N --speed max --out <directory>
         with --paced S, --flight <x,z[,height]> flies the camera: tilted, east, a turn, a climb
         with --paced S, --walk <x,z,heading[,scale]> walks the viewing country's infantry, with
         --walk-keys <keys> held every frame and --walk-flying starting in the air
         with --paced S, --campaign-tab runs each frame as the app's campaign map tab runs it
         --frame-limit HZ sets the app's frame rate limit: the run paces, budgets and reports at
         whatever it and --refresh-hz leave, as a limited map presents at
         --gpu-pass-times records each group of passes' GPU microseconds, for a run measuring the
         GPU; their timestamp commands cost the frame thread CPU, so a CPU timing run omits them
viewer --visual-toggle <replay> --hour H --out <directory> [--mod-a <mod>]... [--mod-b <mod>]...
         [--toggles N] [--refresh-hz HZ] [--settle-frames N] [--timeout S] [--no-fresh]
         toggles visual mods through the app's path; a mod by its listed name or its id
viewer --viewpoint-switch <replay> --hour H --out <directory> --countries <TAG,TAG,...> [--rounds N]
         [--play-switches N] [--play-speed H/S] [--hour-steps N] [--refresh-hz HZ] [--timeout S]
         times switches of the viewing country to the first changed and the settled frame
viewer --dump <replay.battleplan> --hour <n>[.<fraction>] --out <frame.png>
viewer --window-dump <replay.battleplan> --hour <n> --out <frame.png>
viewer --compare <game.png> <viewer.png> [--mask <mask.png>] [--diff <out.png>]
Folders: --game-dir <directory> --user-dir <directory>, default the desktop app's saved folders
Capture artwork: --visual-mod <mod id> (repeatable), mounted above recorded artwork
Options: --branch <id> --hour-fraction <0..1>, how far past --hour the frame is; a moving
         division's counter stands at its settled route point all hour, as the game draws it
         --allow-unverified-content
         --width <pixels> --height <pixels> --camera <x,z,height> --postfx on|off
         --profile-pointer <x,y>, framebuffer hover for a profiled or playback-cost window only
         --free-camera <x,y,z,yaw,pitch[,roll[,fov[,near[,far]]]]>, a free eye at world (x, y, z)
           turned by degrees (yaw clockwise from north, pitch up, roll right side down) with a
           vertical field of view in degrees (default 45) and near/far planes (default 1, 20000)
         --free-overlays on|off, whether a free camera draws the map's interface standing in
           the world, as the fly and walk modes' overlay choice does (default off)
         --perspective <TAG>|recorded, the viewing country (default recorded)
         --fog on|off, the game's fog-of-war switch (default on)
         --allied-plans on|off, viewer-local allied battle-plan visibility
         --day-night on|off, the map's day/night lighting
         --counter-colors allegiance|country, the counters' tint
         --nato-counters on|off, NATO counter frames
           (these four default to the user folder's settings.txt, else the game's defaults)
         --combat <full-handle> | --combat-at <province>, open a recorded combat or retained naval result
         --air-region <region>, open the strategic air dialog; --air-details expands its history
         --state-view <province> | --naval-region <region>, open the state or naval region view
         --air-range days|months|years --air-channels <bitmask>, select the history range and series
         --diagnostics, technical stats and data limitations; dumps and captures report the
           limitations either way
Window dump: --window-dump opens a window, renders through the windowed path at --width/--height
             and --camera or --free-camera, writes the frame to --out and exits
             --walk <x,z,heading[,scale]> draws the walking mode instead: the viewing country's
             infantry standing at world (x, z) facing heading degrees clockwise from north, at
             scale (default 1), seen from behind; --walk-keys <keys> holds keys through every
             frame (w, a, s, d, space, shift, ctrl, plus, minus) and --walk-flying starts in
             flight; each frame moves it by --animation-step
             --animation-time <seconds> --animation-step <seconds> --window-frames <1..1000>
             sample a deterministic sequence; default is one frame at time zero
             --fly <x,y,z,yaw,pitch[,roll]>, the flying mode with its aircraft at world (x, y, z)
               turned by degrees, in place of --camera; each sampled frame flies one step
             --fly-view chase|nose, the flying camera (default chase)
             --fly-steer <turn,climb,throttle>, each -1..1, held through the capture as the
               controller's left stick and triggers (default none)
Dump only: --presentation-time <seconds>, the engine's vGlobalTime and FoWTime, zero in a dump:
             the fog-of-war noise, the entity animation and the map-icon windows' GUI clock
           --map-time <seconds>, the map scene's own second clock: the selection
             pulse, the river flow and the waves
           --settle <seconds>, play that many seconds before the frame so entity events
             (damaged fire and smoke) and particles are under way; renders at the clock plus it
           --building-camera <x,z,height>, repeatable: earlier views whose map buildings the frame keeps
           --demilitarized <state-id,...>, a verification aid that adds the save's demilitarized=yes states
           --debug-borders, the game's console toggle: no gradient fill and no border strips
           --no-water, a verification aid that skips the water plane as the game's debug_water does
           --no-trees, a verification aid that skips the canopy as the game's debug_trees does
           --layers <names>, comma-separated passes to OMIT: counters,labels,trees,borders,gradient,
             rivers,weather,orders,models,sprawl,buildings,rails
             labels include VP/capital sprites; counters include bases/combat; borders include straits;
             rails include trains/bridges; weather omits particles only, preserving terrain snow/mud
           --counters-json <out.json>, submitted plates with interface rectangles
             (device pixels at --ui-scale 1), sprite frames and text
           --province-hover <id>, print hover text and set the local strategic-name hover
           --raid-details <province>, open the raid offer bubble's target details window there
           --find <query>, draw the find view with the query typed and print its results
           --intel-hover <x,y>, print an Operatives map entry's hover (repeatable)
             under that interface point, or that none takes it
           --ui-scale <0.5..3>, --dump and --window-dump: device pixels per map-interface pixel
             (counters, icons, labels, combat window); default 1.0, the game's frame at gui_scale 1
Compare: --exclude-game-ui --exclude <x,y,w,h> --regions <cols>x<rows> --json <out.json>
         --upscale <factor> --dim <factor>";

/// `--free-camera`'s value: the eye's world `x,y,z`, then yaw and pitch in degrees, and optionally
/// roll and the vertical field of view in degrees and the near and far planes.
fn parse_free_camera(value: &str) -> io::Result<Placement> {
    const USAGE: &str = "free camera requires x,y,z,yaw,pitch[,roll[,fov[,near[,far]]]]";
    let values = value
        .split(',')
        .map(str::parse::<f32>)
        .collect::<Result<Vec<_>, _>>()
        .map_err(|_| invalid(USAGE))?;
    if !(5..=9).contains(&values.len()) || !values.iter().all(|value| value.is_finite()) {
        return Err(invalid(USAGE));
    }
    let at = |index: usize, default: f32| values.get(index).copied().unwrap_or(default);
    let lens = Lens {
        fov_y: at(6, Lens::MAP.fov_y.to_degrees()).to_radians(),
        near: at(7, 1.0),
        far: at(8, 20_000.0),
    };
    if !lens.is_valid() {
        return Err(invalid(
            "free camera needs a field of view between 0 and 180 degrees and 0 < near < far",
        ));
    }
    let mut pose = Free::new(values[3].to_radians(), values[4].to_radians(), lens);
    pose.roll = at(5, 0.0).to_radians();
    Ok(Placement {
        center: [values[0], values[2]],
        height: values[1],
        pose: Pose::Free(pose),
    })
}

/// `--fly`'s value: the aircraft's world `x,y,z`, then its heading and pitch in degrees, and
/// optionally its bank.
fn parse_fly(value: &str) -> io::Result<app::fly::Scripted> {
    const USAGE: &str = "fly requires x,y,z,yaw,pitch[,roll]";
    let values = value
        .split(',')
        .map(str::parse::<f32>)
        .collect::<Result<Vec<_>, _>>()
        .map_err(|_| invalid(USAGE))?;
    if !(5..=6).contains(&values.len()) || !values.iter().all(|value| value.is_finite()) {
        return Err(invalid(USAGE));
    }
    Ok(app::fly::Scripted {
        position: [values[0], values[1], values[2]],
        attitude: [
            values[3].to_radians(),
            values[4].to_radians(),
            values.get(5).copied().unwrap_or(0.0).to_radians(),
        ],
        view: controls::flight::View::Chase,
        steer: [0.0; 3],
    })
}

/// `--fly-steer`'s value: turn, climb and throttle, each within -1..=1.
fn parse_fly_steer(value: &str) -> io::Result<[f32; 3]> {
    const USAGE: &str = "fly steer requires turn,climb,throttle, each from -1 to 1";
    let values = value
        .split(',')
        .map(str::parse::<f32>)
        .collect::<Result<Vec<_>, _>>()
        .map_err(|_| invalid(USAGE))?;
    match values.as_slice() {
        &[turn, climb, throttle] if values.iter().all(|value| (-1.0..=1.0).contains(value)) => {
            Ok([turn, climb, throttle])
        }
        _ => Err(invalid(USAGE)),
    }
}

fn parse_profile_pointer(value: &str) -> io::Result<[f32; 2]> {
    let values = value
        .split(',')
        .map(str::parse::<f32>)
        .collect::<Result<Vec<_>, _>>()
        .map_err(|_| invalid("--profile-pointer needs finite nonnegative framebuffer x,y"))?;
    match values.as_slice() {
        &[x, y] if x.is_finite() && y.is_finite() && x >= 0.0 && y >= 0.0 => Ok([x, y]),
        _ => Err(invalid(
            "--profile-pointer needs finite nonnegative framebuffer x,y",
        )),
    }
}

fn parse_placement(value: &str) -> io::Result<Placement> {
    let values = value
        .split(',')
        .map(str::parse::<f32>)
        .collect::<Result<Vec<_>, _>>()
        .map_err(|_| invalid("camera requires world-x,world-z,height"))?;
    if values.len() != 3 || !values.iter().all(|value| value.is_finite()) {
        return Err(invalid("camera requires finite world-x,world-z,height"));
    }
    Ok(Placement {
        center: [values[0], values[1]],
        height: values[2],
        pose: Pose::Map,
    })
}

// counters: a frame between two recorded hours
/// `--hour`'s value: a recorded engine hour, optionally with a fraction of an
/// hour past it.
///
/// `--hour 60832633` is the recorded hour and renders it exactly.
/// `--hour 60832633.5` is half an hour later, a frame the recording does not
/// hold: the hour's own tables are drawn and moving division counters stay at
/// their settled route points. The fraction is parsed off the text rather than
/// through `f64` so a large engine hour cannot lose its last digits on the way.
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
    air_details: bool,
    air_range: usize,
    air_channels: u32,
    replay: Option<PathBuf>,
    /// Where a rendered frame is written: `--dump`'s and `--window-dump`'s output.
    out: Option<PathBuf>,
    /// `--dump`: render one frame without opening a window.
    headless_dump: bool,
    playback_cost_end: Option<Hour>,
    visual_mods: Vec<installed_content_core::content::visual::Choice>,
    // capture
    /// `--window-dump`: open the window and render one frame through the
    /// windowed path at `size` and `camera` into `dump`, then close.
    ///
    /// The window and `--dump` are only comparable if the frame comes out of the
    /// window's own path, so this drives [`app::Viewer`] rather than
    /// [`render::dump_png`]; the readback is the dump's own.
    window_dump: bool,
    /// `--window-dump --fly`: the flying mode's scripted flight, with `--fly-view` and
    /// `--fly-steer` applied.
    fly: Option<app::fly::Scripted>,
    /// `--fly-view` and `--fly-steer` as given, applied to `fly` once every argument is read.
    fly_view: Option<controls::flight::View>,
    fly_steer: Option<[f32; 3]>,
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
    /// reads it, which is the fog-of-war noise and the entity animation, and so
    /// does the map-icon windows' GUI clock (`Time`). Its real-time epoch
    /// is not recorded either.
    presentation_time: f32,
    /// `--dump --settle <seconds>`: frames played on the presentation clock
    /// before the dump, one second apart, so entity events such as a damaged
    /// building's fire and smoke and weather particles are under way as in a
    /// running viewer. The dump then renders at `presentation_time + settle`.
    settle: f32,
    /// `--dump --map-time <seconds>`: the map scene's own clock, which
    /// shows is a different one — `vGlobalTime` 547.981 and `FoWTime` 547.978
    /// against the border pass's `vTime_Transparency.x` 206.792, the river's
    /// `vTimeDirectionSeasonLerp.x` 206.822 and the water's 206.826. It drives
    /// the border selection pulse, the river flow and the waves.
    map_time: f32,
    hour: Option<Hour>,
    // counters: a frame between two recorded hours
    /// How far past `hour` the frame is, in hours, in `[0, 1)`.
    ///
    /// `--hour 60832633.5` and `--hour-fraction 0.5` are two spellings of the
    /// same number, and it is [`document::Document::hour_fraction`]: the
    /// recorded hour is the whole frame's state; moving division counters stay
    /// at their settled route points. Zero renders the recorded hour exactly.
    hour_fraction: f32,
    branch: Option<BranchId>,
    allow_unverified: bool,
    // counters
    /// `--perspective` and `--fog`: the game's two map controls. `recorded`
    /// (the default) views the map as the recording client's viewpoint at that hour and a tag as
    /// that country; `--fog off` is the game's `fow` console switch. See
    /// `viewer_foundation::map::perspective`.
    perspective: Choice,
    /// `--map-mode <token>`: the map mode a window opens in and a dump renders, a picker
    /// mode's token (`default`, `states`, `factions`, ...) or `scripted:<name>`.
    map_mode: viewer_world::map::map_mode::MapMode,
    /// Viewer-local allied-plan switch for dumps and windows; unset takes `settings.txt`'s.
    allied_plans: Option<bool>,
    /// `--day-night`: the map's day/night lighting for dumps and windows, unset taking
    /// `settings.txt`'s; the desktop app's maps take its graphics preferences.
    day_night: Option<bool>,
    /// `--free-overlays`: whether a dump or window dump from a free camera draws the map's
    /// interface standing in the world, as the fly and walk modes' overlay choice does. Off by
    /// default, as those modes start.
    free_overlays: Option<bool>,
    camera: Option<Placement>,
    /// A profiled window's synthetic framebuffer hover, never an OS input.
    profile_pointer: Option<[f32; 2]>,
    /// `--window-dump --walk`: a walk placed on the map, drawn through the walking mode with
    /// the keys `--walk-keys` holds ([`controls::walker::Start`]).
    walk: Option<controls::walker::Start>,
    /// `--dump --building-camera <x,z,height>`, repeatable: cameras the map
    /// visited before this frame, in order. Map buildings keep what those views
    /// created, as a live session does.
    building_cameras: Vec<Placement>,
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
    /// `debug_water` console toggle does; see `viewer_world::map::scene::Request::no_water`.
    no_water: bool,
    /// `--dump --no-shadows` skips the cast-shadow chain, which is what the
    /// game's `Draw.Shadows` console toggle does; see
    /// `viewer_world::map::scene::Request::no_shadows`.
    no_shadows: bool,
    // trees
    /// `--dump --no-trees` skips the canopy, which is what the game's
    /// `debug_trees` console toggle does; see `viewer_world::map::scene::Request::no_trees`.
    no_trees: bool,
    /// Dump-only comma-separated pass omissions, for residual attribution.
    layers: Omissions,
    /// Write the actual rendered counter quads, grouped into plates.
    counters_json: Option<PathBuf>,
    /// Dump-only: print this province's hover text in the dump's map mode, its
    /// control codepoints escaped, and its refusals. Also supplies the local picked
    /// province for strategic-name admission; no pointer means the native whole layer.
    province_hover: Option<u32>,
    /// Dump-only: open the target details window of the raid offer bubble at this province, as
    /// its click opens it.
    raid_details: Option<u32>,
    /// Dump-only: draw the find view with this query typed.
    find: Option<String>,
    /// Dump-only: print the hover text of the intel entry under this interface point.
    intel_hover: Vec<[f32; 2]>,
    /// `--ui-scale`: device pixels per interface pixel of the screen-space map
    /// interface. A dump renders at the game's frame size, where the verified
    /// comparison pipeline uses 1.0, so that is the default; the desktop app's
    /// maps take its setting ([`viewer_foundation::map::ui_scale`]).
    ui_scale: Option<f32>,
    /// `--counter-colors` and `--nato-counters`: the game's counter options
    /// ([`viewer_counters::map::counters::CounterSettings`]) in place of `settings.txt`'s.
    counter_colors: Option<viewer_counters::map::counters::CounterColorMode>,
    nato_counters: Option<bool>,
    /// `--diagnostics`: show the map's technical stats and data limitations
    /// for this run without changing the desktop app's saved choice.
    diagnostics: bool,
    // borders
    /// `--dump --demilitarized <ids>`: state ids to treat as demilitarised **in addition to** the
    /// hour's recorded `StateFlags`, so an hour whose flags are short can still be scored against
    /// a reference pair. Verification aid, `--dump` only; the window has no equivalent and draws
    /// the recording.
    demilitarized: BTreeSet<u32>,
    /// `--dump --debug-borders`: the game's own console toggle, which removes the gradient
    /// political fill and the border strips and leaves bare terrain. Pairing a frame with and
    /// without it permits direct fill comparison; `viewer_world::map::gradient` owns the fill law.
    debug_borders: bool,
}

impl Options {
    /// The counter options: the command line's, else `game`, the user's `settings.txt`.
    pub(crate) fn counter_settings(
        &self,
        game: &installed_content_settings::settings::MapOptions,
    ) -> viewer_counters::map::counters::CounterSettings {
        viewer_counters::map::counters::CounterSettings {
            color_mode: self.counter_colors.unwrap_or(
                viewer_counters::map::counters::CounterColorMode::of(game.country_counter_colours),
            ),
            nato: self.nato_counters.unwrap_or(game.nato_counters),
        }
    }

    fn parse(args: impl Iterator<Item = OsString>) -> io::Result<Self> {
        Self::parse_profiled(args, viewer_foundation::profile_writer::profiling())
    }

    fn parse_profiled(args: impl Iterator<Item = OsString>, profiling: bool) -> io::Result<Self> {
        // Dumps default to 2048x1152; `--upscale 1.25` produces a 2560x1440 image
        // for comparison with screenshots at that size.
        let mut options = Self {
            size: [2048, 1152],
            // Apply post-processing unless `--postfx off` disables the chain.
            postfx: true,
            no_water: false,
            no_shadows: false,
            no_trees: false,
            ..Default::default()
        };
        let mut args = args.peekable();
        // `--walk`, `--walk-keys` and `--walk-flying` in any order, joined after the loop.
        let (mut walk, mut walk_keys, mut walk_flying) = (None, None, false);
        while let Some(arg) = args.next() {
            match arg.to_str() {
                Some("--combat" | "--combat-at" | "--air-region" | "--state-view" | "--naval-region") => {
                    if options.combat.is_some() {
                        return Err(invalid("choose exactly one --combat or --combat-at"));
                    }
                    let value = args
                        .next()
                        .and_then(|s| s.into_string().ok())
                        .ok_or_else(|| invalid("combat selection requires an integer"))?;
                    options.combat = Some(if arg == "--state-view" {
                        combat::Selector::State(value.parse().map_err(|_| invalid("province must be a u32"))?)
                    } else if arg == "--naval-region" {
                        combat::Selector::NavalRegion(value.parse().map_err(|_| invalid("naval region must be a u32"))?)
                    } else if arg == "--air-region" {
                        combat::Selector::AirRegion(
                            value
                                .parse()
                                .map_err(|_| invalid("air region must be a u32"))?,
                        )
                    } else if arg == "--combat-at" {
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
                Some("--air-details") => options.air_details = true,
                Some("--air-range") => {
                    options.air_range =
                        match args.next().as_deref().and_then(|value| value.to_str()) {
                            Some("days") => 0,
                            Some("months") => 1,
                            Some("years") => 2,
                            _ => {
                                return Err(invalid(
                                    "air history range must be days, months or years",
                                ));
                            }
                        };
                }
                Some("--air-channels") => {
                    let value = args
                        .next()
                        .and_then(|value| value.into_string().ok())
                        .ok_or_else(|| invalid("air channels require a bitmask"))?;
                    options.air_channels = value
                        .strip_prefix("0x")
                        .map(|value| u32::from_str_radix(value, 16))
                        .unwrap_or_else(|| value.parse())
                        .map_err(|_| invalid("air channels require a u32 bitmask"))?;
                    if options.air_channels >> 24 != 0 {
                        return Err(invalid("air channels have 24 native series"));
                    }
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
                Some("--intel-hover") => {
                    let value = args.next().ok_or_else(|| invalid(USAGE))?;
                    let value = value.to_string_lossy();
                    let point: Vec<f32> = value
                        .split(',')
                        .map(|part| part.trim().parse::<f32>())
                        .collect::<Result<_, _>>()
                        .map_err(|_| invalid("--intel-hover needs an interface point x,y"))?;
                    let &[x, y] = point.as_slice() else {
                        return Err(invalid("--intel-hover needs an interface point x,y"));
                    };
                    options.intel_hover.push([x, y]);
                }
                Some("--raid-details") => {
                    let value = args.next().ok_or_else(|| invalid(USAGE))?;
                    options.raid_details = Some(
                        value
                            .to_string_lossy()
                            .parse()
                            .map_err(|_| invalid("--raid-details needs a province id"))?,
                    );
                }
                Some("--find") => {
                    let value = args.next().ok_or_else(|| invalid(USAGE))?;
                    options.find = Some(
                        value
                            .into_string()
                            .map_err(|_| invalid("--find needs a query in Unicode"))?,
                    );
                }
                Some("--province-hover") => {
                    let value = args.next().ok_or_else(|| invalid(USAGE))?;
                    options.province_hover = Some(
                        value
                            .to_string_lossy()
                            .parse()
                            .map_err(|_| invalid("--province-hover needs a province id"))?,
                    );
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
                    options.layers = Omissions::parse(&value)?;
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
                    options.demilitarized = viewer_world::map::demilitarized::parse_states(&value)?;
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
                        .filter(|scale| valid(*scale))
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
                Some("--walk-flying") => walk_flying = true,
                Some(flag @ ("--walk" | "--walk-keys")) => {
                    let flag = flag.to_owned();
                    let value = args
                        .next()
                        .and_then(|value| value.into_string().ok())
                        .ok_or_else(|| invalid(USAGE))?;
                    if flag == "--walk" {
                        walk = Some(value);
                    } else {
                        walk_keys = Some(value);
                    }
                }
                Some("--diagnostics") => options.diagnostics = true,
                Some("--out") => {
                    options.out = Some(PathBuf::from(args.next().ok_or_else(|| invalid(USAGE))?))
                }
                Some("--game-dir") => {
                    options.game_dir =
                        Some(PathBuf::from(args.next().ok_or_else(|| invalid(USAGE))?))
                }
                Some("--visual-mod") => {
                    let id = args
                        .next()
                        .and_then(|value| value.into_string().ok())
                        .filter(|value| !value.is_empty())
                        .ok_or_else(|| invalid("--visual-mod needs a mod id, such as mod/name.mod"))?;
                    options.visual_mods.push(installed_content_core::content::visual::Choice {
                        name: id.clone(),
                        id,
                    });
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
                    | "--profile-pointer"
                    | "--free-camera"
                    | "--building-camera"
                    | "--perspective"
                    | "--fog"
                    | "--map-mode"
                    | "--allied-plans"
                    | "--day-night"
                    | "--free-overlays"
                    | "--counter-colors"
                    | "--nato-counters"
                    | "--animation-time"
                    | "--animation-step"
                    | "--fly"
                    | "--fly-view"
                    | "--fly-steer"
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
                        "--camera" | "--free-camera" => {
                            if options.camera.is_some() {
                                return Err(invalid("give one of --camera and --free-camera"));
                            }
                            options.camera = Some(if arg == "--camera" {
                                parse_placement(&value)?
                            } else {
                                parse_free_camera(&value)?
                            });
                        }
                        "--profile-pointer" => {
                            options.profile_pointer = Some(parse_profile_pointer(&value)?)
                        }
                        "--building-camera" => {
                            options.building_cameras.push(parse_placement(&value)?)
                        }
                        "--fly" => options.fly = Some(parse_fly(&value)?),
                        "--fly-view" => {
                            options.fly_view = Some(match value.trim() {
                                "chase" => controls::flight::View::Chase,
                                "nose" => controls::flight::View::Nose,
                                _ => return Err(invalid("--fly-view takes chase or nose")),
                            })
                        }
                        "--fly-steer" => options.fly_steer = Some(parse_fly_steer(&value)?),
                        // counters
                        "--perspective" => options.perspective.country = Country::parse(&value)?,
                        "--fog" => options.perspective.fog = Choice::parse_fog(&value)?,
                        "--map-mode" => {
                            options.map_mode =
                                viewer_world::map::map_mode::MapMode::parse(&value)
                                    .ok_or_else(|| invalid("--map-mode takes a map mode token"))?
                        }
                        "--allied-plans" | "--day-night" | "--free-overlays" => {
                            let enabled = match value.trim() {
                                value if value.eq_ignore_ascii_case("on") => true,
                                value if value.eq_ignore_ascii_case("off") => false,
                                _ => {
                                    return Err(invalid(format!(
                                        "{} takes on or off",
                                        arg.to_string_lossy()
                                    )));
                                }
                            };
                            if arg == "--allied-plans" {
                                options.allied_plans = Some(enabled);
                            } else if arg == "--free-overlays" {
                                options.free_overlays = Some(enabled);
                            } else {
                                options.day_night = Some(enabled);
                            }
                        }
                        "--counter-colors" => {
                            options.counter_colors = Some(match value.trim() {
                                value if value.eq_ignore_ascii_case("allegiance") => {
                                    viewer_counters::map::counters::CounterColorMode::Allegiance
                                }
                                value if value.eq_ignore_ascii_case("country") => {
                                    viewer_counters::map::counters::CounterColorMode::Country
                                }
                                _ => {
                                    return Err(invalid(
                                        "--counter-colors takes allegiance or country",
                                    ));
                                }
                            })
                        }
                        "--nato-counters" => {
                            options.nato_counters = Some(match value.trim() {
                                value if value.eq_ignore_ascii_case("on") => true,
                                value if value.eq_ignore_ascii_case("off") => false,
                                _ => return Err(invalid("--nato-counters takes on or off")),
                            })
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
        match walk {
            Some(place) => {
                options.walk = Some(
                    controls::walker::Start::parse(&place, walk_keys.as_deref(), walk_flying)
                        .map_err(invalid)?,
                );
            }
            None if walk_keys.is_some() || walk_flying => {
                return Err(invalid(
                    "--walk-keys and --walk-flying hold keys for a --walk",
                ));
            }
            None => {}
        }
        if options.walk.is_some() && !options.window_dump {
            return Err(invalid(
                "--walk draws a placed walk through the window and requires --window-dump; the                  desktop app's maps walk from the Camera control",
            ));
        }
        if options.walk.is_some() && options.camera.is_some() {
            return Err(invalid(
                "--walk places its own camera behind the figure; give it or a camera",
            ));
        }
        if options.size.iter().any(|size| !(1..=16384).contains(size)) {
            return Err(invalid("image dimensions must be 1 through 16384"));
        }
        if options.layers != Omissions::default() && !options.headless_dump {
            return Err(invalid("--layers omits named passes and requires --dump"));
        }
        if !options.building_cameras.is_empty() && !options.headless_dump {
            return Err(invalid("--building-camera requires --dump"));
        }
        if options.profile_pointer.is_some()
            && (!profiling || options.headless_dump || options.compare.is_some())
        {
            return Err(invalid(
                "--profile-pointer requires an interactive window with BATTLE_PLAN_VIEWER_PROFILE",
            ));
        }
        if options.province_hover.is_some() && !options.headless_dump {
            return Err(invalid("--province-hover requires --dump"));
        }
        if options.raid_details.is_some() && !options.headless_dump {
            return Err(invalid("--raid-details requires --dump"));
        }
        if options.find.is_some() && !options.headless_dump {
            return Err(invalid("--find requires --dump"));
        }
        if !options.intel_hover.is_empty() && !options.headless_dump {
            return Err(invalid("--intel-hover requires --dump"));
        }
        if options.counters_json.is_some() && !options.headless_dump {
            return Err(invalid("--counters-json requires --dump"));
        }
        if options.ui_scale.is_some() && !(options.headless_dump || options.window_dump) {
            return Err(invalid(
                "--ui-scale sets a dump's map interface scale and requires --dump or --window-dump; \
                 the desktop app sends its maps their UI scale",
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
        if (options.air_details || options.air_range != 0 || options.air_channels != 0)
            && !matches!(options.combat, Some(combat::Selector::AirRegion(_)))
        {
            return Err(invalid("air history options require --air-region"));
        }
        if !options.visual_mods.is_empty() && !options.headless_dump && !options.window_dump {
            return Err(invalid("--visual-mod requires --dump or --window-dump"));
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
        // capture: `--window-dump` needs a destination because it closes the
        // window as soon as it has written one.
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
        // The flying mode's evidence comes from the window's own path.
        if options.fly.is_some() && !options.window_dump {
            return Err(invalid(
                "--fly flies the window's flying mode and requires --window-dump",
            ));
        }
        if options.fly.is_some() && options.walk.is_some() {
            return Err(invalid("give one of --fly and --walk"));
        }
        if options.fly.is_some() && options.camera.is_some() {
            return Err(invalid(
                "--fly places the flying camera; give it or --camera or --free-camera",
            ));
        }
        match (&mut options.fly, options.fly_view, options.fly_steer) {
            (Some(fly), view, steer) => {
                fly.view = view.unwrap_or(fly.view);
                fly.steer = steer.unwrap_or(fly.steer);
            }
            (None, None, None) => {}
            (None, _, _) => return Err(invalid("--fly-view and --fly-steer require --fly")),
        }
        if (options.presentation_time != 0.0 || options.map_time != 0.0 || options.settle != 0.0)
            && !options.headless_dump
        {
            return Err(invalid(
                "--presentation-time, --map-time and --settle set the clocks a dump otherwise renders \
                 at zero; a window runs its own, and --window-dump samples it with --animation-time",
            ));
        }
        if options.out.is_some() && !(options.headless_dump || options.window_dump) {
            return Err(invalid(
                "--out names the frame --dump or --window-dump writes",
            ));
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
        let Some(rects) = compare::game_ui_exclusions(game.width(), game.height()) else {
            return Err(invalid(format!(
                "--exclude-game-ui was measured on {}x{} and {}x{} screenshots; these images are {}x{}",
                compare::GAME_UI_WIDTH,
                compare::GAME_UI_HEIGHT,
                compare::GAME_UI_WIDE_WIDTH,
                compare::GAME_UI_HEIGHT,
                game.width(),
                game.height()
            )));
        };
        for rect in rects {
            mask.exclude_rect(*rect);
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
        println!("{USAGE}\n{}", corpus::USAGE);
        return Ok(());
    }
    if let Some(result) = corpus::dispatch(&args) {
        return Ok(result?);
    }
    if let Some(result) = playback_cost::dispatch(&args) {
        return Ok(result?);
    }
    let options = Options::parse(args.into_iter())?;
    if let Some(paths) = &options.compare {
        run_compare(&options, paths)?;
        return Ok(());
    }
    if !options.headless_dump && !options.window_dump {
        return Err(invalid(
            "The battle-plan app (replay-desktop.exe) shows maps itself. The viewer runs --dump, --window-dump, --compare, --mod-corpus and --mod-corpus-diff; see --help.",
        )
        .into());
    }
    // Tooling reads the desktop app's saved folders as defaults for its own folder options.
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
            return Err(invalid(format!("Content preferences: {error}")).into());
        }
        // The GUI textures load with the document, for the device the dump renders on.
        render::probe_dump_device()?;
        let graphics =
            engine_executable::executable_tables::or_installed(&config.game_dir, || {
                dashboard::map_settings::read_game(&config.user_dir)
            })?
            .graphics;
        let mut document = document::Document::load(
            options.replay.as_ref().unwrap(),
            &config.resolve_options()?,
            options.allow_unverified,
            &document::InitialHour {
                branch: options.branch,
                hour: options.hour,
                hour_fraction: options.hour_fraction,
            },
            (!options.visual_mods.is_empty()).then_some(options.visual_mods.as_slice()),
            graphics.texture_quality,
        )?;
        // The dump works on this recording alone, with its own executable tables.
        engine_executable::executable_tables::enter(document.content.tables);
        // Each frame settles its own flag and portrait decodes, in request
        // order, so a dump lays out the same cells on every run.
        document.icons.lazy.set_background(false);
        // Every product reads rows at the head, and what a layer makes is the layer's.
        let dump_head = Some(dump_head::open(&document)?);
        if let Some(head) = &dump_head {
            let (branch, hour) = (document.playback.branch(), document.playback.hour());
            drop(document.playback.present(branch, hour, playback::Current::Frame(head.frame())));
            for line in head.lines() {
                eprintln!("{line}");
            }
        }
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
        if let Resolution::Absent(tag) = document.vision(&options.perspective)?.resolution() {
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
        let hour = document.hour()?;
        if !document.map_modes.knows(&options.map_mode) {
            return Err(Box::new(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!(
                    "--map-mode {}: the recording's content declares no such map mode",
                    options.map_mode.token()
                ),
            )));
        }
        // A dump is a fresh open: a scripted mode's colour borders start empty.
        let mode = hour.map_mode_fill(&options.map_mode, &options.perspective, None, None)?;
        for line in &mode.warnings {
            eprintln!("Warning: {line}");
        }
        println!("Map mode: {}", options.map_mode.token());
        let layers = hour
            .political_product(&options.map_mode, &options.perspective, None, None)?
            .layers;
        let layers = match &dump_head {
            Some(head) => head.political(&options.map_mode, &options.perspective, layers)?,
            None => layers,
        };
        let mut omitted = options.layers;
        omitted.borders |= options.debug_borders;
        omitted.gradient |= options.debug_borders;
        omitted.trees |= options.no_trees;
        println!("Omitted dump layers: {omitted:?}");
        // weather
        let snow_mud = std::sync::Arc::new(document.snow_mud()?);
        // close zoom
        let (map_building_field, building_summary) =
            document.map_building_field(&options.perspective)?;
        let aircraft_plan = document.aircraft_plan(&options.perspective)?;
        for line in aircraft_plan.report(&document.aircraft) {
            println!("{line}");
        }
        let mut train_plan = document.train_plan(&options.perspective)?;
        println!(
            "Supply trains (recorded simulation; unrecorded presentation phase): {} routes, {} mesh objects at time 0, seed 0x54524149",
            train_plan.route_count(),
            train_plan.field(&document.trains, 0.0).instances.len()
        );
        for line in train_plan.diagnostics(&document.trains) {
            println!("{line}");
        }
        // orders: the recorded player's battle plan
        let show_allied = options
            .allied_plans
            .unwrap_or(document.game_options.allied_plans);
        let order_field = if show_allied {
            document.order_field_with_allied(&options.perspective, true)?
        } else {
            document.order_field(&options.perspective)?
        };
        // Printed before the dump's fallible steps so a failed dump keeps them.
        let mut warnings = document.warnings.clone();
        warnings.extend(layers.warnings().iter().cloned());
        document.uncovered_reads.append_to(&mut warnings);
        for warning in &warnings {
            eprintln!("Warning: {warning}");
        }
        let map_size = MapSize::of(&document.map);
        let defines = document.map_defines;
        let place = |placement: Placement| {
            let [min, max] = defines.heights;
            if placement.pose == Pose::Map && !(min..=max).contains(&placement.height) {
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
        // The convoys sail at the dump's presentation clock where the mode and camera show them.
        let trade_routes =
            viewer_world::map::map_mode::visibility::of(&options.map_mode).trade_routes;
        let (unit_model_field, unit_report) = document.unit_model_field_sailing(
            &options.perspective,
            options.presentation_time,
            &|defines| {
                viewer_world::map::trade_routes::convoys_shown(
                    camera.height,
                    defines,
                    true,
                    trade_routes,
                )
            },
        )?;
        // fog of war: the hour's `IntelMap` for the viewing country and the fog
        // switch. The camera decides `FOWFadeFactor`, so the report needs it.
        let intel_map = document.intel_map(&options.perspective)?;
        let visibility = viewer_world::map::map_mode::visibility::of(&options.map_mode);
        if visibility.fog_of_war {
            println!(
                "{}",
                viewer_foundation::map::textures::intel_line(
                    &intel_map,
                    &camera.defines,
                    camera.height,
                    options.perspective.fog
                )
            );
        } else {
            println!(
                "Fog of war: none drawn; the {} map mode draws no fog of war",
                options.map_mode.token()
            );
        }
        let unverified = document
            .warnings
            .iter()
            .filter(|warning| warning.starts_with("UNVERIFIED CONTENT"))
            .cloned()
            .collect::<Vec<_>>()
            .join("; ");
        let hour = document.playback.hour();
        // names: a headless render has no frame history, so the fade jumps
        // straight to its settled value.
        let fade = LayerFade::instant(camera);
        let placements = document.place_names(&options.map_mode)?;
        for warning in &placements.warnings {
            eprintln!("Warning: {warning}");
        }
        // counters: bases, labels, counters and combats in priority order.
        let ui_scale = options.ui_scale.unwrap_or(1.0);
        let mut dump_overlay = document.dump_overlay(
            camera,
            options.size,
            ui_scale,
            &options.perspective,
            omitted,
            options.counter_settings(&document.game_options),
            &options.map_mode,
        )?;
        if let Some(province) = options.raid_details {
            match dump_overlay.open_raid_details(province) {
                Ok(targets) => println!("Raid details: province {province}, {targets} targets"),
                Err(why) => eprintln!("Warning: raid details: {why}"),
            }
        }
        let mut combat_warnings = Vec::new();
        if let Some(selector) = options.combat {
            let mut window = combat::Window::default();
            let selection = selector.resolve(document.frame()?)?;
            window.open(selection);
            window.configure_air(options.air_details, options.air_range, options.air_channels);
            window.prepare_air(&document.playback.replay_handle(), || {});
            let mut gui_elapsed = 0.0_f64;
            // Draw until every flag and portrait the window asked for is decoded.
            let drawn = loop {
                let air_generation = window.air_generation();
                let before = document.icons.lazy.version();
                let frame = match document.playback.current() {
                    crate::playback::Current::Frame(frame) => Ok(Arc::clone(frame)),
                    _ => Err(document.frame().err().map_or_else(String::new, |error| error.to_string())),
                };
                let prepared = window.prepare(
                    &document.combat,
                    document.playback.branch(),
                    document.playback.hour(),
                    frame,
                    &options.perspective,
                    None,
                    options.counter_settings(&document.game_options).nato,
                )?;
                let drawn = window.draw(
                    &document.combat,
                    prepared,
                    extent(options.size, ui_scale),
                    false,
                    options.presentation_time + gui_elapsed as f32,
                    true,
                    None,
                )?;
                document.icons.lazy.settle();
                if window.settle_air_after(air_generation) {
                    continue;
                }
                if gui_elapsed < f64::from(options.settle) {
                    gui_elapsed = (gui_elapsed + 1.0 / 60.0).min(f64::from(options.settle));
                    continue;
                }
                if document.icons.lazy.version() == before {
                    break drawn;
                }
            };
            // The window's warnings say why an undecided flag showed nothing, so they print
            // whether or not it drew.
            combat_warnings = window.warnings(&document.icons.combat.warnings);
            for warning in &combat_warnings {
                eprintln!("Warning: {warning}");
            }
            if let Some(drawn) = drawn {
                println!("Combat window: {selection:?}, rect {:?}", drawn.rect);
                dump_overlay.extra.quads.extend(drawn.batch.quads);
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
        let nuclear_impacts = match &dump_head {
            Some(head) => head.impacts()?,
            None => document.nuclear_impacts()?,
        };
        let map_text = document
            .hour()?
            .map_text(&*document.vision(&options.perspective)?);
        // A free camera draws the map's interface, country names included, only when asked to.
        let interface = camera.free().is_none() || options.free_overlays.unwrap_or(false);
        if interface {
            match app::picker::dump(&document, &options) {
                Ok(picker) => {
                    for (reason, count) in &picker.drawn.refused {
                        eprintln!("Warning: map-mode picker ({count}): {reason}");
                    }
                    dump_overlay.extra.quads.extend(picker.drawn.batch.quads);
                }
                Err(error) => eprintln!("Warning: map-mode picker: {error}"),
            }
        }
        if let Some(query) = &options.find {
            match app::find_view::dump(&document, &options, query) {
                Ok((drawn, lines)) => {
                    println!("Find view: {query:?}");
                    for line in lines {
                        println!("  find view: {line}");
                    }
                    dump_overlay.extra.quads.extend(drawn.batch.quads);
                }
                Err(error) => eprintln!("Warning: find view: {error}"),
            }
        }
        let report = render::dump_png(
            &document.assets,
            // political fill + borders
            render::MapLayers {
                content: &document.content,
                graphics,
                at: (document.frame()?.branch, document.frame()?.hour),
                map_text: &map_text,
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
                nuclear_impacts: &nuclear_impacts,
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
                weather_field: document.hour()?.weather_field()?,
                // trees
                tree_assets: &document.tree_assets,
                tree_geometry: &document.tree_geometry,
                tree_field: &document.tree_field,
                visibility,
            },
            // names
            &document.icons,
            options.settle,
            &mut dump_overlay,
            Request {
                size: options.size,
                camera,
                day_night: DayNight::at_hour(hour)
                    .with_cycle(options.day_night.unwrap_or(document.game_options.day_night)),
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
                    strategic_names: if document.content.game_strategic_names {
                        StrategicNames::All
                    } else {
                        StrategicNames::Hover(options.province_hover)
                    },
                    sprites: None,
                    ui_scale,
                    interface,
                    country_names: interface,
                    ground: camera
                        .free()
                        .map(|_| viewer_foundation::map::free_icons::Ground {
                            field: document.assets.height_field(),
                            sight: document.assets.terrain_sight(),
                        }),
                },
                // trees
            },
            output,
            (!unverified.is_empty()).then_some(unverified.as_str()),
        )?;
        for warning in &dump_overlay.settlement_warnings {
            eprintln!("Warning: {warning}");
        }
        for warning in dump_overlay.prepared().intel_report().warnings() {
            eprintln!("Warning: {warning}");
        }
        for warning in dump_overlay
            .prepared()
            .mode_icon_warnings()
            .into_iter()
            .chain(dump_overlay.prepared().stripe_warnings())
            .chain(
                dump_overlay
                    .prepared()
                    .naval_selection
                    .refused
                    .iter()
                    .map(|(why, count)| format!("Navy selection: {count} withheld: {why}")),
            )
        {
            eprintln!("Warning: {warning}");
        }
        for point in &options.intel_hover {
            let point = *point;
            let escaped = |text: &str| {
                text.chars()
                    .map(|c| match c {
                        '\u{11}' => "\\u{11}".to_owned(),
                        '\u{12}' => "\\u{12}".to_owned(),
                        c => c.to_string(),
                    })
                    .collect::<String>()
            };
            let [x, y] = point;
            match dump_overlay.hits.intel_at(|hit| {
                // A dump lays its overlay out for the camera it draws, so no window is offset.
                crate::app::map_hits::local(
                    None,
                    dump_overlay.looks(),
                    viewer_foundation::map::free_icons::IconFamily::Intel,
                    hit.layer,
                    point,
                )
            })
            {
                None => println!("Intel hover at {x},{y}: no intel entry takes the point"),
                Some(hit) => {
                    println!("Intel hover at {x},{y}:\n{}", escaped(&hit.tooltip.title));
                    if !hit.tooltip.body.is_empty() {
                        println!(
                            "Intel hover at {x},{y} body:\n{}",
                            escaped(&hit.tooltip.body)
                        );
                    }
                    for (reason, count) in &hit.tooltip.refused {
                        println!("Intel hover at {x},{y} refused ({count}): {reason}");
                    }
                }
            }
        }
        let naval_hover_sources = (options.province_hover.is_some()
            && matches!(
                options.map_mode,
                viewer_world::map::map_mode::MapMode::Stock(
                    viewer_world::map::map_mode::Stock::StrategicNavy
                        | viewer_world::map::map_mode::Stock::Default
                )
            ))
        .then(|| dump_overlay.naval_dominance_sources());
        let (label_batch, label_report, counter_report, raid_report) = (
            dump_overlay.batch,
            dump_overlay.labels,
            dump_overlay.counters,
            dump_overlay.raids,
        );
        for warning in label_report.warnings() {
            eprintln!("Warning: {warning}");
        }
        let frame = document.frame()?;
        let batch = viewer_counter_products::map::counter_tooltips::Batch::new(
            &document.counter_content,
            document.icons.raids.movement_reads(),
            frame,
            &counter_report.perspective,
            viewer_counter_products::map::counter_tooltips::Style {
                nato: options.counter_settings(&document.game_options).nato,
                diagnostics: options.diagnostics,
            },
        );
        let tooltips = counter_report
            .land_hovers
            .iter()
            .map(|hover| batch.build(hover))
            .chain(
                counter_report
                    .task_force_hovers
                    .iter()
                    .filter(|hover| hover.supported)
                    .map(|hover| batch.build_task_force(hover)),
            );
        let tooltips = tooltips.chain(
            counter_report
                .task_force_port_hovers
                .iter()
                .filter(|hover| hover.supported && hover.has_tooltip())
                .map(|hover| batch.build_task_force_port(hover)),
        );
        let tooltip_refused =
            viewer_counter_products::map::counter_tooltips::count_refusals(tooltips);
        for warning in viewer_foundation::diagnostics_panel::tooltip_warning_lines(&tooltip_refused)
        {
            eprintln!("Warning: {warning}");
        }
        if let (Some(province), crate::playback::Current::Frame(frame)) =
            (options.province_hover, document.playback.current())
        {
            let counters = &document.content.counters;
            let mut mode = crate::app::picker::mode_hover(
                &options.map_mode,
                crate::app::picker::picks_region(
                    counters.kinds.get(&province).copied(),
                    counters.region(province),
                ),
                document.map_modes.defines.opinion_thresholds(),
                || {
                    document
                        .map_modes
                        .infrastructure
                        .hover_level(frame, province)
                },
            );
            if let (
                viewer_counter_products::map::province_tooltips::ModeHover::StrategicNavy(sources)
                | viewer_counter_products::map::province_tooltips::ModeHover::SeaRegion(sources),
                Some(prepared_sources),
            ) = (&mut mode, naval_hover_sources)
            {
                *sources = prepared_sources;
            }
            let text = if let viewer_counter_products::map::province_tooltips::ModeHover::Terrain {
                ..
            } = mode
            {
                Arc::new(viewer_combat::terrain_mode::hover(
                    &document.combat,
                    frame,
                    province,
                    &counter_report.perspective,
                ))
            } else {
                // The hover's air statistics wait for the roster history worker, as the
                // interactive hover does through its wake: ask again until nothing is pending.
                let mut cache = viewer_counter_products::map::province_tooltips::Cache::default();
                let selected = Arc::new(Vec::new());
                let rosters = document.playback.roster_sampler();
                loop {
                    let text = cache.request(
                        &document.counter_content,
                        document.icons.raids.movement_reads(),
                        frame,
                        province,
                        &counter_report.perspective,
                        options.counter_settings(&document.game_options).nato,
                        &selected,
                        &rosters,
                        &mode,
                        &[],
                        || {},
                    );
                    if !cache.pending() {
                        break text;
                    }
                    cache.wait();
                }
            };
            let escaped = |text: &str| {
                text.chars()
                    .map(|c| match c {
                        '\u{11}' => "\\u{11}".to_owned(),
                        '\u{12}' => "\\u{12}".to_owned(),
                        c => c.to_string(),
                    })
                    .collect::<String>()
            };
            println!("Province {province} hover:\n{}", escaped(&text.title));
            if !text.body.is_empty() {
                println!("Province {province} hover body:\n{}", escaped(&text.body));
            }
            for (reason, count) in &text.refused {
                println!("Province {province} hover refused ({count}): {reason}");
            }
        }
        if let Some(path) = &options.counters_json {
            let mut census =
                document
                    .icons
                    .counters
                    .census(&label_batch, &document.icons.fonts, ui_scale);
            // The same counted marker and open-window limitations the dump prints.
            census["combat_diagnostics"] = serde_json::json!(
                counter_report
                    .combat_gap_lines()
                    .chain(combat_warnings)
                    .collect::<Vec<_>>()
            );
            census["tooltip_diagnostics"] = serde_json::json!(tooltip_refused);
            std::fs::write(path, serde_json::to_vec_pretty(&census)?)?;
        }
        for warning in &report.pass_warnings {
            eprintln!("{warning}");
        }
        for (reason, count) in &report.shader_refusals {
            eprintln!("GUI shader: {count} refused draws: {reason}");
        }

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
        for requirement in &report.device_requirements {
            eprintln!("Warning: {requirement}");
        }
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
            "Terrain blocks: {} blocks holding {} quads; render {:.1} ms (cpu {:.1}, gpu {:.1})",
            report.blocks,
            report.quads,
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
            shadow_fade_factor(&camera.defines, camera.height),
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
            viewer_world::map::trees::tree_fade(&camera.defines, camera.height),
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
        for warning in &plan.trade_routes {
            println!("  {warning}");
        }
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
            viewer_world::map::models::draws_city_sprawl(&camera),
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
            viewer_world::map::railways::RailFade::default().update(&camera, 0.0),
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
            "Unit models: {} stacks by ladder level ({levels}); {} took the sub-unit's sprite key, {} ride a transport, {} stand along a route, {} have no recorded template, {} have no known country operands, {} have no anchor, {} are hidden from this perspective, {} carry attached entities and {} of those are an attachment rig with no geometry of their own; {} drawn at height {:.3} (land cutoff {}, ship cutoff {}); {} at-sea hulls prepared, {} formations undecided by the mission skip",
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
            unit_report.undecided_mission_formations,
        );
        if let Some(reason) = &unit_report.disabled {
            println!("  the unit-model layer is off: {reason}");
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
        let mut placed: Vec<&viewer_foundation::map::name_draw::WorldName> =
            placements.country.iter().collect();
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
            "Counters: {} land ({} between provinces), {} naval ({} task forces not drawn, {} standing for an unplaced route); {} air bases, {} rocket sites, {} gun emplacements ({} hidden by fog of war), {} radars, {} strategic locations, {} naval bases, {} naval HQs, {} facilities ({} unresolved sprites; undecided: {} bases, {} HQs, {} facilities); {} land combats, {} naval combats ({} undecided), {} naval result icons ({} undecided); {} stacks with no anchor",
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
            counter_report.rocket_sites,
            counter_report.gun_emplacements,
            counter_report.hidden_air_bases,
            counter_report.radars,
            counter_report.strategic_locations,
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
        // counters: naval mission icons
        println!(
            "Naval mission icons: {} entries drawn ({} children hidden until their inputs are recorded), {} with their button's authored sprite for an undefined kind sprite, {} icons without a region position, {} entries or parts not drawn",
            counter_report.naval_mission_entries,
            counter_report
                .undecided_naval_mission_children
                .values()
                .sum::<usize>(),
            counter_report.authored_naval_mission_buttons,
            counter_report.unanchored_naval_missions,
            counter_report.naval_mission_gaps.values().sum::<usize>(),
        );
        // The refused tokens, the task forces and base icons not drawn, the undecided naval
        // icons and the naval mission gaps, as the window lists them.
        for warning in counter_report.warnings() {
            println!("{warning}");
        }
        // The recorded naval engaged bytes against the flag derived for the recording's
        // player, over every recorded hour: the count offline runs read.
        let started = Instant::now();
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
            "Faction-theatre pins: {} drawn; {} theatres refused for a shared first region, {} without a region position, {} unresolved hidden-for links{}",
            counter_report.faction_pings,
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
        // Lazy reads during the dump can add uncovered files; print the refreshed line.
        let mut content = Vec::new();
        document.uncovered_reads.append_to(&mut content);
        for warning in content.iter().filter(|line| !warnings.contains(line)) {
            eprintln!("Warning: {warning}");
        }
        return Ok(());
    }
    host::dump_window(config, options, config_error)?;
    Ok(())
}

/// A background profiling window stays shown, behind the others, so it keeps painting.
pub(crate) fn background_window(value: Option<&std::ffi::OsStr>) -> bool {
    value.is_some_and(|value| value == "1")
}


/// The `viewer` binary: the development harness and tooling.
pub fn cli() -> std::process::ExitCode {
    engine_heap::return_freed_memory_promptly();
    viewer_foundation::parallel::name_shared_pool();
    viewer_foundation::frame_writer::initialize(Instant::now());
    let result = run();
    viewer_foundation::frame_writer::flush();
    // Profiling lines are written by a thread of their own; every mode waits
    // for them, a headless dump included.
    profile_flush();
    perf_log::finish();
    match result {
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
    fn profile_pointer_requires_a_profiled_window_and_finite_pixels() {
        let args = [
            "--window-dump",
            "campaign.battleplan",
            "--out",
            "frame.png",
            "--profile-pointer",
            "1600,700",
        ];
        let parse = |args: &[&str], enabled| {
            Options::parse_profiled(args.iter().map(OsString::from), enabled)
        };
        assert_eq!(
            parse(&args, true).unwrap().profile_pointer,
            Some([1600.0, 700.0])
        );
        assert!(parse(&args, false).is_err());
        assert!(
            parse(
                &[
                    "--dump",
                    "campaign.battleplan",
                    "--out",
                    "frame.png",
                    "--profile-pointer",
                    "1600,700"
                ],
                true
            )
            .is_err()
        );
        assert!(
            parse(
                &["campaign.battleplan", "--profile-pointer", "1600,700"],
                true
            )
            .is_ok()
        );
        assert!(
            parse(
                &["campaign.battleplan", "--profile-pointer", "1600,700"],
                false,
            )
            .is_err()
        );
        for bad in ["", "1", "1,2,3", "NaN,2", "1,inf", "-1,2", "1,-2"] {
            assert!(parse_profile_pointer(bad).is_err(), "{bad}");
        }
        assert_eq!(parse_profile_pointer("0,0").unwrap(), [0.0, 0.0]);
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
    fn a_free_camera_takes_an_eye_degrees_and_a_lens() {
        let options = parse(&[
            "campaign.battleplan",
            "--free-camera",
            "3000,25,1500,90,-10",
        ])
        .unwrap();
        let placement = options.camera.unwrap();
        assert_eq!(placement.center, [3000.0, 1500.0]);
        assert_eq!(placement.height, 25.0);
        let Pose::Free(pose) = placement.pose else {
            panic!("{placement:?}");
        };
        assert!((pose.yaw - std::f32::consts::FRAC_PI_2).abs() < 1e-6);
        assert!((pose.pitch + 10f32.to_radians()).abs() < 1e-6);
        assert_eq!(pose.roll, 0.0);
        assert_eq!((pose.lens.near, pose.lens.far), (1.0, 20_000.0));
        assert!((pose.lens.fov_y - Lens::MAP.fov_y).abs() < 1e-6);
        let lens = parse(&[
            "campaign.battleplan",
            "--free-camera",
            "1,2,3,4,5,6,60,0.5,900",
        ])
        .unwrap()
        .camera
        .unwrap();
        let Pose::Free(pose) = lens.pose else {
            panic!("{lens:?}");
        };
        assert!((pose.roll - 6f32.to_radians()).abs() < 1e-6);
        assert!((pose.lens.fov_y - 60f32.to_radians()).abs() < 1e-6);
        assert_eq!((pose.lens.near, pose.lens.far), (0.5, 900.0));
        // One camera only, a usable lens and five to nine numbers.
        for bad in [
            &[
                "c.battleplan",
                "--camera",
                "1,2,300",
                "--free-camera",
                "1,2,3,4,5",
            ][..],
            &["c.battleplan", "--free-camera", "1,2,3,4"],
            &["c.battleplan", "--free-camera", "1,2,3,4,5,6,7,8,9,10"],
            &["c.battleplan", "--free-camera", "1,2,3,4,5,0,45,10,5"],
            &["c.battleplan", "--free-camera", "1,2,3,4,5,0,180"],
            &["c.battleplan", "--free-camera", "1,2,3,4,nan"],
        ] {
            assert!(parse(bad).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn perspective_and_fog_are_the_two_view_options() {
        let default = parse(&["campaign.battleplan"]).unwrap();
        assert_eq!(default.perspective, Choice::default());
        assert_eq!(
            default.perspective.fog,
            dashboard::map_display::option_defaults::FOG
        );
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
            Choice {
                country: Country::Tag("SOV".into()),
                fog: false,
            }
        );
        assert!(parse(&["campaign.battleplan", "--perspective", "omniscient"]).is_err());
        assert!(parse(&["campaign.battleplan", "--fog", "maybe"]).is_err());
        assert!(parse(&["campaign.battleplan", "--fog"]).is_err());
    }

    /// The counter options default to `settings.txt`'s and take the command line's overrides.
    #[test]
    fn counter_options_override_the_users_settings() {
        use viewer_counters::map::counters::{CounterColorMode, CounterSettings};
        let from_file = &installed_content_settings::settings::MapOptions {
            country_counter_colours: true,
            nato_counters: true,
            ..installed_content_settings::settings::MapOptions::DEFAULT
        };
        let default = parse(&["campaign.battleplan"]).unwrap();
        assert_eq!(
            default.counter_settings(from_file),
            CounterSettings {
                color_mode: CounterColorMode::Country,
                nato: true,
            }
        );
        let options = parse(&[
            "campaign.battleplan",
            "--counter-colors",
            "allegiance",
            "--nato-counters",
            "off",
        ])
        .unwrap();
        assert_eq!(
            options.counter_settings(from_file),
            CounterSettings {
                color_mode: CounterColorMode::Allegiance,
                nato: false,
            }
        );
        assert!(parse(&["campaign.battleplan", "--counter-colors", "flag"]).is_err());
        assert!(parse(&["campaign.battleplan", "--nato-counters", "yes"]).is_err());
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

    // counters: a frame between two recorded hours
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

        // The desktop app forwards a recording and its options to the map.
        let hosted = parse(&["campaign.battleplan", "--branch", "2"]).unwrap();
        assert_eq!(hosted.replay, Some(PathBuf::from("campaign.battleplan")));
        assert!(hosted.out.is_none() && hosted.compare.is_none());
        assert!(!hosted.headless_dump && !hosted.window_dump);

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
            assert!(Omissions::parse(value).is_err());
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

    #[test]
    fn a_background_window_dump_only_drops_focus_and_z_order() {
        use std::ffi::OsStr;
        for value in [None, Some(""), Some("0"), Some("true"), Some(" 1")] {
            assert!(!background_window(value.map(OsStr::new)), "{value:?}");
        }
        assert!(background_window(Some(OsStr::new("1"))));
    }

    #[test]
    fn a_window_dump_places_a_walk_with_its_held_keys() {
        let dump = |extra: &[&str]| {
            let mut args = vec![
                "--window-dump",
                "campaign.battleplan",
                "--hour",
                "1",
                "--out",
                "walk.png",
            ];
            args.extend_from_slice(extra);
            parse(&args)
        };
        let options = dump(&[
            "--walk-keys",
            "w,Shift,plus",
            "--walk",
            "3000,1200,90,2.5",
            "--walk-flying",
        ])
        .unwrap();
        let start = options.walk.expect("placed");
        assert_eq!(start.at, [3000.0, 1200.0]);
        assert!((start.facing - std::f32::consts::FRAC_PI_2).abs() < 1e-6);
        assert_eq!(start.scale, 2.5);
        assert!(start.flying && start.modifiers.shift && !start.modifiers.ctrl);
        assert_eq!(start.held, [eframe::egui::Key::W, eframe::egui::Key::Plus]);
        assert_eq!(dump(&["--walk", "1,2,0"]).unwrap().walk.unwrap().scale, 1.0);
        for refused in [
            &["--walk", "1,2"][..],
            &["--walk", "1,2,0,0"],
            &["--walk", "1,2,0", "--walk-keys", "q"],
            &["--walk-keys", "w"],
            &["--walk-flying"],
            &["--walk", "1,2,0", "--camera", "100,200,300"],
        ] {
            assert!(dump(refused).is_err(), "{refused:?}");
        }
        // It is a window dump's: the desktop app's maps walk from their Camera control.
        assert!(parse(&["campaign.battleplan", "--walk", "1,2,0"]).is_err());
    }

    #[test]
    fn fly_puts_a_window_dump_in_the_flying_mode() {
        let window = |extra: &[&str]| {
            let mut args = vec![
                "--window-dump",
                "c.battleplan",
                "--hour",
                "1",
                "--out",
                "f.png",
            ];
            args.extend_from_slice(extra);
            parse(&args)
        };
        let options = window(&[
            "--fly",
            "3000,40,1000,90,-10,20",
            "--fly-view",
            "nose",
            "--fly-steer",
            "0.5,-1,1",
        ])
        .unwrap();
        let fly = options.fly.expect("a flight");
        assert_eq!(fly.position, [3000.0, 40.0, 1000.0]);
        assert_eq!(
            fly.attitude,
            [
                90f32.to_radians(),
                (-10f32).to_radians(),
                20f32.to_radians()
            ]
        );
        assert_eq!(fly.view, controls::flight::View::Nose);
        assert_eq!(fly.steer, [0.5, -1.0, 1.0]);
        // The defaults: the chase view, nothing held, no bank.
        let plain = window(&["--fly", "1,2,3,4,5"]).unwrap().fly.unwrap();
        assert_eq!(plain.view, controls::flight::View::Chase);
        assert_eq!(plain.steer, [0.0; 3]);
        assert_eq!(plain.attitude[2], 0.0);
        for refused in [
            &["--fly", "1,2,3,4"][..],
            &["--fly", "1,2,3,4,5,6,7"],
            &["--fly", "1,2,3,4,5", "--camera", "1,2,300"],
            &["--fly", "1,2,3,4,5", "--walk", "1,2,0"],
            &["--fly", "1,2,3,4,5", "--fly-view", "side"],
            &["--fly", "1,2,3,4,5", "--fly-steer", "2,0,0"],
            &["--fly-view", "nose"],
            &["--fly-steer", "0,0,0"],
        ] {
            assert!(window(refused).is_err(), "{refused:?}");
        }
        // A flight needs the window's path.
        assert!(
            parse(&[
                "--dump",
                "c.battleplan",
                "--hour",
                "1",
                "--out",
                "f.png",
                "--fly",
                "1,2,3,4,5"
            ])
            .is_err()
        );
    }

    #[test]
    fn out_names_only_a_dump_or_window_dump_frame() {
        assert!(parse(&["campaign.battleplan", "--out", "capture.png"]).is_err());
        assert!(parse(&["--out", "capture.png"]).is_err());
        let dump = parse(&[
            "--window-dump",
            "campaign.battleplan",
            "--hour",
            "1",
            "--out",
            "frame.png",
        ])
        .unwrap();
        assert_eq!(dump.out, Some(PathBuf::from("frame.png")));
    }

    #[test]
    fn dumps_choose_day_night_and_allied_plans_explicitly() {
        let dump = |extra: &[&str]| {
            let mut args = vec!["--dump", "c.battleplan", "--hour", "1", "--out", "f.png"];
            args.extend_from_slice(extra);
            parse(&args)
        };
        let plain = dump(&[]).unwrap();
        assert_eq!(
            (plain.day_night, plain.allied_plans, plain.free_overlays),
            (None, None, None)
        );
        let chosen = dump(&[
            "--day-night",
            "OFF",
            "--allied-plans",
            "on",
            "--free-overlays",
            "On",
        ])
        .unwrap();
        assert_eq!(
            (chosen.day_night, chosen.allied_plans, chosen.free_overlays),
            (Some(false), Some(true), Some(true))
        );
        assert!(dump(&["--free-overlays", "maybe"]).is_err());
        assert!(dump(&["--day-night", "maybe"]).is_err());
        assert!(dump(&["--day-night"]).is_err());
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
