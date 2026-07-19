//! BLOOM1 validation bin: run the file-derived bloom chain on the captured
//! MainScene and compare the result to the captured RestoreBloom, byte for byte.
//!
//! Files-first: the chain parameters come from the installed content via the mod-aware
//! resolver (see `map_viewer::resolve_bloom_params`). The captured
//! restore_gate_1 bundle is VALIDATION only — it supplies the MainScene input,
//! the ground-truth RestoreBloom output, and the PostEffectBase CB whose
//! BrightThreshold/EmissiveBloomStrength we cross-check against the file-derived
//! params.
//!
//! Usage:
//!   dump_bloom_validation --bundle <restore_gate_1 dir or bundle.json>
//!                         --game <the game dir> [--mod <content dir>]...
//!                         [--out <output dir>]
//!
//! Outputs (in --out, default: <bundle dir>/bloom_validation):
//!   main_scene.bmp             tonemapped preview of the captured MainScene
//!   bloom_expected.bmp         captured RestoreBloom (ground truth)
//!   bloom_actual.bmp           our file-derived bloom output
//!   bloom_bright_pass.bmp      the intermediate bright-pass/downsample
//!   bloom_diff_heatmap.bmp     per-pixel abs error, heat-mapped
//!   bloom_validation_stats.json  exact-match %, ULP/8-bit deltas, MAE, params

use map_viewer::{
    load_restore_scene_bundle, resolve_bloom_params, run_bloom_chain, BloomImageRgba, BloomParams,
    GameFileResolver, Result,
};
use serde::Serialize;
use std::env;
use std::fs;
use std::path::{Path, PathBuf};

#[path = "dump_shared/bmp_out.rs"]
mod bmp_out;

const MAIN_SCENE_NAME: &str = "MainScene";
const RESTORE_BLOOM_NAME: &str = "RestoreBloom";
const DXGI_FORMAT_R16G16B16A16_FLOAT: u64 = 10;

fn main() -> Result<()> {
    let args = Args::parse(env::args().skip(1))?;

    // --- Resolve bloom parameters from installed content (source of truth). ---
    let resolver = if args.mods.is_empty() {
        GameFileResolver::vanilla(&args.game)?
    } else {
        GameFileResolver::with_mod_paths(args.game.clone(), args.mods.clone())?
    };
    let params = resolve_bloom_params(&resolver)?;
    let kernel =
        map_viewer::BloomBlurKernel::from_gaussian(params.bloom_width, params.bloom_scale)?;

    // --- Load captured evidence (validation only). ---
    let (bundle_dir, bundle_path) = locate_bundle(&args.bundle)?;
    // Structural validation of the whole restore-pass bundle (fails closed on
    // any inconsistency) and the decoded PostEffectBase CB.
    let restore = load_restore_scene_bundle(&bundle_path)?;
    let cb = &restore.posteffect_cb;

    // Cross-check: the file-derived bright-pass params must match the CB that
    // was actually in effect for the captured frame. If a mod retuned the
    // volume differently from the capture, fail closed rather than validate
    // against a mismatched ground truth.
    check_close(
        "BrightThreshold",
        params.bright_threshold,
        cb.bright_threshold,
        1e-4,
    )?;
    check_close(
        "EmissiveBloomStrength",
        params.emissive_bloom_strength,
        cb.emissive_bloom_strength,
        1e-4,
    )?;

    // --- Read the raw captured MainScene + RestoreBloom payloads. ---
    let raw = serde_json::from_slice::<serde_json::Value>(&fs::read(&bundle_path)?)?;
    let draw = raw
        .get("draws")
        .and_then(|d| d.as_array())
        .and_then(|d| d.first())
        .ok_or("bloom validation blocked: bundle has no draws[0]")?;

    let main_bind = find_ps_srv(draw, MAIN_SCENE_NAME)?;
    let bloom_bind = find_ps_srv(draw, RESTORE_BLOOM_NAME)?;

    let main_scene = load_captured_rgba16f(&bundle_dir, &main_bind, MAIN_SCENE_NAME)?;
    let expected_bloom = load_captured_rgba16f(&bundle_dir, &bloom_bind, RESTORE_BLOOM_NAME)?;

    // The bloom output must be exactly half the MainScene size.
    if expected_bloom.width != main_scene.width / 2
        || expected_bloom.height != main_scene.height / 2
    {
        return Err(format!(
            "bloom validation blocked: RestoreBloom {}x{} is not half of MainScene {}x{}",
            expected_bloom.width, expected_bloom.height, main_scene.width, main_scene.height
        )
        .into());
    }

    // --- Run the file-derived chain. ---
    let output = run_bloom_chain(&main_scene, params)?;
    let actual_bloom = &output.bloom;

    // --- Compare, pixel for pixel. ---
    let stats = compare(&expected_bloom, actual_bloom);

    // --- Write outputs. ---
    let out_dir = args
        .out
        .clone()
        .unwrap_or_else(|| bundle_dir.join("bloom_validation"));
    fs::create_dir_all(&out_dir)?;
    write_bmp_tonemap(&out_dir.join("main_scene.bmp"), &main_scene)?;
    write_bmp_linear(&out_dir.join("bloom_expected.bmp"), &expected_bloom)?;
    write_bmp_linear(&out_dir.join("bloom_actual.bmp"), actual_bloom)?;
    write_bmp_linear(&out_dir.join("bloom_bright_pass.bmp"), &output.bright_pass)?;
    write_diff_heatmap(
        &out_dir.join("bloom_diff_heatmap.bmp"),
        &expected_bloom,
        actual_bloom,
    )?;

    let report = ValidationReport {
        bundle: bundle_path.display().to_string(),
        game_dir: resolver.game_dir().display().to_string(),
        selection_source: resolver.selection_source().to_string(),
        params: ParamsJson::from(&params),
        kernel: KernelJson {
            weight0: kernel.weight0,
            weights: kernel.weights,
            offsets: kernel.offsets,
            pass_weight_sum: kernel.pass_weight_sum(),
        },
        captured_cb: CapturedCbJson {
            bright_threshold: cb.bright_threshold,
            emissive_bloom_strength: cb.emissive_bloom_strength,
            bloom_to_screen_scale: cb.bloom_to_screen_scale,
            inv_down_sample_size: cb.inv_down_sample_size,
            inv_window_size: cb.inv_window_size,
            middle_grey: cb.middle_grey,
        },
        main_scene: DimsJson {
            width: main_scene.width,
            height: main_scene.height,
        },
        bloom: DimsJson {
            width: expected_bloom.width,
            height: expected_bloom.height,
        },
        stats: stats.clone(),
        gate_byte_exact: stats.exact_match_fraction >= 1.0,
    };
    let json = serde_json::to_string_pretty(&report)?;
    fs::write(out_dir.join("bloom_validation_stats.json"), &json)?;

    // --- Console summary. ---
    println!("BLOOM1 validation ({})", bundle_path.display());
    println!(
        "  params (files): BLOOM_WIDTH={} BLOOM_SCALE={} BRIGHT_THRESHOLD={} EMISSIVE_BLOOM_STRENGTH={}",
        params.bloom_width, params.bloom_scale, params.bright_threshold, params.emissive_bloom_strength
    );
    println!(
        "  kernel: Weight0={:.9} Weights={:?} Offsets={:?} pass_sum={:.6}",
        kernel.weight0,
        kernel.weights,
        kernel.offsets,
        kernel.pass_weight_sum()
    );
    println!(
        "  captured CB: BrightThreshold={} EmissiveBloomStrength={} BloomToScreenScale={:?}",
        cb.bright_threshold, cb.emissive_bloom_strength, cb.bloom_to_screen_scale
    );
    println!(
        "  compared {}x{} bloom ({} texels)",
        expected_bloom.width,
        expected_bloom.height,
        expected_bloom.width * expected_bloom.height
    );
    println!(
        "  exact half-float match: {:.4}% ({}/{} channels)",
        stats.exact_match_fraction * 100.0,
        stats.exact_channels,
        stats.total_channels
    );
    println!(
        "  max abs err: {:.6e}  ({} 8-bit levels, {} half-float ULPs)",
        stats.max_abs_error, stats.max_error_8bit_levels, stats.max_error_half_ulps
    );
    println!("  mean abs err: {:.6e}", stats.mean_abs_error);
    println!(
        "  within 1/255: {:.4}%",
        stats.within_one_8bit_level_fraction * 100.0
    );
    println!(
        "  localization: background ({} px, mag<{:.0e}) max err {:.3e}; core ({} px, mag>{}) max {:.3e} mean {:.3e}",
        stats.background_pixels,
        1.0e-6,
        stats.background_max_abs_error,
        stats.core_pixels,
        0.01,
        stats.core_max_abs_error,
        stats.core_mean_abs_error
    );
    println!(
        "  GATE byte-exact: {}",
        if report.gate_byte_exact {
            "PASS"
        } else {
            "NOT REACHED (see stats + heatmap)"
        }
    );
    println!("  outputs: {}", out_dir.display());
    Ok(())
}

struct Args {
    bundle: PathBuf,
    game: PathBuf,
    mods: Vec<PathBuf>,
    out: Option<PathBuf>,
}

impl Args {
    fn parse(mut it: impl Iterator<Item = String>) -> Result<Self> {
        let mut bundle = None;
        let mut game = None;
        let mut mods = Vec::new();
        let mut out = None;
        while let Some(arg) = it.next() {
            match arg.as_str() {
                "--bundle" => bundle = it.next().map(PathBuf::from),
                "--game" => game = it.next().map(PathBuf::from),
                "--mod" => mods.push(PathBuf::from(it.next().ok_or("--mod needs a content dir")?)),
                "--out" => out = it.next().map(PathBuf::from),
                "--help" | "-h" => {
                    println!(
                        "Usage: dump_bloom_validation --bundle <restore_gate_1 dir|bundle.json> --game <the game dir> [--mod <dir>]... [--out <dir>]"
                    );
                    std::process::exit(0);
                }
                other => return Err(format!("unknown argument {other}").into()),
            }
        }
        Ok(Self {
            bundle: bundle.ok_or("missing --bundle <restore_gate_1 dir or bundle.json>")?,
            game: game.ok_or("missing --game <game dir>")?,
            mods,
            out,
        })
    }
}

fn locate_bundle(arg: &Path) -> Result<(PathBuf, PathBuf)> {
    if arg.is_file() {
        let dir = arg
            .parent()
            .ok_or("bloom validation blocked: bundle file has no parent dir")?
            .to_path_buf();
        return Ok((dir, arg.to_path_buf()));
    }
    if arg.is_dir() {
        let path = arg.join(map_viewer::RESTORE_PASS_BUNDLE_FILE);
        if !path.is_file() {
            return Err(format!(
                "bloom validation blocked: {} has no {}",
                arg.display(),
                map_viewer::RESTORE_PASS_BUNDLE_FILE
            )
            .into());
        }
        return Ok((arg.to_path_buf(), path));
    }
    Err(format!(
        "bloom validation blocked: --bundle {} is not a file or directory",
        arg.display()
    )
    .into())
}

/// A minimal view of a PS SRV binding needed to read its raw payload.
struct CapturedBinding {
    file: String,
    width: usize,
    height: usize,
    row_pitch: usize,
    file_offset: usize,
    dxgi_format: u64,
}

fn find_ps_srv(draw: &serde_json::Value, declared_name: &str) -> Result<CapturedBinding> {
    let resources = draw
        .get("shader_resources")
        .and_then(|r| r.as_array())
        .ok_or("bloom validation blocked: draw has no shader_resources")?;
    let resource = resources
        .iter()
        .find(|r| {
            r.get("stage").and_then(|s| s.as_str()) == Some("PS")
                && r.get("declared_name").and_then(|n| n.as_str()) == Some(declared_name)
        })
        .ok_or_else(|| {
            format!("bloom validation blocked: no PS SRV named {declared_name} in the restore draw")
        })?;
    let dxgi_format = resource
        .get("dxgi_format")
        .and_then(|f| f.as_u64())
        .ok_or_else(|| format!("bloom validation blocked: {declared_name} has no dxgi_format"))?;
    let width = json_usize(resource, "width", declared_name)?;
    let height = json_usize(resource, "height", declared_name)?;
    let file = resource
        .get("file")
        .and_then(|f| f.as_str())
        .ok_or_else(|| format!("bloom validation blocked: {declared_name} has no payload file"))?
        .to_string();
    // Prefer the subresource's row pitch / offset; fall back to a tight pitch.
    let sub = resource
        .get("subresources")
        .and_then(|s| s.as_array())
        .and_then(|s| s.first());
    let row_pitch = sub
        .and_then(|s| s.get("row_pitch"))
        .and_then(|p| p.as_u64())
        .map(|p| p as usize)
        .unwrap_or(width * 8);
    let file_offset = sub
        .and_then(|s| s.get("file_offset"))
        .and_then(|p| p.as_u64())
        .map(|p| p as usize)
        .unwrap_or(0);
    Ok(CapturedBinding {
        file,
        width,
        height,
        row_pitch,
        file_offset,
        dxgi_format,
    })
}

fn json_usize(value: &serde_json::Value, key: &str, ctx: &str) -> Result<usize> {
    value
        .get(key)
        .and_then(|v| v.as_u64())
        .map(|v| v as usize)
        .ok_or_else(|| format!("bloom validation blocked: {ctx} has no numeric {key}").into())
}

fn load_captured_rgba16f(
    bundle_dir: &Path,
    binding: &CapturedBinding,
    role: &str,
) -> Result<BloomImageRgba> {
    if binding.dxgi_format != DXGI_FORMAT_R16G16B16A16_FLOAT {
        return Err(format!(
            "bloom validation blocked: {role} DXGI format is {}, expected {DXGI_FORMAT_R16G16B16A16_FLOAT} (R16G16B16A16_FLOAT)",
            binding.dxgi_format
        )
        .into());
    }
    let path = bundle_dir.join(&binding.file);
    let bytes = fs::read(&path).map_err(|err| {
        format!(
            "bloom validation blocked: {role} payload {} is not readable: {err}",
            path.display()
        )
    })?;
    let slice = bytes
        .get(binding.file_offset..)
        .ok_or_else(|| format!("bloom validation blocked: {role} file_offset out of range"))?;
    BloomImageRgba::from_r16g16b16a16_float(slice, binding.width, binding.height, binding.row_pitch)
}

// ---------- Comparison ----------

#[derive(Clone, Serialize)]
struct Stats {
    total_channels: usize,
    exact_channels: usize,
    exact_match_fraction: f64,
    within_one_8bit_level_fraction: f64,
    max_abs_error: f32,
    mean_abs_error: f64,
    max_error_8bit_levels: f64,
    max_error_half_ulps: u32,
    /// Localization: where the max error sits, and per-channel max error.
    max_error_at: [usize; 2],
    max_error_channel: usize,
    per_channel_max_abs_error: [f32; 3],
    /// Divergence localization bins (per pixel, using the max-channel error and
    /// the captured bloom magnitude at that pixel). These back the claim that
    /// the residual is half-float/filter rounding confined to bright bloom
    /// cores, not a stage error: background pixels must be ~exact.
    background_pixels: usize,
    background_max_abs_error: f32,
    core_pixels: usize,
    core_max_abs_error: f32,
    core_mean_abs_error: f64,
}

/// Captured bloom magnitude below this counts as "background" (should be exact);
/// above `CORE_MAGNITUDE` counts as a bright "core" (where half-float LSBs bite).
const BACKGROUND_MAGNITUDE: f32 = 1.0e-6;
const CORE_MAGNITUDE: f32 = 0.01;

fn compare(expected: &BloomImageRgba, actual: &BloomImageRgba) -> Stats {
    // Compare RGB only (bloom.shader writes color to RGB; alpha is a forced 1.0
    // and not consumed by restorescene's bloom add).
    let mut total = 0usize;
    let mut exact = 0usize;
    let mut within = 0usize;
    let mut max_abs = 0.0f32;
    let mut sum_abs = 0.0f64;
    let mut max_ulps = 0u32;
    let mut max_at = [0usize; 2];
    let mut max_ch = 0usize;
    let mut per_ch = [0.0f32; 3];
    let mut bg_pixels = 0usize;
    let mut bg_max = 0.0f32;
    let mut core_pixels = 0usize;
    let mut core_max = 0.0f32;
    let mut core_sum = 0.0f64;
    for y in 0..expected.height {
        for x in 0..expected.width {
            let idx = y * expected.width + x;
            let e = expected.texels[idx];
            let a = actual.texels[idx];
            let mut pixel_max_err = 0.0f32;
            for c in 0..3 {
                total += 1;
                // Half-float exactness: re-quantize our value and compare bits.
                let e_bits = map_viewer::f32_to_half_float(e[c]);
                let a_bits = map_viewer::f32_to_half_float(a[c]);
                if e_bits == a_bits {
                    exact += 1;
                }
                let ulps = half_ulp_distance(e_bits, a_bits);
                if ulps > max_ulps {
                    max_ulps = ulps;
                }
                let d = (e[c] - a[c]).abs();
                sum_abs += d as f64;
                if d < (1.0 / 255.0) {
                    within += 1;
                }
                if d > per_ch[c] {
                    per_ch[c] = d;
                }
                if d > pixel_max_err {
                    pixel_max_err = d;
                }
                if d > max_abs {
                    max_abs = d;
                    max_at = [x, y];
                    max_ch = c;
                }
            }
            // Bin by captured bloom magnitude at this pixel.
            let magnitude = e[0].max(e[1]).max(e[2]);
            if magnitude < BACKGROUND_MAGNITUDE {
                bg_pixels += 1;
                if pixel_max_err > bg_max {
                    bg_max = pixel_max_err;
                }
            } else if magnitude > CORE_MAGNITUDE {
                core_pixels += 1;
                core_sum += pixel_max_err as f64;
                if pixel_max_err > core_max {
                    core_max = pixel_max_err;
                }
            }
        }
    }
    Stats {
        total_channels: total,
        exact_channels: exact,
        exact_match_fraction: exact as f64 / total.max(1) as f64,
        within_one_8bit_level_fraction: within as f64 / total.max(1) as f64,
        max_abs_error: max_abs,
        mean_abs_error: sum_abs / total.max(1) as f64,
        max_error_8bit_levels: (max_abs * 255.0) as f64,
        max_error_half_ulps: max_ulps,
        max_error_at: max_at,
        max_error_channel: max_ch,
        per_channel_max_abs_error: per_ch,
        background_pixels: bg_pixels,
        background_max_abs_error: bg_max,
        core_pixels,
        core_max_abs_error: core_max,
        core_mean_abs_error: core_sum / core_pixels.max(1) as f64,
    }
}

/// Monotonic-ordering ULP distance between two IEEE-754 half-float bit patterns.
fn half_ulp_distance(a: u16, b: u16) -> u32 {
    let ord = |bits: u16| -> i32 {
        if bits & 0x8000 != 0 {
            // Negative: flip to a descending order below zero.
            -(i32::from(bits & 0x7fff))
        } else {
            i32::from(bits)
        }
    };
    (ord(a) - ord(b)).unsigned_abs()
}

// ---------- BMP writers (24-bit BGR, bottom-up; shared writer) ----------

fn write_bmp_linear(path: &Path, image: &BloomImageRgba) -> Result<()> {
    let rgb: Vec<[u8; 3]> = image
        .texels
        .iter()
        .map(|t| [to_u8(t[0]), to_u8(t[1]), to_u8(t[2])])
        .collect();
    bmp_out::write_bmp24_rgb(path, image.width, image.height, &rgb)
}

/// A gamma-ish tonemap preview so the HDR MainScene is legible.
fn write_bmp_tonemap(path: &Path, image: &BloomImageRgba) -> Result<()> {
    let rgb: Vec<[u8; 3]> = image
        .texels
        .iter()
        .map(|t| {
            [
                to_u8(reinhard(t[0])),
                to_u8(reinhard(t[1])),
                to_u8(reinhard(t[2])),
            ]
        })
        .collect();
    bmp_out::write_bmp24_rgb(path, image.width, image.height, &rgb)
}

fn reinhard(x: f32) -> f32 {
    (x / (1.0 + x)).powf(1.0 / 2.2)
}

fn write_diff_heatmap(
    path: &Path,
    expected: &BloomImageRgba,
    actual: &BloomImageRgba,
) -> Result<()> {
    // Per-pixel max-channel abs error, scaled so 1/255 maps near full intensity.
    let rgb: Vec<[u8; 3]> = (0..expected.texels.len())
        .map(|i| {
            let e = expected.texels[i];
            let a = actual.texels[i];
            let d = (0..3).map(|c| (e[c] - a[c]).abs()).fold(0.0f32, f32::max);
            heat(d * 255.0) // scale so a full 8-bit level saturates the ramp
        })
        .collect();
    bmp_out::write_bmp24_rgb(path, expected.width, expected.height, &rgb)
}

/// Blue(0) -> green -> red(>=1) heat ramp over `t` (already scaled so 1.0 ~= a
/// full 8-bit level of error).
fn heat(t: f32) -> [u8; 3] {
    let t = t.clamp(0.0, 1.0);
    if t < 0.5 {
        let u = t / 0.5;
        [0, (u * 255.0) as u8, ((1.0 - u) * 255.0) as u8]
    } else {
        let u = (t - 0.5) / 0.5;
        [(u * 255.0) as u8, ((1.0 - u) * 255.0) as u8, 0]
    }
}

fn to_u8(x: f32) -> u8 {
    (x.clamp(0.0, 1.0) * 255.0).round() as u8
}

fn check_close(name: &str, derived: f32, captured: f32, tol: f32) -> Result<()> {
    if (derived - captured).abs() > tol {
        return Err(format!(
            "bloom validation blocked: file-derived {name}={derived} disagrees with captured CB {name}={captured} (tol {tol}); the capture used a different posteffect volume than the resolved installed content"
        )
        .into());
    }
    Ok(())
}

// ---------- JSON report ----------

#[derive(Serialize)]
struct ValidationReport {
    bundle: String,
    game_dir: String,
    selection_source: String,
    params: ParamsJson,
    kernel: KernelJson,
    captured_cb: CapturedCbJson,
    main_scene: DimsJson,
    bloom: DimsJson,
    stats: Stats,
    gate_byte_exact: bool,
}

#[derive(Serialize)]
struct ParamsJson {
    bloom_width: f32,
    bloom_scale: f32,
    bright_threshold: f32,
    emissive_bloom_strength: f32,
}

impl From<&BloomParams> for ParamsJson {
    fn from(p: &BloomParams) -> Self {
        Self {
            bloom_width: p.bloom_width,
            bloom_scale: p.bloom_scale,
            bright_threshold: p.bright_threshold,
            emissive_bloom_strength: p.emissive_bloom_strength,
        }
    }
}

#[derive(Serialize)]
struct KernelJson {
    weight0: f32,
    weights: [f32; 4],
    offsets: [f32; 4],
    pass_weight_sum: f32,
}

#[derive(Serialize)]
struct CapturedCbJson {
    bright_threshold: f32,
    emissive_bloom_strength: f32,
    bloom_to_screen_scale: [f32; 2],
    inv_down_sample_size: [f32; 2],
    inv_window_size: [f32; 2],
    middle_grey: f32,
}

#[derive(Serialize)]
struct DimsJson {
    width: usize,
    height: usize,
}
