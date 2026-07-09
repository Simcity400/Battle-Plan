//! LAB3: decode of the CAPTURED engine `mapname` label vertex buffer and the
//! law-vs-engine residual it validates.
//!
//! This module is the capture-side counterpart to [`crate::map_label_law`]: it
//! never feeds the runtime label path (captures validate, they do not drive
//! render — see `documents/MAP_LABEL_LAW.md` §5). It exists so a test can decode
//! the REAL captured bytes with field-level assertions and so the placement
//! law's arc model can be scored against the engine's own glyph geometry.
//!
//! ## VB layout (ground truth = the captured VS input signature)
//!
//! The `mapname` draw is a non-indexed `Draw`, topology `TRIANGLELIST` (D3D
//! primitive-topology 4), of `6 * glyph_count` vertices. The vertex layout is
//! the reflected input signature of VS `4e1b73f39c128df0` (the captured
//! `input_layout.elements`, which equals the shipped
//! `gfx/FX/mapname.shader:29`-`33` `VS_INPUT`):
//!
//! | field    | semantic  | DXGI format                | offset | size |
//! |----------|-----------|----------------------------|--------|------|
//! | position | POSITION  | 6  = R32G32B32_FLOAT       | 0      | 12   |
//! | uv       | TEXCOORD0 | 16 = R32G32_FLOAT          | 12     | 8    |
//!
//! stride = 20 bytes (matches the captured `vertex_buffers[0].stride`).
//!
//! `position` is a **world-space** point on the map plane: the shader does
//! `float4 vPos = float4(v.vPosition, 1.0f); vPos = mul(ViewProjectionMatrix,
//! vPos)` (`gfx/FX/mapname.shader:51`,`:55`) and `Out.vPrepos = v.vPosition.xyz`
//! (`:61`) — no arc/spacing math runs in the shader, so ALL placement,
//! curvature, letter-spacing and size are baked into these world XYZ by the
//! engine CPU-side. `uv` indexes the pre-coloured `hoi_mapfont4` atlas.
//!
//! Per glyph the 6 vertices are two triangles of the quad in the fixed order
//! `TL, TR, BL, TR, BR, BL` (verified on the captured `SOVIETUNION` draw: the
//! per-glyph UV bbox matches one `.fnt` char rect to 0.0 texels).

use std::collections::BTreeMap;

/// Bytes per captured mapname vertex (`float3 position + float2 uv`).
///
/// GROUND TRUTH: captured `vertex_buffers[0].stride = 20`, and the reflected
/// input layout places POSITION (DXGI 6, R32G32B32_FLOAT) at offset 0 and
/// TEXCOORD0 (DXGI 16, R32G32_FLOAT) at offset 12 → 12 + 8 = 20.
pub const MAPNAME_VERTEX_STRIDE: usize = 20;
/// Vertices per glyph quad (two triangles, `TRIANGLELIST`).
pub const VERTS_PER_GLYPH: usize = 6;
/// Byte offset of the POSITION field (reflected `aligned_byte_offset = 0`).
pub const POSITION_OFFSET: usize = 0;
/// Byte offset of the TEXCOORD0 field (reflected `aligned_byte_offset = 12`).
pub const TEXCOORD_OFFSET: usize = 12;

/// One decoded vertex: world-space map-plane position + atlas UV.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MapnameVertex {
    /// World X (east), map-plane units (== province-raster X).
    pub x: f32,
    /// World Y (map-plane height; ~10 for the captured labels).
    pub y: f32,
    /// World Z (north/south), map-plane units (== province-raster Y axis).
    pub z: f32,
    /// Atlas U in [0,1] (multiply by `scale_w` for texels).
    pub u: f32,
    /// Atlas V in [0,1] (multiply by `scale_h` for texels).
    pub v: f32,
}

/// One decoded glyph quad (its four distinct corners + UV rect in texels).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GlyphQuad {
    /// Top-left corner (world XZ), = vertex 0 of the 6.
    pub tl: (f32, f32),
    /// Top-right corner (world XZ), = vertex 1.
    pub tr: (f32, f32),
    /// Bottom-left corner (world XZ), = vertex 2.
    pub bl: (f32, f32),
    /// Bottom-right corner (world XZ), = vertex 4.
    pub br: (f32, f32),
    /// Near-constant map-plane height Y of the quad.
    pub y: f32,
    /// UV rect in texels: `(x, y, width, height)`.
    pub uv_texels: (f32, f32, f32, f32),
    /// Baseline midpoint (midpoint of the bottom edge BL–BR), world XZ.
    pub baseline_mid: (f32, f32),
    /// Cap height: distance TL→BL (top edge to bottom edge), world units.
    pub cap_height: f32,
    /// Glyph advance width: distance TL→TR (top edge), world units.
    pub advance_width: f32,
}

/// A fully decoded captured label: its glyph quads plus the fitted baseline.
#[derive(Clone, Debug, PartialEq)]
pub struct DecodedLabel {
    /// Number of glyph quads (`draw_count / 6`).
    pub glyph_count: usize,
    /// Reconstructed text (one char per quad; spaces are not in the VB).
    pub text: String,
    /// The decoded glyph quads, in draw order.
    pub glyphs: Vec<GlyphQuad>,
    /// Baseline-arc fit of the glyph midpoints (the engine's own baseline).
    pub fit: BaselineFit,
}

/// A quadratic-arc fit of the per-glyph baseline midpoints, in the SAME frame
/// [`crate::map_label_law::CountryLabelPlacement`] uses — this is what makes the
/// residual a direct law-vs-engine comparison.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BaselineFit {
    /// Centroid of the glyph baseline midpoints (world XZ).
    pub anchor_x: f32,
    pub anchor_z: f32,
    /// Principal-axis angle (radians) in world XZ, folded to `(-pi/2, pi/2]`.
    pub angle: f32,
    /// Half the along-axis span of the midpoints (world units).
    pub half_length: f32,
    /// Quadratic bow: signed perpendicular peak offset (world units).
    pub bow: f32,
    /// Mean glyph cap height (world units) — the engine label size.
    pub cap_height: f32,
    /// RMS perpendicular residual of the midpoints to the fitted arc (world u).
    pub residual_rms: f32,
    /// Max perpendicular residual to the fitted arc (world units).
    pub residual_max: f32,
}

/// Errors decoding a captured mapname VB.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DecodeError {
    /// Buffer length is not a whole number of `MAPNAME_VERTEX_STRIDE` vertices.
    BadStride { len: usize, stride: usize },
    /// Vertex count is not a whole number of `VERTS_PER_GLYPH` glyph quads.
    BadGlyphCount { verts: usize },
    /// Empty buffer.
    Empty,
}

impl std::fmt::Display for DecodeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DecodeError::BadStride { len, stride } => {
                write!(f, "VB length {len} is not a multiple of stride {stride}")
            }
            DecodeError::BadGlyphCount { verts } => {
                write!(
                    f,
                    "vertex count {verts} is not a multiple of {VERTS_PER_GLYPH}"
                )
            }
            DecodeError::Empty => write!(f, "empty VB"),
        }
    }
}

impl std::error::Error for DecodeError {}

/// Decode the raw captured VB bytes into per-vertex position+uv, using the
/// reflected stride/offsets. No interpretation beyond the byte layout.
pub fn decode_vertices(bytes: &[u8]) -> Result<Vec<MapnameVertex>, DecodeError> {
    if bytes.is_empty() {
        return Err(DecodeError::Empty);
    }
    if !bytes.len().is_multiple_of(MAPNAME_VERTEX_STRIDE) {
        return Err(DecodeError::BadStride {
            len: bytes.len(),
            stride: MAPNAME_VERTEX_STRIDE,
        });
    }
    let count = bytes.len() / MAPNAME_VERTEX_STRIDE;
    let mut out = Vec::with_capacity(count);
    for i in 0..count {
        let base = i * MAPNAME_VERTEX_STRIDE;
        let f = |off: usize| -> f32 {
            let o = base + off;
            f32::from_le_bytes([bytes[o], bytes[o + 1], bytes[o + 2], bytes[o + 3]])
        };
        out.push(MapnameVertex {
            x: f(POSITION_OFFSET),
            y: f(POSITION_OFFSET + 4),
            z: f(POSITION_OFFSET + 8),
            u: f(TEXCOORD_OFFSET),
            v: f(TEXCOORD_OFFSET + 4),
        });
    }
    Ok(out)
}

/// Group decoded vertices into glyph quads (6 verts each, `TL,TR,BL,TR,BR,BL`).
///
/// Positions are taken in the world XZ plane (Y is the near-constant map-plane
/// height, carried per quad). UV texels use the atlas `scale_w`/`scale_h`.
pub fn glyph_quads(
    verts: &[MapnameVertex],
    scale_w: f32,
    scale_h: f32,
) -> Result<Vec<GlyphQuad>, DecodeError> {
    if verts.is_empty() {
        return Err(DecodeError::Empty);
    }
    if !verts.len().is_multiple_of(VERTS_PER_GLYPH) {
        return Err(DecodeError::BadGlyphCount { verts: verts.len() });
    }
    let mut quads = Vec::with_capacity(verts.len() / VERTS_PER_GLYPH);
    for chunk in verts.chunks_exact(VERTS_PER_GLYPH) {
        let (tl, tr, bl, br) = (chunk[0], chunk[1], chunk[2], chunk[4]);
        // UV bbox over all six verts (robust to corner order).
        let us = chunk.iter().map(|v| v.u);
        let vs = chunk.iter().map(|v| v.v);
        let u0 = us.clone().fold(f32::INFINITY, f32::min);
        let u1 = us.fold(f32::NEG_INFINITY, f32::max);
        let v0 = vs.clone().fold(f32::INFINITY, f32::min);
        let v1 = vs.fold(f32::NEG_INFINITY, f32::max);
        let baseline_mid = ((bl.x + br.x) * 0.5, (bl.z + br.z) * 0.5);
        let cap_height = ((bl.x - tl.x).powi(2) + (bl.z - tl.z).powi(2)).sqrt();
        let advance_width = ((tr.x - tl.x).powi(2) + (tr.z - tl.z).powi(2)).sqrt();
        quads.push(GlyphQuad {
            tl: (tl.x, tl.z),
            tr: (tr.x, tr.z),
            bl: (bl.x, bl.z),
            br: (br.x, br.z),
            y: (tl.y + tr.y + bl.y + br.y) * 0.25,
            uv_texels: (
                u0 * scale_w,
                v0 * scale_h,
                (u1 - u0) * scale_w,
                (v1 - v0) * scale_h,
            ),
            baseline_mid,
            cap_height,
            advance_width,
        });
    }
    Ok(quads)
}

/// Fit a quadratic baseline arc to the glyph baseline midpoints, in the frame
/// [`crate::map_label_law::CountryLabelPlacement`] uses (centroid anchor, PCA
/// angle, quadratic bow), and report the perpendicular residual. This is the
/// engine's own baseline — the yardstick the placement law must reproduce.
pub fn fit_baseline(quads: &[GlyphQuad]) -> Option<BaselineFit> {
    let n = quads.len();
    if n < 2 {
        return None;
    }
    let nf = n as f64;
    let mids: Vec<(f64, f64)> = quads
        .iter()
        .map(|q| (q.baseline_mid.0 as f64, q.baseline_mid.1 as f64))
        .collect();
    let mean_x = mids.iter().map(|p| p.0).sum::<f64>() / nf;
    let mean_z = mids.iter().map(|p| p.1).sum::<f64>() / nf;
    let mut cxx = 0.0;
    let mut czz = 0.0;
    let mut cxz = 0.0;
    for &(x, z) in &mids {
        let dx = x - mean_x;
        let dz = z - mean_z;
        cxx += dx * dx;
        czz += dz * dz;
        cxz += dx * dz;
    }
    cxx /= nf;
    czz /= nf;
    cxz /= nf;
    // PCA angle of the largest-variance eigenvector, folded so text reads L→R.
    let (angle, _major_var, _minor_var) = crate::principal_axis(cxx, czz, cxz);
    let (sin, cos) = angle.sin_cos();
    // Project each midpoint to (u along axis, v perpendicular).
    let mut us = Vec::with_capacity(n);
    let mut vs = Vec::with_capacity(n);
    for &(x, z) in &mids {
        let dx = x - mean_x;
        let dz = z - mean_z;
        us.push(dx * cos + dz * sin);
        vs.push(-dx * sin + dz * cos);
    }
    let umax = us.iter().cloned().fold(0.0f64, |a, u| a.max(u.abs()));
    let half_length = umax;
    // Quadratic v = a0 + a2*s^2, s = u/umax (drop the odd term, symmetric domain).
    let scale = if umax > 1e-9 { umax } else { 1.0 };
    let s: Vec<f64> = us.iter().map(|u| u / scale).collect();
    let sum_s2 = s.iter().map(|x| x * x).sum::<f64>();
    let sum_s4 = s.iter().map(|x| x.powi(4)).sum::<f64>();
    let sum_v = vs.iter().sum::<f64>();
    let sum_s2v = s.iter().zip(&vs).map(|(x, v)| x * x * v).sum::<f64>();
    let det = nf * sum_s4 - sum_s2 * sum_s2;
    let (a0, a2) = if det.abs() > 1e-9 {
        let a2 = (nf * sum_s2v - sum_s2 * sum_v) / det;
        let a0 = (sum_v - a2 * sum_s2) / nf;
        (a0, a2)
    } else {
        (sum_v / nf, 0.0)
    };
    // Residual: perpendicular offset minus the quadratic model.
    let mut sum_sq = 0.0;
    let mut max_abs = 0.0f64;
    for i in 0..n {
        let model = a0 + a2 * s[i] * s[i];
        let r = vs[i] - model;
        sum_sq += r * r;
        max_abs = max_abs.max(r.abs());
    }
    let residual_rms = (sum_sq / nf).sqrt();
    let cap_height = quads.iter().map(|q| q.cap_height as f64).sum::<f64>() / nf;
    Some(BaselineFit {
        anchor_x: mean_x as f32,
        anchor_z: mean_z as f32,
        angle: angle as f32,
        half_length: half_length as f32,
        // Peak bow is the centre offset from the chord through the ends: -a2.
        bow: -a2 as f32,
        cap_height: cap_height as f32,
        residual_rms: residual_rms as f32,
        residual_max: max_abs as f32,
    })
}

/// A `.fnt` char rect used to recover a glyph's character from its UV bbox.
#[derive(Clone, Copy, Debug)]
pub struct FntChar {
    pub id: u32,
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
    /// Pen advance (font px) — the BMFont `xadvance` metric (spaces carry a
    /// nonzero advance but a zero-area rect).
    pub xadvance: f32,
}

/// Parse the BMFont *text* `.fnt` header we need: `scaleW`/`scaleH` and the char
/// rects. This is deliberately a tiny local parser (the bin reads a shipped
/// game file directly, like `dump_map_labels`); `game_ui::font` has the full
/// parser for the render path.
pub fn parse_fnt_chars(text: &str) -> (f32, f32, Vec<FntChar>) {
    let mut scale_w = 0.0f32;
    let mut scale_h = 0.0f32;
    let mut chars = Vec::new();
    for line in text.lines() {
        let line = line.trim();
        if let Some(rest) = line.strip_prefix("common ") {
            let f = kv(rest);
            scale_w = f.get("scaleW").and_then(|v| v.parse().ok()).unwrap_or(0.0);
            scale_h = f.get("scaleH").and_then(|v| v.parse().ok()).unwrap_or(0.0);
        } else if let Some(rest) = line.strip_prefix("char ") {
            let f = kv(rest);
            if let (Some(id), Some(x), Some(y), Some(w), Some(h)) = (
                f.get("id").and_then(|v| v.parse().ok()),
                f.get("x").and_then(|v| v.parse().ok()),
                f.get("y").and_then(|v| v.parse().ok()),
                f.get("width").and_then(|v| v.parse().ok()),
                f.get("height").and_then(|v| v.parse().ok()),
            ) {
                let xadvance = f
                    .get("xadvance")
                    .and_then(|v| v.parse().ok())
                    .unwrap_or(0.0);
                chars.push(FntChar {
                    id,
                    x,
                    y,
                    width: w,
                    height: h,
                    xadvance,
                });
            }
        }
    }
    (scale_w, scale_h, chars)
}

fn kv(line: &str) -> BTreeMap<String, String> {
    let mut out = BTreeMap::new();
    for tok in line.split_whitespace() {
        if let Some((k, v)) = tok.split_once('=') {
            out.insert(k.to_string(), v.trim_matches('"').to_string());
        }
    }
    out
}

/// Recover the character for one glyph quad by matching its UV-texel rect to the
/// nearest `.fnt` char rect. Returns `(char_id, l1_error_in_texels)`.
///
/// The atlas packs every glyph at a distinct `(x,y,width,height)`, so the L1
/// distance is 0 for the true char and large for any other (the captured
/// `SOVIETUNION` + 23 `MAPNAME_7B` labels all match at exactly 0.0 texels).
pub fn match_glyph(quad: &GlyphQuad, chars: &[FntChar]) -> Option<(u32, f32)> {
    let (qx, qy, qw, qh) = quad.uv_texels;
    let mut best: Option<(u32, f32)> = None;
    for c in chars {
        let err =
            (c.x - qx).abs() + (c.y - qy).abs() + (c.width - qw).abs() + (c.height - qh).abs();
        if best.map(|(_, be)| err < be).unwrap_or(true) {
            best = Some((c.id, err));
        }
    }
    best
}

/// Full decode: bytes → glyph quads → text (via `.fnt`) → baseline fit.
pub fn decode_label(
    bytes: &[u8],
    scale_w: f32,
    scale_h: f32,
    chars: &[FntChar],
) -> Result<DecodedLabel, DecodeError> {
    let verts = decode_vertices(bytes)?;
    let quads = glyph_quads(&verts, scale_w, scale_h)?;
    let mut text = String::new();
    for q in &quads {
        if let Some((id, _err)) = match_glyph(q, chars) {
            if let Some(c) = char::from_u32(id) {
                text.push(c);
            } else {
                text.push('?');
            }
        } else {
            text.push('?');
        }
    }
    let fit = fit_baseline(&quads).ok_or(DecodeError::BadGlyphCount { verts: verts.len() })?;
    Ok(DecodedLabel {
        glyph_count: quads.len(),
        text,
        glyphs: quads,
        fit,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    // A tiny synthetic glyph quad builder in world XZ at height y.
    fn quad_bytes(
        out: &mut Vec<u8>,
        x: f32,
        z: f32,
        w: f32,
        h: f32,
        y: f32,
        uv: (f32, f32, f32, f32),
    ) {
        // Corners: TL (x,z), TR (x+w,z), BL (x, z+h), BR (x+w, z+h). The engine
        // stores baseline at the BL/BR edge; here z increases "down" the glyph.
        let (u0, v0, u1, v1) = uv;
        let tl = (x, y, z, u0, v0);
        let tr = (x + w, y, z, u1, v0);
        let bl = (x, y, z + h, u0, v1);
        let br = (x + w, y, z + h, u1, v1);
        for v in [tl, tr, bl, tr, br, bl] {
            out.extend_from_slice(&v.0.to_le_bytes());
            out.extend_from_slice(&v.1.to_le_bytes());
            out.extend_from_slice(&v.2.to_le_bytes());
            out.extend_from_slice(&v.3.to_le_bytes());
            out.extend_from_slice(&v.4.to_le_bytes());
        }
    }

    #[test]
    fn stride_and_offsets_match_reflected_layout() {
        // The reflected input layout: POSITION @0 (12B), TEXCOORD0 @12 (8B).
        assert_eq!(POSITION_OFFSET, 0);
        assert_eq!(TEXCOORD_OFFSET, 12);
        assert_eq!(MAPNAME_VERTEX_STRIDE, 20);
        assert_eq!(TEXCOORD_OFFSET + 8, MAPNAME_VERTEX_STRIDE);
    }

    #[test]
    fn decode_vertices_reads_position_and_uv() {
        let mut b = Vec::new();
        quad_bytes(&mut b, 10.0, 20.0, 4.0, 5.0, 9.5, (0.1, 0.2, 0.3, 0.4));
        let v = decode_vertices(&b).unwrap();
        assert_eq!(v.len(), 6);
        assert_eq!(
            v[0],
            MapnameVertex {
                x: 10.0,
                y: 9.5,
                z: 20.0,
                u: 0.1,
                v: 0.2
            }
        );
        // vertex 4 is BR = (x+w, y, z+h, u1, v1)
        assert_eq!(
            v[4],
            MapnameVertex {
                x: 14.0,
                y: 9.5,
                z: 25.0,
                u: 0.3,
                v: 0.4
            }
        );
    }

    #[test]
    fn bad_stride_and_glyph_count_error() {
        assert_eq!(decode_vertices(&[]), Err(DecodeError::Empty));
        assert!(matches!(
            decode_vertices(&[0u8; 21]),
            Err(DecodeError::BadStride { .. })
        ));
        // 2 verts = 40 bytes: valid stride, but not a whole glyph.
        let v = decode_vertices(&[0u8; 40]).unwrap();
        assert!(matches!(
            glyph_quads(&v, 2048.0, 1024.0),
            Err(DecodeError::BadGlyphCount { .. })
        ));
    }

    #[test]
    fn glyph_quad_geometry_and_uv_texels() {
        let mut b = Vec::new();
        quad_bytes(&mut b, 10.0, 20.0, 4.0, 5.0, 9.5, (0.1, 0.2, 0.3, 0.45));
        let v = decode_vertices(&b).unwrap();
        let q = &glyph_quads(&v, 2048.0, 1024.0).unwrap()[0];
        assert!((q.cap_height - 5.0).abs() < 1e-4, "cap {}", q.cap_height);
        assert!((q.advance_width - 4.0).abs() < 1e-4);
        assert!((q.baseline_mid.0 - 12.0).abs() < 1e-4);
        assert!((q.baseline_mid.1 - 25.0).abs() < 1e-4);
        // UV texels: (u0*2048, v0*1024, (u1-u0)*2048, (v1-v0)*1024).
        let (tx, ty, tw, th) = q.uv_texels;
        assert!((tx - 0.1 * 2048.0).abs() < 1e-2);
        assert!((ty - 0.2 * 1024.0).abs() < 1e-2);
        assert!((tw - 0.2 * 2048.0).abs() < 1e-2);
        assert!((th - 0.25 * 1024.0).abs() < 1e-2);
    }

    #[test]
    fn match_glyph_recovers_char_from_uv_rect() {
        let chars = vec![
            FntChar {
                id: 65,
                x: 100.0,
                y: 200.0,
                width: 40.0,
                height: 50.0,
                xadvance: 0.0,
            },
            FntChar {
                id: 66,
                x: 500.0,
                y: 600.0,
                width: 45.0,
                height: 55.0,
                xadvance: 0.0,
            },
        ];
        let mut b = Vec::new();
        // Build a quad whose UV texels land exactly on char 66's rect: choose
        // uv so u0*2048=500, v0*1024=600, width 45 => u1-u0=45/2048, etc.
        let u0 = 500.0 / 2048.0;
        let v0 = 600.0 / 1024.0;
        let u1 = (500.0 + 45.0) / 2048.0;
        let v1 = (600.0 + 55.0) / 1024.0;
        quad_bytes(&mut b, 0.0, 0.0, 1.0, 1.0, 0.0, (u0, v0, u1, v1));
        let v = decode_vertices(&b).unwrap();
        let q = &glyph_quads(&v, 2048.0, 1024.0).unwrap()[0];
        let (id, err) = match_glyph(q, &chars).unwrap();
        assert_eq!(id, 66);
        assert!(err < 1e-1, "err {err}");
    }

    #[test]
    fn straight_baseline_fits_with_near_zero_residual_and_bow() {
        // Five glyphs on a straight, level baseline: bow ~0, residual ~0.
        let mut b = Vec::new();
        for i in 0..5 {
            let x = i as f32 * 5.0;
            quad_bytes(&mut b, x, 0.0, 4.0, 6.0, 10.0, (0.0, 0.0, 0.02, 0.03));
        }
        let v = decode_vertices(&b).unwrap();
        let q = glyph_quads(&v, 2048.0, 1024.0).unwrap();
        let fit = fit_baseline(&q).unwrap();
        assert!(fit.residual_rms < 1e-3, "rms {}", fit.residual_rms);
        assert!(fit.bow.abs() < 1e-2, "bow {}", fit.bow);
        assert!(fit.angle.abs() < 1e-3, "angle {}", fit.angle);
        assert!((fit.cap_height - 6.0).abs() < 1e-3);
    }

    #[test]
    fn bowed_baseline_reports_signed_bow() {
        // Midpoints follow a downward parabola in z: v peaks at the centre.
        let mut b = Vec::new();
        for i in 0..7 {
            let t = (i as f32 - 3.0) / 3.0;
            let x = i as f32 * 5.0;
            let z = 10.0 * (1.0 - t * t); // bump up at centre
            quad_bytes(&mut b, x, z, 4.0, 6.0, 10.0, (0.0, 0.0, 0.02, 0.03));
        }
        let v = decode_vertices(&b).unwrap();
        let q = glyph_quads(&v, 2048.0, 1024.0).unwrap();
        let fit = fit_baseline(&q).unwrap();
        assert!(fit.bow.abs() > 3.0, "expected a bowed fit, got {}", fit.bow);
        // A single quadratic reproduces a parabola to machine precision.
        assert!(fit.residual_rms < 0.2, "rms {}", fit.residual_rms);
    }
}
