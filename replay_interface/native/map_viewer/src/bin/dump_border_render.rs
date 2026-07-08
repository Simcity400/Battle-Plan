//! BORD2 validation: prove the camera-height fade law drives per-class border
//! alpha by rendering the synthesized border mesh at three camera heights and
//! dumping one BMP per height plus a per-class stats line.
//!
//! No live `borders_pass` was ever captured, so this is a VISUAL + STRUCTURAL
//! validation, not a byte-parity gate: it rasterizes the synthesized border
//! polylines on the CPU with each class alpha-composited using the exact
//! transparency [`map_viewer::border_transparency`] returns for that class at
//! the height, so the three dumps must show province borders present at the
//! close height, gone at the mid height (province window 200–300), the state
//! grid fading over the mid→far heights (state window 400–500), and only the
//! opaque classes (country/coast/sea-region) surviving at the far height. The
//! GPU pass consumes the SAME transparency values through the shipped border
//! pixel shader's `vTime_Transparency.y`; this dump is the CPU oracle for that
//! per-class fade.
//!
//! Usage:
//!   dump_border_render --game <the game dir> [--mod <dir>]... --out <dir>
//!       [--region name:x:y:w:h] [--height H]...
//!
//! Defaults: heights 150 (close), 350 (mid), 600 (far); a Europe close-up region.

use map_viewer::{
    all_border_texture_profiles_from_resolver, border_transparency,
    synthesize_border_world_mesh_from_resolver, BorderChain, BorderClassIndexPartition,
    BorderFadeWindows, BorderMaterial, BorderTextureProfile, BorderWorldMesh, GameFileResolver,
    Result,
};
use serde::Serialize;
use std::collections::BTreeMap;
use std::env;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, Serialize)]
struct HeightRegime {
    camera_height: f32,
    bmp: String,
    /// Per-material `(segments_drawn, transparency)` at this height. A class is
    /// "drawn" when its transparency > 0.
    classes: Vec<ClassAtHeight>,
    total_segments_drawn: usize,
}

#[derive(Clone, Debug, Serialize)]
struct ClassAtHeight {
    material: String,
    segments: usize,
    /// Camera-height fade multiplier ([`border_transparency`]) at this height.
    transparency: f32,
    /// The class's base opacity decoded from its shipped DDS peak alpha
    /// (`peak_alpha/255`) — the texture's own translucency before the fade.
    base_opacity: f32,
    /// `transparency * base_opacity` — the effective alpha the CPU oracle
    /// composites, matching the GPU shader's `vColor.a * vTime_Transparency.y`.
    effective_alpha: f32,
    /// The authentic ink RGB from the shipped DDS (never a synthetic tint).
    ink_rgb: [u8; 3],
    drawn: bool,
}

#[derive(Clone, Debug, Serialize)]
struct DumpStats {
    fade_windows: FadeWindowsReport,
    ramp: String,
    region: RegionReport,
    heights: Vec<HeightRegime>,
    draw_order_evidence: String,
}

#[derive(Clone, Debug, Serialize)]
struct FadeWindowsReport {
    province_near: f32,
    province_far: f32,
    state_near: f32,
    state_far: f32,
    impassable_window: String,
    opaque_classes: Vec<String>,
}

#[derive(Clone, Debug, Serialize)]
struct RegionReport {
    name: String,
    x: usize,
    y: usize,
    width: usize,
    height: usize,
}

struct Region {
    name: String,
    x: usize,
    y: usize,
    width: usize,
    height: usize,
}

fn main() -> Result<()> {
    let mut game = None;
    let mut mods: Vec<PathBuf> = Vec::new();
    let mut out = None;
    let mut region: Option<Region> = None;
    let mut heights: Vec<f32> = Vec::new();
    let mut args = env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--game" => game = args.next().map(PathBuf::from),
            "--mod" => mods.push(PathBuf::from(args.next().ok_or("--mod needs a dir")?)),
            "--out" => out = args.next().map(PathBuf::from),
            "--region" => {
                region = Some(parse_region(
                    &args.next().ok_or("--region needs name:x:y:w:h")?,
                )?)
            }
            "--height" => heights.push(
                args.next()
                    .ok_or("--height needs a number")?
                    .parse()
                    .map_err(|e| format!("--height: {e}"))?,
            ),
            "--help" | "-h" => {
                println!(
                    "Usage: dump_border_render --game <the game dir> [--mod <dir>]... --out <dir> \
                     [--region name:x:y:w:h] [--height H]..."
                );
                return Ok(());
            }
            other => return Err(format!("unknown argument {other}").into()),
        }
    }
    let game = game.ok_or("missing --game <game dir>")?;
    let out = out.ok_or("missing --out <dir>")?;
    if heights.is_empty() {
        heights = vec![150.0, 350.0, 600.0];
    }
    let resolver = if mods.is_empty() {
        GameFileResolver::vanilla(game)?
    } else {
        GameFileResolver::with_mod_paths(game, mods)?
    };
    fs::create_dir_all(&out)?;

    let mesh = synthesize_border_world_mesh_from_resolver(&resolver)?;
    let windows = BorderFadeWindows::vanilla();
    let partition = mesh.class_index_partition();
    // BORD3: the CPU oracle paints each class with the AUTHENTIC colour and base
    // opacity decoded from its shipped `border_<class>_0.dds` (never a synthetic
    // per-class tint), so a "red band" can only appear if a real texture is red
    // — which none are. See border_texture_law.rs.
    let profiles: BTreeMap<BorderMaterial, BorderTextureProfile> =
        all_border_texture_profiles_from_resolver(&resolver)?
            .into_iter()
            .map(|p| (p.material, p))
            .collect();
    let region = region.unwrap_or_else(|| default_region(&mesh));
    let region = clamp_region(region, mesh.map_width, mesh.map_height)?;

    let mut height_regimes = Vec::new();
    for &height in &heights {
        let bmp = format!("border_h{}.bmp", height as i32);
        let path = out.join(&bmp);
        let classes = render_height_bmp(
            &mesh, &partition, &windows, &profiles, &region, height, &path,
        )?;
        let total_segments_drawn = classes.iter().filter(|c| c.drawn).map(|c| c.segments).sum();
        height_regimes.push(HeightRegime {
            camera_height: height,
            bmp,
            classes,
            total_segments_drawn,
        });
    }

    let stats = DumpStats {
        fade_windows: FadeWindowsReport {
            province_near: windows.province.near,
            province_far: windows.province.far,
            state_near: windows.state.near,
            state_far: windows.state.far,
            impassable_window: "state (400-500) — impassable is rank=State; tracked approximation"
                .to_string(),
            opaque_classes: vec!["country".into(), "sea_coast".into(), "sea_region".into()],
        },
        ramp: "linear over [NEAR,FAR] (approximation, see BORDER_LAW.md); endpoints certain, \
               curve pending live borders_pass capture"
            .to_string(),
        region: RegionReport {
            name: region.name.clone(),
            x: region.x,
            y: region.y,
            width: region.width,
            height: region.height,
        },
        heights: height_regimes,
        draw_order_evidence: map_viewer::BORDER_PASS_DRAW_ORDER_EVIDENCE.to_string(),
    };
    let stats_path = out.join("border_render_stats.json");
    fs::write(&stats_path, serde_json::to_vec_pretty(&stats)?)?;

    println!("border render validation — fade windows drive per-class alpha:");
    println!(
        "  province window {}-{}, state window {}-{}, impassable=state, opaque: country/coast/sea_region",
        windows.province.near, windows.province.far, windows.state.near, windows.state.far
    );
    for regime in &stats.heights {
        let per_class = regime
            .classes
            .iter()
            .map(|c| {
                format!(
                    "{}={}@fade{:.2}xtex{:.2}=a{:.2}rgb{:?}",
                    c.material,
                    c.segments,
                    c.transparency,
                    c.base_opacity,
                    c.effective_alpha,
                    c.ink_rgb
                )
            })
            .collect::<Vec<_>>()
            .join(" ");
        println!(
            "  h={:>4}: {} | drawn_segments={} -> {}",
            regime.camera_height as i32, per_class, regime.total_segments_drawn, regime.bmp
        );
    }
    println!("  stats: {}", stats_path.display());
    Ok(())
}

/// Rasterize the region at one camera height. Each class is painted with its
/// [`border_transparency`] alpha over the background; a class with transparency
/// 0 contributes nothing (proving the fade). Returns per-class stats.
fn render_height_bmp(
    mesh: &BorderWorldMesh,
    partition: &BorderClassIndexPartition,
    windows: &BorderFadeWindows,
    profiles: &BTreeMap<BorderMaterial, BorderTextureProfile>,
    region: &Region,
    height: f32,
    path: &Path,
) -> Result<Vec<ClassAtHeight>> {
    let mut pixels = vec![[24u8, 29u8, 31u8]; region.width * region.height];
    let mut classes = Vec::new();
    // Paint in the same material draw order the GPU pass and partition use, so
    // the CPU oracle and the GPU pass agree on overlap winners.
    for material in BorderClassIndexPartition::MATERIAL_DRAW_ORDER {
        let fade = border_transparency(windows, material, height);
        // BORD3: the class's authentic ink colour and base opacity come from its
        // shipped DDS profile. The GPU shader outputs `vColor.a *
        // vTime_Transparency.y` (border.shader:146), so the CPU oracle's
        // effective alpha is the texture's peak alpha times the camera-height
        // fade — not a flat synthetic alpha.
        let profile = profiles.get(&material).ok_or_else(|| {
            format!(
                "border render oracle blocked: no decoded texture profile for {}",
                material_name(material)
            )
        })?;
        let color = profile.peak_rgb;
        let base_opacity = profile.peak_alpha as f32 / 255.0;
        let effective_alpha = fade * base_opacity;
        let chains: Vec<&BorderChain> = mesh
            .chains
            .iter()
            .filter(|chain| chain.material == material)
            .collect();
        let drawn = effective_alpha > 0.0 && !chains.is_empty();
        if drawn {
            for chain in &chains {
                draw_chain_alpha(mesh, region, &mut pixels, chain, color, effective_alpha);
            }
        }
        classes.push(ClassAtHeight {
            material: material_name(material).to_string(),
            segments: chains.len(),
            transparency: fade,
            base_opacity,
            effective_alpha,
            ink_rgb: color,
            drawn,
        });
    }
    // Reference the partition so the buffer/order it defines is exercised here
    // too (the GPU pass draws these exact ranges).
    debug_assert_eq!(
        partition.ranges.iter().map(|r| r.index_count).sum::<u32>() as usize,
        mesh.indices.len()
    );
    let flat: Vec<u8> = pixels.iter().flat_map(|p| p.iter().copied()).collect();
    write_bmp24(path, region.width, region.height, &flat)?;
    Ok(classes)
}

/// Paint one chain's smoothed centerline row-to-row. World -> bmp uses the
/// pinned pixel-center convention: pixel (x, y) center = world (x, (H-1) - y).
fn draw_chain_alpha(
    mesh: &BorderWorldMesh,
    region: &Region,
    pixels: &mut [[u8; 3]],
    chain: &BorderChain,
    color: [u8; 3],
    alpha: f32,
) {
    let centerline = mesh.chain_centerline(chain);
    let w = region.width as i32;
    let h = region.height as i32;
    let thickness = mesh.stats.border_width.ceil().max(1.0) as i32;
    let to_bmp = |p: &[f32; 3]| -> (i32, i32) {
        (
            p[0].round() as i32 - region.x as i32,
            ((mesh.map_height - 1) as f32 - p[2]).round() as i32 - region.y as i32,
        )
    };
    for window in centerline.windows(2) {
        let (x0, y0) = to_bmp(&window[0]);
        let (x1, y1) = to_bmp(&window[1]);
        if (x0 < 0 && x1 < 0) || (y0 < 0 && y1 < 0) || (x0 >= w && x1 >= w) || (y0 >= h && y1 >= h)
        {
            continue;
        }
        draw_line(
            pixels,
            region.width,
            region.height,
            x0,
            y0,
            x1,
            y1,
            thickness,
            color,
            alpha,
        );
    }
}

#[allow(clippy::too_many_arguments)]
fn draw_line(
    pixels: &mut [[u8; 3]],
    width: usize,
    height: usize,
    mut x0: i32,
    mut y0: i32,
    x1: i32,
    y1: i32,
    thickness: i32,
    color: [u8; 3],
    alpha: f32,
) {
    let dx = (x1 - x0).abs();
    let sx = if x0 < x1 { 1 } else { -1 };
    let dy = -(y1 - y0).abs();
    let sy = if y0 < y1 { 1 } else { -1 };
    let mut err = dx + dy;
    let radius = (thickness - 1) / 2;
    loop {
        for py in y0 - radius..=y0 + radius {
            for px in x0 - radius..=x0 + radius {
                if px < 0 || py < 0 || px >= width as i32 || py >= height as i32 {
                    continue;
                }
                let idx = py as usize * width + px as usize;
                blend(&mut pixels[idx], color, alpha);
            }
        }
        if x0 == x1 && y0 == y1 {
            break;
        }
        let e2 = 2 * err;
        if e2 >= dy {
            err += dy;
            x0 += sx;
        }
        if e2 <= dx {
            err += dx;
            y0 += sy;
        }
    }
}

/// Source-over alpha blend, matching the shipped border blend
/// (src_alpha / inv_src_alpha) the GPU pass uses.
fn blend(dst: &mut [u8; 3], src: [u8; 3], alpha: f32) {
    let a = alpha.clamp(0.0, 1.0);
    for i in 0..3 {
        let out = src[i] as f32 * a + dst[i] as f32 * (1.0 - a);
        dst[i] = out.round().clamp(0.0, 255.0) as u8;
    }
}

fn material_name(material: BorderMaterial) -> &'static str {
    match material {
        BorderMaterial::Province => "province",
        BorderMaterial::State => "state",
        BorderMaterial::Country => "country",
        BorderMaterial::SeaCoast => "sea_coast",
        BorderMaterial::SeaRegion => "sea_region",
        BorderMaterial::Impassable => "impassable",
    }
}

fn parse_region(raw: &str) -> Result<Region> {
    let parts = raw.split(':').collect::<Vec<_>>();
    if parts.len() != 5 {
        return Err(format!("invalid --region {raw:?}; expected name:x:y:w:h").into());
    }
    Ok(Region {
        name: parts[0].to_string(),
        x: parts[1].parse()?,
        y: parts[2].parse()?,
        width: parts[3].parse()?,
        height: parts[4].parse()?,
    })
}

fn default_region(mesh: &BorderWorldMesh) -> Region {
    Region {
        name: "europe_closeup".to_string(),
        x: 2500.min(mesh.map_width.saturating_sub(1)),
        y: 500.min(mesh.map_height.saturating_sub(1)),
        width: 1100,
        height: 700,
    }
}

fn clamp_region(mut region: Region, map_width: usize, map_height: usize) -> Result<Region> {
    if region.width == 0 || region.height == 0 {
        return Err(format!("region {} has empty dimensions", region.name).into());
    }
    if region.x >= map_width || region.y >= map_height {
        return Err(format!(
            "region {} origin {},{} outside map {}x{}",
            region.name, region.x, region.y, map_width, map_height
        )
        .into());
    }
    region.width = region.width.min(map_width - region.x);
    region.height = region.height.min(map_height - region.y);
    Ok(region)
}

fn write_bmp24(path: &Path, width: usize, height: usize, rgb: &[u8]) -> Result<()> {
    if rgb.len() != width * height * 3 {
        return Err(format!(
            "BMP payload has {} bytes but {}x{} RGB needs {}",
            rgb.len(),
            width,
            height,
            width * height * 3
        )
        .into());
    }
    let row_stride = (width * 3 + 3) & !3;
    let pixel_offset = 14usize + 40usize;
    let image_bytes = row_stride * height;
    let file_size = pixel_offset + image_bytes;
    let mut file = fs::File::create(path)?;
    file.write_all(b"BM")?;
    file.write_all(&(file_size as u32).to_le_bytes())?;
    file.write_all(&[0u8; 4])?;
    file.write_all(&(pixel_offset as u32).to_le_bytes())?;
    file.write_all(&40u32.to_le_bytes())?;
    file.write_all(&(width as i32).to_le_bytes())?;
    file.write_all(&(height as i32).to_le_bytes())?;
    file.write_all(&1u16.to_le_bytes())?;
    file.write_all(&24u16.to_le_bytes())?;
    file.write_all(&0u32.to_le_bytes())?;
    file.write_all(&(image_bytes as u32).to_le_bytes())?;
    file.write_all(&2835u32.to_le_bytes())?;
    file.write_all(&2835u32.to_le_bytes())?;
    file.write_all(&0u32.to_le_bytes())?;
    file.write_all(&0u32.to_le_bytes())?;
    for y in (0..height).rev() {
        let mut row = Vec::with_capacity(row_stride);
        for x in 0..width {
            let offset = (y * width + x) * 3;
            row.extend_from_slice(&[rgb[offset + 2], rgb[offset + 1], rgb[offset]]);
        }
        row.resize(row_stride, 0);
        file.write_all(&row)?;
    }
    Ok(())
}
