//! WATER2 validation bin: reconstruct the game's `WaterRefraction` buffer as
//! the underwater seafloor pass (files-first, from `pdxmap.shader`
//! `Effect underwater` via the mod-aware resolver) and compare it to the
//! captured `WaterRefraction` payload, with per-region divergence localization.
//!
//! Files-first: the seafloor shader + its constants come from the shipped game
//! files (the resolver). The captured session is VALIDATION only.
//!
//! Validation-target reality (see documents/WATER_LAW.md): the underwater PS
//! samples the seafloor textures (TerrainDiffuse/HeightNormal/TerrainColorTint)
//! and a ShadowMap. In a raw captured terrain frame those are descriptor-only
//! and the ShadowMap is null, so those inputs are GENUINELY MISSING (this is
//! the case for `restore_gate_1`). A terrain-chunk-library session (e.g.
//! `live_PAN2`) carries the resolved terrain textures AND the captured
//! ShadowMap, so the pass has all inputs — but the chunk-library geometry is
//! the SYNTHESIZED tiled terrain, so this is an ORACLE/reconstruction test, not
//! a definitive engine byte-exact proof. The bin prefers the chunk library and
//! records which mode it used.
//!
//! Usage:
//!   dump_underwater_validation --session <capture session dir>
//!                              --game <the game dir> [--mod <content dir>]...
//!                              [--out <output dir>]
//!
//! Outputs (in --out, default: <session>/underwater_validation):
//!   underwater_expected.bmp        captured WaterRefraction (ground truth)
//!   underwater_actual.bmp          our file-derived underwater pass
//!   underwater_diff_heatmap.bmp    per-pixel abs error, heat-mapped
//!   underwater_region_map.bmp      per-region classification overlay
//!   underwater_validation_stats.json  exact-match %, deltas, localization

use map_viewer::{
    underwater_clip_height_threshold, underwater_defines, GameFileResolver,
    NativeD3d11TerrainBundle, NativeD3d11TerrainRenderer, Result, UNDERWATER_EFFECT_NAME,
    UNDERWATER_SHADER_SOURCE,
};
use serde::Serialize;
use std::env;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

const TERRAIN_BUNDLE_FILE: &str = "map_frame_bundle.json";
const WATER_BUNDLE_FILE: &str = "map_frame_water_pass_bundle.json";
const CHUNK_LIBRARY_FILE: &str = "terrain_chunk_library.json";
const WATER_REFRACTION_NAME: &str = "WaterRefraction";
const DXGI_FORMAT_B8G8R8A8_UNORM: u64 = 87;

#[derive(Clone, Copy)]
enum OracleMode {
    /// Terrain textures + ShadowMap came from a terrain-chunk-library (the
    /// resolved seafloor inputs). Geometry is the synthesized chunk draws — an
    /// oracle/reconstruction test, not a definitive engine byte-exact proof.
    ChunkLibrary,
    /// Raw captured terrain frame (seafloor textures descriptor-only, ShadowMap
    /// null). Only usable where those inputs happen to be present.
    TerrainFrame,
}

impl OracleMode {
    fn label(self) -> &'static str {
        match self {
            OracleMode::ChunkLibrary => "chunk_library_oracle",
            OracleMode::TerrainFrame => "terrain_frame",
        }
    }
}

fn main() -> Result<()> {
    let args = Args::parse(env::args().skip(1))?;

    let resolver = if args.mods.is_empty() {
        GameFileResolver::vanilla(&args.game)?
    } else {
        GameFileResolver::with_mod_paths(args.game.clone(), args.mods.clone())?
    };

    let water_bundle_path = args.session.join(WATER_BUNDLE_FILE);
    if !water_bundle_path.is_file() {
        return Err(format!(
            "underwater validation blocked: {} has no {WATER_BUNDLE_FILE} (no ground-truth \
             WaterRefraction to validate against)",
            args.session.display()
        )
        .into());
    }

    // --- Load the captured ground-truth WaterRefraction payload. ---
    let expected = load_captured_water_refraction(&args.session, &water_bundle_path)?;

    // --- Reconstruct + render the underwater seafloor pass (files-first). ---
    //
    // The underwater PS samples the seafloor textures (TerrainDiffuse,
    // HeightNormal, TerrainColorTint) + ShadowMap. In a raw terrain frame those
    // are descriptor-only (resolved elsewhere) and ShadowMap is null, so the
    // required inputs are MISSING. A terrain-chunk-library session carries the
    // resolved terrain textures AND the captured ShadowMap, so the pass has all
    // its inputs. Prefer the chunk library; if it is absent, fail closed and
    // name the missing inputs precisely (do NOT silently substitute).
    let chunk_library_path = args.session.join(CHUNK_LIBRARY_FILE);
    let (render, oracle_mode) = if chunk_library_path.is_file() {
        (
            run_underwater_from_chunk_library(&chunk_library_path, &resolver)?,
            OracleMode::ChunkLibrary,
        )
    } else {
        let terrain_bundle_path = args.session.join(TERRAIN_BUNDLE_FILE);
        if !terrain_bundle_path.is_file() {
            return Err(format!(
                "underwater validation blocked: {} has neither {CHUNK_LIBRARY_FILE} nor \
                 {TERRAIN_BUNDLE_FILE}",
                args.session.display()
            )
            .into());
        }
        let terrain_bundle =
            NativeD3d11TerrainBundle::load_with_resolver(&terrain_bundle_path, &resolver)?;
        (
            run_underwater(&terrain_bundle, &resolver)?,
            OracleMode::TerrainFrame,
        )
    };

    if render.width as usize != expected.width || render.height as usize != expected.height {
        return Err(format!(
            "underwater validation blocked: rendered {}x{} does not match captured WaterRefraction \
             {}x{}",
            render.width, render.height, expected.width, expected.height
        )
        .into());
    }
    if render.dxgi_format as u64 != DXGI_FORMAT_B8G8R8A8_UNORM {
        return Err(format!(
            "underwater validation blocked: rendered DXGI format {} is not B8G8R8A8_UNORM ({DXGI_FORMAT_B8G8R8A8_UNORM})",
            render.dxgi_format
        )
        .into());
    }

    let actual = Bgra8Image::from_readback(
        &render.bytes,
        render.width as usize,
        render.height as usize,
        render.row_pitch as usize,
    )?;

    // --- Compare, pixel for pixel, with per-region localization. ---
    let stats = compare(&expected, &actual);

    // --- Write outputs. ---
    let out_dir = args
        .out
        .clone()
        .unwrap_or_else(|| args.session.join("underwater_validation"));
    fs::create_dir_all(&out_dir)?;
    write_bmp_bgra(&out_dir.join("underwater_expected.bmp"), &expected)?;
    write_bmp_bgra(&out_dir.join("underwater_actual.bmp"), &actual)?;
    write_diff_heatmap(
        &out_dir.join("underwater_diff_heatmap.bmp"),
        &expected,
        &actual,
    )?;
    write_region_map(
        &out_dir.join("underwater_region_map.bmp"),
        &expected,
        &actual,
    )?;

    let clip = underwater_clip_height_threshold();
    let report = ValidationReport {
        session: args.session.display().to_string(),
        oracle_mode: oracle_mode.label().to_string(),
        game_dir: resolver.game_dir().display().to_string(),
        selection_source: resolver.selection_source().to_string(),
        shader_source: UNDERWATER_SHADER_SOURCE.to_string(),
        effect_name: UNDERWATER_EFFECT_NAME.to_string(),
        underwater_ps_byte_len: render.ps_bytecode.len(),
        constants: ConstantsJson {
            water_height: underwater_defines::WATER_HEIGHT,
            terrain_water_clip_height: underwater_defines::TERRAIN_WATER_CLIP_HEIGHT,
            clip_height_threshold: clip,
            water_color_lightness: underwater_defines::WATER_COLOR_LIGHTNESS,
            map_specular_width: underwater_defines::MAP_SPECULAR_WIDTH,
            shadow_weight_terrain: underwater_defines::SHADOW_WEIGHT_TERRAIN,
            depth_fog_min_y: underwater_defines::DEPTH_FOG_MIN_Y,
            depth_fog_max_y: underwater_defines::DEPTH_FOG_MAX_Y,
        },
        dims: DimsJson {
            width: expected.width,
            height: expected.height,
        },
        stats: stats.clone(),
        gate_byte_exact: stats.exact_pixel_fraction >= 1.0,
    };
    let json = serde_json::to_string_pretty(&report)?;
    fs::write(out_dir.join("underwater_validation_stats.json"), &json)?;

    // --- Console summary. ---
    println!("WATER2 underwater validation ({})", args.session.display());
    println!("  mode: {}", oracle_mode.label());
    println!(
        "  files: {UNDERWATER_SHADER_SOURCE}:{UNDERWATER_EFFECT_NAME} (underwater PS {} bytes)",
        render.ps_bytecode.len()
    );
    println!(
        "  constants: WATER_HEIGHT={} TERRAIN_WATER_CLIP_HEIGHT={} => clip y<= {} ; \
         WATER_COLOR_LIGHTNESS={} MAP_SPECULAR_WIDTH={} depth-fog [{},{}]",
        underwater_defines::WATER_HEIGHT,
        underwater_defines::TERRAIN_WATER_CLIP_HEIGHT,
        clip,
        underwater_defines::WATER_COLOR_LIGHTNESS,
        underwater_defines::MAP_SPECULAR_WIDTH,
        underwater_defines::DEPTH_FOG_MIN_Y,
        underwater_defines::DEPTH_FOG_MAX_Y,
    );
    println!(
        "  compared {}x{} = {} pixels ({} channels, alpha included)",
        expected.width,
        expected.height,
        expected.width * expected.height,
        stats.total_channels
    );
    println!(
        "  exact channels: {:.4}% ({}/{})",
        stats.exact_channel_fraction * 100.0,
        stats.exact_channels,
        stats.total_channels
    );
    println!(
        "  exact pixels (RGBA): {:.4}% ({}/{})",
        stats.exact_pixel_fraction * 100.0,
        stats.exact_pixels,
        stats.total_pixels
    );
    println!(
        "  within 1/255: {:.4}%  max abs err {} levels  mean abs err {:.4} levels",
        stats.within_one_level_fraction * 100.0,
        stats.max_abs_error,
        stats.mean_abs_error
    );
    println!(
        "  per-channel max abs err (B,G,R,A): {:?}",
        stats.per_channel_max_abs_error
    );
    println!("  localization by region (captured classification):");
    for region in &stats.regions {
        println!(
            "    {:<16} {:>8} px  exact {:>7.3}%  max {:>3}  mean {:>6.3}",
            region.label,
            region.pixels,
            region.exact_pixel_fraction * 100.0,
            region.max_abs_error,
            region.mean_abs_error
        );
    }
    println!(
        "  GATE byte-exact: {}",
        if report.gate_byte_exact {
            "PASS"
        } else {
            "NOT REACHED (see stats + heatmap + region map)"
        }
    );
    println!("  outputs: {}", out_dir.display());
    Ok(())
}

fn run_underwater(
    bundle: &NativeD3d11TerrainBundle,
    resolver: &GameFileResolver,
) -> Result<map_viewer::UnderwaterValidationRender> {
    #[cfg(windows)]
    {
        let renderer = NativeD3d11TerrainRenderer::new()?;
        renderer.render_underwater_validation(bundle, resolver)
    }
    #[cfg(not(windows))]
    {
        let _ = (bundle, resolver);
        Err("underwater validation requires Windows (D3D11)".into())
    }
}

fn run_underwater_from_chunk_library(
    library_path: &Path,
    resolver: &GameFileResolver,
) -> Result<map_viewer::UnderwaterValidationRender> {
    #[cfg(windows)]
    {
        use map_viewer::NativeD3d11TerrainChunkLibrary;
        let library = NativeD3d11TerrainChunkLibrary::load(library_path)?;
        let renderer = NativeD3d11TerrainRenderer::new()?;
        // The canonical scene CB0 (captured camera/sun) is read from the
        // sibling map_frame_bundle.json when present, so the underwater render
        // aligns with the session's captured WaterRefraction. Default tile size
        // 128 matches the synthesis smoke path.
        renderer.render_underwater_validation_from_chunk_library(&library, 128, None, resolver)
    }
    #[cfg(not(windows))]
    {
        let _ = (library_path, resolver);
        Err("underwater validation requires Windows (D3D11)".into())
    }
}

struct Args {
    session: PathBuf,
    game: PathBuf,
    mods: Vec<PathBuf>,
    out: Option<PathBuf>,
}

impl Args {
    fn parse(mut it: impl Iterator<Item = String>) -> Result<Self> {
        let mut session = None;
        let mut game = None;
        let mut mods = Vec::new();
        let mut out = None;
        while let Some(arg) = it.next() {
            match arg.as_str() {
                "--session" | "--bundle" => session = it.next().map(PathBuf::from),
                "--game" => game = it.next().map(PathBuf::from),
                "--mod" => mods.push(PathBuf::from(it.next().ok_or("--mod needs a content dir")?)),
                "--out" => out = it.next().map(PathBuf::from),
                "--help" | "-h" => {
                    println!(
                        "Usage: dump_underwater_validation --session <restore_gate_1 dir> --game <the game dir> [--mod <dir>]... [--out <dir>]"
                    );
                    std::process::exit(0);
                }
                other => return Err(format!("unknown argument {other}").into()),
            }
        }
        Ok(Self {
            session: session.ok_or("missing --session <restore_gate_1 dir>")?,
            game: game.ok_or("missing --game <game dir>")?,
            mods,
            out,
        })
    }
}

// ---------- Captured WaterRefraction load ----------

fn load_captured_water_refraction(session: &Path, water_bundle: &Path) -> Result<Bgra8Image> {
    let raw = serde_json::from_slice::<serde_json::Value>(&fs::read(water_bundle)?)?;
    let draws = raw
        .get("draws")
        .and_then(|d| d.as_array())
        .ok_or("underwater validation blocked: water bundle has no draws")?;
    for draw in draws {
        let resources = match draw.get("shader_resources").and_then(|r| r.as_array()) {
            Some(r) => r,
            None => continue,
        };
        for resource in resources {
            if resource.get("stage").and_then(|s| s.as_str()) != Some("PS") {
                continue;
            }
            if resource.get("declared_name").and_then(|n| n.as_str()) != Some(WATER_REFRACTION_NAME)
            {
                continue;
            }
            let dxgi_format = resource
                .get("dxgi_format")
                .and_then(|f| f.as_u64())
                .ok_or("underwater validation blocked: WaterRefraction has no dxgi_format")?;
            if dxgi_format != DXGI_FORMAT_B8G8R8A8_UNORM {
                return Err(format!(
                    "underwater validation blocked: captured WaterRefraction DXGI format {dxgi_format} is not B8G8R8A8_UNORM ({DXGI_FORMAT_B8G8R8A8_UNORM})"
                )
                .into());
            }
            let width = json_usize(resource, "width")?;
            let height = json_usize(resource, "height")?;
            let file = resource
                .get("file")
                .and_then(|f| f.as_str())
                .ok_or("underwater validation blocked: WaterRefraction has no payload file")?;
            let sub = resource
                .get("subresources")
                .and_then(|s| s.as_array())
                .and_then(|s| s.first());
            let row_pitch = sub
                .and_then(|s| s.get("row_pitch"))
                .and_then(|p| p.as_u64())
                .map(|p| p as usize)
                .unwrap_or(width * 4);
            let file_offset = sub
                .and_then(|s| s.get("file_offset"))
                .and_then(|p| p.as_u64())
                .map(|p| p as usize)
                .unwrap_or(0);
            let bytes = fs::read(session.join(file))?;
            let slice = bytes
                .get(file_offset..)
                .ok_or("underwater validation blocked: WaterRefraction file_offset out of range")?;
            return Bgra8Image::from_readback(slice, width, height, row_pitch);
        }
    }
    Err("underwater validation blocked: no PS WaterRefraction SRV in the water bundle".into())
}

fn json_usize(value: &serde_json::Value, key: &str) -> Result<usize> {
    value
        .get(key)
        .and_then(|v| v.as_u64())
        .map(|v| v as usize)
        .ok_or_else(|| format!("underwater validation blocked: missing numeric {key}").into())
}

// ---------- Image (BGRA8, tightly packed after unpad) ----------

struct Bgra8Image {
    width: usize,
    height: usize,
    /// Row-major, tightly packed BGRA (B,G,R,A) per pixel.
    texels: Vec<[u8; 4]>,
}

impl Bgra8Image {
    fn from_readback(bytes: &[u8], width: usize, height: usize, row_pitch: usize) -> Result<Self> {
        let needed = row_pitch.checked_mul(height).ok_or("image size overflow")?;
        if bytes.len() < needed {
            return Err(format!(
                "underwater validation blocked: BGRA8 payload {} bytes < needed {needed} \
                 ({width}x{height} pitch {row_pitch})",
                bytes.len()
            )
            .into());
        }
        let mut texels = Vec::with_capacity(width * height);
        for y in 0..height {
            let row = &bytes[y * row_pitch..y * row_pitch + width * 4];
            for x in 0..width {
                let o = x * 4;
                texels.push([row[o], row[o + 1], row[o + 2], row[o + 3]]);
            }
        }
        Ok(Self {
            width,
            height,
            texels,
        })
    }
}

// ---------- Comparison ----------

#[derive(Clone, Copy, PartialEq, Eq)]
enum Region {
    /// Clear/background: captured pixel is exactly transparent black (0,0,0,0)
    /// — nothing was drawn there (land clip-discarded above the 12.5 clip
    /// height, or frustum/coverage no-draw; NOT deep ocean — the deep seafloor
    /// is low-y and is kept by the clip). Isolates the clear-color axis.
    Clear,
    /// Fully-opaque seafloor: captured alpha 255, the underwater PS wrote here.
    Seafloor,
    /// Anything else (partial alpha / land-clip edges).
    Edge,
}

impl Region {
    fn classify(px: [u8; 4]) -> Self {
        if px == [0, 0, 0, 0] {
            Region::Clear
        } else if px[3] == 255 {
            Region::Seafloor
        } else {
            Region::Edge
        }
    }
    fn label(self) -> &'static str {
        match self {
            Region::Clear => "clear/background",
            Region::Seafloor => "seafloor",
            Region::Edge => "clip-edge",
        }
    }
    fn color(self) -> [u8; 3] {
        match self {
            Region::Clear => [40, 40, 40],
            Region::Seafloor => [40, 90, 200],
            Region::Edge => [230, 180, 40],
        }
    }
}

#[derive(Clone, Serialize)]
struct RegionStats {
    label: String,
    pixels: usize,
    exact_pixels: usize,
    exact_pixel_fraction: f64,
    max_abs_error: u32,
    mean_abs_error: f64,
}

#[derive(Clone, Serialize)]
struct Stats {
    total_pixels: usize,
    exact_pixels: usize,
    exact_pixel_fraction: f64,
    total_channels: usize,
    exact_channels: usize,
    exact_channel_fraction: f64,
    within_one_level_fraction: f64,
    max_abs_error: u32,
    mean_abs_error: f64,
    /// (B, G, R, A) max abs error.
    per_channel_max_abs_error: [u32; 4],
    max_error_at: [usize; 2],
    regions: Vec<RegionStats>,
}

fn compare(expected: &Bgra8Image, actual: &Bgra8Image) -> Stats {
    let mut exact_pixels = 0usize;
    let mut exact_channels = 0usize;
    let mut within = 0usize;
    let mut max_abs = 0u32;
    let mut sum_abs = 0u64;
    let mut per_ch = [0u32; 4];
    let mut max_at = [0usize; 2];

    // Per-region accumulators.
    let mut region_acc: [(usize, usize, u32, u64); 3] = [(0, 0, 0, 0); 3];

    let total_pixels = expected.width * expected.height;
    for y in 0..expected.height {
        for x in 0..expected.width {
            let idx = y * expected.width + x;
            let e = expected.texels[idx];
            let a = actual.texels[idx];
            let mut pixel_exact = true;
            let mut pixel_max = 0u32;
            for c in 0..4 {
                let d = (e[c] as i32 - a[c] as i32).unsigned_abs();
                if d == 0 {
                    exact_channels += 1;
                } else {
                    pixel_exact = false;
                }
                if d <= 1 {
                    within += 1;
                }
                sum_abs += d as u64;
                if d > per_ch[c] {
                    per_ch[c] = d;
                }
                if d > pixel_max {
                    pixel_max = d;
                }
                if d > max_abs {
                    max_abs = d;
                    max_at = [x, y];
                }
            }
            if pixel_exact {
                exact_pixels += 1;
            }
            let region = Region::classify(e) as usize;
            region_acc[region].0 += 1; // pixels
            if pixel_exact {
                region_acc[region].1 += 1; // exact pixels
            }
            if pixel_max > region_acc[region].2 {
                region_acc[region].2 = pixel_max; // max err
            }
            region_acc[region].3 += pixel_max as u64; // sum of per-pixel max err
        }
    }

    let regions = [Region::Clear, Region::Seafloor, Region::Edge]
        .iter()
        .map(|region| {
            let (pixels, exact, max_err, sum_err) = region_acc[*region as usize];
            RegionStats {
                label: region.label().to_string(),
                pixels,
                exact_pixels: exact,
                exact_pixel_fraction: exact as f64 / pixels.max(1) as f64,
                max_abs_error: max_err,
                mean_abs_error: sum_err as f64 / pixels.max(1) as f64,
            }
        })
        .collect();

    let total_channels = total_pixels * 4;
    Stats {
        total_pixels,
        exact_pixels,
        exact_pixel_fraction: exact_pixels as f64 / total_pixels.max(1) as f64,
        total_channels,
        exact_channels,
        exact_channel_fraction: exact_channels as f64 / total_channels.max(1) as f64,
        within_one_level_fraction: within as f64 / total_channels.max(1) as f64,
        max_abs_error: max_abs,
        mean_abs_error: sum_abs as f64 / total_channels.max(1) as f64,
        per_channel_max_abs_error: per_ch,
        max_error_at: max_at,
        regions,
    }
}

// ---------- BMP writers (24-bit BGR, bottom-up) ----------

fn write_bmp_raw(path: &Path, width: usize, height: usize, rgb: &[[u8; 3]]) -> Result<()> {
    let row_padded = (width * 3 + 3) & !3;
    let image_size = row_padded * height;
    let file_size = 54 + image_size;
    let mut buf = Vec::with_capacity(file_size);
    buf.extend_from_slice(b"BM");
    buf.extend_from_slice(&(file_size as u32).to_le_bytes());
    buf.extend_from_slice(&0u32.to_le_bytes());
    buf.extend_from_slice(&54u32.to_le_bytes());
    buf.extend_from_slice(&40u32.to_le_bytes());
    buf.extend_from_slice(&(width as i32).to_le_bytes());
    buf.extend_from_slice(&(height as i32).to_le_bytes());
    buf.extend_from_slice(&1u16.to_le_bytes());
    buf.extend_from_slice(&24u16.to_le_bytes());
    buf.extend_from_slice(&0u32.to_le_bytes());
    buf.extend_from_slice(&(image_size as u32).to_le_bytes());
    buf.extend_from_slice(&2835i32.to_le_bytes());
    buf.extend_from_slice(&2835i32.to_le_bytes());
    buf.extend_from_slice(&0u32.to_le_bytes());
    buf.extend_from_slice(&0u32.to_le_bytes());
    for y in (0..height).rev() {
        for x in 0..width {
            let [r, g, b] = rgb[y * width + x];
            buf.push(b);
            buf.push(g);
            buf.push(r);
        }
        for _ in 0..(row_padded - width * 3) {
            buf.push(0);
        }
    }
    let mut file = fs::File::create(path)?;
    file.write_all(&buf)?;
    Ok(())
}

fn write_bmp_bgra(path: &Path, image: &Bgra8Image) -> Result<()> {
    // BGRA -> RGB (drop alpha for the preview).
    let rgb: Vec<[u8; 3]> = image.texels.iter().map(|t| [t[2], t[1], t[0]]).collect();
    write_bmp_raw(path, image.width, image.height, &rgb)
}

fn write_diff_heatmap(path: &Path, expected: &Bgra8Image, actual: &Bgra8Image) -> Result<()> {
    let rgb: Vec<[u8; 3]> = (0..expected.texels.len())
        .map(|i| {
            let e = expected.texels[i];
            let a = actual.texels[i];
            let d = (0..4)
                .map(|c| (e[c] as i32 - a[c] as i32).unsigned_abs())
                .max()
                .unwrap_or(0);
            heat(d as f32) // 1 8-bit level -> mid ramp; scaled below
        })
        .collect();
    write_bmp_raw(path, expected.width, expected.height, &rgb)
}

fn write_region_map(path: &Path, expected: &Bgra8Image, actual: &Bgra8Image) -> Result<()> {
    // Region color, dimmed where the pixel matches, bright where it differs.
    let rgb: Vec<[u8; 3]> = (0..expected.texels.len())
        .map(|i| {
            let e = expected.texels[i];
            let a = actual.texels[i];
            let region = Region::classify(e);
            let differs = e != a;
            let mut c = region.color();
            if !differs {
                for ch in &mut c {
                    *ch /= 3;
                }
            }
            c
        })
        .collect();
    write_bmp_raw(path, expected.width, expected.height, &rgb)
}

/// Blue(0) -> green -> red heat ramp. Input is an 8-bit-level abs error;
/// 0 maps to blue, >= ~8 levels saturates red so small residuals are visible.
fn heat(levels: f32) -> [u8; 3] {
    let t = (levels / 8.0).clamp(0.0, 1.0);
    if t < 0.5 {
        let u = t / 0.5;
        [0, (u * 255.0) as u8, ((1.0 - u) * 255.0) as u8]
    } else {
        let u = (t - 0.5) / 0.5;
        [(u * 255.0) as u8, ((1.0 - u) * 255.0) as u8, 0]
    }
}

// ---------- JSON report ----------

#[derive(Serialize)]
struct ValidationReport {
    session: String,
    oracle_mode: String,
    game_dir: String,
    selection_source: String,
    shader_source: String,
    effect_name: String,
    underwater_ps_byte_len: usize,
    constants: ConstantsJson,
    dims: DimsJson,
    stats: Stats,
    gate_byte_exact: bool,
}

#[derive(Serialize)]
struct ConstantsJson {
    water_height: f32,
    terrain_water_clip_height: f32,
    clip_height_threshold: f32,
    water_color_lightness: f32,
    map_specular_width: f32,
    shadow_weight_terrain: f32,
    depth_fog_min_y: f32,
    depth_fog_max_y: f32,
}

#[derive(Serialize)]
struct DimsJson {
    width: usize,
    height: usize,
}
