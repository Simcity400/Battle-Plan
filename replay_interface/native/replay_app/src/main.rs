//! Battle Plan product surface: Win32 + D3D11 swapchain app.
//!
//! Renders the political map over the grounded map-bundle scaffold per
//! `replay_interface/documents/REPLAY_ARCHITECTURE.md` (sequencing step 2)
//! and the Phase 2 architecture of `MAP_REPLICA_PLAN.md`: camera in map
//! pixels, GPU shaders own recoloring, a CPU mirror of `province_ids.r32`
//! owns hit testing, and snapshot scrubbing rewrites only the compact
//! state -> color table (never re-rasterizing the map on the CPU).
//!
//! Usage: `replay_app [--bundle <dir>] [--game <dir>] [--mod <content-dir> ...]
//! [--use-dlc-load --userdata <dir>] [--recording <path>] [--demo-scrub]
//! [--capture <session dir>]`
//!
//! `--recording` opens a snapshot-flavor `.battleplan` (or snapshot store
//! directory) through `replay_core` and drives the map from its recorded
//! state owners. Without it the map shows the grounded 1936 owners;
//! `--demo-scrub` runs a clearly-labeled synthetic scrub demo instead.
//! `--gb-cpu` or `REPLAY_APP_GB_CPU=1` forces the reference CPU
//! GradientBorderChannel generator for parity testing.
//! `--capture` loads a validated map_viewer captured-pass session and uses
//! the game's captured pdxmap/pdxwater shaders as the map base layer; invalid
//! captured sessions fail startup with the validator message.
//!
//! `--game` points at the game install (falls back to the `BATTLE_PLAN_VANILLA`
//! environment variable) and enables the real DDS-backed `hoi_18mbs`
//! overlay font. Without it the app still runs and falls back to the
//! former TGA proxy font when the local vanilla install is present.
//!
//! Controls: middle/right-drag or WASD pan; mouse wheel zooms toward the
//! cursor; `R` resets to whole-map fit; `[`/`]` or Left/Right step one
//! timeline hour; Space toggles ~4 hours/sec autoplay; left click/drag on
//! the scrub bar seeks the timeline; Escape quits.

#[cfg(windows)]
mod buildings;
mod bundle;
mod camera;
mod font;
mod grading_law;
mod palette;
mod post_effect;
mod timeline;

#[cfg(windows)]
mod app;
#[cfg(windows)]
mod d3d;
#[cfg(windows)]
mod gb_field;
#[cfg(windows)]
mod headless;
#[cfg(windows)]
mod map_furniture;
#[cfg(windows)]
mod map_labels;
#[cfg(windows)]
mod overlay;
#[cfg(windows)]
mod render;
#[cfg(windows)]
mod secondary_field;
#[cfg(windows)]
mod text;
#[cfg(windows)]
mod win32;

use std::error::Error;
use std::ffi::OsString;
use std::path::PathBuf;

use map_viewer::{sensitive_file_overrides, GameFileResolver, GameFileResolverOptions};

pub type Result<T> = std::result::Result<T, Box<dyn Error>>;

const USAGE: &str = "usage: replay_app [--bundle <dir>] [--game <the game install dir>] \
[--mod <content-dir> ...] [--use-dlc-load --userdata <dir>] [--recording <path>] [--demo-scrub] [--capture <session dir>] [--dump-frame <out.bmp> [--dump-hour <N>] \
[--hour <0..1>] [--dump-zoom <screen-px-per-map-px>] [--dump-center <x,y>] [--dump-hdr <out.rgba16f>]] [--gb-cpu] \
    [--synthesize-terrain-chunks] [--chunks-from-files] [--trees-from-files] [--no-trees-second-subdraw] [--buildings-from-files] [--buildings-preview-all] [--no-capture-water] [--no-capture-rivers] [--no-capture-trees] [--postfx|--no-postfx] [--postfx-restore-full|--no-postfx-restore-full] [--postfx-validate-capture] [--freeze-clock] [--water-full|--no-water-full] [--borders-from-files|--no-borders-from-files] [--map-labels|--no-map-labels] [--map-furniture|--no-map-furniture]\n\
--game falls back to the BATTLE_PLAN_VANILLA environment variable; \
--mod is repeatable and --use-dlc-load mirrors map_viewer bundle semantics with --userdata; \
--capture loads map_viewer/out/captures session bundles and fails closed on validation errors; \
--synthesize-terrain-chunks uses terrain_chunk_library.json to synthesize terrain-detail chunks; \
--chunks-from-files (CHUNK1, default OFF) makes --synthesize-terrain-chunks bind GENERATED chunk mesh buffers (65x65 grid slot0, checkerboard index, per-LOD slot1 height pool from the mod-resolvable map/heightmap.bmp, CB1) instead of the captured chunk template files; byte-identical to the captured law (fails closed on drift); requires --synthesize-terrain-chunks and --game/BATTLE_PLAN_VANILLA; \
--trees-from-files opts into resolver-loaded trees.bmp/.mesh Tree_tint/Tree_season layer (default OFF; requires --capture and --game/BATTLE_PLAN_VANILLA); \
--no-trees-second-subdraw (TREES2, default OFF) is the opt-out anchor for the historical trees second-sub-draw: pixel shader 6b77b6df9ba8f03d is gfx/FX/tree.shader:tree_low_gfx (the LOW_END_GFX variant), compiled by the engine but bound to no captured draw, so nothing renders for it either way and this flag leaves output byte-identical; \
--buildings-from-files opts into resolver-loaded map/buildings.txt + building .mesh preview (default OFF; requires --game/BATTLE_PLAN_VANILLA); --buildings-preview-all force-renders slot 0..show_on_map-1 per state for visual gates only; \
--gb-cpu or REPLAY_APP_GB_CPU=1 forces the reference CPU gradient-border path; \
--postfx enables the ported scene-restore tone map (default ON: the game's on-screen image is post-tonemap; --no-postfx shows the raw captured-RT look); \
--postfx-restore-full (GRAD1, DEFAULT ON) composes the RestoreScene grade (ColorCube LUT + exposure/MiddleGrey) FROM THE Installed content via the mod-aware resolver (posteffect_volumes.txt selection law + colorcorrection_*.tga + defines; see documents/GRADING_LAW.md); when explicitly passed it fails closed on a missing --game/TGA/selection key; on the default (no flag) a missing input logs a one-line downgrade and falls back to the old capture-anchored path; --no-postfx-restore-full restores the old default (no files-first grade); \
--postfx-validate-capture loads the captured restore-pass bundle and reports file-law vs captured deltas (LUT max delta, MiddleGrey/AverageLuminance diffs) then exits; a regression gate, never a render input (requires --capture and --game); \
--tree-seasons-by-date opts into the hypothesis tree CB1 season override for captured tree passes; modes without recording dates leave it inactive; \
--freeze-clock disables interactive wall-clock map animation for debugging; \
--water-full (WATER1, DEFAULT ON) uploads the real water reflection cube (byte-identical to the shipped, mod-resolvable map/terrain/reflection.dds) plus the mechanism-correct underwater refraction pass instead of the mean-color/approximation; when explicitly passed it fails closed without a --game resolver; on the default (no flag) a missing resolver logs a one-line downgrade and falls back to the old approximation; --no-water-full restores the old default (see documents/WATER_LAW.md); \
--borders-from-files / --no-borders-from-files (BORD5, DEFAULT ON) renders the thin-border ribbons (province/state/country/coast/impassable; sea-region skipped, geometry law underived) with the shipped mod-resolvable gfx/FX/border.shader and camera-height fade; the mesh follows the captured-engine chain law (session_BORDERZOOM_3; the drawn Brittany|Loire strip is a passing 32-vertex answer-key gate); an explicit --borders-from-files fails closed without a --game resolver, the default-ON path degrades to a one-line notice + borders off, and --no-borders-from-files restores the borderless render; \
--map-labels / --no-map-labels (LABGRAD1, DEFAULT ON) draws the reconstructed curved country-name labels, ONE PER CONTROLLED COMPONENT (each detached holding gets its own label; contiguous controlled territory merges), from the mod-resolvable map font (hoi_mapfont4), localisation and 00_graphics.lua DRAW_COUNTRY_NAMES_CUTOFF (see documents/MAP_LABEL_LAW.md); an explicit --map-labels fails closed without a --game resolver, the default-ON path degrades to a one-line notice + labels off, and --no-map-labels restores the byte-identical no-label render; \
--map-furniture / --no-map-furniture (FURN1, DEFAULT ON) draws the city/victory-point map furniture over the map — VP dots / capital stars from the shipped GFX_onmap_victorypoints_strip (neutral gray column; the engine's relation->color map is untraced) + hoi_20bs name text — reconstructed 100% from mod-resolvable installed content (history/states victory_points, history/countries capitals, localisation VICTORY_POINTS_<id>, interface/mapicons.gui/.gfx, 00_graphics.lua VP icon zoom cutoffs); an explicit --map-furniture fails closed without a --game resolver, the default-ON path degrades to a one-line notice + furniture off, and --no-map-furniture restores the byte-identical no-furniture render";

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

/// What drives the timeline: recorded state, the grounded 1936 baseline, or
/// the labeled synthetic scrub demo.
#[derive(Debug, PartialEq)]
enum TimelineMode {
    Static1936,
    DemoScrub,
    Recording(PathBuf),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct CliCaptureLayerConfig {
    water: bool,
    rivers: bool,
    trees: bool,
}

impl Default for CliCaptureLayerConfig {
    fn default() -> Self {
        Self {
            water: true,
            rivers: true,
            trees: true,
        }
    }
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
    gb_cpu: bool,
    synthesize_terrain_chunks: bool,
    capture_layers: CliCaptureLayerConfig,
    postfx_enabled: bool,
    /// GRAD1: files-first RestoreScene grade. DEFAULT ON. `--no-postfx-restore-full`
    /// restores the old capture-anchored default (no files-first grade).
    postfx_restore_full: bool,
    /// GRAD1: true only when the user EXPLICITLY passed `--postfx-restore-full`
    /// (never for the default-ON value, never when `--no-postfx-restore-full` was
    /// passed). Drives the fail-open-vs-fail-closed policy in render.rs: an
    /// explicit request keeps the hard error on a missing resolver/TGA/key; the
    /// default (no flag) downgrades to the old path with a one-line notice so a
    /// recording without grade evidence still renders.
    postfx_restore_full_explicit: bool,
    /// VALIDATION-ONLY: compose the files-first grade and report deltas against
    /// the captured restore bundle (requires --capture and --game), then exit.
    /// Never a render input.
    postfx_validate_capture: bool,
    tree_seasons_by_date: bool,
    trees_from_files: bool,
    /// TREES2 `--no-trees-second-subdraw`: opt-out anchor for the "second tree
    /// sub-draw" work. The 6b77b6df pixel shader the old warning flagged is
    /// `gfx/FX/tree.shader:tree_low_gfx` — a compiled-but-unused LOW_END_GFX
    /// variant that no captured session binds to a draw, so nothing is rendered
    /// for it either way and this flag leaves output byte-identical. It exists
    /// so the identity gate is explicit and so a future capture that genuinely
    /// binds the low-gfx PS can be told to leave it unrendered. Default OFF.
    no_trees_second_subdraw: bool,
    buildings_from_files: bool,
    buildings_preview_all: bool,
    freeze_clock: bool,
    /// LABGRAD1: country-name map labels (map_labels.rs). DEFAULT ON. The opt-out
    /// `--no-map-labels` sets this false and restores the byte-identical
    /// no-label render path.
    map_labels: bool,
    /// LABGRAD1: true only when the user EXPLICITLY asked for labels
    /// (`--map-labels` / the `--map-labels-fallback` back-compat alias), never
    /// for the default-ON value and never when `--no-map-labels` was passed.
    /// Drives fail-closed (explicit) vs fail-open (default) on a missing --game
    /// resolver in map_labels::create_if_enabled.
    map_labels_explicit: bool,
    /// FURN1: the city/VP map-furniture composite (map_furniture.rs — VP dots,
    /// capital stars, hoi_20bs name text). DEFAULT ON. The opt-out
    /// `--no-map-furniture` sets this false and restores the byte-identical
    /// no-furniture render path.
    map_furniture: bool,
    /// FURN1: true only when the user EXPLICITLY passed `--map-furniture`,
    /// never for the default-ON value and never with `--no-map-furniture`.
    /// Drives fail-closed (explicit) vs fail-open (default) on a missing
    /// --game resolver in map_furniture::create_if_enabled.
    map_furniture_explicit: bool,
    /// WATER1/GRAD1 `--water-full`: upload the real reflection cube + render the
    /// mechanism-correct underwater refraction pass instead of the mean-color/
    /// approximation. DEFAULT ON. `--no-water-full` restores the old default.
    water_full: bool,
    /// GRAD1: true only when the user EXPLICITLY passed `--water-full` (never for
    /// the default-ON value, never when `--no-water-full` was passed). Drives the
    /// fail-open-vs-fail-closed policy in render.rs: an explicit request keeps the
    /// hard error on a missing resolver; the default (no flag) downgrades to the
    /// old approximation with a one-line notice.
    water_full_explicit: bool,
    /// BORD5 `--borders-from-files` (DEFAULT ON): render the synthesized
    /// thin-border mesh (province/state/country/coast/impassable; sea-region
    /// skipped — underived geometry law) over the terrain with the shipped
    /// border shader and camera-height fade. `--no-borders-from-files`
    /// restores the borderless render.
    borders_from_files: bool,
    /// True only when the user EXPLICITLY passed `--borders-from-files`.
    /// Drives fail-open (default, downgrade + notice) vs fail-closed
    /// (explicit, hard error) on a missing --game resolver in render.rs.
    borders_from_files_explicit: bool,
    /// CHUNK1 `--chunks-from-files`: with `--synthesize-terrain-chunks`, bind
    /// GENERATED terrain-chunk mesh buffers (slot0 grid, index tessellation,
    /// per-LOD slot1 height pool from map/heightmap.bmp, CB1) instead of the
    /// captured chunk template files. Default OFF keeps the render path
    /// byte-identical. Requires `--game` and `--synthesize-terrain-chunks`.
    chunks_from_files: bool,
}

fn parse_cli(args: &[String]) -> Result<CliOptions> {
    let mut bundle_dir = None;
    let mut game_dir = None;
    let mut demo_scrub = false;
    let mut recording = None;
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
    let mut gb_cpu = false;
    let mut synthesize_terrain_chunks = false;
    let mut capture_layers = CliCaptureLayerConfig::default();
    // The game's on-screen image is the scene-restore tonemap applied to the
    // linear HDR scene; captured RTs are pre-postfx, which is why raw
    // replays look dark. Default ON to match what the player actually sees;
    // --no-postfx reproduces the raw captured-RT look.
    let mut postfx_enabled = true;
    // GRAD1: the files-first RestoreScene grade is now DEFAULT ON (grading is
    // evidence-closed: LUT byte-exact vs capture, per-frame exposure, bloom
    // kernel matched to capture). `--no-postfx-restore-full` restores the old
    // capture-anchored default. `postfx_restore_full_explicit` records an
    // explicit `--postfx-restore-full` so render.rs can fail closed on missing
    // evidence only when the user asked for it (default => fail-open downgrade).
    let mut postfx_restore_full = true;
    let mut postfx_restore_full_explicit = false;
    let mut postfx_validate_capture = false;
    let mut tree_seasons_by_date = false;
    let mut trees_from_files = false;
    let mut no_trees_second_subdraw = false;
    let mut buildings_from_files = false;
    let mut buildings_preview_all = false;
    let mut freeze_clock = false;
    // LABGRAD1: country-name labels graduated to DEFAULT ON. The default value
    // is "on but NOT explicit" so create_if_enabled fails open (downgrades to
    // labels-off + notice) on a missing --game; an explicit --map-labels is
    // "on AND explicit" so it fails closed; --no-map-labels restores the old
    // default (off, byte-identical render).
    let mut map_labels = true;
    let mut map_labels_explicit = false;
    // FURN1: the city/VP furniture composite graduated to DEFAULT ON with the
    // same policy shape as labels: default = "on but NOT explicit" (fail-open
    // downgrade on a missing --game), --map-furniture = "on AND explicit"
    // (fail closed), --no-map-furniture = off (byte-identical render).
    let mut map_furniture = true;
    let mut map_furniture_explicit = false;
    // WATER1/GRAD1: --water-full uploads the real reflection cube (byte-identical
    // to the shipped map/terrain/reflection.dds) + renders the mechanism-correct
    // underwater refraction pass. Now DEFAULT ON (reflection cube 54/54
    // subresources byte-identical; underwater pass is the shipped pdxmap
    // `underwater` PS port). `--no-water-full` restores the old approximation.
    // `water_full_explicit` records an explicit `--water-full` so render.rs fails
    // closed on a missing resolver only when the user asked (default => downgrade).
    let mut water_full = true;
    let mut water_full_explicit = false;
    // BORD5: the synthesized thin-border pass graduated to DEFAULT ON — the
    // border geometry law is captured-engine-derived (session_BORDERZOOM_3;
    // the drawn Brittany|Loire strip is a passing answer-key gate) and the
    // pass renders the mesh through the shipped border.shader with the
    // camera-height fade. Same policy shape as labels/furniture/water:
    // default = "on but NOT explicit" (fail-open downgrade on a missing
    // --game), --borders-from-files = "on AND explicit" (fail closed),
    // --no-borders-from-files = off (byte-identical borderless render).
    let mut borders_from_files = true;
    let mut borders_from_files_explicit = false;
    // CHUNK1: --chunks-from-files binds generated chunk mesh buffers instead of
    // the captured template files. Default OFF => byte-identical replay.
    let mut chunks_from_files = false;
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
            "--demo-scrub" => {
                demo_scrub = true;
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
                gb_cpu = true;
            }
            "--synthesize-terrain-chunks" => {
                synthesize_terrain_chunks = true;
            }
            "--water-full" => {
                water_full = true;
                water_full_explicit = true;
            }
            "--no-water-full" => {
                // GRAD1 opt-out: restore the old capture-anchored default
                // (mean-color reflection + approximation refraction). Clears
                // `explicit` so this is unambiguously "user chose the old path".
                water_full = false;
                water_full_explicit = false;
            }
            "--borders-from-files" => {
                borders_from_files = true;
                borders_from_files_explicit = true;
            }
            "--no-borders-from-files" => {
                // BORD5 opt-out: restore the borderless render (byte-identical
                // to the pre-BORD5 default). Clears `explicit` so this is
                // unambiguously "user chose no borders".
                borders_from_files = false;
                borders_from_files_explicit = false;
            }
            "--chunks-from-files" => {
                chunks_from_files = true;
            }
            "--no-capture-water" => {
                capture_layers.water = false;
            }
            "--no-capture-rivers" => {
                capture_layers.rivers = false;
            }
            "--no-capture-trees" => {
                capture_layers.trees = false;
            }
            "--postfx" => {
                postfx_enabled = true;
            }
            "--no-postfx" => {
                postfx_enabled = false;
            }
            "--postfx-restore-full" => {
                postfx_restore_full = true;
                postfx_restore_full_explicit = true;
            }
            "--no-postfx-restore-full" => {
                // GRAD1 opt-out: restore the old capture-anchored default (no
                // files-first grade). Clears `explicit` so this is unambiguously
                // "user chose the old path", not a fail-open downgrade.
                postfx_restore_full = false;
                postfx_restore_full_explicit = false;
            }
            "--postfx-validate-capture" => {
                postfx_validate_capture = true;
            }
            "--tree-seasons-by-date" => {
                tree_seasons_by_date = true;
            }
            "--trees-from-files" => {
                trees_from_files = true;
            }
            "--no-trees-second-subdraw" => {
                no_trees_second_subdraw = true;
            }
            "--buildings-from-files" => {
                buildings_from_files = true;
            }
            "--buildings-preview-all" => {
                buildings_preview_all = true;
            }
            "--freeze-clock" => {
                freeze_clock = true;
            }
            "--map-labels" => {
                map_labels = true;
                map_labels_explicit = true;
            }
            "--no-map-labels" => {
                // LABGRAD1 opt-out: restore the pre-graduation default (labels
                // off, byte-identical render). Clears `explicit` so this is
                // unambiguously "user chose no labels".
                map_labels = false;
                map_labels_explicit = false;
            }
            "--map-labels-fallback" => {
                // Back-compat alias for the pre-graduation opt-in flag: same as
                // an explicit --map-labels (on + fail-closed on missing --game).
                map_labels = true;
                map_labels_explicit = true;
            }
            "--map-furniture" => {
                map_furniture = true;
                map_furniture_explicit = true;
            }
            "--no-map-furniture" => {
                // FURN1 opt-out: restore the pre-graduation default (furniture
                // off, byte-identical render).
                map_furniture = false;
                map_furniture_explicit = false;
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
    if synthesize_terrain_chunks && capture_dir.is_none() {
        return Err(format!("--synthesize-terrain-chunks requires --capture\n{USAGE}").into());
    }
    if chunks_from_files && !synthesize_terrain_chunks {
        return Err(format!(
            "--chunks-from-files requires --synthesize-terrain-chunks (it regenerates that path's chunk mesh)\n{USAGE}"
        )
        .into());
    }
    if chunks_from_files && game_dir.is_none() {
        return Err(format!(
            "--chunks-from-files requires --game <the game install dir> so map/heightmap.bmp resolves\n{USAGE}"
        )
        .into());
    }
    if trees_from_files && capture_dir.is_none() {
        return Err(format!("--trees-from-files requires --capture\n{USAGE}").into());
    }
    if postfx_restore_full && !postfx_enabled && !postfx_validate_capture {
        // The RestoreScene grade runs inside the postfx block; with --no-postfx
        // the composed grade is loaded but never applied. Warn rather than fail
        // so validation/diagnostic combos still run.
        eprintln!(
            "replay_app: warning: --postfx-restore-full with --no-postfx composes the files-first \
             grade but the postfx pass is disabled, so no grading is applied to the render"
        );
    }
    if buildings_preview_all && !buildings_from_files {
        return Err(
            format!("--buildings-preview-all requires --buildings-from-files\n{USAGE}").into(),
        );
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
    let mode = match (recording, demo_scrub) {
        (Some(_), true) => {
            return Err(
                format!("--recording and --demo-scrub are mutually exclusive\n{USAGE}").into(),
            )
        }
        (Some(path), false) => TimelineMode::Recording(path),
        (None, true) => TimelineMode::DemoScrub,
        (None, false) => TimelineMode::Static1936,
    };
    Ok(CliOptions {
        bundle_dir: bundle_dir.unwrap_or_else(default_bundle_dir),
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
        gb_cpu,
        synthesize_terrain_chunks,
        capture_layers,
        postfx_enabled,
        postfx_restore_full,
        postfx_restore_full_explicit,
        postfx_validate_capture,
        tree_seasons_by_date,
        trees_from_files,
        no_trees_second_subdraw,
        buildings_from_files,
        buildings_preview_all,
        freeze_clock,
        map_labels,
        map_labels_explicit,
        map_furniture,
        map_furniture_explicit,
        water_full,
        water_full_explicit,
        borders_from_files,
        borders_from_files_explicit,
        chunks_from_files,
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

/// Same convention as map_viewer's `default_bundle_dir()`, relative to
/// this crate so no machine paths are committed.
fn default_bundle_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("map_viewer")
        .join("out")
        .join("map_bundle")
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if let Err(err) = run(&args) {
        eprintln!("replay_app: {err}");
        std::process::exit(2);
    }
}

fn run(args: &[String]) -> Result<()> {
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
    if options.trees_from_files && game_dir.is_none() {
        return Err("--trees-from-files requires --game <the game install dir> or BATTLE_PLAN_VANILLA".into());
    }
    if options.buildings_from_files && game_dir.is_none() {
        return Err(
            "--buildings-from-files requires --game <the game install dir> or BATTLE_PLAN_VANILLA".into(),
        );
    }
    let game_resolver = build_game_file_resolver(game_dir, options.game_files)?;
    log_game_file_resolver_warnings(game_resolver.as_ref())?;
    if options.gb_cpu || gb_cpu_env_enabled() {
        eprintln!(
            "replay_app: --gb-cpu/REPLAY_APP_GB_CPU is deprecated and ignored; the \
             gradient-border country layer is always generated on the CPU now"
        );
    }
    // FURN1: --map-furniture/--no-map-furniture policy (fail-open default vs
    // fail-closed explicit) is enforced inside map_furniture::create_if_enabled
    // on the app/headless paths, mirroring map labels.
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
            &options.bundle_dir,
            &options.mode,
            options.dump_hour,
            out,
            game_resolver.as_ref(),
            options.dump_zoom,
            options.dump_center,
            options.dump_hdr.as_deref(),
            options.capture_dir.as_deref(),
            options.synthesize_terrain_chunks,
            options.capture_layers,
            options.postfx_enabled,
            options.postfx_restore_full,
            options.postfx_restore_full_explicit,
            options.day_night_hour,
            options.tree_seasons_by_date,
            options.trees_from_files,
            options.no_trees_second_subdraw,
            options.buildings_from_files,
            options.buildings_preview_all,
            options.map_labels,
            options.map_labels_explicit,
            options.map_furniture,
            options.map_furniture_explicit,
            options.water_full,
            options.water_full_explicit,
            options.borders_from_files,
            options.borders_from_files_explicit,
            options.chunks_from_files,
        ),
        None => run_app(
            bundle,
            options.bundle_dir,
            options.mode,
            game_resolver,
            options.capture_dir,
            options.synthesize_terrain_chunks,
            options.capture_layers,
            options.postfx_enabled,
            options.postfx_restore_full,
            options.postfx_restore_full_explicit,
            options.tree_seasons_by_date,
            options.trees_from_files,
            options.no_trees_second_subdraw,
            options.buildings_from_files,
            options.buildings_preview_all,
            options.freeze_clock,
            options.map_labels,
            options.map_labels_explicit,
            options.map_furniture,
            options.map_furniture_explicit,
            options.water_full,
            options.water_full_explicit,
            options.borders_from_files,
            options.borders_from_files_explicit,
            options.chunks_from_files,
        ),
    }
}

#[cfg(windows)]
fn run_app(
    bundle: bundle::MapBundle,
    bundle_dir: PathBuf,
    mode: TimelineMode,
    game_resolver: Option<GameFileResolver>,
    capture_dir: Option<PathBuf>,
    synthesize_terrain_chunks: bool,
    capture_layers: CliCaptureLayerConfig,
    postfx_enabled: bool,
    postfx_restore_full: bool,
    postfx_restore_full_explicit: bool,
    tree_seasons_by_date: bool,
    trees_from_files: bool,
    no_trees_second_subdraw: bool,
    buildings_from_files: bool,
    buildings_preview_all: bool,
    freeze_clock: bool,
    map_labels: bool,
    map_labels_explicit: bool,
    map_furniture: bool,
    map_furniture_explicit: bool,
    water_full: bool,
    water_full_explicit: bool,
    borders_from_files: bool,
    borders_from_files_explicit: bool,
    chunks_from_files: bool,
) -> Result<()> {
    app::run(
        bundle,
        bundle_dir,
        mode,
        game_resolver,
        capture_dir,
        synthesize_terrain_chunks,
        render_capture_layer_config(capture_layers),
        postfx_enabled,
        postfx_restore_full,
        postfx_restore_full_explicit,
        tree_seasons_by_date,
        trees_from_files,
        no_trees_second_subdraw,
        buildings_from_files,
        buildings_preview_all,
        freeze_clock,
        map_labels,
        map_labels_explicit,
        map_furniture,
        map_furniture_explicit,
        water_full,
        water_full_explicit,
        borders_from_files,
        borders_from_files_explicit,
        chunks_from_files,
    )
}

#[cfg(windows)]
fn dump_frame(
    bundle: &bundle::MapBundle,
    bundle_dir: &std::path::Path,
    mode: &TimelineMode,
    hour: Option<i64>,
    out: &std::path::Path,
    game_resolver: Option<&GameFileResolver>,
    dump_zoom: Option<f64>,
    dump_center: Option<(f64, f64)>,
    dump_hdr: Option<&std::path::Path>,
    capture_dir: Option<&std::path::Path>,
    synthesize_terrain_chunks: bool,
    capture_layers: CliCaptureLayerConfig,
    postfx_enabled: bool,
    postfx_restore_full: bool,
    postfx_restore_full_explicit: bool,
    day_night_hour: Option<f32>,
    tree_seasons_by_date: bool,
    trees_from_files: bool,
    no_trees_second_subdraw: bool,
    buildings_from_files: bool,
    buildings_preview_all: bool,
    map_labels: bool,
    map_labels_explicit: bool,
    map_furniture: bool,
    map_furniture_explicit: bool,
    water_full: bool,
    water_full_explicit: bool,
    borders_from_files: bool,
    borders_from_files_explicit: bool,
    chunks_from_files: bool,
) -> Result<()> {
    headless::dump_frame(
        bundle,
        bundle_dir,
        mode,
        hour,
        out,
        game_resolver,
        dump_zoom,
        dump_center,
        dump_hdr,
        capture_dir,
        synthesize_terrain_chunks,
        render_capture_layer_config(capture_layers),
        postfx_enabled,
        postfx_restore_full,
        postfx_restore_full_explicit,
        day_night_hour,
        tree_seasons_by_date,
        trees_from_files,
        no_trees_second_subdraw,
        buildings_from_files,
        buildings_preview_all,
        map_labels,
        map_labels_explicit,
        map_furniture,
        map_furniture_explicit,
        water_full,
        water_full_explicit,
        borders_from_files,
        borders_from_files_explicit,
        chunks_from_files,
    )
}

#[cfg(windows)]
fn render_capture_layer_config(config: CliCaptureLayerConfig) -> render::CapturedLayerConfig {
    render::CapturedLayerConfig {
        water: config.water,
        rivers: config.rivers,
        trees: config.trees,
    }
}

#[cfg(not(windows))]
fn run_app(
    _bundle: bundle::MapBundle,
    _bundle_dir: PathBuf,
    _mode: TimelineMode,
    _game_resolver: Option<GameFileResolver>,
    _capture_dir: Option<PathBuf>,
    _synthesize_terrain_chunks: bool,
    _capture_layers: CliCaptureLayerConfig,
    _postfx_enabled: bool,
    _postfx_restore_full: bool,
    _postfx_restore_full_explicit: bool,
    _tree_seasons_by_date: bool,
    _trees_from_files: bool,
    _no_trees_second_subdraw: bool,
    _buildings_from_files: bool,
    _buildings_preview_all: bool,
    _freeze_clock: bool,
    _map_labels: bool,
    _map_labels_explicit: bool,
    _map_furniture: bool,
    _map_furniture_explicit: bool,
    _water_full: bool,
    _water_full_explicit: bool,
    _borders_from_files: bool,
    _borders_from_files_explicit: bool,
    _chunks_from_files: bool,
) -> Result<()> {
    Err("replay_app is a Win32 + D3D11 application and requires Windows".into())
}

#[cfg(not(windows))]
fn dump_frame(
    _bundle: &bundle::MapBundle,
    _bundle_dir: &std::path::Path,
    _mode: &TimelineMode,
    _hour: Option<i64>,
    _out: &std::path::Path,
    _game_resolver: Option<&GameFileResolver>,
    _dump_zoom: Option<f64>,
    _dump_center: Option<(f64, f64)>,
    _dump_hdr: Option<&std::path::Path>,
    _capture_dir: Option<&std::path::Path>,
    _synthesize_terrain_chunks: bool,
    _capture_layers: CliCaptureLayerConfig,
    _postfx_enabled: bool,
    _postfx_restore_full: bool,
    _postfx_restore_full_explicit: bool,
    _day_night_hour: Option<f32>,
    _tree_seasons_by_date: bool,
    _trees_from_files: bool,
    _no_trees_second_subdraw: bool,
    _buildings_from_files: bool,
    _buildings_preview_all: bool,
    _map_labels: bool,
    _map_labels_explicit: bool,
    _map_furniture: bool,
    _map_furniture_explicit: bool,
    _water_full: bool,
    _water_full_explicit: bool,
    _borders_from_files: bool,
    _borders_from_files_explicit: bool,
    _chunks_from_files: bool,
) -> Result<()> {
    Err("replay_app frame dumps require Windows (D3D11)".into())
}

fn gb_cpu_env_enabled() -> bool {
    gb_cpu_env_value_enabled(std::env::var_os("REPLAY_APP_GB_CPU"))
}

fn gb_cpu_env_value_enabled(value: Option<OsString>) -> bool {
    value
        .and_then(|value| value.into_string().ok())
        .map(|value| {
            let normalized = value.trim().to_ascii_lowercase();
            matches!(normalized.as_str(), "1" | "true" | "yes" | "on")
        })
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(values: &[&str]) -> Vec<String> {
        values.iter().map(|value| value.to_string()).collect()
    }

    #[test]
    fn cli_defaults_to_map_viewer_bundle_dir() {
        let options = parse_cli(&[]).expect("empty args parse");
        assert_eq!(options.mode, TimelineMode::Static1936);
        assert!(options.bundle_dir.ends_with(
            std::path::Path::new("map_viewer")
                .join("out")
                .join("map_bundle")
        ));
    }

    #[test]
    fn cli_parses_bundle_and_demo_scrub() {
        let options =
            parse_cli(&args(&["--bundle", "some/dir", "--demo-scrub"])).expect("args parse");
        assert_eq!(options.mode, TimelineMode::DemoScrub);
        assert_eq!(options.bundle_dir, PathBuf::from("some/dir"));
    }

    #[test]
    fn cli_parses_recording_path() {
        let options =
            parse_cli(&args(&["--recording", "some/session.battleplan"])).expect("args parse");
        assert_eq!(
            options.mode,
            TimelineMode::Recording(PathBuf::from("some/session.battleplan"))
        );
    }

    #[test]
    fn cli_parses_capture_dir() {
        let options =
            parse_cli(&args(&["--capture", "captures/session_20260702d"])).expect("args parse");
        assert_eq!(
            options.capture_dir,
            Some(PathBuf::from("captures/session_20260702d"))
        );
    }

    #[test]
    fn cli_parses_dump_zoom_and_center() {
        let options = parse_cli(&args(&[
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
        let options = parse_cli(&args(&[
            "--dump-frame",
            "out.bmp",
            "--dump-hdr",
            "scene.rgba16f",
        ]))
        .expect("args parse");
        assert_eq!(options.dump_hdr, Some(PathBuf::from("scene.rgba16f")));
        assert_eq!(parse_cli(&[]).expect("empty parse").dump_hdr, None);
        let err = parse_cli(&args(&["--dump-hdr", "scene.rgba16f"])).expect_err("must fail");
        assert!(
            err.to_string().contains("--dump-hdr requires --dump-frame"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn cli_parses_gb_cpu_escape_hatch() {
        let options = parse_cli(&args(&["--gb-cpu"])).expect("args parse");
        assert!(options.gb_cpu);
        assert!(!parse_cli(&[]).expect("empty parse").gb_cpu);
    }

    #[test]
    fn cli_parses_tree_seasons_by_date() {
        let options = parse_cli(&args(&["--tree-seasons-by-date"])).expect("args parse");
        assert!(options.tree_seasons_by_date);
        assert!(!parse_cli(&[]).expect("empty parse").tree_seasons_by_date);
    }

    #[test]
    fn cli_parses_no_trees_second_subdraw_opt_out() {
        // TREES2 opt-out anchor: default OFF, parses ON, needs no capture/game
        // (it never changes render output — the low-gfx PS is bound to no
        // captured draw — so it is an unconstrained debug flag).
        let options = parse_cli(&args(&["--no-trees-second-subdraw"])).expect("args parse");
        assert!(options.no_trees_second_subdraw);
        assert!(!parse_cli(&[]).expect("empty parse").no_trees_second_subdraw);
    }

    #[test]
    fn cli_parses_trees_from_files_as_capture_gated_opt_in() {
        let options = parse_cli(&args(&[
            "--capture",
            "captures/live_PAN2",
            "--trees-from-files",
        ]))
        .expect("args parse");
        assert!(options.trees_from_files);
        assert!(!parse_cli(&[]).expect("empty parse").trees_from_files);
    }

    #[test]
    fn cli_rejects_trees_from_files_without_capture() {
        let err = parse_cli(&args(&["--trees-from-files"])).expect_err("must fail");
        assert!(
            err.to_string()
                .contains("--trees-from-files requires --capture"),
            "{err}"
        );
    }

    #[test]
    fn cli_parses_buildings_from_files_as_default_off_opt_in() {
        let defaults = parse_cli(&[]).expect("empty parse");
        assert!(!defaults.buildings_from_files);
        assert!(!defaults.buildings_preview_all);

        let options = parse_cli(&args(&["--buildings-from-files"])).expect("args parse");
        assert!(options.buildings_from_files);
        assert!(!options.buildings_preview_all);

        let all = parse_cli(&args(&[
            "--buildings-from-files",
            "--buildings-preview-all",
        ]))
        .expect("args parse");
        assert!(all.buildings_from_files);
        assert!(all.buildings_preview_all);
    }

    #[test]
    fn cli_rejects_buildings_preview_all_without_buildings_layer() {
        let err = parse_cli(&args(&["--buildings-preview-all"])).expect_err("must fail");
        assert!(
            err.to_string()
                .contains("--buildings-preview-all requires --buildings-from-files"),
            "{err}"
        );
    }

    #[test]
    fn cli_parses_freeze_clock() {
        let options = parse_cli(&args(&["--freeze-clock"])).expect("args parse");
        assert!(options.freeze_clock);
        assert!(!parse_cli(&[]).expect("empty parse").freeze_clock);
    }

    #[test]
    fn cli_water_full_defaults_on_with_explicit_opt_out() {
        // GRAD1: --water-full graduated to DEFAULT ON. The default value is
        // "on but NOT explicit" so render.rs fails open (downgrades) on a
        // missing resolver; an explicit --water-full is "on AND explicit" so it
        // fails closed; --no-water-full restores the old default (off).
        let defaults = parse_cli(&[]).expect("empty parse");
        assert!(defaults.water_full, "GRAD1: water-full defaults ON");
        assert!(
            !defaults.water_full_explicit,
            "default-ON must not be treated as an explicit request"
        );

        let explicit = parse_cli(&args(&["--water-full"])).expect("args parse");
        assert!(explicit.water_full);
        assert!(
            explicit.water_full_explicit,
            "an explicit --water-full must fail closed on missing evidence"
        );

        let opt_out = parse_cli(&args(&["--no-water-full"])).expect("args parse");
        assert!(!opt_out.water_full, "--no-water-full restores old default");
        assert!(!opt_out.water_full_explicit);

        // Last-wins ordering: --water-full then --no-water-full => off.
        let last_off = parse_cli(&args(&["--water-full", "--no-water-full"])).expect("args parse");
        assert!(!last_off.water_full);
        assert!(!last_off.water_full_explicit);
        // ...and --no-water-full then --water-full => on+explicit.
        let last_on = parse_cli(&args(&["--no-water-full", "--water-full"])).expect("args parse");
        assert!(last_on.water_full && last_on.water_full_explicit);
    }

    #[test]
    fn cli_map_labels_default_on_with_explicit_opt_out() {
        // LABGRAD1: country-name labels graduated to DEFAULT ON. The default is
        // "on but NOT explicit" so create_if_enabled fails open (downgrade) on a
        // missing --game; an explicit --map-labels is "on AND explicit" (fails
        // closed); --no-map-labels restores the old default (off).
        let defaults = parse_cli(&[]).expect("empty parse");
        assert!(defaults.map_labels, "LABGRAD1: map labels default ON");
        assert!(
            !defaults.map_labels_explicit,
            "default-ON must not be treated as an explicit request"
        );

        let explicit = parse_cli(&args(&["--map-labels"])).expect("args parse");
        assert!(explicit.map_labels && explicit.map_labels_explicit);

        let opt_out = parse_cli(&args(&["--no-map-labels"])).expect("args parse");
        assert!(!opt_out.map_labels, "--no-map-labels restores old default");
        assert!(!opt_out.map_labels_explicit);

        // Back-compat: the old --map-labels-fallback opt-in maps to explicit-on.
        let alias = parse_cli(&args(&["--map-labels-fallback"])).expect("args parse");
        assert!(alias.map_labels && alias.map_labels_explicit);

        // Last-wins ordering: --map-labels then --no-map-labels => off.
        let last_off = parse_cli(&args(&["--map-labels", "--no-map-labels"])).expect("args parse");
        assert!(!last_off.map_labels && !last_off.map_labels_explicit);
        // ...and --no-map-labels then --map-labels => on+explicit.
        let last_on = parse_cli(&args(&["--no-map-labels", "--map-labels"])).expect("args parse");
        assert!(last_on.map_labels && last_on.map_labels_explicit);
    }

    #[test]
    fn cli_map_furniture_default_on_with_explicit_opt_out() {
        // FURN1: the city/VP furniture composite graduated to DEFAULT ON with
        // the LABGRAD1 policy shape: default = on-but-not-explicit (fail-open
        // downgrade on a missing --game), --map-furniture = on-and-explicit
        // (fail closed), --no-map-furniture = off (byte-identical render).
        let defaults = parse_cli(&[]).expect("empty parse");
        assert!(defaults.map_furniture, "FURN1: map furniture default ON");
        assert!(
            !defaults.map_furniture_explicit,
            "default-ON must not be treated as an explicit request"
        );

        let explicit = parse_cli(&args(&["--map-furniture"])).expect("args parse");
        assert!(explicit.map_furniture && explicit.map_furniture_explicit);

        let opt_out = parse_cli(&args(&["--no-map-furniture"])).expect("args parse");
        assert!(!opt_out.map_furniture);
        assert!(!opt_out.map_furniture_explicit);

        // Last-wins ordering both ways.
        let last_off =
            parse_cli(&args(&["--map-furniture", "--no-map-furniture"])).expect("args parse");
        assert!(!last_off.map_furniture && !last_off.map_furniture_explicit);
        let last_on =
            parse_cli(&args(&["--no-map-furniture", "--map-furniture"])).expect("args parse");
        assert!(last_on.map_furniture && last_on.map_furniture_explicit);
    }

    #[test]
    fn cli_parses_borders_from_files() {
        // BORD5: borders graduated to DEFAULT ON (non-explicit, fail-open);
        // --borders-from-files makes the request explicit (fail closed);
        // --no-borders-from-files restores the borderless render.
        let default = parse_cli(&[]).expect("empty parse");
        assert!(default.borders_from_files && !default.borders_from_files_explicit);
        let explicit = parse_cli(&args(&["--borders-from-files"])).expect("args parse");
        assert!(explicit.borders_from_files && explicit.borders_from_files_explicit);
        let off = parse_cli(&args(&["--no-borders-from-files"])).expect("args parse");
        assert!(!off.borders_from_files && !off.borders_from_files_explicit);
        let both = parse_cli(&args(&["--water-full", "--borders-from-files"])).expect("args parse");
        assert!(both.water_full && both.borders_from_files);
    }

    #[test]
    fn cli_parses_synthesize_terrain_chunks() {
        let options = parse_cli(&args(&[
            "--capture",
            "captures/session_20260703",
            "--synthesize-terrain-chunks",
        ]))
        .expect("args parse");
        assert!(options.synthesize_terrain_chunks);
    }

    #[test]
    fn cli_rejects_synthesize_terrain_chunks_without_capture() {
        let err = parse_cli(&args(&["--synthesize-terrain-chunks"])).expect_err("must fail");
        assert!(
            err.to_string()
                .contains("--synthesize-terrain-chunks requires --capture"),
            "{err}"
        );
    }

    #[test]
    fn cli_parses_chunks_from_files_and_defaults_off() {
        // CHUNK1: default OFF => byte-identical render path.
        assert!(!parse_cli(&[]).expect("empty parse").chunks_from_files);
        let options = parse_cli(&args(&[
            "--capture",
            "captures/session_20260703",
            "--synthesize-terrain-chunks",
            "--chunks-from-files",
            "--game",
            "F:/HOI4",
        ]))
        .expect("args parse");
        assert!(options.chunks_from_files);
        assert!(options.synthesize_terrain_chunks);
    }

    #[test]
    fn cli_rejects_chunks_from_files_without_synthesis() {
        let err =
            parse_cli(&args(&["--chunks-from-files", "--game", "F:/HOI4"])).expect_err("must fail");
        assert!(
            err.to_string()
                .contains("--chunks-from-files requires --synthesize-terrain-chunks"),
            "{err}"
        );
    }

    #[test]
    fn cli_rejects_chunks_from_files_without_game() {
        let err = parse_cli(&args(&[
            "--capture",
            "captures/session_20260703",
            "--synthesize-terrain-chunks",
            "--chunks-from-files",
        ]))
        .expect_err("must fail");
        assert!(
            err.to_string()
                .contains("--chunks-from-files requires --game"),
            "{err}"
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
    fn cli_defaults_capture_layers_on_and_postfx_on() {
        let options = parse_cli(&[]).expect("empty args parse");
        assert_eq!(options.capture_layers, CliCaptureLayerConfig::default());
        // The game's on-screen image is post-tonemap; postfx defaults ON.
        assert!(options.postfx_enabled);
        // GRAD1: the files-first RestoreScene grade is now DEFAULT ON, but the
        // default value is NOT an explicit request (so it fails open on missing
        // evidence rather than erroring).
        assert!(
            options.postfx_restore_full,
            "GRAD1: restore-full defaults ON"
        );
        assert!(
            !options.postfx_restore_full_explicit,
            "default-ON must not be treated as an explicit request"
        );
        assert!(!options.trees_from_files);
        assert!(!options.buildings_from_files);
        assert!(!options.buildings_preview_all);
        let raw = parse_cli(&args(&["--no-postfx"])).expect("args parse");
        assert!(!raw.postfx_enabled);
    }

    #[test]
    fn cli_postfx_restore_full_explicit_and_opt_out() {
        // GRAD1: --no-postfx-restore-full restores the old capture-anchored
        // default; an explicit --postfx-restore-full sets the explicit flag so
        // render.rs fails closed on missing evidence.
        let opt_out = parse_cli(&args(&["--no-postfx-restore-full"])).expect("args parse");
        assert!(!opt_out.postfx_restore_full);
        assert!(!opt_out.postfx_restore_full_explicit);

        let explicit = parse_cli(&args(&["--postfx-restore-full"])).expect("args parse");
        assert!(explicit.postfx_restore_full && explicit.postfx_restore_full_explicit);

        // Last-wins ordering both directions.
        let last_off = parse_cli(&args(&[
            "--postfx-restore-full",
            "--no-postfx-restore-full",
        ]))
        .expect("args parse");
        assert!(!last_off.postfx_restore_full && !last_off.postfx_restore_full_explicit);
        let last_on = parse_cli(&args(&[
            "--no-postfx-restore-full",
            "--postfx-restore-full",
        ]))
        .expect("args parse");
        assert!(last_on.postfx_restore_full && last_on.postfx_restore_full_explicit);
    }

    #[test]
    fn cli_parses_capture_layer_and_postfx_flags() {
        let options = parse_cli(&args(&[
            "--no-capture-rivers",
            "--no-capture-trees",
            "--postfx",
            "--postfx-restore-full",
        ]))
        .expect("args parse");
        assert!(options.capture_layers.water);
        assert!(!options.capture_layers.rivers);
        assert!(!options.capture_layers.trees);
        assert!(options.postfx_enabled);
        assert!(options.postfx_restore_full);

        let options = parse_cli(&args(&["--postfx", "--no-postfx"])).expect("args parse");
        assert!(!options.postfx_enabled);
    }

    #[test]
    fn cli_rejects_recording_with_demo_scrub() {
        let err = parse_cli(&args(&["--recording", "r.battleplan", "--demo-scrub"]))
            .expect_err("must fail");
        assert!(err.to_string().contains("mutually exclusive"), "{err}");
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
        let options = parse_cli(&args(&["--game", "some/install"])).expect("args parse");
        assert_eq!(options.game_dir, Some(PathBuf::from("some/install")));
        // Absent flag stays None; the env fallback happens in run().
        assert_eq!(parse_cli(&[]).expect("empty parse").game_dir, None);
    }

    #[test]
    fn cli_parses_repeated_mod_paths() {
        let options = parse_cli(&args(&[
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
        let options =
            parse_cli(&args(&["--use-dlc-load", "--userdata", "userdata"])).expect("args parse");
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
    fn gb_cpu_env_accepts_truthy_values_only() {
        assert!(gb_cpu_env_value_enabled(Some(OsString::from("true"))));
        assert!(gb_cpu_env_value_enabled(Some(OsString::from("1"))));
        assert!(gb_cpu_env_value_enabled(Some(OsString::from("YES"))));
        assert!(!gb_cpu_env_value_enabled(Some(OsString::from("0"))));
        assert!(!gb_cpu_env_value_enabled(None));
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
