//! Gradient-border channel textures: captured static halves + generated
//! country layer.
//!
//! GROUND TRUTH (live_PAN2 capture, 2816x2050 B8G8R8A8_UNORM_SRGB runtime
//! SRVs; layer split per `standardfuncsgfx.fxh:903-917`: top half sampled at
//! uv, bottom half at uv+0.5):
//!
//! - Channel1 TOP    = strategic-region (sea zone) layer: constant RGB
//!   (51,63,89) on 100% of texels, alpha = border distance field. STATIC
//!   world geometry -> reuse captured bytes verbatim.
//! - Channel1 BOTTOM = country layer: owner color RGB on land, black on sea;
//!   alpha = 255 outside a country, ramping down inside:
//!   alpha = clip(255 * (1 - (D - 0.875)/T)), D = euclidean texel distance
//!   to the nearest different-country texel (x-wrapped), T ~= 12.75 half-res
//!   texels for large countries (= GRADIENT_BORDERS_THICKNESS_COUNTRY_HIGH
//!   25 full-res px / 2; fit over 40 captured country blobs, mean residual
//!   2.5-9 alpha). DYNAMIC -> generated from replay ownership each change.
//! - Channel2 TOP    = naval supremacy fill ((255,0,0), alpha {128,255}).
//!   Replay has NO supremacy data -> zeroed. `standardfuncsgfx.fxh:913-919`
//!   gates each layer's contribution by Channel2.g at the layer's uv, so a
//!   zero top half also correctly disables the sea-zone layer, matching the
//!   game's default political mapmode (captured top half has g=0 everywhere).
//!   `dominance_fx_apply` (fxh:993-1028) reads ch2 top .r/.a -> zero = off.
//! - Channel2 BOTTOM = land mask (0,255,0) on all land, alpha 0. STATIC ->
//!   reuse captured bytes verbatim (this is what turns the country layer ON
//!   over land via TranspB).
//!
//! The old generator inverted this structure (country layer in the top half,
//! a fabricated per-state glow layer in the bottom, ch2 green on owned cells
//! in both halves). That rendered a glow around every state, bled country
//! color into the sea-zone layer, and polluted pdxwater (which samples the
//! same channels: pdxwater.shader:349-355).

use std::collections::BTreeMap;
use std::ptr;
use std::time::{Duration, Instant};

use winapi::shared::dxgiformat::{DXGI_FORMAT_B8G8R8A8_TYPELESS, DXGI_FORMAT_B8G8R8A8_UNORM_SRGB};
use winapi::shared::dxgitype::DXGI_SAMPLE_DESC;
use winapi::shared::minwindef::UINT;
use winapi::um::d3d11::{
    ID3D11Resource, ID3D11ShaderResourceView, ID3D11Texture2D, D3D11_BIND_SHADER_RESOURCE,
    D3D11_BOX, D3D11_SHADER_RESOURCE_VIEW_DESC, D3D11_SUBRESOURCE_DATA, D3D11_TEX2D_SRV,
    D3D11_TEXTURE2D_DESC, D3D11_USAGE_DEFAULT,
};
use winapi::um::d3dcommon::D3D11_SRV_DIMENSION_TEXTURE2D;

use map_viewer::GameFileResolver;

use crate::bundle::MapBundle;
use crate::d3d::{hr_ok, ComPtr, Gfx};
use crate::palette;
use crate::Result;

// Captured GradientBorderChannel1/2 descriptor: 2816x2050 for a 5632x2048
// map (two 1024-row halves + 2 guard rows; see GbDimensions).
// `standardfuncsgfx.fxh:903-906` scales the active map UV into the top half
// and samples the second layer at `+0.5`; captured `GB_TextureHeight` is
// 1024 at `dx11_cb0` offset 400.
//
// `common/defines/00_graphics.lua:865-869`. Field units are full-resolution
// provinces.bmp pixels; the GB texture is half-res, so thickness in texels is
// the define value / 2. Country area in cells * 4 = full-res pixel area.
//
// These are the SHIPPED vanilla values, kept only as the fail-open fallback
// used when no game resolver is available (headless/no-`--game` runs). The live
// renderer reads the four defines through the mod-aware resolver
// ([`CountryFieldDefines::from_resolver`]) so a mod that overrides
// `GRADIENT_BORDERS_FIELD_COUNTRY_LOW/HIGH` or
// `GRADIENT_BORDERS_THICKNESS_COUNTRY_LOW/HIGH` is honoured (files-first
// mandate); a mod that widens its country borders must widen the replay's too.
const FIELD_COUNTRY_LOW: f32 = 300.0;
const FIELD_COUNTRY_HIGH: f32 = 9000.0;
const THICKNESS_COUNTRY_LOW: f32 = 5.0;
const THICKNESS_COUNTRY_HIGH: f32 = 25.0;

/// Vanilla `GRADIENT_BORDERS_COUNTRY_CENTER_THICKNESS` (`00_graphics.lua:868`):
/// "The center gradient is linear 1/255 per pixel for this many pixels". This
/// full-res center plateau is the engine-side texture-generation behaviour that
/// [`RAMP_D0`] (0.875 half-res texels) was measured to approximate; the ramp fit
/// is proven only for this vanilla center value. A mod that changes it would
/// invalidate `RAMP_D0`, so [`CountryFieldDefines::from_graphics_defines`] reads
/// it and fails closed on any non-vanilla value (rather than silently rendering
/// a wrong ramp). Deriving `RAMP_D0` from a modded center is a tracked gap.
const CENTER_THICKNESS_COUNTRY_VANILLA: f32 = 2.0;

/// The `00_graphics.lua` relative path the country-field defines are read from,
/// through the mod-aware resolver.
const GRAPHICS_DEFINES_REL: &str = "common/defines/00_graphics.lua";

/// The four `GRADIENT_BORDERS_*_COUNTRY_*` defines that drive the country
/// layer's per-country ramp thickness. Read from the live (mod-resolved)
/// `common/defines/00_graphics.lua` so mod overrides are honoured; the shipped
/// vanilla values are the fail-open fallback when no resolver is present.
///
/// Only the SOURCE of these four numbers changes here — the area→thickness
/// interpolation ([`country_thickness_texels_for_cell_area`]) and the measured
/// ramp ([`RAMP_D0`], chamfer distance) are unchanged, so this stays byte-exact
/// with the vanilla capture-validated field while making a modded field track
/// the mod's own defines.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct CountryFieldDefines {
    pub(crate) field_country_low: f32,
    pub(crate) field_country_high: f32,
    pub(crate) thickness_country_low: f32,
    pub(crate) thickness_country_high: f32,
}

impl CountryFieldDefines {
    /// The shipped vanilla values (`00_graphics.lua:865-869`); the fail-open
    /// fallback and the unit-test oracle.
    pub(crate) const fn vanilla() -> Self {
        Self {
            field_country_low: FIELD_COUNTRY_LOW,
            field_country_high: FIELD_COUNTRY_HIGH,
            thickness_country_low: THICKNESS_COUNTRY_LOW,
            thickness_country_high: THICKNESS_COUNTRY_HIGH,
        }
    }

    /// Read the four defines from the live `00_graphics.lua` through the
    /// resolver. Fails closed if a key is missing or if a field/thickness range
    /// is inverted or non-finite — the shipped file is well-formed, so a
    /// violation is a real data problem, not something to paper over.
    ///
    /// Scope (documented limitation, matching `grading_law`'s defines reader):
    /// only the resolved `00_graphics.lua` is read, and only bare
    /// `KEY = <number>` lines match — an override written as
    /// `NDefines.NGraphics.KEY = ...` or placed in a *different*
    /// `common/defines/*.lua` file is not seen and falls back to the vanilla
    /// value. The vanilla and standard-full-file-override cases are handled;
    /// dotted-path / split-file overrides are UNTRACED.
    pub(crate) fn from_resolver(resolver: &GameFileResolver) -> Result<Self> {
        let path = resolver.resolve_required(GRAPHICS_DEFINES_REL, "graphics defines lua")?;
        let text =
            std::fs::read_to_string(&path).map_err(|e| format!("read {}: {e}", path.display()))?;
        Self::from_graphics_defines(&text)
    }

    /// Parse the four defines out of an already-loaded `00_graphics.lua` body.
    /// Split from [`Self::from_resolver`] so the parse is unit-testable without a
    /// resolver.
    pub(crate) fn from_graphics_defines(text: &str) -> Result<Self> {
        let field_country_low = parse_lua_number(text, "GRADIENT_BORDERS_FIELD_COUNTRY_LOW")?;
        let field_country_high = parse_lua_number(text, "GRADIENT_BORDERS_FIELD_COUNTRY_HIGH")?;
        let thickness_country_low =
            parse_lua_number(text, "GRADIENT_BORDERS_THICKNESS_COUNTRY_LOW")?;
        let thickness_country_high =
            parse_lua_number(text, "GRADIENT_BORDERS_THICKNESS_COUNTRY_HIGH")?;
        // The ramp's center offset (RAMP_D0) is a measured fit to the vanilla
        // center-thickness plateau. If a mod changes CENTER_THICKNESS the fit no
        // longer holds, so fail closed rather than render a silently-wrong ramp
        // (tracked gap: deriving RAMP_D0 from a modded center). The key is
        // present in the shipped file; treat its absence as a real data problem.
        let center_thickness = parse_lua_number(text, "GRADIENT_BORDERS_COUNTRY_CENTER_THICKNESS")?;
        // `(NaN - x).abs() > eps` is false, so a non-finite center would slip
        // past the vanilla comparison below; reject it explicitly first.
        if !center_thickness.is_finite()
            || (center_thickness - CENTER_THICKNESS_COUNTRY_VANILLA).abs() > 1e-3
        {
            return Err(format!(
                "gradient-border country field blocked: {GRAPHICS_DEFINES_REL} \
                 GRADIENT_BORDERS_COUNTRY_CENTER_THICKNESS={center_thickness} differs from the \
                 vanilla {CENTER_THICKNESS_COUNTRY_VANILLA} the country-ramp center offset \
                 (RAMP_D0) was measured against; the modded center thickness is not yet traced"
            )
            .into());
        }
        let defines = Self {
            field_country_low,
            field_country_high,
            thickness_country_low,
            thickness_country_high,
        };
        defines.validate()?;
        Ok(defines)
    }

    fn validate(&self) -> Result<()> {
        for (name, value) in [
            ("GRADIENT_BORDERS_FIELD_COUNTRY_LOW", self.field_country_low),
            (
                "GRADIENT_BORDERS_FIELD_COUNTRY_HIGH",
                self.field_country_high,
            ),
            (
                "GRADIENT_BORDERS_THICKNESS_COUNTRY_LOW",
                self.thickness_country_low,
            ),
            (
                "GRADIENT_BORDERS_THICKNESS_COUNTRY_HIGH",
                self.thickness_country_high,
            ),
        ] {
            if !value.is_finite() || value < 0.0 {
                return Err(format!(
                    "gradient-border country field blocked: {GRAPHICS_DEFINES_REL} {name} has \
                     invalid value {value}"
                )
                .into());
            }
        }
        if self.field_country_high <= self.field_country_low {
            return Err(format!(
                "gradient-border country field blocked: {GRAPHICS_DEFINES_REL} \
                 GRADIENT_BORDERS_FIELD_COUNTRY_HIGH={} must exceed FIELD_COUNTRY_LOW={}",
                self.field_country_high, self.field_country_low
            )
            .into());
        }
        if self.thickness_country_high < self.thickness_country_low {
            return Err(format!(
                "gradient-border country field blocked: {GRAPHICS_DEFINES_REL} \
                 GRADIENT_BORDERS_THICKNESS_COUNTRY_HIGH={} must be >= THICKNESS_COUNTRY_LOW={}",
                self.thickness_country_high, self.thickness_country_low
            )
            .into());
        }
        Ok(())
    }
}

/// COL3: the engine's MAP-FILL country color transform. The colors baked into
/// the gradient-border country layer are NOT the raw `color = rgb {..}` file
/// values — the engine pushes them through the `NDefines.NGraphics`
/// modifiers first (`common/defines/00_graphics.lua:1164-1166`, vanilla
/// HUE 0.0 / SATURATION 0.6 / BRIGHTNESS 0.8; the separate
/// `COUNTRY_UI_COLOR_*` pair is 1.0/1.0, so UI colors stay raw):
///
///   baked = trunc(255 * hsv_to_rgb(h + HUE, min(1, s*SATURATION),
///                                  min(1, v*BRIGHTNESS)))
///
/// BYTE-EXACT against the engine-generated GradientBorderChannel1 country
/// layer in TWO captures (live_PAN2 1936 + session_POLCOLOR_1 1940+):
/// ENG (201,56,93)->(160,91,108), GER (102,96,87)->(81,78,74),
/// SOV (125,13,24)->(100,46,51), BRA (76,145,63)->(82,116,76),
/// SWE (36,132,247)->(96,142,197), RAJ cosmetic (170,10,10)->(136,59,59).
/// Truncation (not rounding) is the engine's float->byte conversion — the
/// round() variant misses by +1 on 5 of the 6 references.
///
/// The HUE modifier's unit is UNTRACED (vanilla is 0.0 = no rotation); it is
/// applied here as a fraction of the hue circle, flagged in the ledger.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct CountryColorModifiers {
    pub(crate) hue: f64,
    pub(crate) saturation: f64,
    pub(crate) brightness: f64,
}

impl CountryColorModifiers {
    pub(crate) const fn vanilla() -> Self {
        Self {
            hue: 0.0,
            saturation: 0.6,
            brightness: 0.8,
        }
    }

    pub(crate) fn from_resolver(resolver: &GameFileResolver) -> Result<Self> {
        let path = resolver.resolve_required(GRAPHICS_DEFINES_REL, "graphics defines lua")?;
        let text =
            std::fs::read_to_string(&path).map_err(|e| format!("read {}: {e}", path.display()))?;
        Self::from_graphics_defines(&text)
    }

    pub(crate) fn from_graphics_defines(text: &str) -> Result<Self> {
        // Parse at f64 so the shipped decimal literals (0.6 / 0.8) keep the
        // exact values the byte-exact bake law was validated with — an f32
        // round-trip (0.6000000238..) would perturb the truncation boundary.
        let modifiers = Self {
            hue: parse_lua_number_f64(text, "COUNTRY_COLOR_HUE_MODIFIER")?,
            saturation: parse_lua_number_f64(text, "COUNTRY_COLOR_SATURATION_MODIFIER")?,
            brightness: parse_lua_number_f64(text, "COUNTRY_COLOR_BRIGHTNESS_MODIFIER")?,
        };
        modifiers.validate()?;
        Ok(modifiers)
    }

    fn validate(&self) -> Result<()> {
        for (name, value) in [
            ("COUNTRY_COLOR_HUE_MODIFIER", self.hue),
            ("COUNTRY_COLOR_SATURATION_MODIFIER", self.saturation),
            ("COUNTRY_COLOR_BRIGHTNESS_MODIFIER", self.brightness),
        ] {
            if !value.is_finite() || !(-1.0..=4.0).contains(&value) {
                return Err(format!(
                    "gradient-border country color blocked: {GRAPHICS_DEFINES_REL} {name} has \
                     implausible value {value}"
                )
                .into());
            }
        }
        Ok(())
    }

    /// Apply the modifier law to a raw file color (see struct docs).
    pub(crate) fn bake(&self, raw: [u8; 3]) -> [u8; 3] {
        let r = f64::from(raw[0]) / 255.0;
        let g = f64::from(raw[1]) / 255.0;
        let b = f64::from(raw[2]) / 255.0;
        let (h, s, v) = rgb_to_hsv(r, g, b);
        let h = (h + self.hue).rem_euclid(1.0);
        let s = (s * self.saturation).min(1.0);
        let v = (v * self.brightness).min(1.0);
        let (r, g, b) = hsv_to_rgb(h, s, v);
        // Truncation is the validated engine conversion; the 1e-9 epsilon only
        // absorbs f64 round-trip REPRESENTATION error so mathematically-exact
        // integer results don't lose a bit (identity modifiers, greys, and
        // e.g. raws (200,100,31)/(0,80,230) whose G channel is exactly 112
        // but evaluates to 111.999999999999986 — verified by exact rational
        // arithmetic, deficit 1.4e-14). Codex review flagged a wider epsilon
        // as able to mispromote a TRUE fraction just below an integer; 1e-9
        // is far above repr error (~1e-14) and far below any real fraction.
        // It cannot promote any byte-exact reference (fractions >= 0.05).
        let to_byte = |c: f64| (c * 255.0 + 1e-9).floor().clamp(0.0, 255.0) as u8;
        [to_byte(r), to_byte(g), to_byte(b)]
    }
}

/// Standard hue/saturation/value decomposition (hue as a fraction in [0,1)),
/// max/min formulation — the engine bake reproduces byte-exactly through it.
fn rgb_to_hsv(r: f64, g: f64, b: f64) -> (f64, f64, f64) {
    let maxc = r.max(g).max(b);
    let minc = r.min(g).min(b);
    let v = maxc;
    if maxc <= minc {
        return (0.0, 0.0, v);
    }
    let s = (maxc - minc) / maxc;
    let span = maxc - minc;
    let rc = (maxc - r) / span;
    let gc = (maxc - g) / span;
    let bc = (maxc - b) / span;
    let h = if r >= maxc {
        bc - gc
    } else if g >= maxc {
        2.0 + rc - bc
    } else {
        4.0 + gc - rc
    };
    ((h / 6.0).rem_euclid(1.0), s, v)
}

fn hsv_to_rgb(h: f64, s: f64, v: f64) -> (f64, f64, f64) {
    if s <= 0.0 {
        return (v, v, v);
    }
    let i = (h * 6.0).floor();
    let f = h * 6.0 - i;
    let p = v * (1.0 - s);
    let q = v * (1.0 - s * f);
    let t = v * (1.0 - s * (1.0 - f));
    match (i as i64).rem_euclid(6) {
        0 => (v, t, p),
        1 => (q, v, p),
        2 => (p, v, t),
        3 => (p, q, v),
        4 => (t, p, v),
        _ => (v, p, q),
    }
}

/// COL3: the political-mapmode scene CB0 defines (see
/// `map_viewer::PoliticalSceneCb0` for the write law and its engine evidence).
/// Resolver-read so a mod's outline cutoffs / shadow fade heights are
/// honoured; vanilla fallback without a resolver.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct PoliticalSceneDefines {
    pub(crate) outline_cutoff_strategic: f32,
    pub(crate) outline_cutoff_country: f32,
    pub(crate) draw_shadows_cutoff: f32,
    pub(crate) draw_shadows_fade_length: f32,
}

impl PoliticalSceneDefines {
    /// Shipped vanilla values (`00_graphics.lua:860-861,880-885`).
    pub(crate) const fn vanilla() -> Self {
        Self {
            outline_cutoff_strategic: 0.98,
            outline_cutoff_country: 0.973,
            draw_shadows_cutoff: 400.0,
            draw_shadows_fade_length: 50.0,
        }
    }

    pub(crate) fn from_resolver(resolver: &GameFileResolver) -> Result<Self> {
        let path = resolver.resolve_required(GRAPHICS_DEFINES_REL, "graphics defines lua")?;
        let text =
            std::fs::read_to_string(&path).map_err(|e| format!("read {}: {e}", path.display()))?;
        Self::from_graphics_defines(&text)
    }

    pub(crate) fn from_graphics_defines(text: &str) -> Result<Self> {
        Ok(Self {
            outline_cutoff_strategic: parse_lua_number(
                text,
                "GRADIENT_BORDERS_OUTLINE_CUTOFF_STRATEGIC_REGIONS",
            )?,
            outline_cutoff_country: parse_lua_number(
                text,
                "GRADIENT_BORDERS_OUTLINE_CUTOFF_COUNTRY",
            )?,
            draw_shadows_cutoff: parse_lua_number(text, "DRAW_SHADOWS_CUTOFF")?,
            draw_shadows_fade_length: parse_lua_number(text, "DRAW_SHADOWS_FADE_LENGTH")?,
        })
    }

    pub(crate) fn to_cb0(self) -> map_viewer::PoliticalSceneCb0 {
        map_viewer::PoliticalSceneCb0 {
            outline_cutoff_strategic: self.outline_cutoff_strategic,
            outline_cutoff_country: self.outline_cutoff_country,
            draw_shadows_cutoff: self.draw_shadows_cutoff,
            draw_shadows_fade_length: self.draw_shadows_fade_length,
            // Product default: keep the captured SnowMudFade byte. The
            // SEASON1 validation flag overrides this at the render layer.
            snow_mud_fade: None,
            // No shadow pass is replayed yet and the product capture's
            // ShadowMap SRV is NULL (see PoliticalSceneCb0::shadow_map_bound
            // docs): a nonzero fade factor against a null map darkens EVERY
            // pixel uniformly, so the factor stays gated to 0 for now.
            shadow_map_bound: false,
        }
    }
}

/// Parse a single `KEY = <number>,` line out of a Lua defines body, tolerating a
/// trailing comma and a `--` comment. Mirrors the robust reader in
/// `grading_law::parse_graphics_grading_defines`; fails closed if the key is
/// absent.
fn parse_lua_number(text: &str, key: &str) -> Result<f32> {
    for raw in text.lines() {
        let line = raw.trim();
        if line.starts_with("--") {
            continue;
        }
        let Some((lhs, rhs)) = line.split_once('=') else {
            continue;
        };
        if lhs.trim() != key {
            continue;
        }
        let rhs = rhs.split("--").next().unwrap_or(rhs);
        let rhs = rhs.trim().trim_end_matches(',').trim();
        if let Ok(value) = rhs.parse::<f32>() {
            return Ok(value);
        }
    }
    Err(
        format!("gradient-border country field blocked: {GRAPHICS_DEFINES_REL} missing {key}")
            .into(),
    )
}

/// Same line matching as [`parse_lua_number`] but parsed at f64 — the COL3
/// color-modifier law is validated byte-exactly at the shipped decimal values
/// and must not pick up f32 round-trip error near truncation boundaries.
fn parse_lua_number_f64(text: &str, key: &str) -> Result<f64> {
    for raw in text.lines() {
        let line = raw.trim();
        if line.starts_with("--") {
            continue;
        }
        let Some((lhs, rhs)) = line.split_once('=') else {
            continue;
        };
        if lhs.trim() != key {
            continue;
        }
        let rhs = rhs.split("--").next().unwrap_or(rhs);
        let rhs = rhs.trim().trim_end_matches(',').trim();
        if let Ok(value) = rhs.parse::<f64>() {
            return Ok(value);
        }
    }
    Err(
        format!("gradient-border country field blocked: {GRAPHICS_DEFINES_REL} missing {key}")
            .into(),
    )
}

/// Measured ramp offset: alpha = clip(255 * (1 - (D - RAMP_D0) / T_texels)).
/// Fit against live_PAN2 channel1 bottom (d0 range 0.78-1.24 across blobs).
const RAMP_D0: f32 = 0.875;

/// Chamfer 5-7-11 distance scale (~2% euclidean error; alpha error <= ~2/255,
/// below the 2.5-9 mean residual of the captured-fit itself).
const CHAMFER_UNIT: u32 = 5;
const CHAMFER_DIAG: u32 = 7;
const CHAMFER_KNIGHT: u32 = 11;
/// Distances beyond the widest possible ramp encode alpha 0; cap propagation.
const CHAMFER_CAP: u32 = 24 * CHAMFER_UNIT;
/// Horizontal wrap padding: >= cap in texels.
const WRAP_PAD: usize = 24;

const FNV_OFFSET: u64 = 0xcbf29ce484222325;
const FNV_PRIME: u64 = 0x100000001b3;

#[derive(Clone, Debug)]
pub(crate) struct GbFieldRebuildStats {
    pub(crate) rebuilt: bool,
    pub(crate) elapsed: Duration,
}

/// Build the static parts of the gradient-border channels from VANILLA
/// bundle data (no capture-session dependency, no modded content):
///
/// - Channel1 top half (sea-zone/strategic-region layer) stays zero: the
///   layer is gated off by channel2's zero top half exactly like the game's
///   default political mapmode (`standardfuncsgfx.fxh:913-919`; the captured
///   default-view ch2 top has g=0 everywhere). When naval-supremacy or
///   strategic-region rendering is added, generate this half from
///   `map/strategicregions` using the measured encoding (constant navy
///   51,63,89; thickness GRADIENT_BORDERS_THICKNESS_STRATEGIC_REGIONS).
/// - Channel2 bottom half = the green land mask (0,255,0, alpha 0) that
///   gates the country layer on over land, generated from the half-res
///   state cells (state != 0 <=> land in the game's map data; matches the
///   measured captured mask). Top half (naval supremacy) stays zero: the
///   replay has no supremacy data, zero = no overlay, the game default.
fn vanilla_channel2_bgra(dimensions: GbDimensions, state_cells: &[u32]) -> Result<Vec<u8>> {
    let mut ch2 = vec![0u8; dimensions.texture_byte_len()?];
    let row_bytes = dimensions.width * 4;
    let bottom_start = dimensions.bottom_half_y * row_bytes;
    for (index, &state_id) in state_cells.iter().enumerate() {
        if state_id == 0 {
            continue;
        }
        // BGRA memory: green at byte offset 1.
        ch2[bottom_start + index * 4 + 1] = 255;
    }
    Ok(ch2)
}

pub(crate) struct GbField {
    width: usize,
    active_half_height: usize,
    bottom_half_y: usize,
    state_cells: Vec<u32>,
    state_cell_counts: Vec<usize>,
    country_field: CountryFieldDefines,
    color_modifiers: CountryColorModifiers,
    channel1_texture: ComPtr<ID3D11Texture2D>,
    channel1_srv: ComPtr<ID3D11ShaderResourceView>,
    _channel2_texture: ComPtr<ID3D11Texture2D>,
    channel2_srv: ComPtr<ID3D11ShaderResourceView>,
    last_signature: u64,
    last_rebuild: Option<Duration>,
}

impl GbField {
    /// Build the field. `resolver`, when present, supplies the country-field
    /// thickness defines from the live (mod-resolved) `00_graphics.lua` so a
    /// mod's widened/narrowed country borders are honoured; without a resolver
    /// (headless / no `--game`) the shipped vanilla defines are used, which is
    /// the same field the capture validated against.
    pub(crate) fn new(
        gfx: &Gfx,
        bundle: &MapBundle,
        resolver: Option<&GameFileResolver>,
    ) -> Result<Self> {
        let country_field = match resolver {
            Some(resolver) => CountryFieldDefines::from_resolver(resolver)?,
            None => CountryFieldDefines::vanilla(),
        };
        // COL3: the engine's map-fill H/S/V modifiers (see
        // CountryColorModifiers docs) — resolver-read like the field defines
        // so a mod's palette shift is honoured.
        let color_modifiers = match resolver {
            Some(resolver) => CountryColorModifiers::from_resolver(resolver)?,
            None => CountryColorModifiers::vanilla(),
        };
        let dimensions = GbDimensions::from_map(bundle.width, bundle.height)?;
        let state_cells = build_half_res_state_cells(
            bundle.width,
            bundle.height,
            &bundle.province_ids,
            &bundle.province_to_state,
        )?;
        let state_cell_counts = build_state_cell_counts(&state_cells);

        let expected_full = dimensions.texture_byte_len()?;
        // Channel1 starts all-zero: the top (sea-zone) half stays inert (see
        // vanilla_channel2_bgra docs) and the bottom (country) half is
        // generated on the first update().
        let ch1_init = vec![0u8; expected_full];
        let ch2_init = vanilla_channel2_bgra(dimensions, &state_cells)?;
        let (channel1_texture, channel1_srv) = unsafe {
            create_gb_texture(gfx, dimensions.width, dimensions.texture_height, &ch1_init)?
        };
        let (channel2_texture, channel2_srv) = unsafe {
            create_gb_texture(gfx, dimensions.width, dimensions.texture_height, &ch2_init)?
        };

        Ok(Self {
            width: dimensions.width,
            active_half_height: dimensions.active_half_height,
            bottom_half_y: dimensions.bottom_half_y,
            state_cells,
            state_cell_counts,
            country_field,
            color_modifiers,
            channel1_texture,
            channel1_srv,
            _channel2_texture: channel2_texture,
            channel2_srv,
            last_signature: 0,
            last_rebuild: None,
        })
    }

    pub(crate) fn channel1_srv(&self) -> *mut ID3D11ShaderResourceView {
        self.channel1_srv.as_raw()
    }

    pub(crate) fn channel2_srv(&self) -> *mut ID3D11ShaderResourceView {
        self.channel2_srv.as_raw()
    }

    pub(crate) fn update(
        &mut self,
        gfx: &Gfx,
        owners: &BTreeMap<u32, String>,
        country_colors: &BTreeMap<String, [u8; 3]>,
    ) -> Result<GbFieldRebuildStats> {
        let signature = ownership_signature(owners, country_colors);
        if signature == self.last_signature {
            return Ok(GbFieldRebuildStats {
                rebuilt: false,
                elapsed: Duration::ZERO,
            });
        }

        let start = Instant::now();
        let bottom = generate_country_layer_bgra(
            self.width,
            self.active_half_height,
            &self.state_cells,
            &self.state_cell_counts,
            owners,
            country_colors,
            self.country_field,
            self.color_modifiers,
        )?;
        unsafe {
            let dest_box = D3D11_BOX {
                left: 0,
                top: self.bottom_half_y as UINT,
                front: 0,
                right: self.width as UINT,
                bottom: (self.bottom_half_y + self.active_half_height) as UINT,
                back: 1,
            };
            (*gfx.context.as_raw()).UpdateSubresource(
                self.channel1_texture.as_raw() as *mut ID3D11Resource,
                0,
                &dest_box,
                bottom.as_ptr() as *const _,
                (self.width * 4) as UINT,
                (self.width * self.active_half_height * 4) as UINT,
            );
        }
        let elapsed = start.elapsed();
        self.last_signature = signature;
        self.last_rebuild = Some(elapsed);
        Ok(GbFieldRebuildStats {
            rebuilt: true,
            elapsed,
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct GbDimensions {
    pub(crate) width: usize,
    pub(crate) active_half_height: usize,
    pub(crate) texture_height: usize,
    pub(crate) bottom_half_y: usize,
}

impl GbDimensions {
    pub(crate) fn from_map(map_width: usize, map_height: usize) -> Result<Self> {
        if map_width == 0 || map_height == 0 || map_width % 2 != 0 || map_height % 2 != 0 {
            return Err(format!(
                "gradient-border field requires non-empty even map dimensions, got {map_width}x{map_height}"
            )
            .into());
        }
        let active_half_height = map_height / 2;
        Ok(Self {
            width: map_width / 2,
            active_half_height,
            texture_height: active_half_height * 2 + 2,
            bottom_half_y: active_half_height + 1,
        })
    }

    fn texture_byte_len(&self) -> Result<usize> {
        self.width
            .checked_mul(self.texture_height)
            .and_then(|pixels| pixels.checked_mul(4))
            .ok_or_else(|| "gradient-border texture byte length overflow".into())
    }
}

/// Generate the country layer (channel1 bottom half) as BGRA rows.
///
/// Encoding measured from the live capture: country cells get the owner
/// color — pushed through the engine's map-fill H/S/V modifier law
/// (`color_modifiers`, COL3; the raw file color is NEVER baked directly) —
/// with alpha = clip(255 * (1 - (D - RAMP_D0)/T)); cells with no owned
/// country (sea, lakes, unmapped) are black with alpha 255 ("outside", which
/// the shader's cutoff math renders as neutral - see module docs).
pub(crate) fn generate_country_layer_bgra(
    width: usize,
    height: usize,
    state_cells: &[u32],
    state_cell_counts: &[usize],
    owners: &BTreeMap<u32, String>,
    country_colors: &BTreeMap<String, [u8; 3]>,
    country_field: CountryFieldDefines,
    color_modifiers: CountryColorModifiers,
) -> Result<Vec<u8>> {
    let cell_count = width
        .checked_mul(height)
        .ok_or("gradient-border cell count overflow")?;
    if state_cells.len() != cell_count {
        return Err(format!(
            "gradient-border state cell plane has {} cells, expected {cell_count}",
            state_cells.len()
        )
        .into());
    }

    // Map states -> country regions (merged by tag) with color + area.
    let mut tag_ids = BTreeMap::<&str, u32>::new();
    let mut region_colors = vec![[0u8; 3]];
    let mut region_areas = vec![0usize];
    let mut state_region = vec![0u32; state_cell_counts.len()];
    for (state_id, tag) in owners {
        let state_index = *state_id as usize;
        let Some(&area) = state_cell_counts.get(state_index) else {
            continue;
        };
        if area == 0 {
            continue;
        }
        // IMPASSABLE states are NOT special-cased: in the engine's political
        // mapmode the country-colour plane ignores the `impassable` flag, so an
        // owned impassable (wasteland) state gets the SAME COL3 owner bake as
        // passable owned land. Verified byte-exact against the engine's captured
        // GradientBorderChannel1: 0/22 impassable states are gray, all are solid
        // owner colours. (The old code merged them all into a fixed 0x5c5c5c
        // "wasteland" region, which is what the user saw as "impassable not
        // rendered for the political colour".)
        let Some(color) = country_colors.get(tag) else {
            continue;
        };
        let region = if let Some(region) = tag_ids.get(tag.as_str()) {
            *region
        } else {
            let region = region_colors.len() as u32;
            tag_ids.insert(tag.as_str(), region);
            // COL3: bake the ENGINE's map-fill color, not the raw file color.
            region_colors.push(color_modifiers.bake(*color));
            region_areas.push(0);
            region
        };
        state_region[state_index] = region;
        region_areas[region as usize] += area;
    }

    let region_thickness: Vec<f32> = region_areas
        .iter()
        .map(|&area| country_thickness_texels_for_cell_area(area, country_field))
        .collect();

    let regions: Vec<u32> = state_cells
        .iter()
        .map(|&state_id| state_region.get(state_id as usize).copied().unwrap_or(0))
        .collect();

    let distances = chamfer_distance_to_different_region(width, height, &regions);

    let mut bgra = vec![0u8; cell_count * 4];
    for (index, &region) in regions.iter().enumerate() {
        let out = index * 4;
        if region == 0 {
            // Outside any owned country: neutral per the shader cutoff math.
            bgra[out + 3] = 255;
            continue;
        }
        let color = region_colors[region as usize];
        let thickness = region_thickness[region as usize];
        let distance = distances[index] as f32 / CHAMFER_UNIT as f32;
        let alpha = if thickness > 0.0 {
            (255.0 * (1.0 - (distance - RAMP_D0) / thickness)).clamp(0.0, 255.0)
        } else {
            0.0
        };
        bgra[out] = color[2];
        bgra[out + 1] = color[1];
        bgra[out + 2] = color[0];
        bgra[out + 3] = alpha.round() as u8;
    }
    Ok(bgra)
}

/// `00_graphics.lua:865-869` in full-res pixels; cells are half-res (x4 area).
/// The four thresholds come from `country_field` (resolver-read, mod-aware);
/// the interpolation and the half-res `/2` are unchanged from the
/// capture-validated vanilla field.
fn country_thickness_texels_for_cell_area(
    cell_area: usize,
    country_field: CountryFieldDefines,
) -> f32 {
    if cell_area == 0 {
        return 0.0;
    }
    let full_px_area = (cell_area * 4) as f32;
    let field_span = country_field.field_country_high - country_field.field_country_low;
    // `validate()` guarantees `field_country_high > field_country_low`, so the
    // span is strictly positive for a resolver-read or vanilla field; guard the
    // divide anyway so a degenerate hand-built value can never emit NaN.
    let t = if field_span > 0.0 {
        ((full_px_area - country_field.field_country_low) / field_span).clamp(0.0, 1.0)
    } else {
        0.0
    };
    (country_field.thickness_country_low
        + (country_field.thickness_country_high - country_field.thickness_country_low) * t)
        / 2.0
}

/// Per-texel chamfer 5-7-11 distance (in CHAMFER_UNIT fifths of a texel) to
/// the nearest texel of a DIFFERENT region, propagated only through the
/// texel's own region, with horizontal wrap via edge padding.
fn chamfer_distance_to_different_region(width: usize, height: usize, regions: &[u32]) -> Vec<u16> {
    let pad = WRAP_PAD.min(width);
    let padded_width = width + pad * 2;
    let mut region_padded = vec![0u32; padded_width * height];
    for y in 0..height {
        let row = &regions[y * width..(y + 1) * width];
        let out = &mut region_padded[y * padded_width..(y + 1) * padded_width];
        out[pad..pad + width].copy_from_slice(row);
        out[..pad].copy_from_slice(&row[width - pad..]);
        out[pad + width..].copy_from_slice(&row[..pad]);
    }

    let cap = CHAMFER_CAP as u16;
    let mut dist = vec![cap; padded_width * height];

    // Seed: texels bordering a different region. Map top/bottom edges are
    // NOT borders: captured land touching row 0 keeps interior alpha 0
    // (live_PAN2 ch1 bottom, e.g. columns 496/2357).
    for y in 0..height {
        for x in 0..padded_width {
            let index = y * padded_width + x;
            let region = region_padded[index];
            let mut best = cap;
            let mut check = |nx: isize, ny: isize, d: u16| {
                if nx < 0 || ny < 0 || nx >= padded_width as isize || ny >= height as isize {
                    return;
                }
                if region_padded[ny as usize * padded_width + nx as usize] != region {
                    best = best.min(d);
                }
            };
            let unit = CHAMFER_UNIT as u16;
            let diag = CHAMFER_DIAG as u16;
            let knight = CHAMFER_KNIGHT as u16;
            check(x as isize - 1, y as isize, unit);
            check(x as isize + 1, y as isize, unit);
            check(x as isize, y as isize - 1, unit);
            check(x as isize, y as isize + 1, unit);
            check(x as isize - 1, y as isize - 1, diag);
            check(x as isize + 1, y as isize - 1, diag);
            check(x as isize - 1, y as isize + 1, diag);
            check(x as isize + 1, y as isize + 1, diag);
            // Knight seeds: without these, a nearest different-region texel
            // at (2,1) would seed via 5+7=12 instead of 11.
            check(x as isize - 2, y as isize - 1, knight);
            check(x as isize + 2, y as isize - 1, knight);
            check(x as isize - 2, y as isize + 1, knight);
            check(x as isize + 2, y as isize + 1, knight);
            check(x as isize - 1, y as isize - 2, knight);
            check(x as isize + 1, y as isize - 2, knight);
            check(x as isize - 1, y as isize + 2, knight);
            check(x as isize + 1, y as isize + 2, knight);
            dist[index] = best;
        }
    }

    // Forward + backward chamfer passes, restricted to same-region texels.
    let w = padded_width as isize;
    let offsets_forward: [(isize, u16); 8] = [
        (-1, CHAMFER_UNIT as u16),
        (-w, CHAMFER_UNIT as u16),
        (-w - 1, CHAMFER_DIAG as u16),
        (-w + 1, CHAMFER_DIAG as u16),
        (-2 * w - 1, CHAMFER_KNIGHT as u16),
        (-2 * w + 1, CHAMFER_KNIGHT as u16),
        (-w - 2, CHAMFER_KNIGHT as u16),
        (-w + 2, CHAMFER_KNIGHT as u16),
    ];
    let offsets_backward: [(isize, u16); 8] = [
        (1, CHAMFER_UNIT as u16),
        (w, CHAMFER_UNIT as u16),
        (w + 1, CHAMFER_DIAG as u16),
        (w - 1, CHAMFER_DIAG as u16),
        (2 * w + 1, CHAMFER_KNIGHT as u16),
        (2 * w - 1, CHAMFER_KNIGHT as u16),
        (w + 2, CHAMFER_KNIGHT as u16),
        (w - 2, CHAMFER_KNIGHT as u16),
    ];

    let total = padded_width * height;
    for index in 0..total {
        let region = region_padded[index];
        let mut best = dist[index];
        for (offset, cost) in offsets_forward {
            let neighbor = index as isize + offset;
            if neighbor < 0 {
                continue;
            }
            let neighbor = neighbor as usize;
            // Row wrap-around across the padded edge is tolerated: padded
            // columns replicate the opposite map edge, so a cross-row read
            // only ever underestimates through real map content.
            if region_padded[neighbor] != region {
                continue;
            }
            let candidate = dist[neighbor].saturating_add(cost);
            if candidate < best {
                best = candidate;
            }
        }
        dist[index] = best.min(cap);
    }
    for index in (0..total).rev() {
        let region = region_padded[index];
        let mut best = dist[index];
        for (offset, cost) in offsets_backward {
            let neighbor = index as isize + offset;
            if neighbor >= total as isize {
                continue;
            }
            let neighbor = neighbor as usize;
            if region_padded[neighbor] != region {
                continue;
            }
            let candidate = dist[neighbor].saturating_add(cost);
            if candidate < best {
                best = candidate;
            }
        }
        dist[index] = best.min(cap);
    }

    let mut out = vec![0u16; width * height];
    for y in 0..height {
        let src = &dist[y * padded_width + pad..y * padded_width + pad + width];
        out[y * width..(y + 1) * width].copy_from_slice(src);
    }
    out
}

pub(crate) fn build_half_res_state_cells(
    map_width: usize,
    map_height: usize,
    province_ids: &[u32],
    province_to_state: &BTreeMap<u32, u32>,
) -> Result<Vec<u32>> {
    let dimensions = GbDimensions::from_map(map_width, map_height)?;
    let expected = map_width
        .checked_mul(map_height)
        .ok_or("province-id map dimensions overflow")?;
    if province_ids.len() != expected {
        return Err(format!(
            "province-id plane has {} pixels, expected {expected}",
            province_ids.len()
        )
        .into());
    }
    let mut out = Vec::with_capacity(dimensions.width * dimensions.active_half_height);
    for gy in 0..dimensions.active_half_height {
        for gx in 0..dimensions.width {
            out.push(majority_state_for_2x2(
                map_width,
                gx * 2,
                gy * 2,
                province_ids,
                province_to_state,
            ));
        }
    }
    Ok(out)
}

fn majority_state_for_2x2(
    map_width: usize,
    x: usize,
    y: usize,
    province_ids: &[u32],
    province_to_state: &BTreeMap<u32, u32>,
) -> u32 {
    let mut counts = [(0u32, 0u8); 4];
    let mut used = 0usize;
    for dy in 0..2 {
        for dx in 0..2 {
            let province = province_ids[(y + dy) * map_width + x + dx];
            let state = province_to_state.get(&province).copied().unwrap_or(0);
            if let Some((_, count)) = counts[..used]
                .iter_mut()
                .find(|(seen_state, _)| *seen_state == state)
            {
                *count += 1;
            } else {
                counts[used] = (state, 1);
                used += 1;
            }
        }
    }
    counts[..used]
        .iter()
        .copied()
        .max_by_key(|(state, count)| (*count, std::cmp::Reverse(*state)))
        .map(|(state, _)| state)
        .unwrap_or(0)
}

fn build_state_cell_counts(state_cells: &[u32]) -> Vec<usize> {
    let max_state = state_cells.iter().copied().max().unwrap_or(0) as usize;
    let mut counts = vec![0usize; max_state + 1];
    for state_id in state_cells.iter().copied() {
        if state_id != 0 {
            counts[state_id as usize] += 1;
        }
    }
    counts
}

fn ownership_signature(
    owners: &BTreeMap<u32, String>,
    country_colors: &BTreeMap<String, [u8; 3]>,
) -> u64 {
    let mut hash = FNV_OFFSET;
    for (state_id, tag) in owners {
        fnv_bytes(&mut hash, &state_id.to_le_bytes());
        fnv_bytes(&mut hash, tag.as_bytes());
        if let Some(color) = country_colors.get(tag) {
            fnv_bytes(&mut hash, color);
            fnv_bytes(&mut hash, &palette::encode_owner_rgba(*color).to_le_bytes());
        }
        fnv_bytes(&mut hash, &[0]);
    }
    hash
}

fn fnv_bytes(hash: &mut u64, bytes: &[u8]) {
    for byte in bytes {
        *hash ^= u64::from(*byte);
        *hash = hash.wrapping_mul(FNV_PRIME);
    }
}

unsafe fn create_gb_texture(
    gfx: &Gfx,
    width: usize,
    height: usize,
    bytes: &[u8],
) -> Result<(ComPtr<ID3D11Texture2D>, ComPtr<ID3D11ShaderResourceView>)> {
    let expected = width
        .checked_mul(height)
        .and_then(|pixels| pixels.checked_mul(4))
        .ok_or("gradient-border texture byte length overflow")?;
    if bytes.len() != expected {
        return Err(format!(
            "gradient-border texture init has {} bytes, expected {expected}",
            bytes.len()
        )
        .into());
    }
    let desc = D3D11_TEXTURE2D_DESC {
        Width: width as UINT,
        Height: height as UINT,
        MipLevels: 1,
        ArraySize: 1,
        Format: DXGI_FORMAT_B8G8R8A8_TYPELESS,
        SampleDesc: DXGI_SAMPLE_DESC {
            Count: 1,
            Quality: 0,
        },
        Usage: D3D11_USAGE_DEFAULT,
        BindFlags: D3D11_BIND_SHADER_RESOURCE,
        CPUAccessFlags: 0,
        MiscFlags: 0,
    };
    let init = D3D11_SUBRESOURCE_DATA {
        pSysMem: bytes.as_ptr() as *const _,
        SysMemPitch: (width * 4) as UINT,
        SysMemSlicePitch: 0,
    };
    let mut texture: *mut ID3D11Texture2D = ptr::null_mut();
    let hr = (*gfx.device.as_raw()).CreateTexture2D(&desc, &init, &mut texture);
    hr_ok(hr, "CreateTexture2D for GradientBorderChannel")?;
    let texture = ComPtr::from_raw(texture, "GradientBorderChannel texture")?;

    let mut srv_desc: D3D11_SHADER_RESOURCE_VIEW_DESC = std::mem::zeroed();
    srv_desc.Format = DXGI_FORMAT_B8G8R8A8_UNORM_SRGB;
    srv_desc.ViewDimension = D3D11_SRV_DIMENSION_TEXTURE2D;
    *srv_desc.u.Texture2D_mut() = D3D11_TEX2D_SRV {
        MostDetailedMip: 0,
        MipLevels: 1,
    };
    let mut srv: *mut ID3D11ShaderResourceView = ptr::null_mut();
    let hr = (*gfx.device.as_raw()).CreateShaderResourceView(
        texture.as_raw() as *mut _,
        &srv_desc,
        &mut srv,
    );
    hr_ok(hr, "CreateShaderResourceView for GradientBorderChannel")?;
    let srv = ComPtr::from_raw(srv, "GradientBorderChannel SRV")?;
    Ok((texture, srv))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn colors() -> BTreeMap<String, [u8; 3]> {
        BTreeMap::from([
            ("FRA".to_string(), [10, 20, 30]),
            ("GER".to_string(), [40, 50, 60]),
        ])
    }

    #[test]
    fn captured_stacking_arithmetic_is_two_active_halves_plus_guard_rows() {
        let dimensions = GbDimensions::from_map(5632, 2048).expect("valid the game dimensions");
        assert_eq!(dimensions.width, 2816);
        assert_eq!(dimensions.active_half_height, 1024);
        assert_eq!(dimensions.bottom_half_y, 1025);
        assert_eq!(dimensions.texture_height, 2050);
    }

    #[test]
    fn outside_owned_land_is_black_with_full_alpha() {
        // 4x2 cells: state 10 (FRA) on the left, sea (0) on the right.
        let state_cells = vec![10, 10, 0, 0, 10, 10, 0, 0];
        let mut counts = vec![0usize; 11];
        counts[10] = 4;
        let owners = BTreeMap::from([(10u32, "FRA".to_string())]);
        let bgra = generate_country_layer_bgra(
            4,
            2,
            &state_cells,
            &counts,
            &owners,
            &colors(),
            CountryFieldDefines::vanilla(),
            CountryColorModifiers::vanilla(),
        )
        .expect("generate");
        // Sea cell (index 2): black, alpha 255.
        assert_eq!(&bgra[8..12], &[0, 0, 0, 255]);
        // Country cell (index 0): BGRA of the ENGINE-BAKED fill color (COL3:
        // raw (10,20,30) through H+0.0/S*0.6/V*0.8 + truncation = (14,19,24)),
        // alpha near 255 (at the border, D=1 -> alpha ~= 255*(1-(1-0.875)/T)).
        assert_eq!(&bgra[0..3], &[24, 19, 14]);
        assert!(
            bgra[3] > 240,
            "border alpha should be near 255, got {}",
            bgra[3]
        );
    }

    #[test]
    fn interior_alpha_ramps_down_with_distance() {
        // Country strip x in [8, 56) with sea on both sides: alpha must
        // decrease monotonically from the border inward and reach 0 in the
        // interior (T for this small area is ~3.9 texels).
        let width = 64;
        let height = 8;
        let mut state_cells = vec![0u32; width * height];
        for y in 0..height {
            for x in 8..56 {
                state_cells[y * width + x] = 10;
            }
        }
        let mut counts = vec![0usize; 11];
        counts[10] = 48 * height;
        let owners = BTreeMap::from([(10u32, "FRA".to_string())]);
        let bgra = generate_country_layer_bgra(
            width,
            height,
            &state_cells,
            &counts,
            &owners,
            &colors(),
            CountryFieldDefines::vanilla(),
            CountryColorModifiers::vanilla(),
        )
        .expect("generate");
        let alpha_at = |x: usize| bgra[(4 * width + x) * 4 + 3];
        assert!(
            alpha_at(8) > alpha_at(11),
            "{} {}",
            alpha_at(8),
            alpha_at(11)
        );
        assert!(alpha_at(11) > alpha_at(14) || alpha_at(14) == 0);
        assert_eq!(alpha_at(31), 0, "deep interior must be alpha 0");
        // Map top/bottom edges are NOT borders: row 0 interior alpha equals
        // row 4 interior alpha (both 0).
        assert_eq!(bgra[(0 * width + 31) * 4 + 3], 0);
    }

    #[test]
    fn horizontal_wrap_sees_borders_across_the_seam() {
        // Country A occupies the two edge columns (wrapping region), B fills
        // the middle. The A cells at x=0 and x=width-1 are adjacent through
        // the wrap, and both border B.
        let width = 48;
        let height = 4;
        let mut state_cells = vec![0u32; width * height];
        for y in 0..height {
            for x in 0..width {
                let state = if x < 2 || x >= width - 2 { 10 } else { 20 };
                state_cells[y * width + x] = state;
            }
        }
        let mut counts = vec![0usize; 21];
        counts[10] = 4 * height;
        counts[20] = (width - 4) * height;
        let owners = BTreeMap::from([(10u32, "FRA".to_string()), (20u32, "GER".to_string())]);
        let bgra = generate_country_layer_bgra(
            width,
            height,
            &state_cells,
            &counts,
            &owners,
            &colors(),
            CountryFieldDefines::vanilla(),
            CountryColorModifiers::vanilla(),
        )
        .expect("generate");
        // x=0 (A) borders B at x=width-3 via wrap? No: x=0's wrap neighbor is
        // x=width-1 (A). Its real border is x=2 (B) at distance 2... check the
        // middle of B instead: B cell at x=width/2 is ~22 texels from a
        // border; with T<=12.5 its alpha must be 0.
        let mid = (1 * width + width / 2) * 4 + 3;
        assert_eq!(bgra[mid], 0);
        // B cell adjacent to A (x=2): near-255 alpha.
        let edge = (1 * width + 2) * 4 + 3;
        assert!(bgra[edge] > 240);
    }

    #[test]
    fn thickness_follows_graphics_defines_in_full_res_pixels() {
        let vanilla = CountryFieldDefines::vanilla();
        // Area >= 9000 full-res px -> 25/2 texels.
        assert!((country_thickness_texels_for_cell_area(2250, vanilla) - 12.5).abs() < 1e-6);
        // Tiny country -> 5/2 texels.
        assert!((country_thickness_texels_for_cell_area(75, vanilla) - 2.5).abs() < 1e-6);
        assert_eq!(country_thickness_texels_for_cell_area(0, vanilla), 0.0);
    }

    #[test]
    fn gate_engine_country_color_bake_law() {
        // COL3 ANSWER KEY: raw file color -> texel of the ENGINE-GENERATED
        // GradientBorderChannel1 country layer, byte-exact, from TWO captures
        // (live_PAN2 1936 new-game + session_POLCOLOR_1 1940+ ongoing save;
        // both agree). Law: trunc(255 * hsv_to_rgb(h + 0.0, s*0.6, v*0.8))
        // with the modifiers from common/defines/00_graphics.lua:1164-1166.
        let vanilla = CountryColorModifiers::vanilla();
        for (tag, raw, engine_baked) in [
            ("ENG", [201u8, 56, 93], [160u8, 91, 108]),
            ("GER", [102, 96, 87], [81, 78, 74]),
            ("SOV", [125, 13, 24], [100, 46, 51]),
            ("BRA", [76, 145, 63], [82, 116, 76]),
            ("SWE", [36, 132, 247], [96, 142, 197]),
            // NAME1 cosmetic RAJ_UK British-Raj red, present in the engine's
            // own 1936 bake — cosmetic colors go through the same law.
            ("RAJ", [170, 10, 10], [136, 59, 59]),
            // FRA (57,113,228) -> (100,127,182): also the color the engine
            // draws for France's IMPASSABLE Saharan states (514/515/782/786) in
            // the captured GradientBorderChannel1 — i.e. an owned impassable
            // state gets the SAME owner bake as passable land, NOT a wasteland
            // gray. ENG (160,91,108) and BRA (82,116,76) above are likewise the
            // engine colors for ENG/BRA impassable states. All 22 impassable
            // states in 1936 are owned and solid owner-colored (0/22 gray).
            ("FRA", [57, 113, 228], [100, 127, 182]),
        ] {
            assert_eq!(
                vanilla.bake(raw),
                engine_baked,
                "{tag}: raw {raw:?} must bake to the captured engine texel {engine_baked:?}"
            );
        }
        // Truncation, not rounding: ENG's round() result (161,91,109) is NOT
        // the engine texel. Guard the greyscale/no-saturation branch too.
        assert_eq!(vanilla.bake([255, 255, 255]), [204, 204, 204]);
        assert_eq!(vanilla.bake([0, 0, 0]), [0, 0, 0]);
        // Integer-boundary snap (Codex review): these raws produce a G channel
        // that is EXACTLY 112 in exact rational arithmetic but evaluates to
        // 111.999999999999986 in f64 — the epsilon must recover the true
        // integer, and must be narrow enough to never promote real fractions.
        assert_eq!(vanilla.bake([200, 100, 31])[1], 112);
        assert_eq!(vanilla.bake([0, 80, 230])[1], 112);
    }

    #[test]
    fn country_color_modifiers_parse_from_graphics_body() {
        let body = "NDefines = {\n  NGraphics = {\n    \
             COUNTRY_COLOR_HUE_MODIFIER = 0.0,\n    \
             COUNTRY_COLOR_SATURATION_MODIFIER = 0.6,\n    \
             COUNTRY_COLOR_BRIGHTNESS_MODIFIER = 0.8,\n  }\n}\n";
        let parsed = CountryColorModifiers::from_graphics_defines(body).expect("parse modifiers");
        assert_eq!(parsed, CountryColorModifiers::vanilla());
        // A modded palette shift is honoured (values change the bake).
        let modded = body.replace("0.6", "1.0").replace("0.8", "1.0");
        let modded = CountryColorModifiers::from_graphics_defines(&modded).expect("parse modded");
        assert_eq!(modded.bake([201, 56, 93]), [201, 56, 93]);
        // Missing key fails closed.
        assert!(CountryColorModifiers::from_graphics_defines("NGraphics = {}").is_err());
    }

    #[test]
    fn political_scene_defines_parse_and_vanilla_values() {
        let body = "NDefines = {\n  NGraphics = {\n    \
             DRAW_SHADOWS_CUTOFF = 400,\n    \
             DRAW_SHADOWS_FADE_LENGTH = 50,\n    \
             GRADIENT_BORDERS_OUTLINE_CUTOFF_COUNTRY = 0.973, -- Magic number\n    \
             GRADIENT_BORDERS_OUTLINE_CUTOFF_STRATEGIC_REGIONS = 0.98,\n  }\n}\n";
        let parsed = PoliticalSceneDefines::from_graphics_defines(body).expect("parse defines");
        assert_eq!(parsed, PoliticalSceneDefines::vanilla());
        // The shadow-fade law through the map_viewer CB0 carrier: exact
        // engine samples from the BORDERZOOM_1 sweep (max err 0.000000 over
        // 1433 samples) — h=387.386566 -> 0.252269 (session_POLCOLOR_1 CB2).
        // Product path: no shadow pass replayed -> factor gated to 0.
        let cb0 = parsed.to_cb0();
        assert!(!cb0.shadow_map_bound);
        assert_eq!(cb0.shadow_fade_factor(387.386_57), 0.0);
        // The LAW itself (engine samples): exact once a shadow map is bound.
        let cb0 = map_viewer::PoliticalSceneCb0 {
            shadow_map_bound: true,
            ..cb0
        };
        assert!((cb0.shadow_fade_factor(387.386_57) - 0.252_269).abs() < 1e-4);
        assert_eq!(cb0.shadow_fade_factor(50.0), 1.0);
        assert_eq!(cb0.shadow_fade_factor(3000.0), 0.0);
    }

    #[test]
    fn country_field_defines_parse_from_graphics_body() {
        // A minimal 00_graphics.lua-shaped body with the four country defines
        // (plus a comment and trailing commas like the shipped file).
        let body = "NDefines = {\n  NGraphics = {\n    \
             GRADIENT_BORDERS_FIELD_COUNTRY_LOW = 300.0, -- country area in sum of pixels\n    \
             GRADIENT_BORDERS_FIELD_COUNTRY_HIGH = 9000.0, -- ... the value is squared\n    \
             GRADIENT_BORDERS_THICKNESS_COUNTRY_LOW = 5.0,\n    \
             GRADIENT_BORDERS_COUNTRY_CENTER_THICKNESS = 2.0,\n    \
             GRADIENT_BORDERS_THICKNESS_COUNTRY_HIGH = 25.0,\n  }\n}\n";
        let defines = CountryFieldDefines::from_graphics_defines(body).expect("parse defines");
        assert_eq!(defines, CountryFieldDefines::vanilla());
        // A mod that doubles the max country thickness is honoured verbatim.
        let modded = body.replace(
            "GRADIENT_BORDERS_THICKNESS_COUNTRY_HIGH = 25.0",
            "GRADIENT_BORDERS_THICKNESS_COUNTRY_HIGH = 50.0",
        );
        let modded = CountryFieldDefines::from_graphics_defines(&modded).expect("parse modded");
        assert_eq!(modded.thickness_country_high, 50.0);
        // The modded field widens the ramp: a large country's thickness scales
        // from 12.5 to 25 texels, so modded > vanilla for the same area.
        assert!(
            country_thickness_texels_for_cell_area(2250, modded)
                > country_thickness_texels_for_cell_area(2250, CountryFieldDefines::vanilla())
        );
    }

    #[test]
    fn country_field_defines_fail_closed_on_missing_or_inverted() {
        // Missing a key fails closed.
        let missing = "GRADIENT_BORDERS_FIELD_COUNTRY_LOW = 300.0,\n\
             GRADIENT_BORDERS_FIELD_COUNTRY_HIGH = 9000.0,\n\
             GRADIENT_BORDERS_THICKNESS_COUNTRY_LOW = 5.0,\n";
        assert!(CountryFieldDefines::from_graphics_defines(missing).is_err());
        // Inverted field range fails closed.
        let inverted = "GRADIENT_BORDERS_FIELD_COUNTRY_LOW = 9000.0,\n\
             GRADIENT_BORDERS_FIELD_COUNTRY_HIGH = 300.0,\n\
             GRADIENT_BORDERS_THICKNESS_COUNTRY_LOW = 5.0,\n\
             GRADIENT_BORDERS_COUNTRY_CENTER_THICKNESS = 2.0,\n\
             GRADIENT_BORDERS_THICKNESS_COUNTRY_HIGH = 25.0,\n";
        let err = CountryFieldDefines::from_graphics_defines(inverted)
            .expect_err("inverted field range must fail closed");
        assert!(err.to_string().contains("must exceed"), "unexpected: {err}");
    }

    #[test]
    fn modded_center_thickness_fails_closed_because_ramp_offset_is_vanilla_only() {
        // A mod that changes CENTER_THICKNESS invalidates the measured RAMP_D0
        // fit, so the field synthesis must refuse rather than render a wrong
        // ramp (tracked gap: deriving RAMP_D0 from a modded center).
        let modded_center = "GRADIENT_BORDERS_FIELD_COUNTRY_LOW = 300.0,\n\
             GRADIENT_BORDERS_FIELD_COUNTRY_HIGH = 9000.0,\n\
             GRADIENT_BORDERS_THICKNESS_COUNTRY_LOW = 5.0,\n\
             GRADIENT_BORDERS_COUNTRY_CENTER_THICKNESS = 4.0,\n\
             GRADIENT_BORDERS_THICKNESS_COUNTRY_HIGH = 25.0,\n";
        let err = CountryFieldDefines::from_graphics_defines(modded_center)
            .expect_err("modded center thickness must fail closed");
        assert!(
            err.to_string().contains("CENTER_THICKNESS"),
            "unexpected: {err}"
        );
        // A non-finite center thickness must also fail closed: `(NaN-2).abs()`
        // is false, so the explicit is_finite() guard is what catches it.
        let nan_center = modded_center.replace(
            "GRADIENT_BORDERS_COUNTRY_CENTER_THICKNESS = 4.0",
            "GRADIENT_BORDERS_COUNTRY_CENTER_THICKNESS = nan",
        );
        assert!(CountryFieldDefines::from_graphics_defines(&nan_center).is_err());
    }

    /// Flag-off / vanilla byte-identity: the SHIPPED `00_graphics.lua`, read
    /// through the mod-aware resolver, must parse to exactly the vanilla
    /// fallback defines — so the resolver-read country layer is byte-identical
    /// to the previous hardcoded-constant path on a vanilla install. Env-gated
    /// on `BATTLE_PLAN_VANILLA` like the other real-game-file gates; a game patch that
    /// re-tunes these defines fails this loudly.
    #[test]
    fn shipped_graphics_defines_match_vanilla_country_field() {
        let Ok(game) = std::env::var("BATTLE_PLAN_VANILLA").or_else(|_| std::env::var("HOI4_VANILLA")) else {
            eprintln!("skipping shipped_graphics_defines test: BATTLE_PLAN_VANILLA unset");
            return;
        };
        let resolver = GameFileResolver::vanilla(game).expect("vanilla resolver");
        let defines = CountryFieldDefines::from_resolver(&resolver)
            .expect("read shipped GRADIENT_BORDERS country defines");
        assert_eq!(
            defines,
            CountryFieldDefines::vanilla(),
            "shipped 00_graphics.lua country-field defines drifted from the vanilla fallback; \
             the resolver-read country layer would no longer be byte-identical"
        );
    }

    #[test]
    fn ownership_signature_changes_when_owner_changes() {
        let owners = BTreeMap::from([(10u32, "FRA".to_string())]);
        let before = ownership_signature(&owners, &colors());
        let mut changed = owners.clone();
        changed.insert(10, "GER".to_string());
        let after = ownership_signature(&changed, &colors());
        assert_ne!(before, after);
    }

    #[test]
    fn majority_state_for_half_res_cell_uses_2x2_source_pixels() {
        let provinces = vec![1, 1, 2, 3];
        let mapping = BTreeMap::from([(1, 10), (2, 20), (3, 30)]);
        assert_eq!(majority_state_for_2x2(2, 0, 0, &provinces, &mapping), 10);
    }

    #[test]
    fn vanilla_channel2_marks_land_green_in_bottom_half_only() {
        let dimensions = GbDimensions::from_map(8, 4).expect("dims");
        // 4x2 half-res cells: two land cells, two sea.
        let state_cells = vec![10, 0, 20, 0, 0, 10, 0, 0];
        let ch2 = vanilla_channel2_bgra(dimensions, &state_cells).expect("ch2");
        assert_eq!(ch2.len(), dimensions.texture_byte_len().unwrap());
        let row_bytes = dimensions.width * 4;
        // Top half + first guard row stay zero (no naval supremacy data).
        assert!(ch2[..dimensions.bottom_half_y * row_bytes]
            .iter()
            .all(|&byte| byte == 0));
        let bottom = dimensions.bottom_half_y * row_bytes;
        // Land cells get green (BGRA offset 1), sea cells stay zero.
        assert_eq!(ch2[bottom + 1], 255);
        assert_eq!(ch2[bottom + 4 + 1], 0);
        assert_eq!(ch2[bottom + 8 + 1], 255);
        // Alpha stays zero everywhere (matches the measured captured mask).
        assert!(ch2[bottom..].chunks_exact(4).all(|texel| texel[3] == 0));
    }
}
