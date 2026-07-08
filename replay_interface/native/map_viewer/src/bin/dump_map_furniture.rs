//! MAPICON1 validation bin: render the reconstructed city/VP name labels,
//! capital markers, and victory-point bullets over a land/water outline for the
//! 1936 vanilla start, for a Europe view and a Pacific view, and emit a JSON
//! evidence ledger of every placed marker plus the game-file citations.
//!
//! Files-first: the province raster (`map/provinces.bmp` + `map/definition.csv`),
//! the victory points and owners (`history/states`), the capitals
//! (`history/countries`), the city/VP name strings (`localisation`), the icon
//! defines (`common/defines/00_graphics.lua`), and the label font
//! (`gfx/fonts/hoi_20bs.fnt` + `.dds`) all come through the mod-aware
//! `GameFileResolver`. The placement law lives in
//! `map_viewer::compute_furniture_markers` / `furniture_layer_from_resolver`.
//! The labels are drawn with the REAL shipped `hoi_20bs` glyph atlas.
//!
//! Usage:
//!   dump_map_furniture --game <the game dir> [--mod <dir>]... --out <dir>
//!       [--region name:x:y:w:h]... [--language english]
//!
//! Outputs (in --out):
//!   furniture_europe.png, furniture_pacific.png (or one per --region)
//!   map_furniture_layout.json (schema + per-marker ledger + citations)

use std::env;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use map_viewer::{
    furniture_layer_from_resolver, load_vp_label_font, vp_strip_texture_path, BmFont,
    FurnitureLayer, FurnitureMarker, FurnitureMarkerKind, GameFileResolver, Result, RgbaFontAtlas,
    VpIconDefines, CORE_GFX_SOURCE, HOI_20BS_ATLAS_SOURCE, HOI_20BS_FNT_SOURCE,
    MAPICONS_GFX_SOURCE, MAPICONS_GUI_SOURCE, VICTORY_POINTS_LOC_PREFIX, VP_LABEL_FONT_NAME,
    VP_STRIP_FRAME_COUNT, VP_STRIP_SPRITE_NAME, VP_STRIP_SPRITE_TEXTURE,
};
use serde::Serialize;

/// Label glyph scale relative to the raw `hoi_20bs` atlas size, so the text is
/// legible at the dump's downscale. This is a VALIDATION-render zoom only, not a
/// placement law — the real GUI text size is engine/camera driven (see the
/// ledger's UNTRACED note). It never changes any marker position.
const LABEL_RENDER_SCALE: f32 = 1.0;

#[derive(Clone, Debug, Serialize)]
struct RegionSpec {
    name: String,
    x: usize,
    y: usize,
    width: usize,
    height: usize,
    #[serde(skip_serializing_if = "String::is_empty")]
    file: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    markers_in_view: Option<usize>,
}

#[derive(Clone, Debug, Serialize)]
struct MarkerJson {
    province_id: u32,
    state_id: u32,
    owner: String,
    kind: String,
    anchor_x: f32,
    anchor_y: f32,
    vp_value: i32,
    dot_tier: u32,
    label: String,
    label_resolved: bool,
    pixel_count: u64,
}

#[derive(Clone, Debug, Serialize)]
struct Citation {
    parameter: String,
    source: String,
    detail: String,
}

#[derive(Clone, Debug, Serialize)]
struct LayoutJson {
    schema: &'static str,
    map_dimensions: [usize; 2],
    language: String,
    marker_count: usize,
    capital_marker_count: usize,
    localisation_key_count: usize,
    capital_count: usize,
    label_font: String,
    label_font_glyphs: usize,
    vp_strip_sprite: String,
    vp_strip_frames: u32,
    vp_icon_defines: VpIconDefinesJson,
    citations: Vec<Citation>,
    untraced: Vec<Citation>,
    regions: Vec<RegionSpec>,
    markers: Vec<MarkerJson>,
}

#[derive(Clone, Debug, Serialize)]
struct VpIconDefinesJson {
    levels: u32,
    map_icon_after: Vec<i64>,
    text_cutoff: Vec<f32>,
    text_cutoff_min: f32,
    text_cutoff_max: f32,
    dot_cutoff_min: f32,
    dot_cutoff_max: f32,
    max_victory_points_for_percent: f32,
    capital_cutoff_max: f32,
    capital_icon_cutoff: f32,
}

impl From<&VpIconDefines> for VpIconDefinesJson {
    fn from(d: &VpIconDefines) -> Self {
        VpIconDefinesJson {
            levels: d.levels,
            map_icon_after: d.map_icon_after.clone(),
            text_cutoff: d.text_cutoff.clone(),
            text_cutoff_min: d.text_cutoff_min,
            text_cutoff_max: d.text_cutoff_max,
            dot_cutoff_min: d.dot_cutoff_min,
            dot_cutoff_max: d.dot_cutoff_max,
            max_victory_points_for_percent: d.max_victory_points_for_percent,
            capital_cutoff_max: d.capital_cutoff_max,
            capital_icon_cutoff: d.capital_icon_cutoff,
        }
    }
}

fn main() -> Result<()> {
    let mut game = None;
    let mut mods = Vec::new();
    let mut out = None;
    let mut regions: Vec<RegionSpec> = Vec::new();
    let mut language = "english".to_string();
    let mut args = env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--game" => game = args.next().map(PathBuf::from),
            "--mod" => mods.push(PathBuf::from(next_arg(&mut args, "--mod needs a dir")?)),
            "--out" => out = args.next().map(PathBuf::from),
            "--language" => language = next_arg(&mut args, "--language needs a value")?,
            "--region" => regions.push(parse_region(&next_arg(
                &mut args,
                "--region needs name:x:y:w:h",
            )?)?),
            "--help" | "-h" => {
                print_usage();
                return Ok(());
            }
            other => return Err(format!("unknown argument {other}").into()),
        }
    }
    let game = game.ok_or("missing --game <game dir>")?;
    let out = out.ok_or("missing --out <dir>")?;
    fs::create_dir_all(&out)?;

    let resolver = if mods.is_empty() {
        GameFileResolver::vanilla(game)?
    } else {
        GameFileResolver::with_mod_paths(game, mods)?
    };

    let layer = furniture_layer_from_resolver(&resolver, &language)?;
    let (font, atlas) = load_vp_label_font(&resolver)?;
    let sprite_present = vp_strip_texture_path(&resolver)?.is_some();

    if regions.is_empty() {
        regions = default_regions(layer.width, layer.height);
    }

    let land = layer.land_mask();
    let mut region_specs = Vec::new();
    for mut region in regions {
        clamp_region(&mut region, layer.width, layer.height)?;
        let file = format!("furniture_{}.png", sanitize(&region.name));
        let in_view = render_region(&layer, &land, &font, &atlas, &region, &out.join(&file))?;
        region.file = file;
        region.markers_in_view = Some(in_view);
        region_specs.push(region);
    }

    let capital_marker_count = layer
        .markers
        .iter()
        .filter(|m| m.kind == FurnitureMarkerKind::Capital)
        .count();

    let layout = LayoutJson {
        schema: "battle-plan-map-furniture-layout-v1",
        map_dimensions: [layer.width, layer.height],
        language: language.clone(),
        marker_count: layer.markers.len(),
        capital_marker_count,
        localisation_key_count: layer.localisation_key_count,
        capital_count: layer.capital_count,
        label_font: VP_LABEL_FONT_NAME.to_string(),
        label_font_glyphs: font.glyphs.len(),
        vp_strip_sprite: VP_STRIP_SPRITE_NAME.to_string(),
        vp_strip_frames: VP_STRIP_FRAME_COUNT,
        vp_icon_defines: (&layer.vp_icon_defines).into(),
        citations: citations(sprite_present),
        untraced: untraced(),
        regions: region_specs.clone(),
        markers: layer.markers.iter().map(marker_json).collect(),
    };
    let json_path = out.join("map_furniture_layout.json");
    fs::write(&json_path, serde_json::to_vec_pretty(&layout)?)?;

    println!(
        "map furniture: {} markers ({} capitals), {} loc keys, {} capitals, font {} ({} glyphs)",
        layer.markers.len(),
        capital_marker_count,
        layer.localisation_key_count,
        layer.capital_count,
        VP_LABEL_FONT_NAME,
        font.glyphs.len(),
    );
    println!("layout json: {}", json_path.display());
    for region in &region_specs {
        println!(
            "dump: {} -> {} ({} markers in view)",
            region.name,
            out.join(&region.file).display(),
            region.markers_in_view.unwrap_or(0)
        );
    }
    Ok(())
}

fn marker_json(m: &FurnitureMarker) -> MarkerJson {
    MarkerJson {
        province_id: m.province_id,
        state_id: m.state_id,
        owner: m.owner.clone(),
        kind: m.kind.as_str().to_string(),
        anchor_x: m.anchor_x,
        anchor_y: m.anchor_y,
        vp_value: m.vp_value,
        dot_tier: m.dot_tier,
        label: m.label.clone(),
        label_resolved: m.label_resolved,
        pixel_count: m.pixel_count,
    }
}

/// The FILE-DERIVED citation ledger emitted into the JSON.
fn citations(sprite_present: bool) -> Vec<Citation> {
    let cite = |parameter: &str, source: &str, detail: &str| Citation {
        parameter: parameter.to_string(),
        source: source.to_string(),
        detail: detail.to_string(),
    };
    vec![
        cite(
            "vp_marker_position (DERIVED_APPROXIMATION)",
            "map/provinces.bmp + map/definition.csv",
            "province pixel centroid in the 5632x2048 raster; map/positions.txt is 0 bytes so NO explicit shipped VP position exists. The centroid is a DERIVED_APPROXIMATION of the anchor, not a game-file parameter; the engine's exact anchor is UNTRACED.",
        ),
        cite(
            "which_provinces_carry_vp + vp_value",
            "history/states/*.txt",
            "victory_points = { <province_id> <value> } inside history = { } (blocks may repeat and be multiline)",
        ),
        cite(
            "city_vp_name_string",
            "localisation/<lang>/victory_points_l_<lang>.yml",
            &format!("{VICTORY_POINTS_LOC_PREFIX}<province_id>: \"Name\" (english: localisation/english/victory_points_l_english.yml:761 VICTORY_POINTS_3838: \"Ajáccio\")"),
        ),
        cite(
            "capital_state",
            "history/countries/<TAG> - <Name>.txt",
            "capital = <state_id> (STATE id, not province; 363 initial capitals, 0 non-state ids; parsed word-boundary aware so anti_capitalism does not match)",
        ),
        cite(
            "vp_marker_sprite",
            &format!("{MAPICONS_GFX_SOURCE}:290-295"),
            &format!(
                "spriteType {VP_STRIP_SPRITE_NAME} texturefile={VP_STRIP_SPRITE_TEXTURE} noOfFrames={VP_STRIP_FRAME_COUNT} (DDS 580x27 = 20 frames of 29x27); present={sprite_present}"
            ),
        ),
        cite(
            "vp_marker_and_capital_container",
            &format!("{MAPICONS_GUI_SOURCE}:628-652 (victory_point_mapicon) + :655-680 (capital_mapicon)"),
            "capital reuses the same VP strip sprite (GFX_stars_small is COMMENTED OUT at core.gfx:702-706); icon pos {x=-5 y=0} (:637-638/:664-665), text pos {x=26 y=5} maxWidth=149 maxHeight=30 font hoi_20bs; capital icon alwaystransparent=yes (:667)",
        ),
        cite(
            "label_font",
            &format!("{CORE_GFX_SOURCE}:1232-1238"),
            &format!(
                "bitmapfont {VP_LABEL_FONT_NAME} fontfiles={{gfx/fonts/hoi_20bs}} color=0xffffffff; metrics {HOI_20BS_FNT_SOURCE}; page atlas {HOI_20BS_ATLAS_SOURCE} (512x256, 32-bit uncompressed A/R/G/B masks). The .fnt's page=\"hoi_20b_0.dds\" is STALE (absent); the basename-atlas match is INFERRED from core.gfx + file existence, not stated in the .fnt."
            ),
        ),
        cite(
            "vp_dot_tier_thresholds + zoom_cutoff_values",
            "common/defines/00_graphics.lua:947-955,963",
            "VICTORY_POINT_LEVELS=3, VICTORY_POINT_MAP_ICON_AFTER={0,9,20}, VICTORY_POINT_MAP_ICON_TEXT_CUTOFF={150,250,500}, *_TEXT_CUTOFF_{MIN=100,MAX=800}, *_DOT_CUTOFF_{MIN=100,MAX=1000}, MAX_VICTORY_POINTS_FOR_PERCENT=22, CAPITAL_CUTOFF_MAX=1000, CAPITAL_ICON_CUTOFF=1100. Values file-derived; the engine's comparison/interp of them is engine-side.",
        ),
    ]
}

/// Honest UNTRACED gaps: engine-side laws with NO shipped numeric value.
fn untraced() -> Vec<Citation> {
    let cite = |parameter: &str, detail: &str| Citation {
        parameter: parameter.to_string(),
        source: "engine-C++ (no shipped file)".to_string(),
        detail: detail.to_string(),
    };
    vec![
        cite(
            "vp_strip_frame_selection",
            "which of the 20 GFX_onmap_victorypoints_strip frames (29x27 each) renders for a given VP tier / capital is chosen in engine C++. VICTORY_POINT_MAP_ICON_AFTER={0,9,20} gives the TIER boundaries (file-derived), but the tier->frame index mapping is not in the files. Close by capturing a live map frame's icon draw.",
        ),
        cite(
            "capital_state_to_marker_province + no_vp_capital_fallback",
            "capital = <state_id> names a STATE; a capital state may hold several VP provinces or NONE (e.g. state 914 Jan Mayen has no victory_points). WHICH province the engine anchors the capital marker on, and what it does for a capital with no VP, is engine-side. This law marks every VP province of the capital state as Capital and emits nothing for a VP-less capital state — labelled, never guessed.",
        ),
        cite(
            "sub_province_marker_offset (centroid = DERIVED_APPROXIMATION)",
            "the engine may nudge the marker off the raw province centroid (e.g. toward the city building or off water); map/positions.txt is empty so no shipped offset exists. The centroid is a derived approximation, not the proven engine anchor. Close via a live capture of VP screen positions.",
        ),
        cite(
            "vp_threshold_comparison + label_fade_scale_interpolation",
            "the inclusive/exclusive comparison of VICTORY_POINT_MAP_ICON_AFTER and the label fade/scale interpolation between the (file-derived) *_CUTOFF distances are engine-side. This law uses a >= bucketing for the tier and never interpolates size (positions are camera-independent).",
        ),
        cite(
            "label_render_pixel_size_vs_camera",
            "the on-screen text size is camera/zoom driven in engine C++; the dump uses a fixed validation scale and never derives a placement from it. Zoom VISIBILITY cutoff VALUES are file-derived (see citations).",
        ),
    ]
}

// ---------------------------------------------------------------------------
// Region rendering: land underlay + VP bullet + real hoi_20bs glyph label.
// ---------------------------------------------------------------------------

/// Render one region to a PNG; returns the number of markers whose anchor falls
/// inside the region.
fn render_region(
    layer: &FurnitureLayer,
    land: &[bool],
    font: &BmFont,
    atlas: &RgbaFontAtlas,
    region: &RegionSpec,
    path: &Path,
) -> Result<usize> {
    // Downscale so the PNG stays a sane size; keep the longest side <= ~1600 px.
    let scale = (region.width.max(region.height) as f32 / 1600.0)
        .ceil()
        .max(1.0) as usize;
    let out_w = (region.width / scale).max(1);
    let out_h = (region.height / scale).max(1);
    let mut pixels = vec![[18u8, 22u8, 28u8]; out_w * out_h]; // water

    // Land underlay (muted grey).
    for oy in 0..out_h {
        for ox in 0..out_w {
            let mx = region.x + ox * scale;
            let my = region.y + oy * scale;
            if mx < layer.width && my < layer.height && land[my * layer.width + mx] {
                pixels[oy * out_w + ox] = [60, 68, 74];
            }
        }
    }

    let to_local = |mx: f32, my: f32| -> (i32, i32) {
        (
            ((mx - region.x as f32) / scale as f32).round() as i32,
            ((my - region.y as f32) / scale as f32).round() as i32,
        )
    };

    let mut in_view = 0usize;
    for m in &layer.markers {
        let (lx, ly) = to_local(m.anchor_x, m.anchor_y);
        if lx < 0 || ly < 0 || lx >= out_w as i32 || ly >= out_h as i32 {
            continue;
        }
        in_view += 1;
        // VP bullet: a filled dot whose radius grows with the file-derived tier.
        // Capitals get a gold dot, plain VPs a light-grey dot (so the two kinds
        // are distinguishable in the eyeball view). This colour split is a
        // VALIDATION cue, not a game colour (the game reuses one strip sprite).
        let radius = 2 + m.dot_tier as i32;
        let dot_color = match m.kind {
            FurnitureMarkerKind::Capital => [240u8, 205, 70],
            FurnitureMarkerKind::VictoryPoint => [210, 214, 220],
        };
        fill_disc(&mut pixels, out_w, out_h, lx, ly, radius, dot_color);

        // The city/VP name in the real hoi_20bs glyphs, white, to the right of
        // the dot (mirroring the GUI text offset direction). Only draw for the
        // larger dots so a full-map view is not a wall of text.
        if m.dot_tier >= 1 || matches!(m.kind, FurnitureMarkerKind::Capital) {
            draw_label(
                &mut pixels,
                out_w,
                out_h,
                font,
                atlas,
                &m.label,
                lx + radius + 3,
                ly,
                [245, 245, 245],
            );
        }
    }

    write_png(path, out_w, out_h, &pixels)?;
    Ok(in_view)
}

/// Blit a string with the real BMFont glyph atlas, left-aligned, vertically
/// centred on `baseline_y`. Coverage from the atlas modulates the text colour
/// over the existing pixels (simple alpha-over).
fn draw_label(
    pixels: &mut [[u8; 3]],
    w: usize,
    h: usize,
    font: &BmFont,
    atlas: &RgbaFontAtlas,
    text: &str,
    x: i32,
    baseline_y: i32,
    color: [u8; 3],
) {
    let mut pen_x = x;
    // Centre the line on baseline_y using the font's base/lineHeight.
    let y_top = baseline_y - (font.base as i32) / 2;
    for ch in text.chars() {
        let Some(g) = font.glyph(ch as u32) else {
            // Unknown glyph: advance a space so text does not collapse.
            pen_x += (font.line_height as i32) / 4;
            continue;
        };
        let gx0 = pen_x + (g.xoffset as f32 * LABEL_RENDER_SCALE) as i32;
        let gy0 = y_top + (g.yoffset as f32 * LABEL_RENDER_SCALE) as i32;
        for row in 0..g.height {
            for col in 0..g.width {
                let cov = atlas.coverage((g.x + col) as usize, (g.y + row) as usize);
                if cov < 24 {
                    continue;
                }
                let px = gx0 + (col as f32 * LABEL_RENDER_SCALE) as i32;
                let py = gy0 + (row as f32 * LABEL_RENDER_SCALE) as i32;
                blend(pixels, w, h, px, py, color, cov);
            }
        }
        pen_x += (g.xadvance as f32 * LABEL_RENDER_SCALE) as i32;
    }
}

fn fill_disc(pixels: &mut [[u8; 3]], w: usize, h: usize, cx: i32, cy: i32, r: i32, color: [u8; 3]) {
    for dy in -r..=r {
        for dx in -r..=r {
            if dx * dx + dy * dy <= r * r {
                put(pixels, w, h, cx + dx, cy + dy, color);
            }
        }
    }
}

fn blend(pixels: &mut [[u8; 3]], w: usize, h: usize, x: i32, y: i32, color: [u8; 3], cov: u8) {
    if x < 0 || y < 0 || x >= w as i32 || y >= h as i32 {
        return;
    }
    let idx = y as usize * w + x as usize;
    let a = cov as u32;
    let inv = 255 - a;
    for c in 0..3 {
        pixels[idx][c] = ((color[c] as u32 * a + pixels[idx][c] as u32 * inv) / 255) as u8;
    }
}

fn put(pixels: &mut [[u8; 3]], w: usize, h: usize, x: i32, y: i32, color: [u8; 3]) {
    if x < 0 || y < 0 || x >= w as i32 || y >= h as i32 {
        return;
    }
    pixels[y as usize * w + x as usize] = color;
}

// ---------------------------------------------------------------------------
// Minimal, dependency-free PNG writer (RGB8, stored/uncompressed zlib blocks).
// ---------------------------------------------------------------------------

fn write_png(path: &Path, width: usize, height: usize, rgb: &[[u8; 3]]) -> Result<()> {
    // Raw image data: each row prefixed with filter byte 0 (None).
    let mut raw = Vec::with_capacity(height * (1 + width * 3));
    for y in 0..height {
        raw.push(0u8);
        for x in 0..width {
            let [r, g, b] = rgb[y * width + x];
            raw.extend_from_slice(&[r, g, b]);
        }
    }
    let zlib = zlib_store(&raw);

    let mut out = Vec::new();
    out.extend_from_slice(&[137, 80, 78, 71, 13, 10, 26, 10]); // PNG signature

    // IHDR
    let mut ihdr = Vec::new();
    ihdr.extend_from_slice(&(width as u32).to_be_bytes());
    ihdr.extend_from_slice(&(height as u32).to_be_bytes());
    ihdr.push(8); // bit depth
    ihdr.push(2); // colour type 2 = truecolour RGB
    ihdr.push(0); // compression
    ihdr.push(0); // filter
    ihdr.push(0); // interlace
    write_chunk(&mut out, b"IHDR", &ihdr);
    write_chunk(&mut out, b"IDAT", &zlib);
    write_chunk(&mut out, b"IEND", &[]);

    let mut file = fs::File::create(path)?;
    file.write_all(&out)?;
    Ok(())
}

fn write_chunk(out: &mut Vec<u8>, kind: &[u8; 4], data: &[u8]) {
    out.extend_from_slice(&(data.len() as u32).to_be_bytes());
    out.extend_from_slice(kind);
    out.extend_from_slice(data);
    let mut crc_input = Vec::with_capacity(4 + data.len());
    crc_input.extend_from_slice(kind);
    crc_input.extend_from_slice(data);
    out.extend_from_slice(&crc32(&crc_input).to_be_bytes());
}

/// zlib stream wrapping raw deflate STORED (uncompressed) blocks. Correct and
/// dependency-free; the PNGs are larger but lossless and standard.
fn zlib_store(data: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    out.push(0x78); // CMF: deflate, 32K window
    out.push(0x01); // FLG: no dict, fastest
    let mut i = 0;
    while i < data.len() {
        let chunk = (data.len() - i).min(0xFFFF);
        let is_last = i + chunk >= data.len();
        out.push(if is_last { 1 } else { 0 }); // BFINAL, BTYPE=00 (stored)
        out.extend_from_slice(&(chunk as u16).to_le_bytes());
        out.extend_from_slice(&(!(chunk as u16)).to_le_bytes());
        out.extend_from_slice(&data[i..i + chunk]);
        i += chunk;
    }
    if data.is_empty() {
        // A single empty final stored block.
        out.push(1);
        out.extend_from_slice(&0u16.to_le_bytes());
        out.extend_from_slice(&0xFFFFu16.to_le_bytes());
    }
    out.extend_from_slice(&adler32(data).to_be_bytes());
    out
}

fn adler32(data: &[u8]) -> u32 {
    let mut a = 1u32;
    let mut b = 0u32;
    for &byte in data {
        a = (a + byte as u32) % 65521;
        b = (b + a) % 65521;
    }
    (b << 16) | a
}

fn crc32(data: &[u8]) -> u32 {
    let mut crc = 0xFFFF_FFFFu32;
    for &byte in data {
        crc ^= byte as u32;
        for _ in 0..8 {
            let mask = (crc & 1).wrapping_neg();
            crc = (crc >> 1) ^ (0xEDB8_8320 & mask);
        }
    }
    !crc
}

// ---------------------------------------------------------------------------
// CLI helpers.
// ---------------------------------------------------------------------------

fn next_arg(args: &mut impl Iterator<Item = String>, message: &str) -> Result<String> {
    args.next().ok_or_else(|| message.into())
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
        markers_in_view: None,
    })
}

/// Default views: Europe (VP-dense) and the Pacific/Asia (the spec's two
/// required views). Clamped to the map in [`clamp_region`].
fn default_regions(width: usize, height: usize) -> Vec<RegionSpec> {
    vec![
        RegionSpec {
            name: "europe".to_string(),
            x: 2500.min(width.saturating_sub(1)),
            y: 250.min(height.saturating_sub(1)),
            width: 1500,
            height: 950,
            file: String::new(),
            markers_in_view: None,
        },
        RegionSpec {
            name: "pacific".to_string(),
            x: 4200.min(width.saturating_sub(1)),
            y: 500.min(height.saturating_sub(1)),
            width: 1400,
            height: 1000,
            file: String::new(),
            markers_in_view: None,
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
        "Usage: dump_map_furniture --game <the game dir> [--mod <dir>]... --out <dir> \
         [--region name:x:y:w:h]... [--language english]"
    );
}
