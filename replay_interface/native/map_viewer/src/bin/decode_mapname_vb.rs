//! LAB3 decode bin: decode the CAPTURED engine `mapname` label vertex buffers
//! for a capture session, recover each label's string + baseline geometry, fit
//! the placement-law arc model to the engine baseline, and emit a JSON dump plus
//! a before/after PNG (engine baseline vs law arc reconstruction) for eyeballing.
//!
//! Captures VALIDATE the law; they never feed the runtime label path. This bin
//! is a validation/diagnostic tool — it reads a read-only capture dir and the
//! shipped `.fnt`, and writes only to `--out`.
//!
//! Usage:
//!   decode_mapname_vb --session <capture session dir> --fnt <hoi_mapfont4.fnt>
//!       --out <dir>
//!
//! The session dir must contain `map_frame_mapname_pass_bundle.json` and the
//! per-draw `mapname_frame_draw###_ia_vertex_buffer_slot0.bin` files it names.
//! `--fnt` is the shipped `gfx/fonts/hoi_mapfont4.fnt` (BMFont text). Outputs:
//!   mapname_decode.json   — every label: string, glyph_count, engine baseline
//!                           fit (anchor/angle/half_length/bow/cap_height) and
//!                           the law-arc residual (rms/max) with glyph detail.
//!   mapname_labels.png    — per label, the engine baseline (green) and the
//!                           law-arc reconstruction of it (magenta) overlaid in
//!                           world XZ, so the fit quality is visible at a glance.

use std::env;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use map_viewer::{
    decode_vertices, fit_baseline, glyph_quads, match_glyph, parse_fnt_chars, BaselineFit,
    CountryLabelPlacement, FntChar, Result,
};
use serde::Serialize;
use serde_json::Value;

#[derive(Serialize)]
struct GlyphJson {
    char: String,
    /// UV rect in atlas texels (x, y, width, height).
    uv_texels: [f32; 4],
    /// Baseline midpoint (world XZ).
    baseline_mid: [f32; 2],
    cap_height: f32,
    advance_width: f32,
}

#[derive(Serialize)]
struct LabelJson {
    draw_index: usize,
    draw_count: u64,
    glyph_count: usize,
    text: String,
    /// Max glyph UV-match error in texels (0 = every glyph matched exactly).
    max_uv_texel_error: f32,
    /// Engine baseline fit (the placement-law frame): anchor/angle/half/bow/size.
    engine_anchor: [f32; 2],
    engine_angle_deg: f32,
    engine_half_length: f32,
    engine_bow: f32,
    engine_cap_height: f32,
    /// Law arc-model residual to the engine baseline (world units).
    law_arc_residual_rms: f32,
    law_arc_residual_max: f32,
    glyphs: Vec<GlyphJson>,
}

#[derive(Serialize)]
struct DecodeJson {
    schema: &'static str,
    session: String,
    label_count: usize,
    /// Aggregate law-arc residual across all labels (world units).
    mean_residual_rms: f32,
    max_residual_max: f32,
    labels: Vec<LabelJson>,
}

fn main() -> Result<()> {
    let mut session = None;
    let mut fnt = None;
    let mut out = None;
    let mut args = env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--session" => session = args.next().map(PathBuf::from),
            "--fnt" => fnt = args.next().map(PathBuf::from),
            "--out" => out = args.next().map(PathBuf::from),
            "--help" | "-h" => {
                eprintln!(
                    "Usage: decode_mapname_vb --session <dir> --fnt <hoi_mapfont4.fnt> --out <dir>"
                );
                return Ok(());
            }
            other => return Err(format!("unknown argument {other}").into()),
        }
    }
    let session = session.ok_or("missing --session <capture session dir>")?;
    let fnt = fnt.ok_or("missing --fnt <hoi_mapfont4.fnt>")?;
    let out = out.ok_or("missing --out <dir>")?;
    fs::create_dir_all(&out)?;

    let fnt_text = fs::read_to_string(&fnt).map_err(|e| format!("read {}: {e}", fnt.display()))?;
    let (scale_w, scale_h, chars) = parse_fnt_chars(&fnt_text);
    if scale_w <= 0.0 || scale_h <= 0.0 || chars.is_empty() {
        return Err(format!(
            "{}: not a BMFont text .fnt (no common/char lines)",
            fnt.display()
        )
        .into());
    }

    let bundle_path = session.join("map_frame_mapname_pass_bundle.json");
    let bundle: Value = serde_json::from_slice(
        &fs::read(&bundle_path).map_err(|e| format!("read {}: {e}", bundle_path.display()))?,
    )?;
    let draws = bundle
        .get("draws")
        .and_then(|d| d.as_array())
        .ok_or("bundle has no draws array")?;

    let mut labels = Vec::new();
    for (i, draw) in draws.iter().enumerate() {
        let Some(label) = decode_one(&session, i, draw, scale_w, scale_h, &chars)? else {
            continue;
        };
        labels.push(label);
    }
    if labels.is_empty() {
        return Err("no mapname draws decoded from the session".into());
    }

    let mean_rms = labels.iter().map(|l| l.law_arc_residual_rms).sum::<f32>() / labels.len() as f32;
    let max_max = labels
        .iter()
        .map(|l| l.law_arc_residual_max)
        .fold(0.0f32, f32::max);

    // PNG overlays: (before) engine baseline vs a STRAIGHT-chord reconstruction
    // that ignores curvature — the pre-arc approximation; (after) engine
    // baseline vs the law's quadratic-arc reconstruction.
    render_overlay(&labels, &out.join("mapname_labels_after.png"), true)?;
    render_overlay(&labels, &out.join("mapname_labels_before.png"), false)?;

    let json = DecodeJson {
        schema: "battle-plan-mapname-decode-v1",
        session: session.display().to_string(),
        label_count: labels.len(),
        mean_residual_rms: mean_rms,
        max_residual_max: max_max,
        labels,
    };
    let json_path = out.join("mapname_decode.json");
    fs::write(&json_path, serde_json::to_vec_pretty(&json)?)?;

    println!(
        "decoded {} labels; law-arc residual mean RMS {:.4}, max {:.4} world units",
        json.label_count, json.mean_residual_rms, json.max_residual_max
    );
    for l in &json.labels {
        println!(
            "  d{:03} {:>2}g  {:<24}  rms {:.3} max {:.3}  h {:.2}  ang {:+.1}deg",
            l.draw_index,
            l.glyph_count,
            l.text,
            l.law_arc_residual_rms,
            l.law_arc_residual_max,
            l.engine_cap_height,
            l.engine_angle_deg,
        );
    }
    println!("json:  {}", json_path.display());
    println!("png:   {}", out.join("mapname_labels_after.png").display());
    println!("png:   {}", out.join("mapname_labels_before.png").display());
    Ok(())
}

fn decode_one(
    session: &Path,
    draw_index: usize,
    draw: &Value,
    scale_w: f32,
    scale_h: f32,
    chars: &[FntChar],
) -> Result<Option<LabelJson>> {
    let draw_count = draw.get("draw_count").and_then(|v| v.as_u64()).unwrap_or(0);
    let vb = draw
        .get("vertex_buffers")
        .and_then(|v| v.get("0"))
        .and_then(|v| v.get("file"))
        .and_then(|v| v.as_str());
    let Some(vb_file) = vb else {
        return Ok(None);
    };
    let bytes = fs::read(session.join(vb_file))
        .map_err(|e| format!("read {}: {e}", session.join(vb_file).display()))?;
    let verts = decode_vertices(&bytes).map_err(|e| format!("draw {draw_index}: {e}"))?;
    let quads =
        glyph_quads(&verts, scale_w, scale_h).map_err(|e| format!("draw {draw_index}: {e}"))?;
    let fit = fit_baseline(&quads).ok_or(format!("draw {draw_index}: too few glyphs to fit"))?;

    let mut text = String::new();
    let mut max_err = 0.0f32;
    let mut glyph_json = Vec::with_capacity(quads.len());
    for q in &quads {
        let (id, err) = match_glyph(q, chars).unwrap_or((b'?' as u32, f32::INFINITY));
        max_err = max_err.max(if err.is_finite() { err } else { 0.0 });
        let c = char::from_u32(id).unwrap_or('?');
        text.push(c);
        glyph_json.push(GlyphJson {
            char: c.to_string(),
            uv_texels: [q.uv_texels.0, q.uv_texels.1, q.uv_texels.2, q.uv_texels.3],
            baseline_mid: [q.baseline_mid.0, q.baseline_mid.1],
            cap_height: q.cap_height,
            advance_width: q.advance_width,
        });
    }
    Ok(Some(LabelJson {
        draw_index,
        draw_count,
        glyph_count: quads.len(),
        text,
        max_uv_texel_error: max_err,
        engine_anchor: [fit.anchor_x, fit.anchor_z],
        engine_angle_deg: fit.angle.to_degrees(),
        engine_half_length: fit.half_length,
        engine_bow: fit.bow,
        engine_cap_height: fit.cap_height,
        law_arc_residual_rms: fit.residual_rms,
        law_arc_residual_max: fit.residual_max,
        glyphs: glyph_json,
    }))
}

/// Build a `CountryLabelPlacement` (the law's arc container) from a captured
/// baseline fit, so we can render the law's arc reconstruction of the engine
/// baseline and see they overlay. Note this uses the ENGINE's fitted
/// anchor/angle/half/bow — it reconstructs the arc, not the placement (which
/// would need the province raster); the PNG is the "does the arc model track the
/// engine baseline" check.
fn placement_from_fit(fit: &BaselineFit) -> CountryLabelPlacement {
    CountryLabelPlacement {
        tag: String::new(),
        // This container reconstructs one engine baseline arc; the home/component
        // fields are placement-law bookkeeping, unused here.
        is_home: true,
        component_id: 0,
        anchor_x: fit.anchor_x,
        anchor_y: fit.anchor_z,
        angle: fit.angle,
        half_length: fit.half_length,
        bow: fit.bow,
        world_height: fit.cap_height,
        pixel_count: 0,
        major_extent: 2.0 * fit.half_length,
        minor_extent: 2.0 * fit.cap_height,
        minor_extent_incl: 2.0 * fit.cap_height,
        breadth_span: 2.0 * fit.cap_height,
        breadth_p2_98: 2.0 * fit.cap_height,
        curve: None,
    }
}

/// Render all labels' engine baselines (green) and the law reconstruction
/// (magenta) in one world-XZ image, auto-fit to the labels' bounds. When `arc`
/// is true the reconstruction is the placement law's quadratic-arc baseline
/// (the validated "after"); when false it is the straight anchor±half_length
/// chord (the pre-arc "before"), so the two PNGs show what curvature buys.
fn render_overlay(labels: &[LabelJson], path: &Path, arc: bool) -> Result<()> {
    // Collect world extents from glyph baseline mids.
    let mut min_x = f32::INFINITY;
    let mut max_x = f32::NEG_INFINITY;
    let mut min_z = f32::INFINITY;
    let mut max_z = f32::NEG_INFINITY;
    for l in labels {
        for g in &l.glyphs {
            min_x = min_x.min(g.baseline_mid[0]);
            max_x = max_x.max(g.baseline_mid[0]);
            min_z = min_z.min(g.baseline_mid[1]);
            max_z = max_z.max(g.baseline_mid[1]);
        }
    }
    if !min_x.is_finite() {
        return Err("no glyphs to render".into());
    }
    let pad = ((max_x - min_x).max(max_z - min_z)) * 0.05 + 5.0;
    min_x -= pad;
    max_x += pad;
    min_z -= pad;
    max_z += pad;
    let out_w = 1600usize;
    let scale = out_w as f32 / (max_x - min_x);
    let out_h = ((max_z - min_z) * scale).ceil().max(1.0) as usize;
    let mut px = vec![[16u8, 18u8, 22u8]; out_w * out_h];
    let to_px = |x: f32, z: f32| -> (i32, i32) {
        (
            ((x - min_x) * scale).round() as i32,
            ((z - min_z) * scale).round() as i32,
        )
    };

    for l in labels {
        // Engine baseline (green): polyline through the glyph baseline mids.
        let mut prev: Option<(i32, i32)> = None;
        for g in &l.glyphs {
            let (cx, cz) = to_px(g.baseline_mid[0], g.baseline_mid[1]);
            if let Some((px0, pz0)) = prev {
                line(&mut px, out_w, out_h, px0, pz0, cx, cz, [90, 220, 120]);
            }
            prev = Some((cx, cz));
            dot(&mut px, out_w, out_h, cx, cz, [140, 255, 160]);
        }
        // Law arc reconstruction (magenta): sample the fitted arc t=-1..1.
        let fit = BaselineFit {
            anchor_x: l.engine_anchor[0],
            anchor_z: l.engine_anchor[1],
            angle: l.engine_angle_deg.to_radians(),
            half_length: l.engine_half_length,
            bow: l.engine_bow,
            cap_height: l.engine_cap_height,
            residual_rms: 0.0,
            residual_max: 0.0,
        };
        let placement = placement_from_fit(&fit);
        let mut prev: Option<(i32, i32)> = None;
        for i in 0..=48 {
            let t = (i as f32 / 48.0) * 2.0 - 1.0;
            let (mx, mz) = if arc {
                // Law's quadratic-arc baseline (curvature included).
                placement.baseline_point(t)
            } else {
                // Straight chord: anchor + t*half_length along the axis, no bow.
                let (s, c) = placement.angle.sin_cos();
                (
                    placement.anchor_x + t * placement.half_length * c,
                    placement.anchor_y + t * placement.half_length * s,
                )
            };
            let (cx, cz) = to_px(mx, mz);
            if let Some((px0, pz0)) = prev {
                line(&mut px, out_w, out_h, px0, pz0, cx, cz, [230, 90, 220]);
            }
            prev = Some((cx, cz));
        }
    }
    write_png_rgb(path, out_w, out_h, &px)
}

fn dot(px: &mut [[u8; 3]], w: usize, h: usize, x: i32, y: i32, c: [u8; 3]) {
    for dy in -1..=1 {
        for dx in -1..=1 {
            put(px, w, h, x + dx, y + dy, c);
        }
    }
}

fn line(
    px: &mut [[u8; 3]],
    w: usize,
    h: usize,
    mut x0: i32,
    mut y0: i32,
    x1: i32,
    y1: i32,
    c: [u8; 3],
) {
    let dx = (x1 - x0).abs();
    let sx = if x0 < x1 { 1 } else { -1 };
    let dy = -(y1 - y0).abs();
    let sy = if y0 < y1 { 1 } else { -1 };
    let mut err = dx + dy;
    loop {
        put(px, w, h, x0, y0, c);
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

fn put(px: &mut [[u8; 3]], w: usize, h: usize, x: i32, y: i32, c: [u8; 3]) {
    if x < 0 || y < 0 || x >= w as i32 || y >= h as i32 {
        return;
    }
    px[y as usize * w + x as usize] = c;
}

/// Write a true PNG (RGB8, uncompressed/stored deflate blocks). Dependency-free:
/// the repo's other dump bins write BMP, but the spec asks for PNGs and a stored
/// PNG is trivially small and universally viewable.
fn write_png_rgb(path: &Path, width: usize, height: usize, rgb: &[[u8; 3]]) -> Result<()> {
    let path = path.with_extension("png");
    // Raw scanlines with filter byte 0 per row.
    let mut raw = Vec::with_capacity(height * (1 + width * 3));
    for y in 0..height {
        raw.push(0u8);
        for x in 0..width {
            let [r, g, b] = rgb[y * width + x];
            raw.extend_from_slice(&[r, g, b]);
        }
    }
    let idat = zlib_stored(&raw);

    let mut png = Vec::new();
    png.extend_from_slice(&[0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a]);
    // IHDR
    let mut ihdr = Vec::new();
    ihdr.extend_from_slice(&(width as u32).to_be_bytes());
    ihdr.extend_from_slice(&(height as u32).to_be_bytes());
    ihdr.extend_from_slice(&[8, 2, 0, 0, 0]); // 8-bit, colour type 2 (RGB)
    write_chunk(&mut png, b"IHDR", &ihdr);
    write_chunk(&mut png, b"IDAT", &idat);
    write_chunk(&mut png, b"IEND", &[]);
    fs::File::create(&path)?.write_all(&png)?;
    Ok(())
}

fn write_chunk(out: &mut Vec<u8>, tag: &[u8; 4], data: &[u8]) {
    out.extend_from_slice(&(data.len() as u32).to_be_bytes());
    out.extend_from_slice(tag);
    out.extend_from_slice(data);
    let mut crc_input = Vec::with_capacity(4 + data.len());
    crc_input.extend_from_slice(tag);
    crc_input.extend_from_slice(data);
    out.extend_from_slice(&crc32(&crc_input).to_be_bytes());
}

/// zlib stream wrapping `data` in uncompressed (stored) deflate blocks.
fn zlib_stored(data: &[u8]) -> Vec<u8> {
    let mut out = vec![0x78, 0x01]; // zlib header (CMF, FLG)
    let mut i = 0;
    while i < data.len() || data.is_empty() {
        let remaining = data.len() - i;
        let block = remaining.min(0xFFFF);
        let last = i + block >= data.len();
        out.push(if last { 1 } else { 0 }); // BFINAL, BTYPE=00
        out.extend_from_slice(&(block as u16).to_le_bytes());
        out.extend_from_slice(&(!(block as u16)).to_le_bytes());
        out.extend_from_slice(&data[i..i + block]);
        i += block;
        if last {
            break;
        }
    }
    out.extend_from_slice(&adler32(data).to_be_bytes());
    out
}

fn adler32(data: &[u8]) -> u32 {
    let (mut a, mut b) = (1u32, 0u32);
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
