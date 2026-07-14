//! Battle Plan product surface: Win32 + D3D11 swapchain app.
//!
//! Product map rendering enters through validated the game D3D11 capture bundles.
//! The replay app keeps the map bundle's province/state data for picking and
//! snapshot metadata, but it does not render the removed CPU `map_visual.rgbx`
//! scaffold or generated political-color SRV substitutions.
//!
//! Usage: `replay_app [--bundle <dir>] [--game <dir>] [--mod <content-dir> ...]
//! [--use-dlc-load --userdata <dir>] [--recording <path>]
//! [--capture <session dir>]`
//!
//! `--recording` opens a snapshot-flavor `.battleplan` (or snapshot store
//! directory) through `replay_core` and drives the map from its recorded
//! state owners. Without it the map shows the grounded 1936 owners.
//! Replay-driven political color substitution is blocked until the engine's
//! primary fill / exact GradientBorder generation law is ported.
//! `--capture` loads a validated map_viewer captured-pass session and uses
//! the game's captured pdxmap/pdxwater shaders as the map base layer; invalid
//! captured sessions fail startup with the validator message.
//!
//! `--game` points at the game install (falls back to the `BATTLE_PLAN_VANILLA`
//! environment variable) and enables the real DDS-backed `hoi_18mbs`
//! overlay font. Without it the app still runs, but overlay text is omitted
//! instead of rendered with a proxy font.
//!
//! Controls: middle/right-drag or WASD pan; mouse wheel zooms toward the
//! cursor; `R` resets to whole-map fit; `[`/`]` or Left/Right step one
//! timeline hour; Space toggles ~4 hours/sec autoplay; left click/drag on
//! the scrub bar seeks the timeline; Escape quits.

mod anomaly;
mod blockers;
mod bundle;
mod camera;
mod clausewitz;
#[cfg(test)]
mod e2e_playback;
mod font;
mod grading_law;
mod observer;
mod palette;
#[cfg(test)]
mod post_effect;
mod timeline;
mod transport;
mod validate;

#[cfg(windows)]
mod app;
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
mod text;
#[cfg(windows)]
mod win32;

use std::error::Error;
use std::ffi::OsString;
use std::path::PathBuf;

use blockers::BlockerId;
use map_viewer::{sensitive_file_overrides, GameFileResolver, GameFileResolverOptions};

pub type Result<T> = std::result::Result<T, Box<dyn Error>>;

const USAGE: &str = "usage: replay_app [--list-blockers] [--validate-recording <path>] [--bundle <dir>] [--game <the game install dir>] \
[--mod <content-dir> ...] [--use-dlc-load --userdata <dir>] [--recording <path> [--frame-cache <entries>]] [--capture <session dir>] [--dump-frame <out.bmp> [--dump-hour <N>] \
[--hour <0..1>] [--dump-zoom <screen-px-per-map-px>] [--dump-center <x,y>] [--dump-hdr <out.rgba16f>]] \
    [--postfx] [--postfx-restore-full] [--postfx-validate-capture] [--water-full]\n\
--frame-cache <entries> shrinks the recording's decoded-frame LRU budget for weak-PC profiles (default 16 hot frames; requires --recording); \
--game falls back to the BATTLE_PLAN_VANILLA environment variable; \
--mod is repeatable and --use-dlc-load mirrors map_viewer bundle semantics with --userdata; \
--capture loads map_viewer/out/captures session bundles and is required for map rendering; the removed CPU map_visual.rgbx scaffold is blocked from product output; \
--demo-scrub is blocked because it generated synthetic, non-replay state changes; \
terrain chunk synthesis is blocked in replay_app until the remaining runtime/LOD state is engine-derived; use map_viewer terrain tooling only for development; --synthesize-terrain-chunks and --chunks-from-files are blocked; \
--gb-cpu and REPLAY_APP_GB_CPU are blocked; generated political GradientBorder textures are research-only and not used for product rendering; \
--postfx enables the ported scene-restore tone map (default ON: the game's on-screen image is post-tonemap; --no-postfx is blocked); \
--postfx-restore-full (DEFAULT ON) composes the RestoreScene grade from installed content via the mod-aware resolver and fails closed when evidence is missing; --no-postfx-restore-full is blocked; \
--postfx-validate-capture loads the captured restore-pass bundle and reports file-law vs captured deltas (LUT max delta, MiddleGrey/AverageLuminance diffs) then exits; a regression gate, never a render input (requires --capture and --game); \
--freeze-clock is blocked because replay_app preserves captured map-clock constant-buffer bytes until real runtime timing is capture-derived; \
--water-full (DEFAULT ON) uploads only the real engine/file-derived water path and fails closed when required inputs are missing; --no-water-full is blocked because it selected the old app-owned approximation; \
borders are blocked in replay_app until the remaining thin-border fade/loop/sea-region law is ported; use map_viewer border dumps as development tooling; --borders-from-files is blocked; \
country labels are blocked in replay_app only on owner-vs-controller keying and per-label Transp_OffsetX.x runtime values; the product glyph-VB path exists but --map-labels and --map-labels-fallback remain blocked; \
map furniture is blocked in replay_app only on highlight ring rendering and the 0.40 map-text scale origin; the relation-column law is engine-traced in map_viewer, but non-neutral capture coverage is still pending; the furniture module/projection API exists but --map-furniture remains blocked; \
tree file-derived replacement layers are blocked until scatter/rotation/model-selection/season-CB law is engine-derived; --trees-from-files and --tree-seasons-by-date are blocked; \
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
    bundle_dir: PathBuf,
    mode: TimelineMode,
    /// the game install for fonts (`--game`; `BATTLE_PLAN_VANILLA` env fallback).
    game_dir: Option<PathBuf>,
    game_files: GameFileResolverOptions,
    /// Headless: render one hour to this BMP and exit (no window).
    dump_frame: Option<PathBuf>,
    dump_hour: Option<i64>,
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
    let mut dump_frame = None;
    let mut dump_hour = None;
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
        return Err(
            format!("pass --userdata <the game user-data dir> with --use-dlc-load\n{USAGE}").into(),
        );
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
            TimelineMode::Static1936
        }
    };
    Ok(CliOptions {
        bundle_dir: bundle_dir.ok_or_else(|| {
            format!("--bundle <dir> is required; no implicit generated bundle is loaded\n{USAGE}")
        })?,
        mode,
        game_dir,
        game_files: GameFileResolverOptions {
            mod_paths,
            use_dlc_load,
            userdata,
        },
        dump_frame,
        dump_hour,
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

fn run(args: &[String]) -> Result<()> {
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
    // Fail closed: no bundle, no window; nothing is fabricated.
    let bundle = bundle::MapBundle::load(&options.bundle_dir).map_err(|err| {
        format!(
            "failed to open map bundle at {}: {}\n{}",
            options.bundle_dir.display(),
            err,
            bundle::generation_guidance()
        )
    })?;
    bundle.validate_product_dimensions().map_err(|err| {
        format!(
            "map bundle at {} failed validation: {}\n{}",
            options.bundle_dir.display(),
            err,
            bundle::generation_guidance()
        )
    })?;
    let game_dir = resolve_game_dir(options.game_dir, std::env::var_os("BATTLE_PLAN_VANILLA").or_else(|| std::env::var_os("HOI4_VANILLA")));
    let game_resolver = build_game_file_resolver(game_dir, options.game_files)?;
    log_game_file_resolver_warnings(game_resolver.as_ref())?;
    reject_gb_cpu_env(std::env::var_os("REPLAY_APP_GB_CPU"))?;
    reject_product_debug_env("REPLAY_DEBUG_DUMP_UNDERWATER")?;
    reject_product_debug_env("REPLAY_DEBUG_CLEAR_BEFORE_WATER")?;
    reject_product_debug_env("REPLAY_DEBUG_UNDERWATER_INPUTS")?;
    reject_product_debug_env("REPLAY_DEBUG_CONSTANT_REFRACTION")?;
    reject_product_debug_env("REPLAY_DUMP_LABELS")?;
    // Map labels and furniture are product-blocked. Their constructors stay
    // fail-closed for validation-only callers.
    if options.postfx_validate_capture {
        let capture_dir = options.capture_dir.as_deref().ok_or(
            "--postfx-validate-capture requires --capture <session dir> with a restore-pass bundle",
        )?;
        let report = render::restore_full_postfx_validate_capture(
            game_resolver.as_ref(),
            capture_dir,
            bundle.width,
            bundle.height,
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
            &bundle,
            &options.mode,
            options.dump_hour,
            out,
            game_resolver.as_ref(),
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
            options.capture_dir,
            options.postfx_enabled,
            options.postfx_restore_full,
            options.water_full,
        ),
    }
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
            for warning in &report.warnings {
                eprintln!("replay_app: recording warning: {warning}");
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
    bundle: bundle::MapBundle,
    mode: TimelineMode,
    game_resolver: Option<GameFileResolver>,
    capture_dir: Option<PathBuf>,
    postfx_enabled: bool,
    postfx_restore_full: bool,
    water_full: bool,
) -> Result<()> {
    app::run(
        bundle,
        mode,
        game_resolver,
        capture_dir,
        postfx_enabled,
        postfx_restore_full,
        water_full,
    )
}

#[cfg(windows)]
fn dump_frame(
    bundle: &bundle::MapBundle,
    mode: &TimelineMode,
    hour: Option<i64>,
    out: &std::path::Path,
    game_resolver: Option<&GameFileResolver>,
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
        out,
        game_resolver,
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
    _bundle: bundle::MapBundle,
    _mode: TimelineMode,
    _game_resolver: Option<GameFileResolver>,
    _capture_dir: Option<PathBuf>,
    _postfx_enabled: bool,
    _postfx_restore_full: bool,
    _water_full: bool,
) -> Result<()> {
    Err("replay_app is a Win32 + D3D11 application and requires Windows".into())
}

#[cfg(not(windows))]
fn dump_frame(
    _bundle: &bundle::MapBundle,
    _mode: &TimelineMode,
    _hour: Option<i64>,
    _out: &std::path::Path,
    _game_resolver: Option<&GameFileResolver>,
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

fn reject_gb_cpu_env(value: Option<std::ffi::OsString>) -> Result<()> {
    if value.is_some() {
        return Err(blockers::reason(BlockerId::GbCpuEnv).into());
    }
    Ok(())
}

fn reject_product_debug_env(name: &str) -> Result<()> {
    reject_product_debug_env_value(name, std::env::var_os(name))
}

fn reject_product_debug_env_value(name: &str, value: Option<std::ffi::OsString>) -> Result<()> {
    if value.is_some() {
        // The message is sourced from the canonical registry row for this env
        // var, keeping `blockers` the single source of the fail-closed text.
        let reason = blockers::env_reason_by_name(name).map(str::to_owned).unwrap_or_else(|| format!(
            "{name} is blocked in replay_app: debug and validation environment knobs must not mutate or side-effect the final product render path"
        ));
        return Err(reason.into());
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
    fn cli_requires_explicit_bundle_path() {
        let err = parse_cli(&[]).expect_err("empty args must fail");
        assert!(
            err.to_string().contains("--bundle <dir> is required"),
            "{err}"
        );
    }

    #[test]
    fn cli_parses_bundle_path() {
        let options = parse_cli(&args(&["--bundle", "some/dir"])).expect("args parse");
        assert_eq!(options.mode, TimelineMode::Static1936);
        assert_eq!(options.bundle_dir, PathBuf::from("some/dir"));
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
    fn gb_cpu_env_is_blocked_when_present() {
        let err = reject_gb_cpu_env(Some(std::ffi::OsString::from("0"))).expect_err("must fail");
        assert!(
            err.to_string().contains("REPLAY_APP_GB_CPU is blocked"),
            "{err}"
        );
        reject_gb_cpu_env(None).expect("absent env is fine");
    }

    #[test]
    fn product_debug_env_is_blocked_when_present() {
        reject_product_debug_env_value("REPLAY_DEBUG_CLEAR_BEFORE_WATER", None)
            .expect("absent env is fine");
        let err = reject_product_debug_env_value(
            "REPLAY_DEBUG_CLEAR_BEFORE_WATER",
            Some(std::ffi::OsString::from("1")),
        )
        .expect_err("debug env must fail");
        assert!(
            err.to_string()
                .contains("REPLAY_DEBUG_CLEAR_BEFORE_WATER is blocked"),
            "{err}"
        );
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
        assert!(message.contains("failed to open map bundle"), "{message}");
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
        run(&args(&[
            "--validate-recording",
            &path.to_string_lossy(),
        ]))
        .expect("positive fixture must validate-open through the CLI");
    }

    #[test]
    fn validate_recording_cli_surfaces_negative_rejection() {
        // The CLI path surfaces the specific validator string for each negative.
        let cases = [
            ("neg_malformed_manifest", "required_features"),
            // stitching fix6 CRC-32-at-open catches the corrupted payload before the sha256 check.
            ("neg_bad_snapshot_hash", "crc32 mismatch"),
            ("neg_modset_mismatch", "disagrees with snapshot provenance.mod"),
            ("neg_empty_snapshot_index", "empty snapshot index"),
        ];
        for (name, expected) in cases {
            let path = fixture_root().join(format!("negative/{name}.battleplan"));
            let err = run(&args(&[
                "--validate-recording",
                &path.to_string_lossy(),
            ]))
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
            err.to_string().contains("--validate-recording requires a path"),
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
        const NEEDLE: &str = "_product_path_blocked";
        let src = include_str!("main.rs");
        let bytes = src.as_bytes();
        // Collect every `fn <name>_product_path_blocked` definition in the crate.
        let mut gate_symbols: BTreeSet<&str> = BTreeSet::new();
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
            // Keep only true `fn NAME` definitions (skip string-literal mentions
            // like this test's own NEEDLE and any comment prose).
            if src[..start].trim_end().ends_with("fn") {
                gate_symbols.insert(&src[start..end]);
            }
        }
        assert_eq!(
            gate_symbols.len(),
            10,
            "expected 10 *_product_path_blocked gates, found {gate_symbols:?}"
        );
        // The set the registry claims to cover.
        let covered: BTreeSet<&str> = blockers::REGISTRY
            .iter()
            .filter_map(|b| b.product_path_test)
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
        assert_eq!(gate_symbols, covered, "gate set must equal registry-covered set");
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
