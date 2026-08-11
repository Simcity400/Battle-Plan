//! BORD3 — the shipped border-texture appearance law (files-first, evidence-decoded).
//!
//! This module decodes the *authentic* across-path appearance of each border
//! class straight out of its shipped `map/terrain/border_<class>_0.dds` file,
//! so nothing about border colour or edge softness is invented. It is the
//! answer to the standing user correction ("do not make things up; use the way
//! vanilla does things"): every appearance number here traces to a byte in a
//! shipped DDS, not to taste.
//!
//! # What the shader does with the texture (the mapping the profile lives in)
//!
//! `gfx/FX/border.shader:127` samples the class texture as
//! `tex2D( BorderDiffuse, float2( Input.uv.y * BORDER_TILE, Input.uv.x ) )`:
//!
//! * texture **U** (width axis) = `uv.y * BORDER_TILE` — the along-path station
//!   (`BORDER_TILE = 0.4`, `constants.fxh:103`), `AddressU = Wrap` (tiles along
//!   the border);
//! * texture **V** (height axis) = `uv.x` — the **across-path** coordinate, which
//!   the synthesized quad sweeps `0 → 1` from one side of the border to the
//!   other, `AddressV = Clamp` (`border.shader:22`-`23`).
//!
//! So a *column* of the DDS (a fixed U, varying V = varying row) is the border's
//! **across-path cross-section**: the alpha ramp that gives vanilla its soft
//! edge, and the RGB that gives its (never red) colour. The final fragment alpha
//! is `max( vColor.a, vPulseFactor - 0.2 ) * vTime_Transparency.y`
//! (`border.shader:146`) — i.e. the texture's own alpha, scaled by the
//! camera-height fade ([`crate::border_transparency`]). With no selection pulse
//! in the replay, the texture alpha IS the base opacity.
//!
//! # Decoded evidence (vanilla 1.16, `map/terrain/border_*_0.dds`)
//!
//! Measured from the top mip, middle column, by [`decode_border_texture_profile`]
//! (the numbers are asserted in the unit tests as a regression guard; a mod or a
//! patch that re-authors a border texture will change them and fail loudly):
//!
//! | class      | dims    | fmt   | peak α | ink rows (of h)      | RGB at peak      |
//! |------------|---------|-------|--------|----------------------|------------------|
//! | province   | 64×32   | DXT5  | 179    | 14–17  (≈12% centre) | black `(0,0,0)`  |
//! | state      | 128×64  | DXT5  | 114    | 21–42  (≈34% centre) | black `(0,0,0)`  |
//! | country    | 256×128 | RGBA8 | 207    | 32–95  (≈47% centre) | navy `(43,26,24)`|
//! | sea_coast  | 64×32   | RGBA8 | 114    | 10–21  (soft bell)   | black `(0,0,0)`  |
//! | sea_region | 64×32   | DXT5  | 219    | 4–27   (wide bell)   | black `(0,0,0)`  |
//! | impassable | 128×64  | RGBA8 | 255    | 0–63   (full, soft)  | brown            |
//!
//! Two facts settle two user-reported defects:
//!
//! 1. **No border texture is red.** province/state/sea are pure-black RGB, the
//!    country stripe is dark navy, impassable is brown. Any "hot red band" in a
//!    render therefore does NOT come from this pass's textures — it is either the
//!    separate gradient-border/political layer or a synthetic colour. The old CPU
//!    debug oracle (`bin/dump_border_render.rs`) painted classes with invented
//!    colours (country = `[255,83,73]`); this module replaces those with the real
//!    decoded colour so the oracle can no longer imply a colour vanilla never
//!    draws.
//! 2. **The soft edge is authored into the texture alpha**, not into geometry.
//!    sea/sea_region/country/impassable have smooth alpha ramps across V; the
//!    thin classes (province, state) are deliberately narrow. So the correct
//!    across-path mapping is exactly `uv.x = 0..1 → V = 0..1` (which the
//!    synthesized quad already emits), and the visible width per class is the
//!    ink-band fraction of the `BORDER_WIDTH`-wide quad — smaller for province,
//!    full for impassable — which is why vanilla's province lines are hair-thin
//!    and its impassable/sea lines are fat and soft.

use crate::{load_dds_rgba_mips, BorderMaterial, GameFileResolver, Result, RgbaImage};

/// `constants.fxh:103` — the along-path tiling factor the border PS multiplies
/// into `uv.y` before sampling `BorderDiffuse` (`border.shader:127`). Not an
/// appearance guess: read here so a mod override of the constant is a one-line
/// change and the value is documented next to the law it drives.
pub const BORDER_TILE: f32 = 0.4;

/// The authentic across-path appearance of one border class, decoded from its
/// shipped `border_<class>_0.dds`.
///
/// The "profile" is the middle-column cross-section of the top mip: for each
/// texture row (V, the across-path coordinate the shader reads as `uv.x`), the
/// sampled RGBA. Everything a caller needs to paint the class the way vanilla
/// does — peak colour, peak alpha, and how wide the visible ink is relative to
/// the quad — is derived from that column, never invented.
#[derive(Clone, Debug, PartialEq)]
pub struct BorderTextureProfile {
    pub material: BorderMaterial,
    /// Shipped DDS this was decoded from.
    pub source: String,
    pub width: usize,
    pub height: usize,
    /// Per-row `(r,g,b,a)` down the middle column (len == `height`); index is the
    /// texture V row, i.e. the across-path coordinate `uv.x * (height-1)`.
    pub cross_section: Vec<[u8; 4]>,
    /// Highest alpha in the cross-section = the class's base opacity at the
    /// border centre (before the camera-height fade multiply).
    pub peak_alpha: u8,
    /// RGB at the first peak-alpha row = the class's authentic ink colour.
    pub peak_rgb: [u8; 3],
    /// First and last row (inclusive) whose alpha exceeds
    /// [`INK_ALPHA_THRESHOLD`] — the extent of the visible ink band.
    pub ink_first_row: usize,
    pub ink_last_row: usize,
}

/// Alpha at/below which a texel is treated as transparent margin rather than
/// visible ink, when measuring the ink band. Chosen just above the DXT5
/// quantisation floor (province's shoulder texels read `4`); the two thin
/// classes' plateaus (114/179) and the wide classes all sit far above it.
pub const INK_ALPHA_THRESHOLD: u8 = 10;

impl BorderTextureProfile {
    /// The ink band width in texture rows (`ink_last_row - ink_first_row + 1`).
    pub fn ink_rows(&self) -> usize {
        self.ink_last_row.saturating_sub(self.ink_first_row) + 1
    }

    /// The ink band as a fraction of the full texture height. Because the
    /// synthesized quad maps `uv.x = 0..1` across the whole `BORDER_WIDTH`-wide
    /// quad and the shader samples V = `uv.x`, this fraction is exactly the
    /// fraction of the quad the visible ink occupies — i.e. the class's on-quad
    /// line thickness relative to `BORDER_WIDTH`.
    pub fn ink_fraction_of_quad(&self) -> f32 {
        if self.height == 0 {
            return 0.0;
        }
        self.ink_rows() as f32 / self.height as f32
    }

    /// The class's authentic ink colour with its base opacity, as an RGBA the
    /// CPU debug oracle can composite. This is the shipped-texture colour, never
    /// a synthetic per-class tint.
    pub fn ink_rgba(&self) -> [u8; 4] {
        [
            self.peak_rgb[0],
            self.peak_rgb[1],
            self.peak_rgb[2],
            self.peak_alpha,
        ]
    }
}

/// Decode the middle-column across-path cross-section of one border class from
/// an already-loaded top-mip RGBA image.
///
/// Split from the resolver entry point so it is unit-testable against a
/// synthetic image without touching the filesystem. Crate-internal because
/// `RgbaImage` is a crate-internal type; the public entry points
/// ([`border_texture_profile_from_resolver`],
/// [`all_border_texture_profiles_from_resolver`]) take a resolver instead.
pub(crate) fn decode_border_texture_profile(
    material: BorderMaterial,
    source: &str,
    top_mip: &RgbaImage,
) -> Result<BorderTextureProfile> {
    if top_mip.width == 0 || top_mip.height == 0 {
        return Err(format!(
            "border texture law blocked: {source} decoded to a {}x{} image",
            top_mip.width, top_mip.height
        )
        .into());
    }
    let expected = top_mip.width * top_mip.height * 4;
    if top_mip.pixels.len() != expected {
        return Err(format!(
            "border texture law blocked: {source} has {} bytes for {}x{} RGBA (expected {expected})",
            top_mip.pixels.len(),
            top_mip.width,
            top_mip.height
        )
        .into());
    }
    let mid_col = top_mip.width / 2;
    let mut cross_section = Vec::with_capacity(top_mip.height);
    for row in 0..top_mip.height {
        let idx = (row * top_mip.width + mid_col) * 4;
        cross_section.push([
            top_mip.pixels[idx],
            top_mip.pixels[idx + 1],
            top_mip.pixels[idx + 2],
            top_mip.pixels[idx + 3],
        ]);
    }
    let peak_alpha = cross_section.iter().map(|p| p[3]).max().unwrap_or(0);
    // RGB at the first row that reaches the peak alpha = the ink colour.
    let peak_rgb = cross_section
        .iter()
        .find(|p| p[3] == peak_alpha)
        .map(|p| [p[0], p[1], p[2]])
        .unwrap_or([0, 0, 0]);
    let ink_rows: Vec<usize> = cross_section
        .iter()
        .enumerate()
        .filter(|(_, p)| p[3] > INK_ALPHA_THRESHOLD)
        .map(|(row, _)| row)
        .collect();
    let (ink_first_row, ink_last_row) = match (ink_rows.first(), ink_rows.last()) {
        (Some(&first), Some(&last)) => (first, last),
        _ => {
            return Err(format!(
                "border texture law blocked: {source} cross-section has no ink row above alpha \
                 {INK_ALPHA_THRESHOLD}; a border DDS must have a visible stripe"
            )
            .into());
        }
    };
    Ok(BorderTextureProfile {
        material,
        source: source.to_string(),
        width: top_mip.width,
        height: top_mip.height,
        cross_section,
        peak_alpha,
        peak_rgb,
        ink_first_row,
        ink_last_row,
    })
}

/// Decode the across-path appearance profile for a border class from its shipped
/// DDS through the mod-aware resolver (files-first; a mod's overriding border
/// texture is honoured). Uses [`crate::border_diffuse_source`] as the single
/// class → file mapping the GPU pass also binds.
pub fn border_texture_profile_from_resolver(
    resolver: &GameFileResolver,
    material: BorderMaterial,
) -> Result<BorderTextureProfile> {
    let source = crate::border_diffuse_source(material);
    let path = resolver
        .resolve_required(source, "border diffuse DDS")
        .map_err(|err| format!("border texture law blocked: {err}"))?;
    let mips = load_dds_rgba_mips(&path)
        .map_err(|err| format!("border texture law blocked: failed to decode {source}: {err}"))?;
    let top = mips
        .first()
        .ok_or_else(|| format!("border texture law blocked: {source} decoded zero mips"))?;
    decode_border_texture_profile(material, source, top)
}

/// Decode every border class's profile in a stable order (for the CPU oracle and
/// for reporting). Fails closed if any class's DDS is missing or malformed.
pub fn all_border_texture_profiles_from_resolver(
    resolver: &GameFileResolver,
) -> Result<Vec<BorderTextureProfile>> {
    [
        BorderMaterial::Province,
        BorderMaterial::State,
        BorderMaterial::Country,
        BorderMaterial::SeaCoast,
        BorderMaterial::SeaRegion,
        BorderMaterial::Impassable,
    ]
    .into_iter()
    .map(|material| border_texture_profile_from_resolver(resolver, material))
    .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn image_from_alpha_column(width: usize, height: usize, column: &[(u8, [u8; 3])]) -> RgbaImage {
        assert_eq!(column.len(), height);
        let mut pixels = vec![0u8; width * height * 4];
        for (row, (alpha, rgb)) in column.iter().enumerate() {
            for col in 0..width {
                let idx = (row * width + col) * 4;
                pixels[idx] = rgb[0];
                pixels[idx + 1] = rgb[1];
                pixels[idx + 2] = rgb[2];
                pixels[idx + 3] = *alpha;
            }
        }
        RgbaImage {
            width,
            height,
            pixels,
        }
    }

    #[test]
    fn decodes_centered_soft_bell_as_ink_band() {
        // A soft, black, centred bell like the shipped sea_coast texture: alpha
        // ramps up to a peak at the centre and back down, RGB black.
        let column = [
            (0u8, [0u8; 3]),
            (0, [0; 3]),
            (33, [0; 3]),
            (96, [0; 3]),
            (114, [0; 3]),
            (114, [0; 3]),
            (96, [0; 3]),
            (33, [0; 3]),
            (0, [0; 3]),
            (0, [0; 3]),
        ];
        let img = image_from_alpha_column(8, 10, &column);
        let p =
            decode_border_texture_profile(BorderMaterial::SeaCoast, "test/sea.dds", &img).unwrap();
        assert_eq!(p.peak_alpha, 114);
        assert_eq!(p.peak_rgb, [0, 0, 0], "sea ink is black, never red");
        // Ink band: rows 2..=7 (alpha > 10).
        assert_eq!(p.ink_first_row, 2);
        assert_eq!(p.ink_last_row, 7);
        assert_eq!(p.ink_rows(), 6);
        // The bell is symmetric about the middle -> ink centred in the quad.
        let centre = (p.ink_first_row + p.ink_last_row) as f32 / 2.0;
        assert!((centre - (p.height as f32 - 1.0) / 2.0).abs() <= 1.0);
    }

    #[test]
    fn thin_black_plateau_is_thin_and_black_like_province() {
        // Province: a hard, narrow, black plateau (peak 179) over ~12% of height.
        let mut column = vec![(0u8, [0u8; 3]); 32];
        for row in 14..=17 {
            column[row] = (179, [0, 0, 0]);
        }
        column[13] = (4, [0, 0, 0]); // sub-threshold shoulder
        column[18] = (4, [0, 0, 0]);
        let img = image_from_alpha_column(64, 32, &column);
        let p =
            decode_border_texture_profile(BorderMaterial::Province, "test/prov.dds", &img).unwrap();
        assert_eq!(p.peak_alpha, 179);
        assert_eq!(p.peak_rgb, [0, 0, 0]);
        assert_eq!(p.ink_first_row, 14);
        assert_eq!(p.ink_last_row, 17);
        // Sub-threshold shoulders (alpha 4) are NOT counted as ink.
        assert_eq!(p.ink_rows(), 4);
        // ≈12.5% of the quad -> province is the thinnest class.
        assert!((p.ink_fraction_of_quad() - 0.125).abs() < 0.01);
    }

    #[test]
    fn ink_rgba_never_invents_a_colour() {
        // A navy country-like stripe: the ink colour is whatever the texture
        // holds, not a synthetic tint.
        let mut column = vec![(0u8, [0u8; 3]); 16];
        for row in 6..=9 {
            column[row] = (207, [24, 26, 43]);
        }
        let img = image_from_alpha_column(32, 16, &column);
        let p = decode_border_texture_profile(BorderMaterial::Country, "test/country.dds", &img)
            .unwrap();
        assert_eq!(p.ink_rgba(), [24, 26, 43, 207]);
        // Not red: red channel is not dominant.
        let [r, g, b, _] = p.ink_rgba();
        assert!(
            !(r > g + 40 && r > b + 40),
            "country ink must not be red-dominant: {:?}",
            p.ink_rgba()
        );
    }

    #[test]
    fn all_transparent_texture_fails_closed() {
        let column = vec![(0u8, [0u8; 3]); 8];
        let img = image_from_alpha_column(8, 8, &column);
        let err = decode_border_texture_profile(BorderMaterial::Province, "test/blank.dds", &img)
            .expect_err("a fully transparent border texture must fail closed");
        assert!(err.to_string().contains("no ink row"), "unexpected: {err}");
    }

    #[test]
    fn empty_dimensions_fail_closed() {
        let img = RgbaImage {
            width: 0,
            height: 0,
            pixels: Vec::new(),
        };
        assert!(
            decode_border_texture_profile(BorderMaterial::State, "test/empty.dds", &img).is_err()
        );
    }

    /// Real shipped-DDS evidence gate (env-gated on `BATTLE_PLAN_GAME_DIR`, like the
    /// other capture-evidence tests): decode the actual vanilla
    /// `border_*_0.dds` and assert they match the documented BORD3 profile
    /// table — same dims, same peak alpha, never red, and the thin/fat class
    /// ordering that reproduces vanilla's line thicknesses. A game patch that
    /// re-authors a texture will fail this loudly.
    #[test]
    #[cfg_attr(
        not(gate_hoi4_install),
        ignore = "the game install: set BATTLE_PLAN_GAME_ROOT (or BATTLE_PLAN_GAME_DIR / BATTLE_PLAN_VANILLA) to it, or install it at the default Steam path"
    )]
    fn shipped_border_textures_match_documented_profiles() {
        let game = replay_test_gates::require_vanilla_game_root();
        let resolver = GameFileResolver::vanilla(&game).expect("vanilla resolver");
        let profiles = all_border_texture_profiles_from_resolver(&resolver)
            .expect("decode all shipped border textures");
        let by: std::collections::BTreeMap<_, _> =
            profiles.iter().map(|p| (p.material, p)).collect();

        // Documented dims + peak alpha (BORDER_LAW.md BORD3 table).
        let expect: [(BorderMaterial, usize, usize, u8); 6] = [
            (BorderMaterial::Province, 64, 32, 179),
            (BorderMaterial::State, 128, 64, 114),
            (BorderMaterial::Country, 256, 128, 207),
            (BorderMaterial::SeaCoast, 64, 32, 114),
            (BorderMaterial::SeaRegion, 64, 32, 219),
            (BorderMaterial::Impassable, 128, 64, 255),
        ];
        for (material, w, h, peak) in expect {
            let p = by.get(&material).expect("class profile present");
            assert_eq!((p.width, p.height), (w, h), "{material:?} dims");
            assert_eq!(p.peak_alpha, peak, "{material:?} peak alpha");
            // No border texture ink is red.
            let [r, g, b] = p.peak_rgb;
            assert!(
                !(r > g + 40 && r > b + 40),
                "{material:?} ink must not be red-dominant: {:?}",
                p.peak_rgb
            );
        }

        // Thickness ordering the textures encode: province is the thinnest ink
        // band, impassable the widest; sea classes sit in between. This is what
        // makes vanilla's province lines hair-thin and sea lines fat.
        let frac = |m: BorderMaterial| by.get(&m).unwrap().ink_fraction_of_quad();
        assert!(
            frac(BorderMaterial::Province) < frac(BorderMaterial::State),
            "province ink must be thinner than state"
        );
        assert!(
            frac(BorderMaterial::State) < frac(BorderMaterial::Impassable),
            "state ink must be thinner than impassable"
        );
        assert!(
            frac(BorderMaterial::Impassable) >= 0.9,
            "impassable fills nearly the whole quad"
        );
    }
}
