//! Colorcorrection LUT decode + lut_blender CPU port (OFF-5).
//!
//! Measured facts (2026-07-04, all 25 `gfx/world/*colorcorrection*.tga`):
//! every cube ships as an uncompressed bottom-up 24bpp TGA (type 2),
//! 1024x32 — a strip of 32 slices of 32x32. Orientation law verified
//! texel-exact against `colorcorrection_neutral.tga` (a true identity
//! cube): `texel(x = blue_index*32 + red_index, y_topdown = green_index)
//! = round(index * 255 / 31)` per channel.
//!
//! `gfx/FX/lut_blender.shader` blend math (Point/Clamp sampling, so the
//! fullscreen pass is a pure per-texel operation — the CPU port below is
//! exact, not approximate):
//!
//! ```text
//! LutBlend1:  out = lerp(BaseLUT1, BlendLUT1, vBlendFactor2)
//! LutBlend2 (DOUBLE_BLEND):
//!   base  = lerp(BaseLUT1, BaseLUT2, vBlendFactor1)
//!   blend = lerp(BlendLUT1, BlendLUT2, vBlendFactor1)
//!   out   = lerp(base, blend, vBlendFactor2)
//! ```
//!
//! WHICH cubes the engine selects per region/season/night (and the blend
//! factors over time) remains an acknowledged [ENGINE-LAW] gap — this
//! module makes every ingredient ready for the census evidence.
//!
//! Mod compatibility: reads go through [`GameFileResolver`]
//! (MOD_COMPATIBILITY.md).

use crate::*;

pub const COLOR_CORRECTION_DIR: &str = "gfx/world";

/// One decoded colour cube: `slice_size` = cube edge (vanilla 32),
/// texels stored top-down as RGB f32 in [0, 1].
#[derive(Clone, Debug, PartialEq)]
pub struct ColorCorrectionLut {
    pub width: usize,
    pub height: usize,
    pub slice_size: usize,
    pub texels: Vec<[f32; 3]>,
}

impl ColorCorrectionLut {
    /// Texel for integer cube indices (red, green, blue) in
    /// `0..slice_size`, per the verified orientation law.
    pub fn texel(&self, red: usize, green: usize, blue: usize) -> [f32; 3] {
        let x = blue * self.slice_size + red;
        self.texels[green * self.width + x]
    }

    /// Nearest-texel lookup for an RGB colour in [0, 1] (the shader
    /// samples with Point/Clamp).
    pub fn sample_nearest(&self, rgb: [f32; 3]) -> [f32; 3] {
        let max_index = (self.slice_size - 1) as f32;
        let index = |v: f32| (v.clamp(0.0, 1.0) * max_index).round() as usize;
        self.texel(index(rgb[0]), index(rgb[1]), index(rgb[2]))
    }

    /// Max absolute deviation (0..255 scale) from the identity cube.
    pub fn identity_deviation(&self) -> f32 {
        let max_index = (self.slice_size - 1) as f32;
        let mut worst = 0f32;
        for green in 0..self.slice_size {
            for blue in 0..self.slice_size {
                for red in 0..self.slice_size {
                    let texel = self.texel(red, green, blue);
                    for (channel, index) in texel.iter().zip([red, green, blue]) {
                        let expected = (index as f32 * 255.0 / max_index).round() / 255.0;
                        worst = worst.max((channel - expected).abs() * 255.0);
                    }
                }
            }
        }
        worst
    }
}

/// Strict TGA decode for the colorcorrection cubes: uncompressed 24/32bpp
/// (type 2), any origin. Fails closed on anything else — all 25 shipped
/// files are type-2 24bpp bottom-up.
pub fn parse_color_correction_tga(bytes: &[u8], label: &str) -> Result<ColorCorrectionLut> {
    if bytes.len() < 18 {
        return Err(format!("{label}: TGA shorter than its header").into());
    }
    let id_length = bytes[0] as usize;
    let color_map_type = bytes[1];
    let image_type = bytes[2];
    if color_map_type != 0 || image_type != 2 {
        return Err(format!(
            "{label}: unsupported TGA (color_map_type {color_map_type}, image_type \
             {image_type}); shipped cubes are type-2 truecolor"
        )
        .into());
    }
    let width = u16::from_le_bytes([bytes[12], bytes[13]]) as usize;
    let height = u16::from_le_bytes([bytes[14], bytes[15]]) as usize;
    let bits_per_pixel = bytes[16] as usize;
    let descriptor = bytes[17];
    if !(bits_per_pixel == 24 || bits_per_pixel == 32) {
        return Err(format!("{label}: unsupported TGA bpp {bits_per_pixel}").into());
    }
    if width == 0 || height == 0 || width != height * height {
        return Err(format!(
            "{label}: {width}x{height} is not a LUT strip (width must equal height*height)"
        )
        .into());
    }
    let top_down = descriptor & 0x20 != 0;
    let bytes_per_pixel = bits_per_pixel / 8;
    let pixel_offset = 18 + id_length;
    let needed = pixel_offset + width * height * bytes_per_pixel;
    if bytes.len() < needed {
        return Err(format!(
            "{label}: TGA truncated ({} bytes, wanted {needed})",
            bytes.len()
        )
        .into());
    }
    let mut texels = vec![[0f32; 3]; width * height];
    for row in 0..height {
        let source_row = if top_down { row } else { height - 1 - row };
        let row_offset = pixel_offset + source_row * width * bytes_per_pixel;
        for x in 0..width {
            let offset = row_offset + x * bytes_per_pixel;
            // TGA stores BGR(A).
            texels[row * width + x] = [
                bytes[offset + 2] as f32 / 255.0,
                bytes[offset + 1] as f32 / 255.0,
                bytes[offset] as f32 / 255.0,
            ];
        }
    }
    Ok(ColorCorrectionLut {
        width,
        height,
        slice_size: height,
        texels,
    })
}

pub fn color_correction_lut_from_resolver(
    resolver: &GameFileResolver,
    rel_path: &str,
) -> Result<ColorCorrectionLut> {
    let path = resolver.resolve_required(rel_path, "colorcorrection LUT")?;
    let bytes = std::fs::read(&path).map_err(|err| format!("read {}: {err}", path.display()))?;
    parse_color_correction_tga(&bytes, rel_path)
}

fn check_same_shape(label: &str, a: &ColorCorrectionLut, b: &ColorCorrectionLut) -> Result<()> {
    if a.width != b.width || a.height != b.height {
        return Err(format!(
            "{label}: LUT shape mismatch {}x{} vs {}x{}",
            a.width, a.height, b.width, b.height
        )
        .into());
    }
    Ok(())
}

fn lerp3(a: [f32; 3], b: [f32; 3], t: f32) -> [f32; 3] {
    [
        a[0] + (b[0] - a[0]) * t,
        a[1] + (b[1] - a[1]) * t,
        a[2] + (b[2] - a[2]) * t,
    ]
}

/// CPU port of `Effect LutBlend1`: `lerp(base, blend, blend_factor2)`.
pub fn lut_blend1(
    base: &ColorCorrectionLut,
    blend: &ColorCorrectionLut,
    blend_factor2: f32,
) -> Result<ColorCorrectionLut> {
    check_same_shape("LutBlend1", base, blend)?;
    let texels = base
        .texels
        .iter()
        .zip(&blend.texels)
        .map(|(a, b)| lerp3(*a, *b, blend_factor2))
        .collect();
    Ok(ColorCorrectionLut {
        width: base.width,
        height: base.height,
        slice_size: base.slice_size,
        texels,
    })
}

/// CPU port of `Effect LutBlend2` (DOUBLE_BLEND).
pub fn lut_blend2(
    base1: &ColorCorrectionLut,
    base2: &ColorCorrectionLut,
    blend1: &ColorCorrectionLut,
    blend2: &ColorCorrectionLut,
    blend_factor1: f32,
    blend_factor2: f32,
) -> Result<ColorCorrectionLut> {
    check_same_shape("LutBlend2 base", base1, base2)?;
    check_same_shape("LutBlend2 blend", blend1, blend2)?;
    check_same_shape("LutBlend2 cross", base1, blend1)?;
    let texels = (0..base1.texels.len())
        .map(|index| {
            let base = lerp3(base1.texels[index], base2.texels[index], blend_factor1);
            let blend = lerp3(blend1.texels[index], blend2.texels[index], blend_factor1);
            lerp3(base, blend, blend_factor2)
        })
        .collect();
    Ok(ColorCorrectionLut {
        width: base1.width,
        height: base1.height,
        slice_size: base1.slice_size,
        texels,
    })
}

#[derive(Clone, Debug)]
pub struct ColorLutProbeRow {
    pub rel_path: String,
    pub width: usize,
    pub height: usize,
    pub slice_size: usize,
    pub fnv1a64: String,
    pub identity_deviation: f32,
}

/// Inventory every `*colorcorrection*.tga` under gfx/world through the
/// resolver: dimensions, hash, and deviation from the identity cube.
pub fn scan_color_correction_luts_from_resolver(
    resolver: &GameFileResolver,
) -> Result<Vec<ColorLutProbeRow>> {
    let mut rows = Vec::new();
    for (rel, path) in resolver.resolve_dir_files(COLOR_CORRECTION_DIR, Some("tga"))? {
        let stem = rel.rsplit('/').next().unwrap_or(&rel).to_ascii_lowercase();
        if !stem.contains("colorcorrection") {
            continue;
        }
        let bytes =
            std::fs::read(&path).map_err(|err| format!("read {}: {err}", path.display()))?;
        let lut = parse_color_correction_tga(&bytes, &rel)?;
        rows.push(ColorLutProbeRow {
            rel_path: rel,
            width: lut.width,
            height: lut.height,
            slice_size: lut.slice_size,
            fnv1a64: fnv1a64_bytes(&bytes),
            identity_deviation: lut.identity_deviation(),
        });
    }
    Ok(rows)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::temp_test_dir;
    use std::fs;
    use std::path::{Path, PathBuf};

    const VANILLA_GAME_DIR: &str = r"X:\SteamLibrary\steamapps\common\Hearts of Iron IV";

    /// Build an identity cube TGA (bottom-up, 24bpp) with edge `size`.
    fn identity_tga(size: usize) -> Vec<u8> {
        let width = size * size;
        let mut bytes = vec![0u8; 18];
        bytes[2] = 2;
        bytes[12..14].copy_from_slice(&(width as u16).to_le_bytes());
        bytes[14..16].copy_from_slice(&(size as u16).to_le_bytes());
        bytes[16] = 24;
        let max_index = (size - 1) as f32;
        for row_from_bottom in 0..size {
            let green = size - 1 - row_from_bottom;
            for x in 0..width {
                let blue = x / size;
                let red = x % size;
                let scale = |index: usize| (index as f32 * 255.0 / max_index).round() as u8;
                bytes.extend_from_slice(&[scale(blue), scale(green), scale(red)]);
            }
        }
        bytes
    }

    fn constant_lut(size: usize, value: [f32; 3]) -> ColorCorrectionLut {
        ColorCorrectionLut {
            width: size * size,
            height: size,
            slice_size: size,
            texels: vec![value; size * size * size],
        }
    }

    #[test]
    fn identity_tga_round_trips_orientation_law() {
        let lut = parse_color_correction_tga(&identity_tga(8), "identity").expect("decode");
        assert_eq!(lut.slice_size, 8);
        assert_eq!(lut.identity_deviation(), 0.0);
        let sampled = lut.sample_nearest([0.5, 0.25, 1.0]);
        // Nearest index of 0.5*7 = 4 -> 4/7 etc.
        assert!((sampled[0] - (4.0f32 * 255.0 / 7.0).round() / 255.0).abs() < 1e-6);
        assert!((sampled[2] - 1.0).abs() < 1e-6);
    }

    #[test]
    fn non_cube_strip_is_a_typed_refusal_not_a_panic() {
        // 64x32 passes the old `width % height == 0` check but is not a
        // LUT strip (width must equal height*height = 1024); texel()
        // would index out of bounds. Must fail closed at parse time.
        let mut bytes = vec![0u8; 18];
        bytes[2] = 2;
        bytes[12..14].copy_from_slice(&64u16.to_le_bytes());
        bytes[14..16].copy_from_slice(&32u16.to_le_bytes());
        bytes[16] = 24;
        bytes.extend_from_slice(&[0u8; 64 * 32 * 3]);
        let err = parse_color_correction_tga(&bytes, "non_cube").expect_err("non-cube strip");
        assert!(
            err.to_string().contains("not a LUT strip"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn blend1_matches_shader_lerp_endpoints_and_midpoint() {
        let base = constant_lut(4, [0.2, 0.4, 0.6]);
        let blend = constant_lut(4, [1.0, 0.0, 0.2]);
        let close = |a: [f32; 3], b: [f32; 3]| a.iter().zip(b).all(|(x, y)| (x - y).abs() < 1e-6);
        assert!(close(
            lut_blend1(&base, &blend, 0.0).expect("f=0").texels[0],
            [0.2, 0.4, 0.6]
        ));
        assert!(close(
            lut_blend1(&base, &blend, 1.0).expect("f=1").texels[0],
            [1.0, 0.0, 0.2]
        ));
        let mid = lut_blend1(&base, &blend, 0.5).expect("f=0.5").texels[0];
        assert!((mid[0] - 0.6).abs() < 1e-6);
        assert!((mid[1] - 0.2).abs() < 1e-6);
        assert!((mid[2] - 0.4).abs() < 1e-6);
    }

    #[test]
    fn blend2_double_blend_law() {
        let base1 = constant_lut(4, [0.0, 0.0, 0.0]);
        let base2 = constant_lut(4, [1.0, 1.0, 1.0]);
        let blend1 = constant_lut(4, [1.0, 0.0, 0.0]);
        let blend2 = constant_lut(4, [0.0, 1.0, 0.0]);
        // f1=0.5: base=(0.5,0.5,0.5), blend=(0.5,0.5,0); f2=0.5 ->
        // (0.5, 0.5, 0.25).
        let out = lut_blend2(&base1, &base2, &blend1, &blend2, 0.5, 0.5).expect("blend2");
        let texel = out.texels[0];
        assert!((texel[0] - 0.5).abs() < 1e-6);
        assert!((texel[1] - 0.5).abs() < 1e-6);
        assert!((texel[2] - 0.25).abs() < 1e-6);
        // Blending identical cubes is a no-op regardless of factors.
        let same = lut_blend2(&base1, &base1, &base1, &base1, 0.3, 0.7).expect("no-op");
        assert_eq!(same.texels[0], [0.0, 0.0, 0.0]);
    }

    #[test]
    fn blend_rejects_shape_mismatch() {
        let a = constant_lut(4, [0.0; 3]);
        let b = constant_lut(8, [0.0; 3]);
        let err = lut_blend1(&a, &b, 0.5).expect_err("shape mismatch");
        assert!(err.to_string().contains("shape mismatch"), "{err}");
    }

    #[test]
    fn lut_from_resolver_uses_mod_override() {
        let root = temp_test_dir("color_lut_mod_override");
        let vanilla = root.join("vanilla");
        let mod_root = root.join("mod");
        let rel = "gfx/world/colorcorrection_neutral.tga";
        let write = |base: &Path, bytes: &[u8]| {
            let path = rel
                .split('/')
                .fold(base.to_path_buf(), |p, part| p.join(part));
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, bytes).unwrap();
        };
        write(&vanilla, &identity_tga(8));
        // Mod cube: identity with red channel forced to full.
        let mut modded = identity_tga(8);
        let width = 64usize;
        for row in 0..8 {
            for x in 0..width {
                modded[18 + (row * width + x) * 3 + 2] = 255;
            }
        }
        write(&mod_root, &modded);

        let resolver =
            GameFileResolver::with_mod_paths(vanilla, vec![mod_root.clone()]).expect("resolver");
        let lut = color_correction_lut_from_resolver(&resolver, rel).expect("modded lut");
        assert_eq!(lut.texel(0, 0, 0)[0], 1.0, "mod layer must win");
        assert_eq!(resolver.stats().overridden_file_count, 1);
        fs::remove_dir_all(root).ok();
    }

    #[test]
    fn gate_vanilla_colorcorrection_inventory() {
        let game_dir = PathBuf::from(VANILLA_GAME_DIR);
        if !game_dir.is_dir() {
            eprintln!("skipping vanilla color LUT gate: {VANILLA_GAME_DIR} is absent");
            return;
        }
        let resolver = GameFileResolver::vanilla(game_dir).expect("vanilla resolver");
        let rows = scan_color_correction_luts_from_resolver(&resolver).expect("scan");
        assert!(
            rows.len() >= 24,
            "expected the shipped colorcorrection set, got {}",
            rows.len()
        );
        for row in &rows {
            assert_eq!(
                (row.width, row.height, row.slice_size),
                (1024, 32, 32),
                "{}: unexpected LUT shape",
                row.rel_path
            );
            println!(
                "gate: {} {}x{} slice {} fnv {} identity_dev {:.1}",
                row.rel_path,
                row.width,
                row.height,
                row.slice_size,
                row.fnv1a64,
                row.identity_deviation
            );
        }
        let neutral = rows
            .iter()
            .find(|row| row.rel_path == "gfx/world/colorcorrection_neutral.tga")
            .expect("neutral cube present");
        assert_eq!(
            neutral.identity_deviation, 0.0,
            "colorcorrection_neutral must be a true identity cube"
        );
    }
}
