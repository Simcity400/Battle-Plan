//! Files-first: province geometry + ownership come from the committed rust map
//! bundle (which is generated from the shipped map files); display strings come

use std::collections::{BTreeMap, BTreeSet};
use std::env;
use std::fs;
use std::path::{Path, PathBuf};

#[path = "dump_shared/bmp_out.rs"]
mod bmp_out;

use map_viewer::{
    compute_country_label_placements, localisation_table_from_resolver, CountryLabelPlacement,
    GameFileResolver, LabelLawInputs, ProvinceKind, Result,
};
use serde::Serialize;

const BASELINE_SAMPLES: usize = 9;

#[derive(Clone, Debug, Serialize)]
struct RegionSpec {
    name: String,
    x: usize,
    y: usize,
    width: usize,
    height: usize,
    #[serde(skip_serializing_if = "String::is_empty")]
    file: String,
}

#[derive(Clone, Debug, Serialize)]
struct LabelJson {
    tag: String,
    name: String,
    anchor_x: f32,
    anchor_y: f32,
    angle_deg: f32,
    angle_rad: f32,
    half_length: f32,
    bow: f32,
    world_height: f32,
    pixel_count: u64,
    baseline: Vec<[f32; 2]>,
}

#[derive(Clone, Debug, Serialize)]
struct LayoutJson {
    schema: &'static str,
    bundle_dimensions: [usize; 2],
    language: String,
    label_count: usize,
    regions: Vec<RegionSpec>,
    labels: Vec<LabelJson>,
}

fn main() -> Result<()> {
    let mut bundle_dir = None;
    let mut game = None;
    let mut mods = Vec::new();
    let mut out = None;
    let mut regions: Vec<RegionSpec> = Vec::new();
    let mut language = "english".to_string();
    let mut home_only = false;
    let mut args = env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--bundle" => bundle_dir = args.next().map(PathBuf::from),
            "--game" => game = args.next().map(PathBuf::from),
            "--mod" => mods.push(PathBuf::from(args.next().ok_or("--mod needs a dir")?)),
            "--out" => out = args.next().map(PathBuf::from),
            "--language" => language = args.next().ok_or("--language needs a value")?,
            "--home-only" => home_only = true,
            "--region" => regions.push(parse_region(
                &args.next().ok_or("--region needs name:x:y:w:h")?,
            )?),
            "--help" | "-h" => {
                print_usage();
                return Ok(());
            }
            other => return Err(format!("unknown argument {other}").into()),
        }
    }
    let bundle_dir = bundle_dir.ok_or("missing --bundle <map bundle dir>")?;
    let game = game.ok_or("missing --game <game dir>")?;
    let out = out.ok_or("missing --out <dir>")?;
    fs::create_dir_all(&out)?;

    let bundle = LabelBundle::load(&bundle_dir)?;
    let resolver = if mods.is_empty() {
        GameFileResolver::vanilla(game)?
    } else {
        GameFileResolver::with_mod_paths(game, mods)?
    };
    let names = localisation_table_from_resolver(&resolver, &language)?;

    let inputs = LabelLawInputs {
        province_ids: &bundle.province_ids,
        width: bundle.width,
        height: bundle.height,
        province_to_state: &bundle.province_to_state,
        owners: &bundle.owners,
        impassable_states: &bundle.impassable_states,
        province_importance: &bundle.province_importance,
        province_type: &bundle.province_type,
    };
    let mut placements = compute_country_label_placements(&inputs);
    if home_only {
        placements.retain(|p| p.is_home);
    }

    if regions.is_empty() {
        regions = default_regions(bundle.width, bundle.height);
    }

    let land = bundle.land_mask();

    let mut region_specs = Vec::new();
    for mut region in regions {
        clamp_region(&mut region, bundle.width, bundle.height)?;
        let file = format!("labels_{}.bmp", sanitize(&region.name));
        render_region(
            &bundle,
            &land,
            &placements,
            &names,
            &region,
            &out.join(&file),
        )?;
        region.file = file;
        region_specs.push(region);
    }

    let labels_json: Vec<LabelJson> = placements.iter().map(|p| label_json(p, &names)).collect();
    let layout = LayoutJson {
        schema: "battle-plan-map-label-layout-v1",
        bundle_dimensions: [bundle.width, bundle.height],
        language,
        label_count: labels_json.len(),
        regions: region_specs.clone(),
        labels: labels_json,
    };
    let json_path = out.join("map_label_layout.json");
    fs::write(&json_path, serde_json::to_vec_pretty(&layout)?)?;

    let distinct_tags = placements
        .iter()
        .map(|p| p.tag.as_str())
        .collect::<BTreeSet<_>>()
        .len();
    println!(
        "map label layout ({}): {} labels across {} countries, {} localisation keys",
        if home_only {
            "home-only (pre-LABGRAD1)"
        } else {
            "per-component (LABGRAD1 default)"
        },
        placements.len(),
        distinct_tags,
        names.len()
    );
    println!("layout json: {}", json_path.display());
    for region in &region_specs {
        println!(
            "dump: {} -> {}",
            region.name,
            out.join(&region.file).display()
        );
    }
    for tag in ["GER", "SOV", "USA"] {
        if let Some(p) = placements.iter().find(|p| p.tag == tag) {
            println!(
                "  {tag}: anchor ({:.1},{:.1}) angle {:.1} deg half_len {:.1} bow {:.1} h {:.1} px {}",
                p.anchor_x,
                p.anchor_y,
                p.angle.to_degrees(),
                p.half_length,
                p.bow,
                p.world_height,
                p.pixel_count
            );
        } else {
            println!("  {tag}: NOT LABELLED (no passable owned territory sampled)");
        }
    }
    Ok(())
}

fn label_json(p: &CountryLabelPlacement, names: &BTreeMap<String, String>) -> LabelJson {
    let mut baseline = Vec::with_capacity(BASELINE_SAMPLES);
    for i in 0..BASELINE_SAMPLES {
        let t = (i as f32 / (BASELINE_SAMPLES - 1) as f32) * 2.0 - 1.0;
        let (x, y) = p.baseline_point(t);
        baseline.push([x, y]);
    }
    LabelJson {
        tag: p.tag.clone(),
        name: names.get(&p.tag).cloned().unwrap_or_else(|| p.tag.clone()),
        anchor_x: p.anchor_x,
        anchor_y: p.anchor_y,
        angle_deg: p.angle.to_degrees(),
        angle_rad: p.angle,
        half_length: p.half_length,
        bow: p.bow,
        world_height: p.world_height,
        pixel_count: p.pixel_count,
        baseline,
    }
}

/// Minimal bundle read: exactly the raster + ownership the law needs.
struct LabelBundle {
    width: usize,
    height: usize,
    province_ids: Vec<u32>,
    province_to_state: BTreeMap<u32, u32>,
    owners: BTreeMap<u32, String>,
    impassable_states: BTreeSet<u32>,
    province_importance: BTreeMap<u32, u64>,
    province_type: BTreeMap<u32, ProvinceKind>,
}

impl LabelBundle {
    fn load(dir: &Path) -> Result<LabelBundle> {
        let manifest: serde_json::Value =
            serde_json::from_slice(&read(&dir.join("manifest.json"))?)?;
        let dims = manifest
            .get("dimensions")
            .ok_or("manifest has no dimensions")?;
        let width = dims
            .get("width")
            .and_then(|v| v.as_u64())
            .ok_or("no width")? as usize;
        let height = dims
            .get("height")
            .and_then(|v| v.as_u64())
            .ok_or("no height")? as usize;
        let file = |key: &str, default: &str| -> String {
            manifest
                .get("files")
                .and_then(|f| f.get(key))
                .and_then(|v| v.as_str())
                .unwrap_or(default)
                .to_string()
        };
        let province_ids = read_u32_raw(
            &dir.join(file("province_ids", "province_ids.r32")),
            width * height,
        )?;
        let province_to_state =
            read_u32_u32(&dir.join(file("province_to_state", "province_to_state.json")))?;
        let owners = read_u32_string(&dir.join(file("state_owner_1936", "state_owner_1936.json")))?;
        let (impassable_states, province_importance) =
            read_state_meta(&dir.join(file("state_meta", "state_meta.json")))?;
        let province_type = read_province_types(
            &dir.join(file("province_definitions", "province_definitions.json")),
        )?;
        Ok(LabelBundle {
            width,
            height,
            province_ids,
            province_to_state,
            owners,
            impassable_states,
            province_importance,
            province_type,
        })
    }

    fn land_mask(&self) -> Vec<bool> {
        let mut mask = vec![false; self.width * self.height];
        for (i, &pid) in self.province_ids.iter().enumerate() {
            if pid != 0 && self.province_to_state.contains_key(&pid) {
                mask[i] = true;
            }
        }
        mask
    }
}

fn read(path: &Path) -> Result<Vec<u8>> {
    fs::read(path).map_err(|e| format!("read {}: {e}", path.display()).into())
}

fn read_u32_raw(path: &Path, expected: usize) -> Result<Vec<u32>> {
    let bytes = read(path)?;
    if bytes.len() != expected * 4 {
        return Err(format!(
            "{}: {} bytes, expected {} u32",
            path.display(),
            bytes.len(),
            expected
        )
        .into());
    }
    Ok(bytes
        .chunks_exact(4)
        .map(|c| u32::from_le_bytes([c[0], c[1], c[2], c[3]]))
        .collect())
}

fn read_u32_u32(path: &Path) -> Result<BTreeMap<u32, u32>> {
    let map: BTreeMap<String, serde_json::Value> = serde_json::from_slice(&read(path)?)?;
    let mut out = BTreeMap::new();
    for (k, v) in map {
        let key = k
            .parse::<u32>()
            .map_err(|_| format!("{}: bad key {k}", path.display()))?;
        let val =
            v.as_u64()
                .ok_or_else(|| format!("{}: bad value at {k}", path.display()))? as u32;
        out.insert(key, val);
    }
    Ok(out)
}

fn read_u32_string(path: &Path) -> Result<BTreeMap<u32, String>> {
    let map: BTreeMap<String, serde_json::Value> = serde_json::from_slice(&read(path)?)?;
    let mut out = BTreeMap::new();
    for (k, v) in map {
        let key = k
            .parse::<u32>()
            .map_err(|_| format!("{}: bad key {k}", path.display()))?;
        let val = v
            .as_str()
            .ok_or_else(|| format!("{}: bad tag at {k}", path.display()))?;
        out.insert(key, val.to_string());
    }
    Ok(out)
}

fn read_province_types(path: &Path) -> Result<BTreeMap<u32, ProvinceKind>> {
    let map: BTreeMap<String, serde_json::Value> = serde_json::from_slice(&read(path)?)?;
    let mut out = BTreeMap::new();
    for (k, v) in map {
        let Ok(pid) = k.parse::<u32>() else { continue };
        let kind = v
            .get("province_type")
            .and_then(|t| t.as_str())
            .map(ProvinceKind::from_type_str)
            .unwrap_or(ProvinceKind::Land);
        out.insert(pid, kind);
    }
    Ok(out)
}

fn read_state_meta(path: &Path) -> Result<(BTreeSet<u32>, BTreeMap<u32, u64>)> {
    let map: BTreeMap<String, serde_json::Value> = serde_json::from_slice(&read(path)?)?;
    let mut impassable = BTreeSet::new();
    let mut importance: BTreeMap<u32, u64> = BTreeMap::new();
    for (k, v) in map {
        if v.get("impassable").and_then(|b| b.as_bool()) == Some(true) {
            if let Ok(id) = k.parse::<u32>() {
                impassable.insert(id);
            }
        }
        if let Some(vps) = v.get("victory_points").and_then(|o| o.as_object()) {
            for (province_id, vp) in vps {
                if let (Ok(pid), Some(points)) = (province_id.parse::<u32>(), vp.as_i64()) {
                    if points > 0 {
                        *importance.entry(pid).or_insert(0) += points as u64;
                    }
                }
            }
        }
    }
    Ok((impassable, importance))
}

fn parse_region(raw: &str) -> Result<RegionSpec> {
    let parts: Vec<&str> = raw.split(':').collect();
    if parts.len() != 5 {
        return Err(format!("invalid --region {raw:?}; expected name:x:y:w:h").into());
    }
    Ok(RegionSpec {
        name: parts[0].to_string(),
        x: parts[1].parse()?,
        y: parts[2].parse()?,
        width: parts[3].parse()?,
        height: parts[4].parse()?,
        file: String::new(),
    })
}

fn default_regions(width: usize, height: usize) -> Vec<RegionSpec> {
    vec![
        RegionSpec {
            name: "full".to_string(),
            x: 0,
            y: 0,
            width,
            height,
            file: String::new(),
        },
        RegionSpec {
            name: "europe".to_string(),
            x: 2500.min(width.saturating_sub(1)),
            y: 250.min(height.saturating_sub(1)),
            width: 1400,
            height: 900,
            file: String::new(),
        },
        RegionSpec {
            name: "asia".to_string(),
            x: 3600.min(width.saturating_sub(1)),
            y: 300.min(height.saturating_sub(1)),
            width: 1900,
            height: 1100,
            file: String::new(),
        },
    ]
}

fn clamp_region(region: &mut RegionSpec, width: usize, height: usize) -> Result<()> {
    if region.width == 0 || region.height == 0 {
        return Err(format!("region {} has empty dimensions", region.name).into());
    }
    if region.x >= width || region.y >= height {
        return Err(format!(
            "region {} origin {},{} outside map {}x{}",
            region.name, region.x, region.y, width, height
        )
        .into());
    }
    region.width = region.width.min(width - region.x);
    region.height = region.height.min(height - region.y);
    Ok(())
}

fn render_region(
    bundle: &LabelBundle,
    land: &[bool],
    placements: &[CountryLabelPlacement],
    names: &BTreeMap<String, String>,
    region: &RegionSpec,
    path: &Path,
) -> Result<()> {
    let scale = (region.width.max(region.height) as f32 / 1600.0)
        .ceil()
        .max(1.0) as usize;
    let out_w = region.width / scale;
    let out_h = region.height / scale;
    let mut pixels = vec![[20u8, 24u8, 30u8]; out_w * out_h];

    for oy in 0..out_h {
        for ox in 0..out_w {
            let mx = region.x + ox * scale;
            let my = region.y + oy * scale;
            if mx < bundle.width && my < bundle.height && land[my * bundle.width + mx] {
                pixels[oy * out_w + ox] = [64, 72, 78];
            }
        }
    }

    let to_local = |mx: f32, my: f32| -> (i32, i32) {
        (
            ((mx - region.x as f32) / scale as f32).round() as i32,
            ((my - region.y as f32) / scale as f32).round() as i32,
        )
    };

    for p in placements {
        let mut prev: Option<(i32, i32)> = None;
        for i in 0..=32 {
            let t = (i as f32 / 32.0) * 2.0 - 1.0;
            let (mx, my) = p.baseline_point(t);
            let (lx, ly) = to_local(mx, my);
            if let Some((px, py)) = prev {
                draw_line(&mut pixels, out_w, out_h, px, py, lx, ly, [90, 220, 120]);
            }
            prev = Some((lx, ly));
        }
        draw_text_box(&mut pixels, out_w, out_h, p, &to_local, [235, 235, 235]);
        let (ax, ay) = to_local(p.anchor_x, p.anchor_y);
        draw_cross(&mut pixels, out_w, out_h, ax, ay, [235, 90, 80]);
        let _ = names;
    }

    bmp_out::write_bmp24_rgb(path, out_w, out_h, &pixels)
}

fn draw_text_box(
    pixels: &mut [[u8; 3]],
    w: usize,
    h: usize,
    p: &CountryLabelPlacement,
    to_local: &impl Fn(f32, f32) -> (i32, i32),
    color: [u8; 3],
) {
    let half_h = p.world_height * 0.5;
    let mut top_prev: Option<(i32, i32)> = None;
    let mut bot_prev: Option<(i32, i32)> = None;
    for i in 0..=32 {
        let t = (i as f32 / 32.0) * 2.0 - 1.0;
        let (bx, by) = p.baseline_point(t);
        let ang = p.tangent_angle(t);
        let (sin, cos) = ang.sin_cos();
        let (tx, ty) = (bx - half_h * sin, by + half_h * cos);
        let (bx2, by2) = (bx + half_h * sin, by - half_h * cos);
        let (tlx, tly) = to_local(tx, ty);
        let (blx, bly) = to_local(bx2, by2);
        if let Some((px, py)) = top_prev {
            draw_line(pixels, w, h, px, py, tlx, tly, color);
        }
        if let Some((px, py)) = bot_prev {
            draw_line(pixels, w, h, px, py, blx, bly, color);
        }
        top_prev = Some((tlx, tly));
        bot_prev = Some((blx, bly));
    }
}

fn draw_cross(pixels: &mut [[u8; 3]], w: usize, h: usize, cx: i32, cy: i32, color: [u8; 3]) {
    for d in -3..=3 {
        put(pixels, w, h, cx + d, cy, color);
        put(pixels, w, h, cx, cy + d, color);
    }
}

fn draw_line(
    pixels: &mut [[u8; 3]],
    w: usize,
    h: usize,
    mut x0: i32,
    mut y0: i32,
    x1: i32,
    y1: i32,
    color: [u8; 3],
) {
    let dx = (x1 - x0).abs();
    let sx = if x0 < x1 { 1 } else { -1 };
    let dy = -(y1 - y0).abs();
    let sy = if y0 < y1 { 1 } else { -1 };
    let mut err = dx + dy;
    loop {
        put(pixels, w, h, x0, y0, color);
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

fn put(pixels: &mut [[u8; 3]], w: usize, h: usize, x: i32, y: i32, color: [u8; 3]) {
    if x < 0 || y < 0 || x >= w as i32 || y >= h as i32 {
        return;
    }
    pixels[y as usize * w + x as usize] = color;
}

fn sanitize(name: &str) -> String {
    name.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '_' || c == '-' {
                c
            } else {
                '_'
            }
        })
        .collect()
}

fn print_usage() {
    println!(
        "Usage: dump_map_labels --bundle <map bundle dir> --game <the game dir> [--mod <dir>]... \
         --out <dir> [--region name:x:y:w:h]... [--language english] [--home-only]\n  \
         --home-only reproduces the pre-LABGRAD1 one-label-per-country law (home component \
         only); the default renders the graduated per-component set (one label per detached \
         controlled region)."
    );
}
