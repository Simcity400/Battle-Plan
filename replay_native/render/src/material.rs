//! `material` — the REAL per-pixel material textures the game's `ApplySnow` / `GetMudColor`
//! / `ApplyIce` blend, and the color-grading `ColorCube`, loaded from disk so the dynamic
//! snow/mud/ice and the final postprocess are sampled material — not a flat colour.
//!
//! Every texture here is a real game asset (resolved mod-over-vanilla by the caller):
//!   * `snow_normal_rgb_diffuse_a.dds`  — snow material: RGB = tangent normal, A = diffuse.
//!     Tiled by `worldXZ * SNOW_TILING` and blended as `vSnowTexture.a * SNOW_COLOR`.
//!   * `citylights_rgb_snowmask_a_0.dds` — the `CityLightsAndSnowNoise` texture: RGB = city
//!     lights, A = the snow/mud NOISE the gates use. (One texture, two jobs in the shader.)
//!   * `mud_diffuse_rgb_gloss_a_0.dds`   — mud material: RGB = diffuse, A = gloss.
//!   * `mud_normal_rgb_spec_a_0.dds`     — mud normal/spec: RGB = normal, A = spec.
//!   * `ice_diffuse.dds`                 — sea-ice diffuse (RGB) + alpha; tinted by ICE_COLOR.
//!   * `ice_noise_0.dds`                 — the ice noise that breaks up the freeze edge.
//!   * `colorcorrection.tga`             — the 32³ ColorCube LUT (1024×32) the restorescene
//!     postprocess samples (graded, NOT identity — Codex-verified; required for parity).
//!
//! All are decoded to top-left-origin RGBA8 here; the renderer uploads them as sampled
//! (linear, wrap) GPU textures. The ColorCube is uploaded clamp.

use std::path::Path;

/// A decoded RGBA8 image (top-left origin) ready for GPU upload.
pub struct Image {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

impl Image {
    /// Decode a DDS material texture (DXT1/DXT5/uncompressed) via the shared reader.
    pub fn load_dds(path: &Path) -> Result<Image, String> {
        let bytes = std::fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
        let img = assets::dds::decode(&bytes).map_err(|e| format!("{}: {e}", path.display()))?;
        Ok(Image {
            width: img.width,
            height: img.height,
            rgba: img.rgba,
        })
    }

    /// Decode a TGA (the ColorCube LUT) via the shared reader.
    pub fn load_tga(path: &Path) -> Result<Image, String> {
        let bytes = std::fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
        let img = assets::tga::decode(&bytes).map_err(|e| format!("{}: {e}", path.display()))?;
        Ok(Image {
            width: img.width,
            height: img.height,
            rgba: img.rgba,
        })
    }

    /// A 1×1 fallback (used when a non-critical material asset is missing so the renderer
    /// still runs; the corresponding effect degrades to a no-op, never a panic).
    pub fn solid(rgba: [u8; 4]) -> Image {
        Image {
            width: 1,
            height: 1,
            rgba: rgba.to_vec(),
        }
    }

    /// The IDENTITY 32³ ColorCube (1024×32), used when no `colorcorrection.tga` is present so
    /// the postprocess LUT step is a true no-op. Layout matches the game's `SampleColorCube`:
    /// 32 horizontal slices (one per blue level), each 32×32 (red across x, green down y);
    /// identity maps lattice index i → round(i*255/31).
    pub fn identity_color_cube() -> Image {
        let size = 32u32;
        let w = size * size; // 1024
        let h = size; // 32
        let mut rgba = vec![0u8; (w * h * 4) as usize];
        let q = |i: u32| -> u8 { ((i as f32) * 255.0 / 31.0).round() as u8 };
        for b in 0..size {
            for g in 0..size {
                for r in 0..size {
                    let x = b * size + r;
                    let y = g;
                    let o = ((y * w + x) * 4) as usize;
                    rgba[o] = q(r);
                    rgba[o + 1] = q(g);
                    rgba[o + 2] = q(b);
                    rgba[o + 3] = 255;
                }
            }
        }
        Image {
            width: w,
            height: h,
            rgba,
        }
    }
}

/// The full set of per-pixel material + grading textures the dynamic/postprocess passes need.
pub struct Materials {
    pub snow: Image,            // snow_normal_rgb_diffuse_a (RGB=normal, A=diffuse)
    pub city_snow_noise: Image, // citylights_rgb_snowmask_a (RGB=citylights, A=snow noise)
    pub mud_diffuse: Image,     // mud_diffuse_rgb_gloss_a (RGB=diffuse, A=gloss)
    pub mud_normal: Image,      // mud_normal_rgb_spec_a (RGB=normal, A=spec)
    pub ice_diffuse: Image,     // ice_diffuse (RGB=diffuse, A)
    pub ice_noise: Image,       // ice_noise_0
    pub color_cube: Image,      // colorcorrection.tga (32^3 LUT, 1024x32)
    // --- animated WATER SURFACE (the lean-wave normals + spec map the water shader uses) ---
    pub lean1: Image,      // lean1.dds (LEAN wave-normal moments)
    pub lean2: Image,      // lean2.dds
    pub water_spec: Image, // fow_rgb_waterspec_a.dds (A = the per-pixel water spec map)
    /// the REAL naval-dominance colours from `naval_dominance_fx.dds` (2×1 BGRA): index 0 =
    /// ENEMY colour, index 1 = FRIENDLY colour. Used by the `dominance_fx_apply` pulse.
    /// [standardfuncsgfx.fxh:1007-1008 tex2D(Texture3,(0,0)) / (1,0)]. Linear-decoded sRGB.
    pub dominance_enemy: [f32; 3],
    pub dominance_friendly: [f32; 3],
}

impl Materials {
    /// Load all material textures via a path resolver (mod-over-vanilla). The resolver returns
    /// the effective on-disk path for a `map/...` relpath. Non-critical misses fall back to a
    /// no-op image so the renderer still runs; a missing ColorCube falls back to IDENTITY.
    pub fn load(resolve: impl Fn(&str) -> Option<std::path::PathBuf>) -> Materials {
        let dds = |rel: &str, fallback: [u8; 4]| -> Image {
            match resolve(rel) {
                Some(p) => Image::load_dds(&p).unwrap_or_else(|e| {
                    eprintln!("[material] WARN {rel}: {e} — using fallback");
                    Image::solid(fallback)
                }),
                None => Image::solid(fallback),
            }
        };
        // ColorCube: prefer the active colorcorrection.tga (graded). If absent, IDENTITY.
        let color_cube = match resolve("gfx/world/colorcorrection.tga") {
            Some(p) => Image::load_tga(&p).unwrap_or_else(|e| {
                eprintln!("[material] WARN colorcorrection.tga: {e} — using identity cube");
                Image::identity_color_cube()
            }),
            None => Image::identity_color_cube(),
        };
        // naval-dominance colours: read the 2 pixels of naval_dominance_fx.dds (linear).
        let (dom_enemy, dom_friendly) = match resolve("map/terrain/naval_dominance_fx.dds") {
            Some(p) => match Image::load_dds(&p) {
                Ok(img) if img.rgba.len() >= 8 => {
                    let lin = |o: usize| {
                        [
                            (img.rgba[o] as f32 / 255.0).powf(2.2),
                            (img.rgba[o + 1] as f32 / 255.0).powf(2.2),
                            (img.rgba[o + 2] as f32 / 255.0).powf(2.2),
                        ]
                    };
                    (lin(0), lin(4)) // px0 = enemy, px1 = friendly
                }
                _ => ([0.55, 0.06, 0.06], [0.06, 0.18, 0.55]),
            },
            None => ([0.55, 0.06, 0.06], [0.06, 0.18, 0.55]),
        };
        Materials {
            // RGB normal ~ (128,128,255) flat-up, A diffuse 255 (no darken) as fallbacks.
            snow: dds(
                "map/terrain/snow_normal_rgb_diffuse_a.dds",
                [128, 128, 255, 255],
            ),
            city_snow_noise: dds(
                "map/terrain/citylights_rgb_snowmask_a_0.dds",
                [0, 0, 0, 128],
            ),
            mud_diffuse: dds(
                "map/terrain/mud_diffuse_rgb_gloss_a_0.dds",
                [60, 50, 40, 128],
            ),
            mud_normal: dds(
                "map/terrain/mud_normal_rgb_spec_a_0.dds",
                [128, 128, 255, 64],
            ),
            ice_diffuse: dds("map/terrain/ice_diffuse.dds", [200, 220, 255, 255]),
            ice_noise: dds("map/terrain/ice_noise_0.dds", [128, 128, 128, 255]),
            color_cube,
            dominance_enemy: dom_enemy,
            dominance_friendly: dom_friendly,
            // water surface: lean wave moments + the spec map. Fallbacks: flat lean (no waves),
            // spec=255 (fully specular/normal water) so the absence is a no-op.
            lean1: dds("map/terrain/lean1.dds", [128, 128, 128, 255]),
            lean2: dds("map/terrain/lean2.dds", [128, 128, 128, 255]),
            water_spec: dds("map/terrain/fow_rgb_waterspec_a.dds", [128, 128, 128, 255]),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The identity ColorCube is 1024×32 and maps the lattice exactly: pure-red corner
    /// (r=31,g=0,b=0) → (255,0,0); mid grey (16,16,16) → ~(131,131,131); black → (0,0,0).
    #[test]
    fn identity_cube_layout_and_values() {
        let c = Image::identity_color_cube();
        assert_eq!((c.width, c.height), (1024, 32));
        let at = |r: u32, g: u32, b: u32| -> [u8; 3] {
            let x = b * 32 + r;
            let y = g;
            let o = ((y * 1024 + x) * 4) as usize;
            [c.rgba[o], c.rgba[o + 1], c.rgba[o + 2]]
        };
        assert_eq!(at(0, 0, 0), [0, 0, 0]);
        assert_eq!(at(31, 0, 0), [255, 0, 0]);
        assert_eq!(at(0, 31, 0), [0, 255, 0]);
        assert_eq!(at(0, 0, 31), [0, 0, 255]);
        // mid lattice (16,16,16) → round(16*255/31)=132 each.
        assert_eq!(at(16, 16, 16), [132, 132, 132]);
    }

    #[test]
    fn solid_fallback_is_1x1() {
        let s = Image::solid([1, 2, 3, 4]);
        assert_eq!((s.width, s.height), (1, 1));
        assert_eq!(s.rgba, vec![1, 2, 3, 4]);
    }
}
