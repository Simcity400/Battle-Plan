
use std::collections::BTreeMap;

pub const MAPNAME_VERTEX_STRIDE: usize = 20;
pub const VERTS_PER_GLYPH: usize = 6;
pub const POSITION_OFFSET: usize = 0;
pub const TEXCOORD_OFFSET: usize = 12;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MapnameVertex {
    pub x: f32,
    pub y: f32,
    pub z: f32,
    pub u: f32,
    pub v: f32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GlyphQuad {
    pub tl: (f32, f32),
    pub tr: (f32, f32),
    pub bl: (f32, f32),
    pub br: (f32, f32),
    pub y: f32,
    pub uv_texels: (f32, f32, f32, f32),
    pub baseline_mid: (f32, f32),
    pub cap_height: f32,
    pub advance_width: f32,
}

#[derive(Clone, Debug, PartialEq)]
pub struct DecodedLabel {
    pub glyph_count: usize,
    pub text: String,
    pub glyphs: Vec<GlyphQuad>,
    pub fit: BaselineFit,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BaselineFit {
    pub anchor_x: f32,
    pub anchor_z: f32,
    pub angle: f32,
    pub half_length: f32,
    pub bow: f32,
    pub cap_height: f32,
    pub residual_rms: f32,
    pub residual_max: f32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DecodeError {
    BadStride { len: usize, stride: usize },
    BadGlyphCount { verts: usize },
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
    let (angle, _major_var, _minor_var) = crate::principal_axis(cxx, czz, cxz);
    let (sin, cos) = angle.sin_cos();
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
        bow: -a2 as f32,
        cap_height: cap_height as f32,
        residual_rms: residual_rms as f32,
        residual_max: max_abs as f32,
    })
}

#[derive(Clone, Copy, Debug)]
pub struct FntChar {
    pub id: u32,
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
    pub xadvance: f32,
}

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
