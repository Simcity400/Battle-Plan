//! Synthesize thin border geometry from resolver-routed installed content and dump
//! debug BMP regions plus stats JSON.
//!
//! Usage:
//!   dump_border_synthesis --game <the game dir> [--mod <dir>]... --out <dir>
//!       [--region name:x:y:w:h]... [--source-excerpts]

use map_viewer::{
    border_material_sources_from_resolver, border_source_evidence_from_resolver,
    synthesize_border_world_mesh_from_resolver, BorderChain, BorderMaterial, BorderWorldMesh,
    GameFileResolver, Result,
};
use serde::Serialize;
use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::Instant;

#[path = "dump_shared/bmp_out.rs"]
mod bmp_out;

#[derive(Clone, Debug, Serialize)]
struct DumpRegion {
    name: String,
    x: usize,
    y: usize,
    width: usize,
    height: usize,
    file: String,
}

#[derive(Clone, Debug, Serialize)]
struct DumpStats {
    renderer: String,
    renderer_reason: String,
    synthesis_runtime_ms: f64,
    synthesis: map_viewer::BorderWorldSynthesisStats,
    material_sources: Vec<map_viewer::BorderMaterialSource>,
    regions: Vec<DumpRegion>,
}

fn main() -> Result<()> {
    let mut game = None;
    let mut mods = Vec::new();
    let mut out = None;
    let mut regions = Vec::new();
    let mut source_excerpts = false;
    let mut args = env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--game" => game = args.next().map(PathBuf::from),
            "--mod" => mods.push(PathBuf::from(args.next().ok_or("--mod needs a dir")?)),
            "--out" => out = args.next().map(PathBuf::from),
            "--region" => regions.push(parse_region(
                &args.next().ok_or("--region needs name:x:y:w:h")?,
            )?),
            "--source-excerpts" => source_excerpts = true,
            "--help" | "-h" => {
                print_usage();
                return Ok(());
            }
            other => return Err(format!("unknown argument {other}").into()),
        }
    }
    let game = game.ok_or("missing --game <game dir>")?;
    let resolver = if mods.is_empty() {
        GameFileResolver::vanilla(game)?
    } else {
        GameFileResolver::with_mod_paths(game, mods)?
    };

    if source_excerpts {
        let evidence = border_source_evidence_from_resolver(&resolver)?;
        println!("border source excerpts:");
        for excerpt in &evidence.excerpts {
            println!("{}:{} {}", excerpt.source, excerpt.line, excerpt.text);
        }
        println!("border material sources:");
        for material in &evidence.material_sources {
            println!(
                "{} {} {}x{} {} mips={} fnv1a64={}",
                material.material,
                material.source,
                material.width,
                material.height,
                material.format,
                material.mip_count,
                material.fnv1a64
            );
        }
    }

    let out = out.ok_or("missing --out <dir>")?;
    fs::create_dir_all(&out)?;
    let started = Instant::now();
    let mesh = synthesize_border_world_mesh_from_resolver(&resolver)?;
    let runtime_ms = started.elapsed().as_secs_f64() * 1000.0;
    if regions.is_empty() {
        regions = default_regions(&mesh);
    }

    let mut dumped_regions = Vec::new();
    for mut region in regions {
        clamp_region(&mut region, mesh.map_width, mesh.map_height)?;
        let file = format!("border_{}.bmp", sanitize_name(&region.name));
        let path = out.join(&file);
        render_region_bmp(&mesh, &region, &path)?;
        region.file = file;
        dumped_regions.push(region);
    }

    let material_sources = border_material_sources_from_resolver(&resolver)?;
    let stats = DumpStats {
        renderer: "debug_rasterization_fallback".to_string(),
        renderer_reason: "structural CPU dump of the BORD5 captured-engine chain law (the GPU \
                          pass renders the same mesh through the shipped border.shader)"
            .to_string(),
        synthesis_runtime_ms: runtime_ms,
        synthesis: mesh.stats.clone(),
        material_sources,
        regions: dumped_regions,
    };
    let stats_path = out.join("border_synthesis_stats.json");
    fs::write(&stats_path, serde_json::to_vec_pretty(&stats)?)?;

    println!(
        "border synthesis: {} chains ({} loops), {} raster edges, {} vertices, {} indices, {:.1} ms",
        mesh.chains.len(),
        mesh.stats.closed_loop_chains,
        mesh.stats.raster_edges,
        mesh.vertices.len(),
        mesh.indices.len(),
        runtime_ms
    );
    println!("stats: {}", stats_path.display());
    for region in &stats.regions {
        println!(
            "dump: {} -> {}",
            region.name,
            out.join(&region.file).display()
        );
    }
    println!(
        "coastline check: synthesized={} water-mask={} disagreement={}",
        stats.synthesis.coastline_edges,
        stats.synthesis.water_mask_coastline_edges,
        stats.synthesis.coastline_disagreement_edges
    );
    println!(
        "edges by material: {}",
        stats
            .synthesis
            .edge_count_by_material
            .iter()
            .map(|(material, count)| format!("{material}={count}"))
            .collect::<Vec<_>>()
            .join(" ")
    );
    println!(
        "chains by material: {}",
        stats
            .synthesis
            .chain_count_by_material
            .iter()
            .map(|(material, count)| format!("{material}={count}"))
            .collect::<Vec<_>>()
            .join(" ")
    );
    println!(
        "adjacency impassables: pairs={} edges={} non_land_rows={}",
        stats.synthesis.impassable_adjacency_pairs,
        stats.synthesis.impassable_adjacency_edges,
        stats.synthesis.impassable_adjacency_non_land_rows
    );
    Ok(())
}

fn print_usage() {
    println!(
        "Usage: dump_border_synthesis --game <the game dir> [--mod <dir>]... --out <dir> \
         [--region name:x:y:w:h]... [--source-excerpts]"
    );
}

fn parse_region(raw: &str) -> Result<DumpRegion> {
    let parts = raw.split(':').collect::<Vec<_>>();
    if parts.len() != 5 {
        return Err(format!("invalid --region {raw:?}; expected name:x:y:w:h").into());
    }
    Ok(DumpRegion {
        name: parts[0].to_string(),
        x: parts[1].parse()?,
        y: parts[2].parse()?,
        width: parts[3].parse()?,
        height: parts[4].parse()?,
        file: String::new(),
    })
}

fn default_regions(mesh: &BorderWorldMesh) -> Vec<DumpRegion> {
    vec![
        DumpRegion {
            name: "europe_closeup".to_string(),
            x: 2500.min(mesh.map_width.saturating_sub(1)),
            y: 500.min(mesh.map_height.saturating_sub(1)),
            width: 1100,
            height: 700,
            file: String::new(),
        },
        DumpRegion {
            name: "channel_coastline".to_string(),
            x: 2750.min(mesh.map_width.saturating_sub(1)),
            y: 570.min(mesh.map_height.saturating_sub(1)),
            width: 520,
            height: 340,
            file: String::new(),
        },
    ]
}

fn clamp_region(region: &mut DumpRegion, map_width: usize, map_height: usize) -> Result<()> {
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
    Ok(())
}

fn render_region_bmp(mesh: &BorderWorldMesh, region: &DumpRegion, path: &Path) -> Result<()> {
    let mut pixels = vec![24u8; region.width * region.height * 3];
    for chunk in pixels.chunks_exact_mut(3) {
        chunk.copy_from_slice(&[24, 29, 31]);
    }
    for material in [
        BorderMaterial::SeaRegion,
        BorderMaterial::SeaCoast,
        BorderMaterial::Province,
        BorderMaterial::Impassable,
        BorderMaterial::State,
        BorderMaterial::Country,
    ] {
        for chain in mesh
            .chains
            .iter()
            .filter(|chain| chain.material == material)
        {
            draw_chain(
                mesh,
                region,
                &mut pixels,
                chain,
                color_for_material(material),
            );
        }
    }
    bmp_out::write_bmp24_rgb_flat(path, region.width, region.height, &pixels)
}

/// Draw the chain's smoothed centerline row-to-row. World -> bmp uses the
/// pinned pixel-center convention: pixel (x, y) center = world (x, (H-1) - y).
fn draw_chain(
    mesh: &BorderWorldMesh,
    region: &DumpRegion,
    pixels: &mut [u8],
    chain: &BorderChain,
    color: [u8; 3],
) {
    let centerline = mesh.chain_centerline(chain);
    let to_bmp = |p: &[f32; 3]| -> (i32, i32) {
        (
            p[0].round() as i32 - region.x as i32,
            ((mesh.map_height - 1) as f32 - p[2]).round() as i32 - region.y as i32,
        )
    };
    for window in centerline.windows(2) {
        let (x0, y0) = to_bmp(&window[0]);
        let (x1, y1) = to_bmp(&window[1]);
        if (x0 < -3 && x1 < -3)
            || (y0 < -3 && y1 < -3)
            || (x0 > region.width as i32 + 3 && x1 > region.width as i32 + 3)
            || (y0 > region.height as i32 + 3 && y1 > region.height as i32 + 3)
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
            mesh.stats.border_width.ceil().max(1.0) as i32,
            color,
        );
    }
}

#[allow(clippy::too_many_arguments)]
fn draw_line(
    pixels: &mut [u8],
    width: usize,
    height: usize,
    mut x0: i32,
    mut y0: i32,
    x1: i32,
    y1: i32,
    thickness: i32,
    color: [u8; 3],
) {
    let dx = (x1 - x0).abs();
    let sx = if x0 < x1 { 1 } else { -1 };
    let dy = -(y1 - y0).abs();
    let sy = if y0 < y1 { 1 } else { -1 };
    let mut err = dx + dy;
    loop {
        plot_thick(pixels, width, height, x0, y0, thickness, color);
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

fn plot_thick(
    pixels: &mut [u8],
    width: usize,
    height: usize,
    cx: i32,
    cy: i32,
    thickness: i32,
    color: [u8; 3],
) {
    let radius = (thickness - 1) / 2;
    for y in cy - radius..=cy + radius {
        for x in cx - radius..=cx + radius {
            if x < 0 || y < 0 || x >= width as i32 || y >= height as i32 {
                continue;
            }
            let offset = (y as usize * width + x as usize) * 3;
            pixels[offset..offset + 3].copy_from_slice(&color);
        }
    }
}

/// LEGEND colours for the *structural* geometry dump — these make the six border
/// CLASSES visually separable so the synthesis (which edge got which rank) can be
/// eyeballed. They are explicitly a category legend, **NOT vanilla appearance**:
/// the authentic per-class colours are near-black/navy/brown (decoded in
/// `map_viewer::border_texture_law`; see BORDER_LAW.md), which would be
/// indistinguishable in a structural dump. Because the user-facing appearance
/// mandate forbids implying vanilla draws a red/hot country border, the country
/// legend hue is a neutral teal here rather than the old `[255,83,73]` red — no
/// legend colour resembles a "hot" border. For the AUTHENTIC per-class colour and
/// opacity, see `dump_border_render` (which paints the decoded DDS colours) and
/// the `material_sources` in this dump's stats JSON.
fn color_for_material(material: BorderMaterial) -> [u8; 3] {
    match material {
        BorderMaterial::Province => [178, 184, 188],
        BorderMaterial::State => [236, 205, 120],
        BorderMaterial::Country => [96, 208, 176],
        BorderMaterial::SeaCoast => [92, 225, 255],
        BorderMaterial::SeaRegion => [87, 140, 255],
        BorderMaterial::Impassable => [200, 150, 120],
    }
}

fn sanitize_name(name: &str) -> String {
    name.chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || ch == '_' || ch == '-' {
                ch
            } else {
                '_'
            }
        })
        .collect()
}
