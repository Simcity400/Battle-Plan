//! Battle Plan product surface: Win32 + D3D11 swapchain app.
//!
//! Product map rendering enters through the portable replay's admitted,
//! authenticated production-render carrier. The normal app executes its exact
//! D3D11 program on the app-owned device and swapchain target; an absent or
//! incomplete carrier is feature-locally omitted. Validated capture bundles
//! remain debug/evidence inputs only. The replay app does not render the removed
//! CPU `map_visual.rgbx` scaffold or generated political-color substitutions.
//!
//! Usage: `replay_app [--configure] [--config <file>] [--recording <portable .battleplan>]
//! [--game <dir>] [--userdata <dir>] [--recording <path>]
//! [--capture <session dir>]`
//!
//! `--recording` opens a snapshot-flavor or compact-stream-only `.battleplan`
//! (or a snapshot store directory) through the single bounded product policy
//! and drives the map from its recorded state owners. Without it the map shows
//! the grounded 1936 owners.
//! Replay-driven political color substitution is blocked until the engine's
//! primary fill / exact GradientBorder generation law is ported.
//! Debug-only `--capture` loads a validated map_viewer captured-pass session and uses
//! the game's captured pdxmap/pdxwater shaders as the map base layer; invalid
//! captured sessions fail startup with the validator message.
//!
//! On first launch the app asks for the game content root and the active
//! user-data/profile root. The versioned local profile retains only those two
//! roots; replay files remain independently movable and shareable.
//!
//! Controls: middle/right-drag or A/D pan horizontally; wheel zoom and
//! vertical pan are blocked until the portable replay carries the exact C02/C03
//! 3D current/target camera state; `R` resets to whole-map fit; `[`/`]` or
//! Left/Right step one timeline hour; Space toggles ~4 hours/sec autoplay;
//! left click/drag on the scrub bar seeks the timeline; Escape quits.

mod anomaly;
mod blockers;
mod bundle;
mod camera;
mod character_runtime;
mod clausewitz;
mod content_compatibility;
#[cfg(test)]
mod e2e_playback;
mod font;
mod grading_law;
mod gui_definition_admission;
mod install_paths;
mod observer;
mod palette;
#[cfg(test)]
mod post_effect;
mod recording_open;
mod timeline;
mod tooltip_runtime;
mod transport;
mod validate;

#[cfg(windows)]
mod app;
#[cfg(windows)]
mod counter_gpu;
#[cfg(windows)]
mod d3d;
#[cfg(windows)]
mod headless;
#[cfg(windows)]
mod map_furniture;
#[cfg(windows)]
mod map_labels;
#[cfg(windows)]
mod overlay;
#[cfg(windows)]
mod political_scene;
#[cfg(windows)]
mod render;
#[cfg(windows)]
mod submitted_draw;
#[cfg(windows)]
mod text;
#[cfg(windows)]
mod ui_runtime;
#[cfg(windows)]
mod win32;

use std::error::Error;
use std::ffi::OsString;
use std::path::PathBuf;

use blockers::BlockerId;
use map_viewer::{sensitive_file_overrides, GameFileResolver, GameFileResolverOptions};

pub type Result<T> = std::result::Result<T, Box<dyn Error>>;

const USAGE: &str = "usage: replay_app [--configure] [--config <profile.json>] [--non-interactive] [--list-blockers] [--validate-recording <path>] [--validate-playback <path>] [--bundle <dir>] [--game <the game install dir>] \
[--mod <content-dir> ...] [--use-dlc-load --userdata <dir>] [--recording <path> [--frame-cache <entries>]] [--capture <session dir>] [--dump-frame <out.bmp> [--dump-hour <canonical-hour-index> | --dump-nth-hour <ordinal>] \
[--hour <0..1>] [--dump-zoom <screen-px-per-map-px>] [--dump-center <x,y>] [--dump-hdr <out.rgba16f>]] \
    [--postfx] [--postfx-restore-full] [--postfx-validate-capture] [--water-full]\n\
--dump-hour selects the exact canonical REPLAY_SCHEMA hour_index; --dump-nth-hour selects a zero-based ordinal in the recording snapshot index and resolves it to that canonical hour; \
--frame-cache <entries> shrinks the recording's decoded-frame LRU budget for weak-PC profiles (default 16 hot frames; requires --recording); \
--configure selects/replaces the two machine-local content roots without opening a replay; --config provides a portable/headless configuration location; \
normal product playback reconstructs the launcher's effective selection from the configured user-data dlc_load.json; --mod remains an explicit development/test override; \
--bundle and --capture are debug/evidence inputs only; normal portable recording playback never requires either. `--recording <portable> --dump-frame <bmp>` executes one exact recorded hour on an app-owned offscreen D3D11 target and fails instead of writing when any production carrier/input/output is omitted; \
--demo-scrub is blocked because it generated synthetic, non-replay state changes; \
terrain chunk synthesis is blocked in replay_app until the remaining runtime/LOD state is engine-derived; use map_viewer terrain tooling only for development; --synthesize-terrain-chunks and --chunks-from-files are blocked; \
--gb-cpu and REPLAY_APP_GB_CPU are blocked; generated political GradientBorder textures are research-only and not used for product rendering; \
--postfx enables the ported scene-restore tone map (default ON: the game's on-screen image is post-tonemap; --no-postfx is blocked); \
--postfx-restore-full (DEFAULT ON) composes the RestoreScene grade from installed content via the mod-aware resolver and fails closed when evidence is missing; --no-postfx-restore-full is blocked; \
--postfx-validate-capture loads the captured restore-pass bundle and reports file-law vs captured deltas (LUT max delta, MiddleGrey/AverageLuminance diffs) then exits; a regression gate, never a render input (requires --capture and --game); \
--freeze-clock is blocked because replay_app preserves captured map-clock constant-buffer bytes until real runtime timing is capture-derived; \
--water-full (DEFAULT ON) uploads only the real engine/file-derived water path and fails closed when required inputs are missing; --no-water-full is blocked because it selected the old app-owned approximation; \
borders are blocked in replay_app until the remaining thin-border fade/loop/sea-region law is ported; use map_viewer border dumps as development tooling; --borders-from-files is blocked; \
country-label production integration is closed: measured control_by_province feeds the B10 controller law, authenticated reflection stamps mapname CB1, and normal D80 playback executes the exact captured glyph draw automatically; --map-labels remains blocked as a legacy manual-layer switch; \
map furniture is blocked in replay_app only on highlight ring rendering and the 0.40 map-text scale origin; the relation-column law is engine-traced in map_viewer, but non-neutral capture coverage is still pending; the furniture module/projection API exists but --map-furniture remains blocked; \
tree runtime replay admits the exact retained 13-buffer/three-pass model/LOD carrier (4,781 visible cells); file-derived full-map replacement remains blocked because 95,275 of 99,056 global nonzero cells lack an authenticated carrier, and season CB1 is separate; --trees-from-files and --tree-seasons-by-date are blocked; \
building previews are blocked until map_objects_pass or mesh/material import law exists; --buildings-from-files and --buildings-preview-all are blocked";

const SENSITIVE_GAME_FILE_PATHS: &[&str] = &[
    "gfx/FX/constants.fxh",
    "gfx/FX/pdxmap.shader",
    "gfx/FX/pdxmesh.shader",
    "gfx/entities/buildings.asset",
    "gfx/entities/buildings.gfx",
    "common/buildings/00_buildings.txt",
    "map/buildings.txt",
    "map/seasons.txt",
];

/// What drives the timeline: recorded state or the grounded 1936 baseline.
#[derive(Debug, PartialEq)]
enum TimelineMode {
    Static1936,
    Recording {
        path: PathBuf,
        /// F2 memory budget for the decoded-frame LRU (default 16 entries;
        /// `--frame-cache <entries>` shrinks it for weak-PC profiles).
        timeline: replay_core::TimelineConfig,
    },
}

#[derive(Debug)]
struct CliOptions {
    /// Optional evidence/debug map bundle. Portable recording playback never
    /// requires a pre-generated machine-local bundle.
    bundle_dir: Option<PathBuf>,
    mode: TimelineMode,
    /// the game install for fonts (`--game`; `BATTLE_PLAN_VANILLA` env fallback).
    game_dir: Option<PathBuf>,
    game_files: GameFileResolverOptions,
    config_path: Option<PathBuf>,
    non_interactive: bool,
    /// Headless: render one hour to this BMP and exit (no window).
    dump_frame: Option<PathBuf>,
    dump_hour: Option<i64>,
    dump_nth_hour: Option<usize>,
    day_night_hour: Option<f32>,
    dump_zoom: Option<f64>,
    dump_center: Option<(f64, f64)>,
    /// COLOR-lane debug readback: also write the rendered HDR MainScene (the
    /// captured-pass RGBA16F output target, at the capture's native
    /// dimensions, BEFORE the composite/postfx) to this path, with a
    /// `<path>.json` sidecar. This is the byte-level artifact the engine
    /// post-pass RT referees diff against. Requires `--dump-frame`.
    dump_hdr: Option<PathBuf>,
    capture_dir: Option<PathBuf>,
    postfx_enabled: bool,
    /// Files-first RestoreScene grade. DEFAULT ON and fail-closed.
    postfx_restore_full: bool,
    /// VALIDATION-ONLY: compose the files-first grade and report deltas against
    /// the captured restore bundle (requires --capture and --game), then exit.
    /// Never a render input.
    postfx_validate_capture: bool,
    /// `--water-full`: upload the real reflection cube + render the
    /// mechanism-correct underwater refraction pass. DEFAULT ON and fail-closed.
    water_full: bool,
}

fn parse_cli(args: &[String]) -> Result<CliOptions> {
    let mut bundle_dir = None;
    let mut game_dir = None;
    let mut recording = None;
    let mut frame_cache: Option<usize> = None;
    let mut mod_paths = Vec::new();
    let mut use_dlc_load = false;
    let mut userdata = None;
    let mut config_path = None;
    let mut non_interactive = false;
    let mut dump_frame = None;
    let mut dump_hour = None;
    let mut dump_nth_hour = None;
    let mut day_night_hour = None;
    let mut dump_zoom = None;
    let mut dump_center = None;
    let mut dump_hdr = None;
    let mut capture_dir = None;
    // The game's on-screen image is the scene-restore tonemap applied to the
    // linear HDR scene; captured RTs are pre-postfx, which is why raw captures
    // look dark. Product rendering keeps postfx on.
    let mut postfx_enabled = true;
    // Files-first RestoreScene grade is DEFAULT ON and fail-closed. Raw captured
    // RT output is not the game's final image and is blocked at the CLI layer.
    let mut postfx_restore_full = true;
    let mut postfx_validate_capture = false;
    // --water-full is DEFAULT ON and fail-closed. The old mean-color/
    // approximation path is blocked rather than exposed as an opt-out.
    let mut water_full = true;
    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "--bundle" => {
                index += 1;
                let value = args
                    .get(index)
                    .ok_or_else(|| format!("--bundle requires a directory\n{USAGE}"))?;
                bundle_dir = Some(PathBuf::from(value));
            }
            "--game" => {
                index += 1;
                let value = args
                    .get(index)
                    .ok_or_else(|| format!("--game requires a directory\n{USAGE}"))?;
                game_dir = Some(PathBuf::from(value));
            }
            "--mod" => {
                index += 1;
                let value = args
                    .get(index)
                    .ok_or_else(|| format!("--mod requires a content directory\n{USAGE}"))?;
                mod_paths.push(PathBuf::from(value));
            }
            "--use-dlc-load" => {
                use_dlc_load = true;
            }
            "--userdata" => {
                index += 1;
                let value = args
                    .get(index)
                    .ok_or_else(|| format!("--userdata requires a directory\n{USAGE}"))?;
                userdata = Some(PathBuf::from(value));
            }
            "--config" => {
                index += 1;
                let value = args
                    .get(index)
                    .ok_or_else(|| format!("--config requires a file path\n{USAGE}"))?;
                config_path = Some(PathBuf::from(value));
            }
            "--non-interactive" => {
                non_interactive = true;
            }
            "--configure" => {
                return Err(format!("--configure is a standalone command and cannot be combined with playback options\n{USAGE}").into());
            }
            "--recording" => {
                index += 1;
                let value = args
                    .get(index)
                    .ok_or_else(|| format!("--recording requires a path\n{USAGE}"))?;
                recording = Some(PathBuf::from(value));
            }
            "--frame-cache" => {
                index += 1;
                let value = args
                    .get(index)
                    .ok_or_else(|| format!("--frame-cache requires an entry count\n{USAGE}"))?;
                let entries = value.parse::<usize>().map_err(|err| {
                    format!("--frame-cache: invalid entry count {value:?}: {err}\n{USAGE}")
                })?;
                if entries == 0 {
                    return Err(format!(
                        "--frame-cache must be >= 1 (0 would disable frame caching entirely; \
                         the smallest supported product budget is 1 hot frame)\n{USAGE}"
                    )
                    .into());
                }
                frame_cache = Some(entries);
            }
            "--demo-scrub" => {
                return Err(blockers::reason(BlockerId::DemoScrub).into());
            }
            "--dump-frame" => {
                index += 1;
                let value = args
                    .get(index)
                    .ok_or_else(|| format!("--dump-frame requires an output path\n{USAGE}"))?;
                dump_frame = Some(PathBuf::from(value));
            }
            "--dump-hour" => {
                index += 1;
                let value = args
                    .get(index)
                    .ok_or_else(|| format!("--dump-hour requires an hour index\n{USAGE}"))?;
                dump_hour = Some(
                    value
                        .parse::<i64>()
                        .map_err(|err| format!("--dump-hour: {err}\n{USAGE}"))?,
                );
            }
            "--dump-nth-hour" => {
                index += 1;
                let value = args.get(index).ok_or_else(|| {
                    format!("--dump-nth-hour requires a snapshot ordinal\n{USAGE}")
                })?;
                dump_nth_hour = Some(
                    value
                        .parse::<usize>()
                        .map_err(|err| format!("--dump-nth-hour: {err}\n{USAGE}"))?,
                );
            }
            "--hour" => {
                index += 1;
                let value = args.get(index).ok_or_else(|| {
                    format!("--hour requires a normalized value in [0,1)\n{USAGE}")
                })?;
                let hour = value
                    .parse::<f32>()
                    .map_err(|err| format!("--hour: {err}\n{USAGE}"))?;
                if !hour.is_finite() || !(0.0..1.0).contains(&hour) {
                    return Err(format!("--hour must be finite in [0,1)\n{USAGE}").into());
                }
                day_night_hour = Some(hour);
            }
            "--dump-zoom" => {
                index += 1;
                let value = args
                    .get(index)
                    .ok_or_else(|| format!("--dump-zoom requires a positive number\n{USAGE}"))?;
                let zoom = value
                    .parse::<f64>()
                    .map_err(|err| format!("--dump-zoom: {err}\n{USAGE}"))?;
                if !zoom.is_finite() || zoom <= 0.0 {
                    return Err(format!("--dump-zoom must be positive and finite\n{USAGE}").into());
                }
                dump_zoom = Some(zoom);
            }
            "--dump-center" => {
                index += 1;
                let value = args
                    .get(index)
                    .ok_or_else(|| format!("--dump-center requires x,y\n{USAGE}"))?;
                dump_center = Some(parse_dump_center(value)?);
            }
            "--dump-hdr" => {
                index += 1;
                let value = args
                    .get(index)
                    .ok_or_else(|| format!("--dump-hdr requires an output path\n{USAGE}"))?;
                dump_hdr = Some(PathBuf::from(value));
            }
            "--capture" => {
                index += 1;
                let value = args
                    .get(index)
                    .ok_or_else(|| format!("--capture requires a session directory\n{USAGE}"))?;
                capture_dir = Some(PathBuf::from(value));
            }
            "--gb-cpu" => {
                return Err(blockers::reason(BlockerId::GbCpuFlag).into());
            }
            "--synthesize-terrain-chunks" => {
                return Err(blockers::reason(BlockerId::SynthesizeTerrainChunks).into());
            }
            "--water-full" => {
                water_full = true;
            }
            "--no-water-full" => {
                return Err(blockers::reason(BlockerId::NoWaterFull).into());
            }
            "--borders-from-files" => {
                return Err(blockers::reason(BlockerId::BordersFromFiles).into());
            }
            "--no-borders-from-files" => {
                return Err(blockers::reason(BlockerId::NoBordersFromFiles).into());
            }
            "--chunks-from-files" => {
                return Err(blockers::reason(BlockerId::ChunksFromFiles).into());
            }
            "--no-capture-water" => {
                return Err(blockers::reason(BlockerId::NoCaptureWater).into());
            }
            "--no-capture-rivers" => {
                return Err(blockers::reason(BlockerId::NoCaptureRivers).into());
            }
            "--no-capture-trees" => {
                return Err(blockers::reason(BlockerId::NoCaptureTrees).into());
            }
            "--postfx" => {
                postfx_enabled = true;
            }
            "--no-postfx" => {
                return Err(blockers::reason(BlockerId::NoPostfx).into());
            }
            "--postfx-restore-full" => {
                postfx_restore_full = true;
            }
            "--no-postfx-restore-full" => {
                return Err(blockers::reason(BlockerId::NoPostfxRestoreFull).into());
            }
            "--postfx-validate-capture" => {
                postfx_validate_capture = true;
            }
            "--tree-seasons-by-date" => {
                return Err(blockers::reason(BlockerId::TreeSeasonsByDate).into());
            }
            "--trees-from-files" => {
                return Err(blockers::reason(BlockerId::TreesFromFiles).into());
            }
            "--no-trees-second-subdraw" => {
                return Err(blockers::reason(BlockerId::NoTreesSecondSubdraw).into());
            }
            "--buildings-from-files" => {
                return Err(blockers::reason(BlockerId::BuildingsFromFiles).into());
            }
            "--buildings-preview-all" => {
                return Err(blockers::reason(BlockerId::BuildingsPreviewAll).into());
            }
            "--freeze-clock" => {
                return Err(blockers::reason(BlockerId::FreezeClock).into());
            }
            "--map-labels" => {
                return Err(blockers::reason(BlockerId::MapLabels).into());
            }
            "--no-map-labels" => {
                return Err(blockers::reason(BlockerId::NoMapLabels).into());
            }
            "--map-labels-fallback" => {
                return Err(blockers::reason(BlockerId::MapLabelsFallback).into());
            }
            "--map-furniture" => {
                return Err(blockers::reason(BlockerId::MapFurniture).into());
            }
            "--no-map-furniture" => {
                return Err(blockers::reason(BlockerId::NoMapFurniture).into());
            }
            other => {
                return Err(format!("unknown argument {other:?}\n{USAGE}").into());
            }
        }
        index += 1;
    }
    if dump_hour.is_some() && dump_frame.is_none() {
        return Err(format!("--dump-hour requires --dump-frame\n{USAGE}").into());
    }
    if dump_nth_hour.is_some() && dump_frame.is_none() {
        return Err(format!("--dump-nth-hour requires --dump-frame\n{USAGE}").into());
    }
    if dump_hour.is_some() && dump_nth_hour.is_some() {
        return Err(
            format!("pass either --dump-hour or --dump-nth-hour, not both\n{USAGE}").into(),
        );
    }
    if day_night_hour.is_some() && dump_frame.is_none() {
        return Err(format!("--hour requires --dump-frame\n{USAGE}").into());
    }
    if dump_zoom.is_some() && dump_frame.is_none() {
        return Err(format!("--dump-zoom requires --dump-frame\n{USAGE}").into());
    }
    if dump_center.is_some() && dump_frame.is_none() {
        return Err(format!("--dump-center requires --dump-frame\n{USAGE}").into());
    }
    if dump_hdr.is_some() && dump_frame.is_none() {
        return Err(format!("--dump-hdr requires --dump-frame\n{USAGE}").into());
    }
    if use_dlc_load && !mod_paths.is_empty() {
        return Err(format!(
            "pass either repeated --mod content dirs or --use-dlc-load, not both\n{USAGE}"
        )
        .into());
    }
    if use_dlc_load && userdata.is_none() {
        return Err(format!(
            "--use-dlc-load requires --userdata <the game user-data dir> so the selected \n\
             dlc_load.json is unambiguous\n{USAGE}"
        )
        .into());
    }
    let mode = match recording {
        Some(path) => TimelineMode::Recording {
            path,
            timeline: match frame_cache {
                Some(entries) => replay_core::TimelineConfig {
                    frame_cache_entries: entries,
                },
                None => replay_core::TimelineConfig::default(),
            },
        },
        None => {
            // Fail closed: a memory budget that silently applies to nothing
            // would let a weak-PC profile believe it took effect.
            if frame_cache.is_some() {
                return Err(format!(
                    "--frame-cache requires --recording (the static 1936 baseline has no \
                     frame cache)\n{USAGE}"
                )
                .into());
            }
            if dump_nth_hour.is_some() {
                return Err(format!(
                    "--dump-nth-hour requires --recording (the static 1936 baseline has no recording snapshot index)\n{USAGE}"
                )
                .into());
            }
            TimelineMode::Static1936
        }
    };
    Ok(CliOptions {
        bundle_dir,
        mode,
        game_dir,
        game_files: GameFileResolverOptions {
            mod_paths,
            use_dlc_load,
            userdata,
        },
        config_path,
        non_interactive,
        dump_frame,
        dump_hour,
        dump_nth_hour,
        day_night_hour,
        dump_zoom,
        dump_center,
        dump_hdr,
        capture_dir,
        postfx_enabled,
        postfx_restore_full,
        postfx_validate_capture,
        water_full,
    })
}

fn parse_dump_center(value: &str) -> Result<(f64, f64)> {
    let (x, y) = value
        .split_once(',')
        .ok_or_else(|| format!("--dump-center requires x,y\n{USAGE}"))?;
    let x = x
        .parse::<f64>()
        .map_err(|err| format!("--dump-center x: {err}\n{USAGE}"))?;
    let y = y
        .parse::<f64>()
        .map_err(|err| format!("--dump-center y: {err}\n{USAGE}"))?;
    if !x.is_finite() || !y.is_finite() {
        return Err(format!("--dump-center values must be finite\n{USAGE}").into());
    }
    Ok((x, y))
}

/// The game install for font loading: the `--game` flag wins, the
/// `BATTLE_PLAN_VANILLA` environment variable is the fallback, and neither being
/// set means "no install" (overlay text fails closed with a warning).
fn resolve_game_dir(flag: Option<PathBuf>, env: Option<OsString>) -> Option<PathBuf> {
    flag.or_else(|| env.map(PathBuf::from))
}

/// Test helper for the legacy pre-resolution shape. Shipped playback resolves
/// the selector against the already-open provider so a compact recording is
/// never folded twice merely to translate an ordinal.
#[cfg(test)]
fn resolve_headless_recording_hour(options: &mut CliOptions) -> Result<()> {
    if options.dump_frame.is_none() {
        return Ok(());
    }
    let request = match (options.dump_hour, options.dump_nth_hour) {
        (Some(hour), None) => Some(timeline::DumpHourRequest::Canonical(hour)),
        (None, Some(ordinal)) => Some(timeline::DumpHourRequest::Ordinal(ordinal)),
        (None, None) => None,
        (Some(_), Some(_)) => unreachable!("parse_cli rejects competing dump selectors"),
    };
    let (Some(request), TimelineMode::Recording { path, .. }) = (request, &options.mode) else {
        return Ok(());
    };
    let source = recording_open::open_product_recording(path)
        .map_err(|err| format!("open recording {}: {err}", path.display()))?;
    options.dump_hour = Some(timeline::resolve_dump_hour(source.hours(), request)?);
    Ok(())
}

fn build_game_file_resolver(
    game_dir: Option<PathBuf>,
    options: GameFileResolverOptions,
) -> Result<Option<GameFileResolver>> {
    let has_mod_flags = !options.mod_paths.is_empty() || options.use_dlc_load;
    let Some(game_dir) = game_dir else {
        if has_mod_flags {
            return Err("pass --game <the game install dir> or set BATTLE_PLAN_VANILLA when using --mod/--use-dlc-load".into());
        }
        return Ok(None);
    };
    Ok(Some(GameFileResolver::from_options(game_dir, options)?))
}

fn log_game_file_resolver_warnings(resolver: Option<&GameFileResolver>) -> Result<()> {
    let Some(resolver) = resolver else {
        return Ok(());
    };
    for warning in resolver.warnings() {
        eprintln!("replay_app: mod warning: {warning}");
    }
    let report = sensitive_file_overrides(resolver, SENSITIVE_GAME_FILE_PATHS)?;
    if !report.overridden.is_empty() {
        eprintln!(
            "replay_app: WARNING: active modset overrides sensitive installed content: {}",
            report.overridden.join(", ")
        );
    }
    Ok(())
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if let Err(err) = run(&args) {
        eprintln!("replay_app: {err}");
        std::process::exit(2);
    }
}

/// Standalone first-launch/reselection command. This deliberately has a tiny
/// argument surface so configuration works in a terminal or deployment script
/// without a map bundle, renderer, or replay file.
fn run_configure(args: &[String]) -> Result<()> {
    let mut game = None;
    let mut user_data = None;
    let mut config = None;
    let mut non_interactive = false;
    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "--configure" => {}
            "--game" => {
                index += 1;
                game = Some(PathBuf::from(
                    args.get(index)
                        .ok_or_else(|| format!("--game requires a directory\n{USAGE}"))?,
                ));
            }
            "--userdata" => {
                index += 1;
                user_data = Some(PathBuf::from(
                    args.get(index)
                        .ok_or_else(|| format!("--userdata requires a directory\n{USAGE}"))?,
                ));
            }
            "--config" => {
                index += 1;
                config = Some(PathBuf::from(
                    args.get(index)
                        .ok_or_else(|| format!("--config requires a file path\n{USAGE}"))?,
                ));
            }
            "--non-interactive" => non_interactive = true,
            other => {
                return Err(format!(
                    "{other:?} is not valid with --configure; use only --game, --userdata, --config, and --non-interactive\n{USAGE}"
                )
                .into())
            }
        }
        index += 1;
    }
    let selected = install_paths::resolve_process_paths(
        config,
        install_paths::ResolveOptions {
            game_override: game,
            user_data_override: user_data,
            force_reselect: true,
            non_interactive,
        },
    )?;
    let resolver =
        GameFileResolver::from_dlc_load(selected.game_install.clone(), selected.user_data.clone())
            .map_err(|error| {
                format!(
            "the selected roots do not produce a valid launcher-equivalent content profile: {error}"
        )
            })?;
    install_paths::playback_environment_from_paths(
        &selected,
        &resolver,
        [
            "physical_width",
            "physical_height",
            "min_gui_width",
            "min_gui_height",
            "gui_scale_f32_bits",
            "refresh_rate_hz",
            "max_refresh_rate_hz",
            "fullscreen",
            "borderless",
            "display_mode",
            "settings_file_sha256",
        ]
        .into_iter()
        .map(str::to_string),
    )
    .map_err(|error| format!("selected playback profile metadata is invalid: {error}"))?;
    println!("Playback profile saved and validated.");
    println!("  the game content root: {}", selected.game_install.display());
    println!("  User-data/profile: {}", selected.user_data.display());
    println!("The replay file location is not stored; .battleplan files remain portable.");
    println!(
        "  Effective content layers: {} (lowest to highest priority)",
        resolver.active_sources().len()
    );
    for warning in resolver.warnings() {
        eprintln!("replay_app: profile warning: {warning}");
    }
    Ok(())
}

fn run(args: &[String]) -> Result<()> {
    if args.iter().any(|arg| arg == "--configure") {
        return run_configure(args);
    }
    if args.iter().any(|arg| arg == "--validate-playback") {
        return run_validate_playback(args);
    }
    // E4 open/validate product entry: `--validate-recording <path>` runs the
    // SHARED validator and reports a readable diagnostic without needing a map
    // bundle. It is intercepted before `parse_cli` so it works standalone.
    if let Some(index) = args.iter().position(|arg| arg == "--validate-recording") {
        let path = args
            .get(index + 1)
            .ok_or_else(|| format!("--validate-recording requires a path\n{USAGE}"))?;
        return run_validate_recording(std::path::Path::new(path));
    }
    // G3: emit the canonical product-blocker list mechanically for audits and
    // the definition-of-done. Intercepted before `parse_cli` so it needs no
    // map bundle.
    if args.iter().any(|arg| arg == "--list-blockers") {
        print!("{}", blockers::render_list());
        return Ok(());
    }
    let options = parse_cli(args)?;
    preflight_snapshot_recording_dump_request(&options)?;
    let bundle = options
        .bundle_dir
        .as_deref()
        .map(|bundle_dir| {
            let bundle = bundle::MapBundle::load(bundle_dir).map_err(|err| {
                format!(
                    "failed to open debug/evidence map bundle at {}: {}\n{}",
                    bundle_dir.display(),
                    err,
                    bundle::generation_guidance()
                )
            })?;
            bundle.validate_product_dimensions().map_err(|err| {
                format!(
                    "debug/evidence map bundle at {} failed validation: {}\n{}",
                    bundle_dir.display(),
                    err,
                    bundle::generation_guidance()
                )
            })?;
            Ok::<_, Box<dyn std::error::Error>>(bundle)
        })
        .transpose()?;
    let configured_paths = install_paths::resolve_process_paths(
        options.config_path.clone(),
        install_paths::ResolveOptions {
            game_override: resolve_game_dir(options.game_dir, std::env::var_os("BATTLE_PLAN_VANILLA").or_else(|| std::env::var_os("HOI4_VANILLA"))),
            user_data_override: options.game_files.userdata.clone(),
            force_reselect: false,
            non_interactive: options.non_interactive,
        },
    )?;
    let game_dir = Some(configured_paths.game_install.clone());
    let mut game_file_options = options.game_files;
    game_file_options.userdata = Some(configured_paths.user_data.clone());
    if game_file_options.mod_paths.is_empty() {
        // Production playback follows the launcher's current effective profile.
        // Explicit --mod roots are retained only as a deliberate test/developer
        // selection and never inferred from a replay's source-machine paths.
        game_file_options.use_dlc_load = true;
    }
    // Portable recording playback reconstructs only the replay-required VFS
    // after opening its content manifest. Do not parse/materialize every local
    // launcher layer first: an unrelated corrupt or missing layer is irrelevant.
    let game_resolver = if matches!(options.mode, TimelineMode::Recording { .. }) {
        None
    } else {
        build_game_file_resolver(game_dir, game_file_options)?
    };
    log_game_file_resolver_warnings(game_resolver.as_ref())?;
    // Preserve first-launch profile selection: an empty invocation must ask
    // for the two machine-local roots before explaining that Static1936 is a
    // debug-only mode. Portable product playback itself needs only --recording.
    if matches!(options.mode, TimelineMode::Static1936) && bundle.is_none() {
        return Err(
            "static 1936 is a debug baseline and requires --bundle; production playback requires --recording <portable .battleplan>"
                .into(),
        );
    }
    reject_env_overrides(|name| std::env::var_os(name))?;
    // Map labels and furniture are product-blocked. Their constructors stay
    // fail-closed for validation-only callers.
    if options.postfx_validate_capture {
        let capture_dir = options.capture_dir.as_deref().ok_or(
            "--postfx-validate-capture requires --capture <session dir> with a restore-pass bundle",
        )?;
        let report = render::restore_full_postfx_validate_capture(
            game_resolver.as_ref(),
            capture_dir,
            bundle
                .as_ref()
                .ok_or("--postfx-validate-capture requires --bundle debug evidence")?
                .width,
            bundle
                .as_ref()
                .ok_or("--postfx-validate-capture requires --bundle debug evidence")?
                .height,
            options.day_night_hour,
        )?;
        println!("postfx-validate-capture (file-law vs captured):");
        println!("  selection: {}", report.selection_reason);
        println!("  file-law value: {}", report.selected_value);
        println!("  composed LUT: {}", report.composed_lut_rel_path);
        println!("  captured bundle: {}", report.captured_bundle_file);
        println!("  LUT max per-channel delta: {}", report.lut_max_delta);
        println!(
            "  MiddleGrey: file {:.6} vs captured {:.6} (delta {:.6})",
            report.file_middle_grey, report.captured_middle_grey, report.middle_grey_delta
        );
        println!(
            "  AverageLuminance: file {:.6} vs captured {:.6} (delta {:.6})",
            report.file_average_luminance,
            report.captured_average_luminance,
            report.average_luminance_delta
        );
        let lut_ok = report.lut_max_delta == 0;
        let mg_ok = report.middle_grey_delta < 1e-4;
        let al_ok = report.average_luminance_delta < 1e-4;
        println!(
            "  RESULT: LUT {}, MiddleGrey {}, AverageLuminance {}",
            if lut_ok { "EXACT" } else { "MISMATCH" },
            if mg_ok { "EXACT" } else { "MISMATCH" },
            if al_ok { "EXACT" } else { "MISMATCH" },
        );
        // A real regression gate: non-zero exit on any mismatch so CI/scripts
        // catch a drift between the files-first law and the captured evidence.
        if !(lut_ok && mg_ok && al_ok) {
            return Err(
                "postfx-validate-capture: file-law grade does not match captured evidence \
                        (see LUT/MiddleGrey/AverageLuminance deltas above)"
                    .into(),
            );
        }
        return Ok(());
    }
    match &options.dump_frame {
        Some(out) => dump_frame(
            bundle.as_ref(),
            &options.mode,
            options.dump_hour,
            options.dump_nth_hour,
            out,
            game_resolver.as_ref(),
            &configured_paths,
            options.dump_zoom,
            options.dump_center,
            options.dump_hdr.as_deref(),
            options.capture_dir.as_deref(),
            options.postfx_enabled,
            options.postfx_restore_full,
            options.day_night_hour,
            options.water_full,
        ),
        None => run_app(
            bundle,
            options.mode,
            game_resolver,
            configured_paths,
            options.capture_dir,
            options.postfx_enabled,
            options.postfx_restore_full,
            options.water_full,
        ),
    }
}

/// Reject invalid canonical/ordinal requests for snapshot-native recordings
/// before machine-local profile discovery. Compact-only recordings are not
/// folded here: their one certified product fold remains owned by the actual
/// provider open in `headless`, avoiding a duplicate long-recording pass.
fn preflight_snapshot_recording_dump_request(options: &CliOptions) -> Result<()> {
    if options.dump_frame.is_none() {
        return Ok(());
    }
    let request = match (options.dump_hour, options.dump_nth_hour) {
        (Some(hour), None) => Some(timeline::DumpHourRequest::Canonical(hour)),
        (None, Some(ordinal)) => Some(timeline::DumpHourRequest::Ordinal(ordinal)),
        (None, None) => None,
        (Some(_), Some(_)) => unreachable!("parse_cli rejects competing dump selectors"),
    };
    let (Some(request), TimelineMode::Recording { path, .. }) = (request, &options.mode) else {
        return Ok(());
    };
    match replay_core::open_recording(path) {
        Ok(source) => {
            timeline::resolve_dump_hour(source.hours(), request)?;
            Ok(())
        }
        Err(replay_core::SourceError::Unsupported(message))
            if message.contains("compact-stream-only production recording") =>
        {
            Ok(())
        }
        Err(error) => Err(format!("open recording {}: {error}", path.display()).into()),
    }
}

/// Full headless playback-installation gate. Unlike `--validate-recording`,
/// this verifies the selected local VFS against the replay content manifest.
fn run_validate_playback(args: &[String]) -> Result<()> {
    let mut replay = None;
    let mut game = None;
    let mut user_data = None;
    let mut config = None;
    let mut non_interactive = false;
    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "--validate-playback" => {
                index += 1;
                replay = Some(PathBuf::from(args.get(index).ok_or_else(|| {
                    format!("--validate-playback requires a replay path\n{USAGE}")
                })?));
            }
            "--game" => {
                index += 1;
                game = Some(PathBuf::from(args.get(index).ok_or_else(|| {
                    format!("--game requires a directory\n{USAGE}")
                })?));
            }
            "--userdata" => {
                index += 1;
                user_data = Some(PathBuf::from(args.get(index).ok_or_else(|| {
                    format!("--userdata requires a directory\n{USAGE}")
                })?));
            }
            "--config" => {
                index += 1;
                config = Some(PathBuf::from(args.get(index).ok_or_else(|| {
                    format!("--config requires a file path\n{USAGE}")
                })?));
            }
            "--non-interactive" => non_interactive = true,
            other => {
                return Err(format!(
                    "{other:?} is not valid with --validate-playback; use only --game, --userdata, --config, and --non-interactive\n{USAGE}"
                )
                .into())
            }
        }
        index += 1;
    }
    let replay =
        replay.ok_or_else(|| format!("--validate-playback requires a replay path\n{USAGE}"))?;
    let paths = install_paths::resolve_process_paths(
        config,
        install_paths::ResolveOptions {
            game_override: resolve_game_dir(game, std::env::var_os("BATTLE_PLAN_VANILLA").or_else(|| std::env::var_os("HOI4_VANILLA"))),
            user_data_override: user_data,
            force_reselect: false,
            non_interactive,
        },
    )?;
    let timeline = match validate::open_timeline(&replay) {
        Ok(timeline) => timeline,
        Err(outcome) => {
            let summary = outcome.summary();
            println!("{summary}");
            return Err(summary.into());
        }
    };
    content_compatibility::validate_timeline_content(&timeline, &paths)
        .map(|_| ())
        .map_err(|error| error.into())
}

/// E4: open a recording through the shared validator and print a readable
/// diagnostic. Exit success on a clean open, error (exit 2) on any rejection —
/// the specific validator string (schema / hash / modset / version) is
/// surfaced verbatim in the message.
fn run_validate_recording(path: &std::path::Path) -> Result<()> {
    let outcome = validate::open_and_validate(path);
    println!("{}", outcome.summary());
    match outcome {
        validate::OpenOutcome::Opened(report) => {
            // Warnings are diagnostics of a successful open (exit 0), and they
            // belong on stdout with the summary: `--validate-recording X >
            // report.txt` must capture the N warning texts, not just the count.
            for warning in &report.warnings {
                println!("warning: {warning}");
            }
            Ok(())
        }
        validate::OpenOutcome::Rejected { kind, message } => {
            Err(format!("{}: {message}", kind.headline()).into())
        }
    }
}

#[cfg(windows)]
fn run_app(
    bundle: Option<bundle::MapBundle>,
    mode: TimelineMode,
    game_resolver: Option<GameFileResolver>,
    configured_paths: install_paths::ProductPaths,
    capture_dir: Option<PathBuf>,
    postfx_enabled: bool,
    postfx_restore_full: bool,
    water_full: bool,
) -> Result<()> {
    app::run(
        bundle,
        mode,
        game_resolver,
        configured_paths,
        capture_dir,
        postfx_enabled,
        postfx_restore_full,
        water_full,
    )
}

#[cfg(windows)]
fn dump_frame(
    bundle: Option<&bundle::MapBundle>,
    mode: &TimelineMode,
    hour: Option<i64>,
    nth_hour: Option<usize>,
    out: &std::path::Path,
    game_resolver: Option<&GameFileResolver>,
    configured_paths: &install_paths::ProductPaths,
    dump_zoom: Option<f64>,
    dump_center: Option<(f64, f64)>,
    dump_hdr: Option<&std::path::Path>,
    capture_dir: Option<&std::path::Path>,
    postfx_enabled: bool,
    postfx_restore_full: bool,
    day_night_hour: Option<f32>,
    water_full: bool,
) -> Result<()> {
    headless::dump_frame(
        bundle,
        mode,
        hour,
        nth_hour,
        out,
        game_resolver,
        configured_paths,
        dump_zoom,
        dump_center,
        dump_hdr,
        capture_dir,
        postfx_enabled,
        postfx_restore_full,
        day_night_hour,
        water_full,
    )
}

#[cfg(not(windows))]
fn run_app(
    _bundle: Option<bundle::MapBundle>,
    _mode: TimelineMode,
    _game_resolver: Option<GameFileResolver>,
    _configured_paths: install_paths::ProductPaths,
    _capture_dir: Option<PathBuf>,
    _postfx_enabled: bool,
    _postfx_restore_full: bool,
    _water_full: bool,
) -> Result<()> {
    Err("replay_app is a Win32 + D3D11 application and requires Windows".into())
}

#[cfg(not(windows))]
fn dump_frame(
    _bundle: Option<&bundle::MapBundle>,
    _mode: &TimelineMode,
    _hour: Option<i64>,
    _nth_hour: Option<usize>,
    _out: &std::path::Path,
    _game_resolver: Option<&GameFileResolver>,
    _configured_paths: &install_paths::ProductPaths,
    _dump_zoom: Option<f64>,
    _dump_center: Option<(f64, f64)>,
    _dump_hdr: Option<&std::path::Path>,
    _capture_dir: Option<&std::path::Path>,
    _postfx_enabled: bool,
    _postfx_restore_full: bool,
    _day_night_hour: Option<f32>,
    _water_full: bool,
) -> Result<()> {
    Err("replay_app frame dumps require Windows (D3D11)".into())
}

/// Reject every `blockers::REGISTRY` env-override row that is present in the
/// environment. Registry declaration order is the rejection order (first
/// present row wins), matching the historical per-variable call sequence.
/// Registry-driven so a new env row cannot be added — or an existing one
/// dropped — without this single gate enforcing it; the message is the
/// registry row's reason verbatim (blockers stays the single source of text).
/// The environment is injected so tests never touch real process env vars.
fn reject_env_overrides(get: impl Fn(&str) -> Option<std::ffi::OsString>) -> Result<()> {
    for b in blockers::REGISTRY {
        if let blockers::Surface::EnvOverride(name) = b.surface {
            if get(name).is_some() {
                return Err(b.reason.into());
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(values: &[&str]) -> Vec<String> {
        values.iter().map(|value| value.to_string()).collect()
    }

    fn args_with_bundle(values: &[&str]) -> Vec<String> {
        let mut out = args(&["--bundle", "some/dir"]);
        out.extend(args(values));
        out
    }

    #[test]
    fn cli_does_not_require_machine_local_bundle() {
        let options = parse_cli(&[]).expect("first launch reaches profile selection");
        assert_eq!(options.bundle_dir, None);
        assert_eq!(options.mode, TimelineMode::Static1936);
    }

    #[test]
    fn cli_parses_bundle_path() {
        let options = parse_cli(&args(&["--bundle", "some/dir"])).expect("args parse");
        assert_eq!(options.mode, TimelineMode::Static1936);
        assert_eq!(options.bundle_dir, Some(PathBuf::from("some/dir")));
    }

    #[test]
    fn cli_demo_scrub_product_path_blocked() {
        let err =
            parse_cli(&args(&["--bundle", "some/dir", "--demo-scrub"])).expect_err("must fail");
        assert!(err.to_string().contains("--demo-scrub is blocked"), "{err}");
    }

    #[test]
    fn cli_parses_recording_path() {
        let options = parse_cli(&args_with_bundle(&[
            "--recording",
            "some/session.battleplan",
        ]))
        .expect("args parse");
        assert_eq!(
            options.mode,
            TimelineMode::Recording {
                path: PathBuf::from("some/session.battleplan"),
                timeline: replay_core::TimelineConfig::default(),
            }
        );
    }

    #[test]
    fn cli_portable_recording_needs_no_bundle_or_capture() {
        let options = parse_cli(&args(&[
            "--recording",
            "shared/session.battleplan",
            "--non-interactive",
        ]))
        .expect("portable recording parse");
        assert_eq!(options.bundle_dir, None);
        assert_eq!(options.capture_dir, None);
        assert!(matches!(options.mode, TimelineMode::Recording { .. }));
    }

    #[test]
    fn cli_production_recording_dump_needs_no_debug_bundle_or_capture() {
        let options = parse_cli(&args(&[
            "--recording",
            "shared/session.battleplan",
            "--dump-frame",
            "frame.bmp",
            "--dump-hour",
            "42",
            "--non-interactive",
        ]))
        .expect("portable production dump parse");
        assert_eq!(options.bundle_dir, None);
        assert_eq!(options.capture_dir, None);
        assert_eq!(options.dump_frame, Some(PathBuf::from("frame.bmp")));
        assert_eq!(options.dump_hour, Some(42));
        assert!(matches!(options.mode, TimelineMode::Recording { .. }));
    }

    #[test]
    fn cli_frame_cache_shrinks_recording_budget() {
        // F2: the weak-PC knob threads through the existing mode/config path.
        let options = parse_cli(&args_with_bundle(&[
            "--recording",
            "some/session.battleplan",
            "--frame-cache",
            "4",
        ]))
        .expect("args parse");
        assert_eq!(
            options.mode,
            TimelineMode::Recording {
                path: PathBuf::from("some/session.battleplan"),
                timeline: replay_core::TimelineConfig {
                    frame_cache_entries: 4
                },
            }
        );
        // Default stays 16 when the flag is absent (no silent global).
        assert_eq!(
            replay_core::TimelineConfig::default().frame_cache_entries,
            16
        );
    }

    #[test]
    fn cli_frame_cache_fails_closed() {
        // Zero would silently disable caching in the product; rejected.
        let err = parse_cli(&args_with_bundle(&[
            "--recording",
            "r.battleplan",
            "--frame-cache",
            "0",
        ]))
        .expect_err("must fail");
        assert!(
            err.to_string().contains("--frame-cache must be >= 1"),
            "{err}"
        );
        // A budget with nothing to budget is an error, not a no-op.
        let err = parse_cli(&args_with_bundle(&["--frame-cache", "4"])).expect_err("must fail");
        assert!(
            err.to_string()
                .contains("--frame-cache requires --recording"),
            "{err}"
        );
        // Non-numeric budgets are rejected with the offending value named.
        let err = parse_cli(&args_with_bundle(&[
            "--recording",
            "r.battleplan",
            "--frame-cache",
            "many",
        ]))
        .expect_err("must fail");
        assert!(err.to_string().contains("invalid entry count"), "{err}");
    }

    #[test]
    fn cli_parses_capture_dir() {
        let options = parse_cli(&args_with_bundle(&[
            "--capture",
            "captures/session_20260702d",
        ]))
        .expect("args parse");
        assert_eq!(
            options.capture_dir,
            Some(PathBuf::from("captures/session_20260702d"))
        );
    }

    #[test]
    fn cli_parses_dump_zoom_and_center() {
        let options = parse_cli(&args_with_bundle(&[
            "--dump-frame",
            "out.bmp",
            "--hour",
            "0.75",
            "--dump-zoom",
            "2.5",
            "--dump-center",
            "3000,600",
        ]))
        .expect("args parse");
        assert_eq!(options.day_night_hour, Some(0.75));
        assert_eq!(options.dump_zoom, Some(2.5));
        assert_eq!(options.dump_center, Some((3000.0, 600.0)));
    }

    #[test]
    fn cli_dump_hour_is_canonical_and_nth_hour_resolves_the_recording_index() {
        let recording = fixture_root().join("positive/fx_owner_controller_flip.battleplan");
        let recording_arg = recording.to_string_lossy().into_owned();
        let source = replay_core::open_recording(&recording).expect("fixture source");
        let hours = source.hours().to_vec();

        let mut canonical = parse_cli(&args(&[
            "--recording",
            &recording_arg,
            "--dump-frame",
            "out.bmp",
            "--dump-hour",
            &hours[1].to_string(),
        ]))
        .expect("canonical dump args");
        resolve_headless_recording_hour(&mut canonical).expect("canonical selection");
        assert_eq!(canonical.dump_hour, Some(hours[1]));

        let mut ordinal = parse_cli(&args(&[
            "--recording",
            &recording_arg,
            "--dump-frame",
            "out.bmp",
            "--dump-nth-hour",
            "1",
        ]))
        .expect("ordinal dump args");
        resolve_headless_recording_hour(&mut ordinal).expect("ordinal selection");
        assert_eq!(ordinal.dump_hour, Some(hours[1]));
        assert_eq!(ordinal.dump_nth_hour, Some(1));

        let both = parse_cli(&args(&[
            "--recording",
            &recording_arg,
            "--dump-frame",
            "out.bmp",
            "--dump-hour",
            &hours[0].to_string(),
            "--dump-nth-hour",
            "0",
        ]))
        .expect_err("competing selectors must fail");
        assert!(both
            .to_string()
            .contains("either --dump-hour or --dump-nth-hour"));
    }

    #[test]
    fn cli_parses_dump_hdr_and_requires_dump_frame() {
        let options = parse_cli(&args_with_bundle(&[
            "--dump-frame",
            "out.bmp",
            "--dump-hdr",
            "scene.rgba16f",
        ]))
        .expect("args parse");
        assert_eq!(options.dump_hdr, Some(PathBuf::from("scene.rgba16f")));
        assert_eq!(
            parse_cli(&args_with_bundle(&[]))
                .expect("bundle-only parse")
                .dump_hdr,
            None
        );
        let err = parse_cli(&args(&["--dump-hdr", "scene.rgba16f"])).expect_err("must fail");
        assert!(
            err.to_string().contains("--dump-hdr requires --dump-frame"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn cli_rejects_gb_cpu_escape_hatch() {
        let err = parse_cli(&args(&["--gb-cpu"])).expect_err("must fail");
        assert!(err.to_string().contains("--gb-cpu is blocked"), "{err}");
    }

    #[test]
    fn cli_tree_season_date_patch_product_path_blocked() {
        let err = parse_cli(&args(&["--tree-seasons-by-date"])).expect_err("must fail");
        assert!(
            err.to_string()
                .contains("--tree-seasons-by-date is blocked"),
            "{err}"
        );
    }

    #[test]
    fn cli_rejects_no_trees_second_subdraw_opt_out() {
        let err = parse_cli(&args(&["--no-trees-second-subdraw"])).expect_err("must fail");
        assert!(
            err.to_string()
                .contains("--no-trees-second-subdraw is removed"),
            "{err}"
        );
    }

    #[test]
    fn cli_trees_from_files_product_path_blocked() {
        let err = parse_cli(&args(&["--trees-from-files"])).expect_err("must fail");
        assert!(
            err.to_string().contains("--trees-from-files is blocked"),
            "{err}"
        );
    }

    #[test]
    fn cli_building_preview_product_path_blocked() {
        let explicit = parse_cli(&args(&["--buildings-from-files"])).expect_err("must fail");
        assert!(
            explicit
                .to_string()
                .contains("--buildings-from-files is blocked"),
            "{explicit}"
        );

        let err = parse_cli(&args(&["--buildings-preview-all"])).expect_err("must fail");
        assert!(
            err.to_string()
                .contains("--buildings-preview-all is blocked"),
            "{err}"
        );
    }

    #[test]
    fn cli_freeze_clock_product_path_blocked() {
        let err = parse_cli(&args(&["--freeze-clock"])).expect_err("must fail");
        assert!(
            err.to_string().contains("--freeze-clock is blocked"),
            "{err}"
        );
    }

    #[test]
    fn cli_water_full_defaults_on_and_blocks_old_approximation() {
        let defaults = parse_cli(&args_with_bundle(&[])).expect("bundle-only parse");
        assert!(defaults.water_full, "water-full defaults ON");

        let explicit = parse_cli(&args_with_bundle(&["--water-full"])).expect("args parse");
        assert!(explicit.water_full);

        let err = parse_cli(&args(&["--no-water-full"])).expect_err("must fail");
        assert!(
            err.to_string().contains("--no-water-full is blocked"),
            "{err}"
        );
    }

    #[test]
    fn cli_map_labels_product_path_blocked() {
        let explicit = parse_cli(&args(&["--map-labels"])).expect_err("must fail");
        assert!(
            explicit.to_string().contains("--map-labels is blocked"),
            "{explicit}"
        );

        let opt_out = parse_cli(&args(&["--no-map-labels"])).expect_err("must fail");
        assert!(
            opt_out.to_string().contains("--no-map-labels is blocked"),
            "{opt_out}"
        );

        let alias = parse_cli(&args(&["--map-labels-fallback"])).expect_err("must fail");
        assert!(
            alias
                .to_string()
                .contains("--map-labels-fallback is blocked"),
            "{alias}"
        );

        let err = parse_cli(&args(&["--map-labels", "--no-map-labels"])).expect_err("must fail");
        assert!(err.to_string().contains("--map-labels is blocked"), "{err}");
    }

    #[test]
    fn cli_map_furniture_product_path_blocked() {
        let explicit = parse_cli(&args(&["--map-furniture"])).expect_err("must fail");
        assert!(
            explicit.to_string().contains("--map-furniture is blocked"),
            "{explicit}"
        );

        let opt_out = parse_cli(&args(&["--no-map-furniture"])).expect_err("must fail");
        assert!(
            opt_out
                .to_string()
                .contains("--no-map-furniture is blocked"),
            "{opt_out}"
        );
        let err =
            parse_cli(&args(&["--map-furniture", "--no-map-furniture"])).expect_err("must fail");
        assert!(
            err.to_string().contains("--map-furniture is blocked"),
            "{err}"
        );
    }

    #[test]
    fn cli_borders_from_files_product_path_blocked() {
        let explicit = parse_cli(&args(&["--borders-from-files"])).expect_err("must fail");
        assert!(
            explicit
                .to_string()
                .contains("--borders-from-files is blocked"),
            "{explicit}"
        );

        let off = parse_cli(&args(&["--no-borders-from-files"])).expect_err("must fail");
        assert!(
            off.to_string()
                .contains("--no-borders-from-files is blocked"),
            "{off}"
        );
        let both =
            parse_cli(&args(&["--water-full", "--borders-from-files"])).expect_err("must fail");
        assert!(
            both.to_string().contains("--borders-from-files is blocked"),
            "{both}"
        );
    }

    #[test]
    fn cli_synthesize_terrain_chunks_product_path_blocked() {
        let err = parse_cli(&args(&["--synthesize-terrain-chunks"])).expect_err("must fail");
        assert!(
            err.to_string()
                .contains("--synthesize-terrain-chunks is blocked"),
            "{err}"
        );

        let with_capture = parse_cli(&args(&[
            "--capture",
            "captures/session_20260703",
            "--synthesize-terrain-chunks",
        ]))
        .expect_err("must fail");
        assert!(
            with_capture
                .to_string()
                .contains("--synthesize-terrain-chunks is blocked"),
            "{with_capture}"
        );
    }

    #[test]
    fn cli_chunks_from_files_product_path_blocked() {
        let err =
            parse_cli(&args(&["--chunks-from-files", "--game", "F:/HOI4"])).expect_err("must fail");
        assert!(
            err.to_string().contains("--chunks-from-files is blocked"),
            "{err}"
        );
        let with_synthesis = parse_cli(&args(&[
            "--capture",
            "captures/session_20260703",
            "--synthesize-terrain-chunks",
            "--chunks-from-files",
        ]))
        .expect_err("must fail");
        assert!(
            with_synthesis
                .to_string()
                .contains("--synthesize-terrain-chunks is blocked"),
            "{with_synthesis}"
        );
    }

    #[test]
    fn cli_rejects_dump_camera_without_dump_frame() {
        let err = parse_cli(&args(&["--dump-zoom", "2.5"])).expect_err("must fail");
        assert!(
            err.to_string()
                .contains("--dump-zoom requires --dump-frame"),
            "{err}"
        );

        let err = parse_cli(&args(&["--hour", "0.5"])).expect_err("must fail");
        assert!(
            err.to_string().contains("--hour requires --dump-frame"),
            "{err}"
        );

        let err = parse_cli(&args(&["--dump-center", "3000,600"])).expect_err("must fail");
        assert!(
            err.to_string()
                .contains("--dump-center requires --dump-frame"),
            "{err}"
        );
    }

    #[test]
    fn cli_rejects_bad_dump_camera_values() {
        let err = parse_cli(&args(&["--dump-frame", "out.bmp", "--dump-zoom", "0"]))
            .expect_err("must fail");
        assert!(err.to_string().contains("--dump-zoom must be positive"));

        let err = parse_cli(&args(&["--dump-frame", "out.bmp", "--dump-center", "3000"]))
            .expect_err("must fail");
        assert!(err.to_string().contains("--dump-center requires x,y"));

        let err =
            parse_cli(&args(&["--dump-frame", "out.bmp", "--hour", "1.0"])).expect_err("must fail");
        assert!(err.to_string().contains("--hour must be finite in [0,1)"));
    }

    #[test]
    fn cli_defaults_postfx_on() {
        let options = parse_cli(&args_with_bundle(&[])).expect("bundle-only parse");
        // The game's on-screen image is post-tonemap; postfx defaults ON.
        assert!(options.postfx_enabled);
        // Files-first RestoreScene grade is DEFAULT ON and fail-closed.
        assert!(
            options.postfx_restore_full,
            "GRAD1: restore-full defaults ON"
        );
        let err = parse_cli(&args(&["--no-postfx"])).expect_err("must fail");
        assert!(err.to_string().contains("--no-postfx is blocked"), "{err}");
    }

    #[test]
    fn cli_postfx_restore_full_blocks_opt_out() {
        let explicit =
            parse_cli(&args_with_bundle(&["--postfx-restore-full"])).expect("args parse");
        assert!(explicit.postfx_restore_full);

        let err = parse_cli(&args(&["--no-postfx-restore-full"])).expect_err("must fail");
        assert!(
            err.to_string()
                .contains("--no-postfx-restore-full is blocked"),
            "{err}"
        );

        let err = parse_cli(&args(&[
            "--postfx-restore-full",
            "--no-postfx-restore-full",
        ]))
        .expect_err("must fail");
        assert!(
            err.to_string()
                .contains("--no-postfx-restore-full is blocked"),
            "{err}"
        );
    }

    #[test]
    fn cli_rejects_capture_layer_opt_outs_and_parses_postfx_flags() {
        let options = parse_cli(&args_with_bundle(&["--postfx", "--postfx-restore-full"]))
            .expect("args parse");
        assert!(options.postfx_enabled);
        assert!(options.postfx_restore_full);

        let err = parse_cli(&args(&["--no-capture-water"])).expect_err("must fail");
        assert!(
            err.to_string().contains("--no-capture-water is blocked"),
            "{err}"
        );
        let err = parse_cli(&args(&["--no-capture-rivers"])).expect_err("must fail");
        assert!(
            err.to_string().contains("--no-capture-rivers is blocked"),
            "{err}"
        );
        let err = parse_cli(&args(&["--no-capture-trees"])).expect_err("must fail");
        assert!(
            err.to_string().contains("--no-capture-trees is blocked"),
            "{err}"
        );

        let err = parse_cli(&args(&["--postfx", "--no-postfx"])).expect_err("must fail");
        assert!(err.to_string().contains("--no-postfx is blocked"), "{err}");
    }

    #[test]
    fn cli_recording_with_demo_scrub_still_blocks_demo_scrub() {
        let err = parse_cli(&args(&["--recording", "r.battleplan", "--demo-scrub"]))
            .expect_err("must fail");
        assert!(err.to_string().contains("--demo-scrub is blocked"), "{err}");
    }

    #[test]
    fn cli_rejects_missing_recording_value() {
        let err = parse_cli(&args(&["--recording"])).expect_err("must fail");
        assert!(err.to_string().contains("--recording requires"), "{err}");
    }

    #[test]
    fn cli_rejects_missing_bundle_value() {
        let err = parse_cli(&args(&["--bundle"])).expect_err("must fail");
        assert!(err.to_string().contains("--bundle requires"), "{err}");
    }

    #[test]
    fn cli_rejects_missing_capture_value() {
        let err = parse_cli(&args(&["--capture"])).expect_err("must fail");
        assert!(err.to_string().contains("--capture requires"), "{err}");
    }

    #[test]
    fn cli_parses_game_dir() {
        let options =
            parse_cli(&args_with_bundle(&["--game", "some/install"])).expect("args parse");
        assert_eq!(options.game_dir, Some(PathBuf::from("some/install")));
        // Absent flag stays None; the env fallback happens in run().
        assert_eq!(
            parse_cli(&args_with_bundle(&[]))
                .expect("bundle-only parse")
                .game_dir,
            None
        );
    }

    #[test]
    fn cli_parses_repeated_mod_paths() {
        let options = parse_cli(&args_with_bundle(&[
            "--game",
            "some/install",
            "--mod",
            "mod/low",
            "--mod",
            "mod/high",
        ]))
        .expect("args parse");
        assert_eq!(
            options.game_files.mod_paths,
            vec![PathBuf::from("mod/low"), PathBuf::from("mod/high")]
        );
        assert!(!options.game_files.use_dlc_load);
    }

    #[test]
    fn cli_parses_dlc_load_selection() {
        let options = parse_cli(&args_with_bundle(&[
            "--use-dlc-load",
            "--userdata",
            "userdata",
        ]))
        .expect("args parse");
        assert!(options.game_files.use_dlc_load);
        assert_eq!(options.game_files.userdata, Some(PathBuf::from("userdata")));
    }

    #[test]
    fn cli_rejects_missing_game_value() {
        let err = parse_cli(&args(&["--game"])).expect_err("must fail");
        assert!(err.to_string().contains("--game requires"), "{err}");
    }

    #[test]
    fn cli_rejects_bad_mod_selection_flags() {
        let err = parse_cli(&args(&["--mod"])).expect_err("must fail");
        assert!(err.to_string().contains("--mod requires"), "{err}");

        let err = parse_cli(&args(&[
            "--mod",
            "mod/a",
            "--use-dlc-load",
            "--userdata",
            "u",
        ]))
        .expect_err("must fail");
        assert!(err.to_string().contains("either repeated --mod"), "{err}");

        let err = parse_cli(&args(&["--use-dlc-load"])).expect_err("must fail");
        assert!(err.to_string().contains("--userdata"), "{err}");
    }

    #[test]
    fn game_dir_flag_wins_over_environment() {
        let resolved = resolve_game_dir(
            Some(PathBuf::from("flag/dir")),
            Some(OsString::from("env/dir")),
        );
        assert_eq!(resolved, Some(PathBuf::from("flag/dir")));
    }

    #[test]
    fn game_dir_falls_back_to_environment() {
        let resolved = resolve_game_dir(None, Some(OsString::from("env/dir")));
        assert_eq!(resolved, Some(PathBuf::from("env/dir")));
        assert_eq!(resolve_game_dir(None, None), None);
    }

    #[test]
    fn mod_selection_requires_game_dir_or_environment() {
        let err = build_game_file_resolver(
            None,
            GameFileResolverOptions {
                mod_paths: vec![PathBuf::from("mod/a")],
                use_dlc_load: false,
                userdata: None,
            },
        )
        .expect_err("mod flags need game dir");
        assert!(err.to_string().contains("pass --game"), "{err}");
    }

    #[test]
    fn every_env_override_row_is_rejected_through_the_registry() {
        // Mirror of `every_blocked_cli_switch_arm_routes_through_registry` for
        // the env surface: each EnvOverride registry row must be rejected by
        // the single `reject_env_overrides` gate with the registry's own
        // message, so deleting a row's enforcement cannot pass silently. The
        // environment is closure-injected — never real process env vars
        // (parallel tests share the process environment).
        for b in blockers::REGISTRY {
            if let blockers::Surface::EnvOverride(name) = b.surface {
                let err = reject_env_overrides(|candidate| {
                    (candidate == name).then(|| OsString::from("1"))
                })
                .expect_err(&format!("{name} must be rejected when present"));
                assert_eq!(
                    err.to_string(),
                    b.reason,
                    "{name}: gate must return the registry reason verbatim"
                );
            }
        }
        reject_env_overrides(|_| None).expect("absent env overrides are fine");
    }

    #[test]
    fn cli_rejects_unknown_argument() {
        let err = parse_cli(&args(&["--frobnicate"])).expect_err("must fail");
        assert!(err.to_string().contains("unknown argument"), "{err}");
    }

    #[test]
    fn missing_bundle_fails_closed_with_generation_guidance() {
        let dir = crate::bundle::tests::unique_temp_dir("fail_closed").join("absent");
        let err = run(&args(&["--bundle", &dir.to_string_lossy()]))
            .expect_err("missing bundle must fail closed");
        let message = err.to_string();
        assert!(
            message.contains("failed to open debug/evidence map bundle"),
            "{message}"
        );
        assert!(
            message.contains("cargo run --release -p map_viewer -- bundle --game"),
            "{message}"
        );
    }

    fn fixture_root() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../..")
            .join("fixtures")
    }

    #[test]
    fn validate_recording_cli_opens_positive_fixture() {
        let path = fixture_root().join("positive/fx_owner_controller_flip.battleplan");
        run(&args(&["--validate-recording", &path.to_string_lossy()]))
            .expect("positive fixture must validate-open through the CLI");
    }

    #[test]
    fn validate_recording_cli_surfaces_negative_rejection() {
        // The CLI path surfaces the specific validator string for each negative.
        let cases = [
            ("neg_malformed_manifest", "required_features"),
            // stitching fix6 CRC-32-at-open catches the corrupted payload before the sha256 check.
            ("neg_bad_snapshot_hash", "crc32 mismatch"),
            (
                "neg_modset_mismatch",
                "disagrees with snapshot provenance.mod",
            ),
            (
                "neg_empty_snapshot_index",
                "incompatible replay index schema_version",
            ),
        ];
        for (name, expected) in cases {
            let path = fixture_root().join(format!("negative/{name}.battleplan"));
            let err = run(&args(&["--validate-recording", &path.to_string_lossy()]))
                .expect_err("negative fixture must be rejected through the CLI");
            assert!(
                err.to_string().contains(expected),
                "{name}: expected {expected:?}, got {err}"
            );
        }
    }

    #[test]
    fn validate_recording_cli_requires_a_path() {
        let err = run(&args(&["--validate-recording"])).expect_err("must fail");
        assert!(
            err.to_string()
                .contains("--validate-recording requires a path"),
            "{err}"
        );
    }

    /// G3 coverage gate: every `*_product_path_blocked` gate defined in the
    /// crate must be referenced by a `blockers::REGISTRY` row's
    /// `product_path_test`, and the registry must reference no phantom test — so
    /// a new product-path block cannot be added without a registry row.
    #[test]
    fn registry_covers_every_product_path_gate() {
        use std::collections::BTreeSet;
        use std::path::Path;

        const NEEDLE: &str = "_product_path_blocked";

        fn scan_file(path: &Path, gate_symbols: &mut BTreeSet<String>) {
            let src = std::fs::read_to_string(path).unwrap_or_else(|error| {
                panic!("read blocker coverage source {}: {error}", path.display())
            });
            let bytes = src.as_bytes();
            for (idx, _) in src.match_indices(NEEDLE) {
                let end = idx + NEEDLE.len();
                let mut start = idx;
                while start > 0 {
                    let c = bytes[start - 1];
                    if c.is_ascii_alphanumeric() || c == b'_' {
                        start -= 1;
                    } else {
                        break;
                    }
                }
                if src[..start].trim_end().ends_with("fn") {
                    gate_symbols.insert(src[start..end].to_string());
                }
            }
        }

        fn scan_tree(path: &Path, gate_symbols: &mut BTreeSet<String>) {
            for entry in std::fs::read_dir(path).unwrap_or_else(|error| {
                panic!("read blocker coverage tree {}: {error}", path.display())
            }) {
                let entry = entry.expect("read blocker coverage directory entry");
                let path = entry.path();
                if path.is_dir() {
                    scan_tree(&path, gate_symbols);
                } else if path.extension().and_then(|value| value.to_str()) == Some("rs") {
                    scan_file(&path, gate_symbols);
                }
            }
        }

        // Product blockers also live at renderer/runtime seams. Scan the
        // complete crate tree instead of treating main.rs as the whole product.
        let mut gate_symbols = BTreeSet::new();
        scan_tree(
            &Path::new(env!("CARGO_MANIFEST_DIR")).join("src"),
            &mut gate_symbols,
        );
        assert_eq!(
            gate_symbols.len(),
            11,
            "expected 11 *_product_path_blocked gates across src/, found {gate_symbols:?}"
        );
        // The set the registry claims to cover.
        let covered: BTreeSet<String> = blockers::REGISTRY
            .iter()
            .filter_map(|b| b.product_path_test)
            .map(str::to_string)
            .collect();
        for sym in &gate_symbols {
            assert!(
                covered.contains(sym),
                "product-path gate `{sym}` has no blockers::REGISTRY row referencing it"
            );
        }
        for cov in &covered {
            assert!(
                gate_symbols.contains(cov),
                "registry references `{cov}` but no such *_product_path_blocked gate exists"
            );
        }
        assert_eq!(
            gate_symbols, covered,
            "gate set must equal registry-covered set"
        );
    }

    #[test]
    fn list_blockers_cli_runs_without_a_bundle() {
        // The G3 emission is a standalone entry: no map bundle required.
        run(&args(&["--list-blockers"])).expect("--list-blockers must succeed standalone");
    }

    #[test]
    fn every_blocked_cli_switch_arm_routes_through_registry() {
        // Each blocked CLI-switch registry row must actually be rejected by
        // parse_cli with the registry's own message (indirection is wired, and
        // no gate was weakened). Env rows are exercised by their own unit tests.
        for b in blockers::REGISTRY {
            if let blockers::Surface::CliSwitch(token) = b.surface {
                let err = parse_cli(&args(&["--bundle", "some/dir", token]))
                    .expect_err(&format!("{token} must be rejected"));
                assert_eq!(
                    err.to_string(),
                    b.reason,
                    "{token}: gate must return the registry reason verbatim"
                );
            }
        }
    }

    #[test]
    fn undersized_bundle_fails_product_validation() {
        let dir = crate::bundle::tests::unique_temp_dir("dims_cli");
        crate::bundle::tests::write_fixture_bundle(&dir);
        let err = run(&args(&["--bundle", &dir.to_string_lossy()]))
            .expect_err("8x4 bundle must fail the product gate");
        assert!(err.to_string().contains("failed validation"), "{err}");
        std::fs::remove_dir_all(&dir).ok();
    }
}
