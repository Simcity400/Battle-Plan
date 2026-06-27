//! `terrain` — the MAP VISUAL FIDELITY base layers (Codex-reviewed 2026-06-26, v2).
//!
//! The political/mode LUT (see [`crate::map`]) decides WHICH colour a province is
//! tinted; this module builds WHAT that tint sits on top of: the game's REAL terrain
//! colourmap, relief-shaded, with subtle ground-texture detail, rivers and game water —
//! exactly the layers the in-game map draws under its translucent country tint.
//!
//! All of this is purely VISUAL. The political/mode DATA (province → owner → colour) is
//! untouched; fidelity changes only how the map is DRAWN. None of these layers can
//! invent ownership: they are derived solely from the read-only game map assets
//! (`colormap_rgb_cityemissivemask_a.dds`, `atlas0.dds`, `terrain.bmp`,
//! `world_normal.bmp`, `rivers.bmp`) and the terrain definitions
//! (`common/terrain/00_terrain.txt`).
//!
//! ## Layers (v2 — REAL colourmap base)
//!
//!   * **Terrain base = the game COLOURMAP** — `colormap_rgb_cityemissivemask_a.dds` is
//!     a BGRA8 texture whose RGB channels ARE the large-scale land colourmap (the actual
//!     terrain colours the game draws; alpha is a city-emissive mask we IGNORE — Codex A:
//!     never composite via that alpha, force the base opaque). It is 2816×1024 and spans
//!     the same world extent as the 5632×2048 province raster (a clean 2:1), so we sample
//!     it by normalised, pixel-CENTRE map UV (bilinear, clamp) onto the base. This
//!     REPLACES the old flat per-terrain-type palette (which read as blobs).
//!   * **Relief (hillshade)** — `world_normal.bmp` is a (half-resolution) tangent-space
//!     normal map. We decode each texel to a unit normal in [-1,1], dot it with a fixed
//!     light direction, and multiply the resulting shade onto the colourmap base so the
//!     land gains depth. Normal maps are LINEAR data; we decode/normalise in float.
//!   * **Ground-texture detail** — `atlas0.dds` (2048×2048 DXT5) is a 4×4 grid of 16
//!     close-range material tiles (512×512 each), indexed 0..=15 by the `texture = N`
//!     field of each terrain entry in `00_terrain.txt`. We composite the per-terrain-type
//!     tile, TILED, as a SUBTLE, luminance-only, MEAN-PRESERVING modulation of the
//!     colourmap base (Codex C: a low-blend multiply around each tile's own mean, so it
//!     adds material grain without shifting colour or fighting the macro colourmap / no
//!     moire at full-world zoom). This is a faithful approximation of the in-game
//!     close-zoom atlas blend (which the engine does per-zoom in a shader); see the
//!     `ATLAS_DETAIL_BLEND` constant for the fidelity dial.
//!   * **Water** — terrain type `ocean`/`lakes` use the game water colour (a
//!     `colormap_water`-like blue), NOT the colourmap's flat sea-fill value, so the sea
//!     reads as clean game water. Province metadata still wins over terrain at the
//!     shader (a known land province stays land even if the terrain index is ocean slop).
//!   * **Rivers** — `rivers.bmp` is 8-bit indexed; indices 3..=11 are river pixels. We
//!     bake them as a separate MASK so the shader draws them as blue lines over the
//!     composited terrain.
//!
//! The terrain base + river mask are produced at FULL raster resolution (5632×2048),
//! matching the province-id raster 1:1 so the shader samples all visual layers and the
//! integer id texture in the same coordinate space (no cross-space border artifacts).
//! All source rasters (colourmap, terrain.bmp, world_normal, rivers) are decoded to the
//! SAME top-left row convention (Codex B), so registration is consistent.

use std::collections::HashMap;
use std::path::Path;

/// Exact named constants extracted from the GAME's shader/define files (Codex-verified).
/// Every magic number used by the faithful terrain math lives here, cited to its source —
/// no eyeballed values. See `pdxmap.shader`, `standardfuncsgfx.fxh`, `constants.fxh`,
/// `common/defines/00_graphics.lua`, and the mod's `msb_defines.lua`.
pub mod gc {
    // --- color space (standardfuncsgfx.fxh:38-56) ---
    /// ToGamma exponent: pow(linear, GAMMA_ENCODE). [standardfuncsgfx.fxh:40]
    pub const GAMMA_ENCODE: f32 = 0.45;
    /// ToLinear exponent: pow(gamma, GAMMA_DECODE). [standardfuncsgfx.fxh:50]
    pub const GAMMA_DECODE: f32 = 2.2;

    // --- colormap / atlas overlay (constants.fxh) ---
    /// Strength of the colormap OVERLAY over the atlas detail (GetOverlay last arg).
    /// [pdxmap.shader:317; constants.fxh:93]
    pub const COLORMAP_OVERLAY_STRENGTH: f32 = 0.75;
    /// Mud overlay strength (GetMudColor). [constants.fxh:94]
    pub const COLORMAP_MUD_OVERLAY_STRENGTH: f32 = 0.5;
    /// Atlas tile repeat frequency across the map. [constants.fxh:53]
    pub const TERRAIN_TILE_FREQ: f32 = 128.0;
    /// Atlas grid is MAP_NUM_TILES × MAP_NUM_TILES (=4×4=16 tiles). [constants.fxh:54]
    pub const MAP_NUM_TILES: f32 = 4.0;

    // --- political mode blend (defines) ---
    /// How much TERRAIN shows through the political country colour. MOD OVERRIDE:
    /// vanilla=0.5 [00_graphics.lua:64], mod=0.8 [msb_defines.lua:152] → MOD WINS.
    /// Direction (Codex-verified): final = lerp(country_color, terrain, THIS), i.e. at
    /// 0.8 the terrain is 80% visible and the country colour only 20%.
    pub const MAP_MODE_TERRAIN_TRANSPARENCY: f32 = 0.8;

    // --- water (constants.fxh) ---
    /// Water colormap is multiplied by this. [constants.fxh:90; pdxwater/underwater]
    pub const WATER_COLOR_LIGHTNESS: f32 = 0.5;
    /// Ice colour (linear). [constants.fxh:87]
    pub const ICE_COLOR: [f32; 3] = [0.5, 0.6, 0.9];
    /// The deliberate water "Darkness" offset: lerp(water, black, THIS). [pdxwater.shader:361]
    pub const WATER_DARKNESS: f32 = 0.2;

    // --- city lights (constants.fxh) ---
    /// City-lights tiling for the noise. [constants.fxh:49]
    pub const CITY_LIGHTS_TILING: f32 = 0.09103;
    /// City-lights additive intensity at night. [constants.fxh:50]
    pub const CITY_LIGHTS_INTENSITY: f32 = 5.5;

    // --- distance fog (constants.fxh) ---
    /// Distance-fog colour (linear). [constants.fxh:152]
    pub const FOG_COLOR: [f32; 3] = [0.12, 0.28, 0.6];
    /// Fog max factor. [constants.fxh:155]
    pub const FOG_MAX: f32 = 0.35;

    // --- lighting (00_graphics.lua + standardfuncsgfx.fxh) ---
    /// Sun diffuse colour, stored HSV in the define. [00_graphics.lua:1060]
    pub const SUN_DIFFUSE_COLOR_HSV: [f32; 3] = [0.14, 0.0, 1.0];
    /// Sun intensity (SunDiffuseIntensity.a). [00_graphics.lua:1061]
    pub const SUN_INTENSITY: f32 = 1.0;
    /// The TERRAIN/MAP ambient cube — the BAKED `DayAmbientMap*` shader constants
    /// (`ComposeLight` → `AmbientLight(normal, dayFactor)` → the 2-arg overload at
    /// standardfuncsgfx.fxh:475 which reads these, NOT the Lua `AMBIENT_LIGHT_*` HSV
    /// values; those fill the unit-mesh cube used by `ComposeLightMesh`). Already LINEAR.
    /// Order: +X, -X, +Y, -Y, +Z, -Z. [constants.fxh:17-22] (Codex-corrected).
    pub const AMBIENT_DAY: [[f32; 3]; 6] = [
        [0.1, 0.1, 0.05],          // DayAmbientMapPosX (right) [17]
        [0.15, 0.15, 0.15],        // DayAmbientMapNegX (left)  [18]
        [0.03, 0.03, 0.06],        // DayAmbientMapPosY          [19]
        [0.0, 0.0, 0.0],           // DayAmbientMapNegY (under)  [20]
        [0.0502, 0.05023, 0.1023], // DayAmbientMapPosZ (top)  [21]
        [0.03, 0.033, 0.033],      // DayAmbientMapNegZ (bottom) [22]
    ];
    /// HDR range multiplier (LIGHT_HDR_RANGE). [00_graphics.lua:772]
    pub const HDR_RANGE: f32 = 1.0;
    /// Light direction (LIGHT_DIRECTION_*), a world-space sun direction. [00_graphics.lua:766-768]
    pub const LIGHT_DIRECTION: [f32; 3] = [-1.0, -1.0, 0.5];

    // --- snow / season (constants.fxh + seasons.txt) ---
    /// Snow colour (linear). [constants.fxh:78]
    pub const SNOW_COLOR: [f32; 3] = [0.46, 0.48, 0.69];
    /// Snow appears above this world height. [constants.fxh, SNOW_START_HEIGHT]
    pub const SNOW_START_HEIGHT: f32 = 8.0;
}

/// FALLBACK river line colour, used only if the RiverSurface diffuse asset can't be read.
/// The faithful river colour comes from `RiverSurface_diffuse_0.dds` (see [`RiverSurface`]).
pub const RIVER_COLOR: [u8; 3] = [70, 110, 190];
/// FALLBACK sea colour (only used for the data-LUT "neutral / not-land" path and as a
/// last-resort water colour if `colormap_water_0.dds` is unreadable). The faithful sea
/// colour comes from `colormap_water_0.dds × WATER_COLOR_LIGHTNESS` (see [`WaterColorMap`]).
pub const WATER_COLOR: [u8; 3] = [40, 70, 120];
/// FALLBACK lake colour (last resort only).
pub const LAKE_COLOR: [u8; 3] = [58, 91, 160];

// ---------------------------------------------------------------------------
// Faithful color-space + blend helpers (exact game math, linear working space).
// ---------------------------------------------------------------------------

/// Decode an 8-bit sRGB channel to LINEAR, the way the game's sRGB texture views do
/// before the shader sees the colormap/atlas. (The game uses a plain gamma 2.2 curve, not
/// the sRGB piecewise curve — see ToLinear=pow(x,2.2) [standardfuncsgfx.fxh:50].)
#[inline]
pub fn srgb_to_linear(c8: u8) -> f32 {
    (c8 as f32 / 255.0).powf(gc::GAMMA_DECODE)
}
/// Encode a LINEAR channel back to 8-bit sRGB (gamma 0.45 then ×255), the way the game's
/// sRGB framebuffer encodes the final linear colour. [standardfuncsgfx.fxh:40 ToGamma]
#[inline]
pub fn linear_to_srgb8(l: f32) -> u8 {
    (l.clamp(0.0, 1.0).powf(gc::GAMMA_ENCODE) * 255.0)
        .round()
        .clamp(0.0, 255.0) as u8
}

/// The game's `GetOverlay(base, overlay, pct)` — a Photoshop "overlay" blend done in GAMMA
/// space, then lerp(base, result, pct). Inputs/outputs are LINEAR (gamma applied internally).
/// Exact translation of [standardfuncsgfx.fxh:159-169].
#[inline]
pub fn get_overlay(base: [f32; 3], overlay: [f32; 3], pct: f32) -> [f32; 3] {
    let to_gamma = |x: f32| x.max(0.0).powf(gc::GAMMA_ENCODE);
    let to_linear = |x: f32| x.max(0.0).powf(gc::GAMMA_DECODE);
    let mut out = [0.0f32; 3];
    for i in 0..3 {
        let cg = to_gamma(base[i]);
        let og = to_gamma(overlay[i]);
        let res = if og < 0.5 {
            2.0 * og * cg
        } else {
            1.0 - 2.0 * (1.0 - og) * (1.0 - cg)
        };
        let res = to_linear(res);
        out[i] = base[i] + (res - base[i]) * pct; // lerp(base, res, pct)
    }
    out
}

/// Convert an HSV triple (the form the game's lighting defines are stored in) to LINEAR
/// RGB. The game's `HSVtoRGB` builds an RGB then `ToLinear`s it [standardfuncsgfx.fxh:145-151];
/// here the result is already linear (we feed it straight into linear lighting math).
pub fn hsv_to_linear_rgb(hsv: [f32; 3]) -> [f32; 3] {
    // game Hue(): R=|H*6-3|-1, G=2-|H*6-2|, B=2-|H*6-4|, saturated.
    let h = hsv[0];
    let hue = [
        (((h * 6.0 - 3.0).abs()) - 1.0).clamp(0.0, 1.0),
        (2.0 - (h * 6.0 - 2.0).abs()).clamp(0.0, 1.0),
        (2.0 - (h * 6.0 - 4.0).abs()).clamp(0.0, 1.0),
    ];
    let (s, v) = (hsv[1], hsv[2]);
    // val = (hue-1)*S + 1; val *= V; then ToLinear(val).
    let mut out = [0.0f32; 3];
    for i in 0..3 {
        let val = ((hue[i] - 1.0) * s + 1.0) * v;
        out[i] = val.max(0.0).powf(gc::GAMMA_DECODE);
    }
    out
}

/// A logical terrain type (the `type =` of a `00_terrain.txt` terrain entry). The
/// `categories` block gives each a display colour; `is_water` marks sea/lake types.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TerrainType {
    Plains,
    Forest,
    Hills,
    Mountain,
    Desert,
    Marsh,
    Urban,
    Jungle,
    Lakes,
    Ocean,
    Unknown,
}

impl TerrainType {
    fn from_str(s: &str) -> TerrainType {
        match s {
            "plains" => TerrainType::Plains,
            "forest" => TerrainType::Forest,
            "hills" => TerrainType::Hills,
            "mountain" => TerrainType::Mountain,
            "desert" => TerrainType::Desert,
            "marsh" => TerrainType::Marsh,
            "urban" => TerrainType::Urban,
            "jungle" => TerrainType::Jungle,
            "lakes" => TerrainType::Lakes,
            "ocean" => TerrainType::Ocean,
            _ => TerrainType::Unknown,
        }
    }
    pub fn is_water(self) -> bool {
        matches!(self, TerrainType::Ocean | TerrainType::Lakes)
    }
}

/// The terrain definitions parsed from `common/terrain/00_terrain.txt`:
///   * `category_color[type]` — the display RGB for each terrain TYPE (from the
///     `categories` block), and
///   * `index_type[palette_index]` — the terrain TYPE for each `terrain.bmp` palette
///     index (from the `terrain = { … }` block; `color = { N }` is the palette index).
///
/// Together they collapse to [`TerrainDefs::palette`]: a 256-entry palette index → RGB
/// land colour, plus an `is_water` flag per index.
#[derive(Debug, Clone)]
pub struct TerrainDefs {
    category_color: HashMap<TerrainType, [u8; 3]>,
    /// palette index (0..=255) -> terrain type. Gaps default to Plains.
    index_type: [TerrainType; 256],
    /// palette index (0..=255) -> atlas tile index (the `texture = N` field; 0..=15 for a
    /// real `atlas0.dds` tile, [`NO_ATLAS`] for "no detail tile" e.g. lakes' 255).
    index_atlas: [u8; 256],
    /// palette index (0..=255) -> `perm_snow = yes` (permanent snow terrain, e.g. the
    /// polar / high-mountain caps the game always draws white). [00_terrain.txt]
    index_perm_snow: [bool; 256],
}

/// Sentinel `index_atlas` value meaning "no atlas detail tile for this terrain index"
/// (e.g. the `texture = 255` lakes entry). Detail is skipped for these pixels.
pub const NO_ATLAS: u8 = 255;
/// The atlas is a 4×4 grid of tiles; only indices `0..ATLAS_TILES` are valid.
pub const ATLAS_TILES: u8 = 16;

impl TerrainDefs {
    /// Parse the (Clausewitz-script) terrain file. We do a lightweight block scan: find
    /// the `categories = { … }` block to read each type's `color`, and the
    /// `terrain = { … }` block to read each `type =` / `color = { index }` pair.
    pub fn load(path: &Path) -> Result<TerrainDefs, String> {
        let txt = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
        Ok(Self::parse(&txt))
    }

    pub fn parse(txt: &str) -> TerrainDefs {
        let mut category_color: HashMap<TerrainType, [u8; 3]> = HashMap::new();
        let mut index_type = [TerrainType::Plains; 256];
        let mut index_atlas = [NO_ATLAS; 256];
        let mut index_perm_snow = [false; 256];

        // --- categories block: `name = { … color = { r g b } … }` ---------------
        if let Some(cat) = extract_block(txt, "categories") {
            for (name, body) in iter_named_blocks(cat) {
                let ty = TerrainType::from_str(name);
                if ty == TerrainType::Unknown && name != "unknown" {
                    continue;
                }
                if let Some(rgb) = parse_color_triplet(body) {
                    category_color.insert(ty, rgb);
                }
            }
        }

        // --- terrain block: `entry = { type = X color = { IDX } texture = N … }` --
        // `color = { IDX }` is a SINGLE number = the terrain.bmp palette index; the
        // `texture = N` field is the atlas0.dds tile index for that terrain (ground
        // detail). 255 = no atlas tile (lakes).
        if let Some(terr) = extract_block(txt, "terrain") {
            for (_name, body) in iter_named_blocks(terr) {
                let ty = body
                    .split("type")
                    .nth(1)
                    .and_then(|s| {
                        s.trim_start_matches([' ', '\t', '='])
                            .split_whitespace()
                            .next()
                    })
                    .map(TerrainType::from_str);
                let idx = parse_color_single_index(body);
                let atlas = parse_scalar(body, "texture");
                let perm_snow = body.contains("perm_snow")
                    && body
                        .split("perm_snow")
                        .nth(1)
                        .map(|s| s.trim_start_matches([' ', '\t', '=']).starts_with("yes"))
                        .unwrap_or(false);
                if let (Some(ty), Some(idx)) = (ty, idx) {
                    if (idx as usize) < 256 {
                        index_type[idx as usize] = ty;
                        // Only 0..=15 are real atlas tiles; anything else (e.g. 255) ->
                        // NO_ATLAS so the detail layer is skipped for that index.
                        index_atlas[idx as usize] = match atlas {
                            Some(n) if n < ATLAS_TILES as i64 && n >= 0 => n as u8,
                            _ => NO_ATLAS,
                        };
                        index_perm_snow[idx as usize] = perm_snow;
                    }
                }
            }
        }

        TerrainDefs {
            category_color,
            index_type,
            index_atlas,
            index_perm_snow,
        }
    }

    /// Display colour for a terrain type (from the categories block), with sensible
    /// fallbacks if a category was missing.
    pub fn type_color(&self, ty: TerrainType) -> [u8; 3] {
        if let Some(c) = self.category_color.get(&ty) {
            return *c;
        }
        // Fallbacks (only used if the file lacked a category — keeps the underlay sane).
        match ty {
            TerrainType::Plains => [180, 168, 120],
            TerrainType::Forest => [86, 130, 70],
            TerrainType::Hills => [170, 158, 110],
            TerrainType::Mountain => [150, 150, 150],
            TerrainType::Desert => [210, 190, 130],
            TerrainType::Marsh => [110, 130, 95],
            TerrainType::Urban => [120, 120, 120],
            TerrainType::Jungle => [70, 120, 60],
            TerrainType::Lakes => LAKE_COLOR,
            TerrainType::Ocean => WATER_COLOR,
            TerrainType::Unknown => [140, 130, 110],
        }
    }

    /// The terrain TYPE for a `terrain.bmp` palette index.
    pub fn index_type(&self, idx: u8) -> TerrainType {
        self.index_type[idx as usize]
    }

    /// The atlas0.dds tile index (`texture = N`) for a `terrain.bmp` palette index, or
    /// [`NO_ATLAS`] when that terrain has no detail tile (e.g. lakes).
    pub fn index_atlas(&self, idx: u8) -> u8 {
        self.index_atlas[idx as usize]
    }

    /// Whether a `terrain.bmp` palette index is `perm_snow = yes` (permanent snow cap).
    pub fn index_perm_snow(&self, idx: u8) -> bool {
        self.index_perm_snow[idx as usize]
    }

    /// Build the flat 256-entry palette: palette index → land RGB. Water types map to
    /// the water/lake colour. Used to colour `terrain.bmp` directly.
    pub fn palette(&self) -> [[u8; 3]; 256] {
        let mut p = [[0u8; 3]; 256];
        for i in 0..256 {
            let ty = self.index_type[i];
            p[i] = match ty {
                TerrainType::Ocean => WATER_COLOR,
                TerrainType::Lakes => LAKE_COLOR,
                _ => self.type_color(ty),
            };
        }
        p
    }
}

/// A decoded 8-bit INDEXED bitmap: per-pixel palette index, top-left origin (rows
/// already de-flipped from the BMP's bottom-up storage).
pub struct IndexedBmp {
    pub width: u32,
    pub height: u32,
    /// One palette index per pixel, row-major, top-left origin.
    pub indices: Vec<u8>,
}

/// A decoded 24-bit RGB bitmap, top-left origin.
pub struct RgbBmp {
    pub width: u32,
    pub height: u32,
    /// 3 bytes (R,G,B) per pixel, row-major, top-left origin.
    pub rgb: Vec<u8>,
}

/// Read the BMP header common fields. Supports the 40-byte BITMAPINFOHEADER and the
/// larger V4/V5 headers (terrain.bmp uses a 124-byte V5 header) — we only need the
/// width/height/bpp/data-offset, which are at fixed positions in all of them.
fn bmp_header(data: &[u8]) -> Result<(u32, u32, u16, usize, usize), String> {
    if data.len() < 54 || &data[0..2] != b"BM" {
        return Err("not a BMP".into());
    }
    let dataoff = u32::from_le_bytes([data[10], data[11], data[12], data[13]]) as usize;
    let hsize = u32::from_le_bytes([data[14], data[15], data[16], data[17]]) as usize;
    let w = i32::from_le_bytes([data[18], data[19], data[20], data[21]]);
    let h = i32::from_le_bytes([data[22], data[23], data[24], data[25]]);
    let bpp = u16::from_le_bytes([data[28], data[29]]);
    if w <= 0 || h == 0 {
        return Err(format!("bad BMP dims {w}x{h}"));
    }
    Ok((w as u32, h.unsigned_abs(), bpp, dataoff, hsize))
}

impl IndexedBmp {
    /// Load an 8-bit indexed BMP (terrain.bmp / rivers.bmp). Rows are de-flipped to a
    /// top-left origin to match the province-id raster.
    pub fn load(path: &Path) -> Result<IndexedBmp, String> {
        let data = std::fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
        Self::parse(&data).map_err(|e| format!("{}: {e}", path.display()))
    }

    pub fn parse(data: &[u8]) -> Result<IndexedBmp, String> {
        let (w, h, bpp, dataoff, _hsize) = bmp_header(data)?;
        if bpp != 8 {
            return Err(format!("expected 8-bit indexed BMP, got {bpp}bpp"));
        }
        let stride = ((w as usize) + 3) & !3; // rows padded to 4 bytes
        let need = dataoff + stride * (h as usize);
        if data.len() < need {
            return Err(format!("truncated: need {need}, have {}", data.len()));
        }
        let top_down = {
            // height was negative => already top-down. We stored unsigned; re-check.
            let hraw = i32::from_le_bytes([data[22], data[23], data[24], data[25]]);
            hraw < 0
        };
        let mut indices = vec![0u8; (w as usize) * (h as usize)];
        for y in 0..(h as usize) {
            // BMP default is bottom-up: source row (h-1-y) feeds output row y.
            let src_row = if top_down { y } else { (h as usize) - 1 - y };
            let src = dataoff + src_row * stride;
            let dst = y * (w as usize);
            indices[dst..dst + (w as usize)].copy_from_slice(&data[src..src + (w as usize)]);
        }
        Ok(IndexedBmp {
            width: w,
            height: h,
            indices,
        })
    }
}

impl RgbBmp {
    /// Load a 24-bit RGB BMP (world_normal.bmp). De-flipped to top-left origin; BGR→RGB.
    pub fn load(path: &Path) -> Result<RgbBmp, String> {
        let data = std::fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
        Self::parse(&data).map_err(|e| format!("{}: {e}", path.display()))
    }

    pub fn parse(data: &[u8]) -> Result<RgbBmp, String> {
        let (w, h, bpp, dataoff, _hsize) = bmp_header(data)?;
        if bpp != 24 {
            return Err(format!("expected 24-bit RGB BMP, got {bpp}bpp"));
        }
        let stride = (((w as usize) * 3) + 3) & !3;
        let need = dataoff + stride * (h as usize);
        if data.len() < need {
            return Err(format!("truncated: need {need}, have {}", data.len()));
        }
        let hraw = i32::from_le_bytes([data[22], data[23], data[24], data[25]]);
        let top_down = hraw < 0;
        let mut rgb = vec![0u8; (w as usize) * (h as usize) * 3];
        for y in 0..(h as usize) {
            let src_row = if top_down { y } else { (h as usize) - 1 - y };
            let src = dataoff + src_row * stride;
            for x in 0..(w as usize) {
                let s = src + x * 3;
                let d = (y * (w as usize) + x) * 3;
                // BMP stores BGR.
                rgb[d] = data[s + 2];
                rgb[d + 1] = data[s + 1];
                rgb[d + 2] = data[s];
            }
        }
        Ok(RgbBmp {
            width: w,
            height: h,
            rgb,
        })
    }
}

/// The game's large-scale terrain COLOURMAP — the RGB channels of
/// `colormap_rgb_cityemissivemask_a.dds` (a BGRA8 DDS; alpha is a city-emissive mask we
/// ignore). Decoded to top-left-origin RGBA8 via [`assets::dds::decode`]. Spans the full
/// world extent identically to the province raster (a smaller, ~2:1 resolution), so we
/// sample it by normalised pixel-CENTRE map UV.
pub struct ColorMap {
    pub width: u32,
    pub height: u32,
    /// RGBA8 (straight), top-left origin. RGB = the land colour; A ignored.
    rgba: Vec<u8>,
}

impl ColorMap {
    /// Decode the colourmap DDS (any format the `assets` DDS reader supports; the real
    /// the game file is uncompressed BGRA8). RGB is the land colour; alpha is discarded.
    pub fn load(path: &Path) -> Result<ColorMap, String> {
        let bytes = std::fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
        let img = assets::dds::decode(&bytes).map_err(|e| format!("{}: {e}", path.display()))?;
        Ok(ColorMap {
            width: img.width,
            height: img.height,
            rgba: img.rgba,
        })
    }

    /// Construct directly from decoded RGBA8 (for tests).
    pub fn from_rgba(width: u32, height: u32, rgba: Vec<u8>) -> ColorMap {
        debug_assert_eq!(rgba.len(), (width as usize) * (height as usize) * 4);
        ColorMap {
            width,
            height,
            rgba,
        }
    }

    /// Bilinearly sample the colourmap at normalised UV in [0,1] (Codex B: destination
    /// pixel-centre UVs are passed by the caller). Returns the land RGB (alpha dropped /
    /// forced opaque). Clamp-to-edge at the borders.
    pub fn sample(&self, u: f32, v: f32) -> [u8; 3] {
        let (w, h) = (self.width as usize, self.height as usize);
        if w == 0 || h == 0 {
            return [0, 0, 0];
        }
        // map normalised UV onto source pixel space, sampling at texel centres.
        let fx = (u * w as f32 - 0.5).clamp(0.0, w as f32 - 1.0);
        let fy = (v * h as f32 - 0.5).clamp(0.0, h as f32 - 1.0);
        let x0 = fx.floor() as usize;
        let y0 = fy.floor() as usize;
        let x1 = (x0 + 1).min(w - 1);
        let y1 = (y0 + 1).min(h - 1);
        let dx = fx - x0 as f32;
        let dy = fy - y0 as f32;
        let g = |x: usize, y: usize, c: usize| -> f32 { self.rgba[(y * w + x) * 4 + c] as f32 };
        let mut out = [0u8; 3];
        for c in 0..3 {
            let top = g(x0, y0, c) * (1.0 - dx) + g(x1, y0, c) * dx;
            let bot = g(x0, y1, c) * (1.0 - dx) + g(x1, y1, c) * dx;
            out[c] = (top * (1.0 - dy) + bot * dy).round().clamp(0.0, 255.0) as u8;
        }
        out
    }

    /// Nearest-tap of the colourmap ALPHA channel = the city-emissive mask the game reads
    /// as `CityLightsMask = TerrainColor.a` [pdxmap.shader:313]. Returns 0..=255.
    pub fn alpha(&self, u: f32, v: f32) -> u8 {
        let (w, h) = (self.width as usize, self.height as usize);
        if w == 0 || h == 0 {
            return 0;
        }
        let x = ((u * w as f32) as usize).min(w - 1);
        let y = ((v * h as f32) as usize).min(h - 1);
        self.rgba[(y * w + x) * 4 + 3]
    }
}

/// The ground-detail TEXTURE ATLAS — `atlas0.dds` (2048×2048 DXT5), a 4×4 grid of 16
/// close-range material tiles (512×512 each), indexed 0..=15 by the `texture = N` field of
/// a terrain entry. We use it ONLY for a subtle, mean-preserving luminance modulation of
/// the colourmap base (Codex C), so we precompute each tile's mean luma once.
pub struct Atlas {
    /// RGBA8, top-left origin, full atlas.
    rgba: Vec<u8>,
    width: u32,
    height: u32,
    /// tile grid columns — 4 for atlas0 (rows are implied; tiles are addressed 0..16).
    cols: u32,
    tile_w: u32,
    tile_h: u32,
    /// per-tile mean luma (0..255), so the modulation can be made mean-preserving.
    tile_mean: [f32; 16],
}

impl Atlas {
    /// Decode `atlas0.dds` and precompute per-tile mean luma. Assumes a 4×4 tile grid
    /// (the real atlas0 layout); a non-square or non-4×4 atlas still works (cols/rows from
    /// dims), but only the first 16 tiles are addressed by `texture = N`.
    pub fn load(path: &Path) -> Result<Atlas, String> {
        let bytes = std::fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
        let img = assets::dds::decode(&bytes).map_err(|e| format!("{}: {e}", path.display()))?;
        Ok(Self::from_rgba(img.width, img.height, img.rgba))
    }

    /// Build from decoded RGBA8 with the fixed 4×4 atlas grid.
    pub fn from_rgba(width: u32, height: u32, rgba: Vec<u8>) -> Atlas {
        let (cols, rows) = (4u32, 4u32);
        let tile_w = (width / cols).max(1);
        let tile_h = (height / rows).max(1);
        let mut tile_mean = [128.0f32; 16];
        for ti in 0..16u32 {
            let (tc, tr) = (ti % cols, ti / cols);
            let (ox, oy) = (tc * tile_w, tr * tile_h);
            // sample a sparse grid of the tile for the mean (cheap, stable).
            let mut sum = 0.0f64;
            let mut n = 0u32;
            let step = (tile_w.max(tile_h) / 32).max(1);
            let mut y = oy;
            while y < oy + tile_h && y < height {
                let mut x = ox;
                while x < ox + tile_w && x < width {
                    let o = ((y * width + x) * 4) as usize;
                    sum += luma(rgba[o], rgba[o + 1], rgba[o + 2]) as f64;
                    n += 1;
                    x += step;
                }
                y += step;
            }
            if n > 0 {
                tile_mean[ti as usize] = (sum / n as f64) as f32;
            }
        }
        let _ = rows;
        Atlas {
            rgba,
            width,
            height,
            cols,
            tile_w,
            tile_h,
            tile_mean,
        }
    }

    /// The MEAN-PRESERVING luminance modulation factor for atlas tile `ti` at tile-local
    /// UV (`tu`,`tv` in [0,1), wrapped/tiled by the caller). Returns a multiplier centred
    /// on 1.0: `1 + blend * (luma(texel) - tile_mean) / 255`. Because it varies around
    /// the tile's own mean, applying it does NOT shift the colourmap's average colour —
    /// it only adds material grain (Codex C). `blend` is the fidelity dial.
    pub fn detail_factor(&self, ti: u8, tu: f32, tv: f32, blend: f32) -> f32 {
        if ti >= 16 {
            return 1.0;
        }
        let (tc, tr) = ((ti as u32) % self.cols, (ti as u32) / self.cols);
        let lx = ((tu.fract().abs()) * self.tile_w as f32) as u32;
        let ly = ((tv.fract().abs()) * self.tile_h as f32) as u32;
        let x = (tc * self.tile_w + lx.min(self.tile_w - 1)).min(self.width - 1);
        let y = (tr * self.tile_h + ly.min(self.tile_h - 1)).min(self.height - 1);
        let o = ((y * self.width + x) * 4) as usize;
        let l = luma(self.rgba[o], self.rgba[o + 1], self.rgba[o + 2]);
        let mean = self.tile_mean[ti as usize];
        (1.0 + blend * (l - mean) / 255.0).clamp(0.4, 1.6)
    }

    /// Sample atlas tile `ti`'s RGB (in LINEAR space) at tile-local UV (`tu`,`tv`), wrapped.
    /// This is the game's `tex2Dlod(TerrainDiffuse, sample_terrain(...))` — the close-range
    /// ground material that forms `diffuse.rgb`, the BASE of the GetOverlay with the
    /// colourmap. The atlas DDS is sRGB-encoded, so we decode to linear here.
    pub fn tile_linear(&self, ti: u8, tu: f32, tv: f32) -> [f32; 3] {
        if ti >= 16 {
            return [0.5, 0.5, 0.5];
        }
        let (tc, tr) = ((ti as u32) % self.cols, (ti as u32) / self.cols);
        let lx = ((tu.rem_euclid(1.0)) * self.tile_w as f32) as u32;
        let ly = ((tv.rem_euclid(1.0)) * self.tile_h as f32) as u32;
        let x = (tc * self.tile_w + lx.min(self.tile_w - 1)).min(self.width - 1);
        let y = (tr * self.tile_h + ly.min(self.tile_h - 1)).min(self.height - 1);
        let o = ((y * self.width + x) * 4) as usize;
        [
            srgb_to_linear(self.rgba[o]),
            srgb_to_linear(self.rgba[o + 1]),
            srgb_to_linear(self.rgba[o + 2]),
        ]
    }
}

/// The ground-detail NORMAL atlas — `atlas_normal0.dds` (2048×2048 DXT5), the per-tile
/// tangent-space normal + specular the game samples alongside `atlas0.dds` and rotates into
/// the world relief (`terrain_normal` at pdxmap.shader:259-307). 4×4 tile grid like atlas0.
/// The alpha channel carries the per-texel specular (`vSpec = terrain_normalRaw.a`, [295]).
pub struct AtlasNormal {
    rgba: Vec<u8>,
    width: u32,
    height: u32,
    cols: u32,
    tile_w: u32,
    tile_h: u32,
}

impl AtlasNormal {
    pub fn load(path: &Path) -> Result<AtlasNormal, String> {
        let bytes = std::fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
        let img = assets::dds::decode(&bytes).map_err(|e| format!("{}: {e}", path.display()))?;
        Ok(Self::from_rgba(img.width, img.height, img.rgba))
    }
    pub fn from_rgba(width: u32, height: u32, rgba: Vec<u8>) -> AtlasNormal {
        let (cols, rows) = (4u32, 4u32);
        let tile_w = (width / cols).max(1);
        let tile_h = (height / rows).max(1);
        let _ = rows;
        AtlasNormal {
            rgba,
            width,
            height,
            cols,
            tile_w,
            tile_h,
        }
    }
    /// Sample atlas-normal tile `ti` at tile-local UV → (tangent-space normal in the game's
    /// `.rbg-0.5` convention, normalised) and the specular (alpha). [pdxmap.shader:259,290,295]
    pub fn tile_normal_spec(&self, ti: u8, tu: f32, tv: f32) -> ([f32; 3], f32) {
        if ti >= 16 {
            return ([0.0, 1.0, 0.0], 0.0);
        }
        let (tc, tr) = ((ti as u32) % self.cols, (ti as u32) / self.cols);
        let lx = ((tu.rem_euclid(1.0)) * self.tile_w as f32) as u32;
        let ly = ((tv.rem_euclid(1.0)) * self.tile_h as f32) as u32;
        let x = (tc * self.tile_w + lx.min(self.tile_w - 1)).min(self.width - 1);
        let y = (tr * self.tile_h + ly.min(self.tile_h - 1)).min(self.height - 1);
        let o = ((y * self.width + x) * 4) as usize;
        // game: terrain_normal = terrain_normalRaw.rbg - 0.5 (x=R, y=B, z=G); spec = .a.
        let nx = self.rgba[o] as f32 / 255.0 - 0.5;
        let ny = self.rgba[o + 2] as f32 / 255.0 - 0.5;
        let nz = self.rgba[o + 1] as f32 / 255.0 - 0.5;
        let len = (nx * nx + ny * ny + nz * nz).sqrt().max(1e-4);
        let spec = self.rgba[o + 3] as f32 / 255.0;
        ([nx / len, ny / len, nz / len], spec)
    }
}

/// The game's `RotateVectorByVector(v1, v2)` — rotates `v1` into the frame whose up-axis is
/// `v2` (used to compose the atlas tile normal onto the world normal). This mirrors the HLSL
/// at standardfuncsgfx.fxh:59: build a basis from v2 and transform v1 into it.
fn rotate_vector_by_vector(v1: [f32; 3], v2: [f32; 3]) -> [f32; 3] {
    // Exact translation of the game's RotateVectorByVector (Frisvad-style basis from v2).
    let up = v2;
    // pick a tangent not parallel to `up`.
    let q = if up[2].abs() < 0.999 {
        [0.0, 0.0, 1.0]
    } else {
        [1.0, 0.0, 0.0]
    };
    let t = cross(q, up);
    let t = normalize3(t);
    let b = cross(up, t);
    // v1 expressed in (t, up, b): the game treats v1.y as the up component.
    [
        v1[0] * t[0] + v1[1] * up[0] + v1[2] * b[0],
        v1[0] * t[1] + v1[1] * up[1] + v1[2] * b[1],
        v1[0] * t[2] + v1[1] * up[2] + v1[2] * b[2],
    ]
}
#[inline]
fn cross(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}
#[inline]
fn normalize3(v: [f32; 3]) -> [f32; 3] {
    let l = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt().max(1e-4);
    [v[0] / l, v[1] / l, v[2] / l]
}

/// The GAME WATER COLOURMAP — `colormap_water_0.dds` (2816×1024 DXT5). Its RGB is the
/// real sea/coast colour the game's water surface tints toward; the game multiplies it by
/// `WATER_COLOR_LIGHTNESS=0.5`. Spans the full world extent like the land colourmap, so we
/// sample it by normalised pixel-centre map UV. Decoded RGBA8 (sRGB) top-left origin.
pub struct WaterColorMap {
    pub width: u32,
    pub height: u32,
    rgba: Vec<u8>,
}

impl WaterColorMap {
    pub fn load(path: &Path) -> Result<WaterColorMap, String> {
        let bytes = std::fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
        let img = assets::dds::decode(&bytes).map_err(|e| format!("{}: {e}", path.display()))?;
        Ok(WaterColorMap {
            width: img.width,
            height: img.height,
            rgba: img.rgba,
        })
    }
    pub fn from_rgba(width: u32, height: u32, rgba: Vec<u8>) -> WaterColorMap {
        WaterColorMap {
            width,
            height,
            rgba,
        }
    }
    /// Sample the water colourmap at normalised UV → LINEAR RGB × `WATER_COLOR_LIGHTNESS`.
    /// Nearest-tap (the macro colourmap is smooth; nearest avoids a second bilinear pass).
    pub fn sample_linear(&self, u: f32, v: f32) -> [f32; 3] {
        let (w, h) = (self.width as usize, self.height as usize);
        if w == 0 || h == 0 {
            return [
                srgb_to_linear(WATER_COLOR[0]),
                srgb_to_linear(WATER_COLOR[1]),
                srgb_to_linear(WATER_COLOR[2]),
            ];
        }
        let x = ((u * w as f32) as usize).min(w - 1);
        let y = ((v * h as f32) as usize).min(h - 1);
        let o = (y * w + x) * 4;
        [
            srgb_to_linear(self.rgba[o]) * gc::WATER_COLOR_LIGHTNESS,
            srgb_to_linear(self.rgba[o + 1]) * gc::WATER_COLOR_LIGHTNESS,
            srgb_to_linear(self.rgba[o + 2]) * gc::WATER_COLOR_LIGHTNESS,
        ]
    }
}

/// The river SURFACE diffuse colour — sampled once from `RiverSurface_diffuse_0.dds`
/// (the mean of the texture is the line colour the game tints rivers with). We use a
/// single representative colour because, at political-map zoom, the per-texel river
/// material is sub-pixel; the river LINE position comes from `rivers.bmp`.
pub struct RiverSurface {
    /// representative river diffuse colour in 8-bit sRGB.
    pub color: [u8; 3],
}

impl RiverSurface {
    pub fn load(path: &Path) -> Result<RiverSurface, String> {
        let bytes = std::fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
        let img = assets::dds::decode(&bytes).map_err(|e| format!("{}: {e}", path.display()))?;
        // mean colour over a sparse grid (the river material is a fairly uniform blue).
        let (w, h) = (img.width as usize, img.height as usize);
        if w == 0 || h == 0 {
            return Ok(RiverSurface { color: RIVER_COLOR });
        }
        let (mut sr, mut sg, mut sb, mut n) = (0u64, 0u64, 0u64, 0u64);
        let step = (w.max(h) / 64).max(1);
        let mut y = 0;
        while y < h {
            let mut x = 0;
            while x < w {
                let o = (y * w + x) * 4;
                sr += img.rgba[o] as u64;
                sg += img.rgba[o + 1] as u64;
                sb += img.rgba[o + 2] as u64;
                n += 1;
                x += step;
            }
            y += step;
        }
        let n = n.max(1);
        Ok(RiverSurface {
            color: [(sr / n) as u8, (sg / n) as u8, (sb / n) as u8],
        })
    }
}

/// Rec.601 luma of an RGB triple, in 0..=255.
#[inline]
fn luma(r: u8, g: u8, b: u8) -> f32 {
    0.299 * r as f32 + 0.587 * g as f32 + 0.114 * b as f32
}

/// Is a `rivers.bmp` palette index a drawable river pixel? Per the game river encoding:
/// index 0/1/2 are land + source/junction markers, 3..=11 are river segments of
/// increasing width, and the high sentinels (254/255 and ≥12 land-tint) are not rivers.
pub fn is_river_index(idx: u8) -> bool {
    (3..=11).contains(&idx)
}

/// Decode a `world_normal.bmp` texel to a unit normal in the game's convention. The game
/// reads `tex2D(HeightNormal, uv).rbg - 0.5` (note the .rbg swizzle: R→x, B→y(up), G→z),
/// normalised. [pdxmap.shader:250]. Stored RGB is LINEAR data (no sRGB decode).
#[inline]
fn decode_world_normal(r: u8, g: u8, b: u8) -> [f32; 3] {
    // game swizzle .rbg: x=R, y=B, z=G, each centred at 0.5 (texel 128).
    let x = r as f32 / 255.0 - 0.5;
    let y = b as f32 / 255.0 - 0.5;
    let z = g as f32 / 255.0 - 0.5;
    let len = (x * x + y * y + z * z).sqrt().max(1e-4);
    [x / len, y / len, z / len]
}

/// The game's `AmbientLight(normal, dayFactor=0)` day branch — the directional ambient
/// cube. Squared-normal weighting picks the +/- face per axis. [standardfuncsgfx.fxh:456-472].
/// `amb` is the 6 cube faces in LINEAR rgb, order [+X,-X,+Y,-Y,+Z,-Z].
#[inline]
fn ambient_light(n: [f32; 3], amb: &[[f32; 3]; 6]) -> [f32; 3] {
    let sq = [n[0] * n[0], n[1] * n[1], n[2] * n[2]];
    let fx = if n[0] < 0.0 { 1 } else { 0 };
    let fy = if n[1] < 0.0 { 1 } else { 0 };
    let fz = if n[2] < 0.0 { 1 } else { 0 };
    let cx = amb[fx]; // +X=0, -X=1
    let cy = amb[2 + fy]; // +Y=2, -Y=3
    let cz = amb[4 + fz]; // +Z=4, -Z=5
    [
        sq[0] * cx[0] + sq[1] * cy[0] + sq[2] * cz[0],
        sq[0] * cx[1] + sq[1] * cy[1] + sq[2] * cz[1],
        sq[0] * cx[2] + sq[1] * cy[2] + sq[2] * cz[2],
    ]
}

/// The fully-built, GPU-ready base layers, all at the province-id raster resolution.
/// Stored as 8-bit sRGB (the WGSL decodes back to linear for the political blend, matching
/// how the game's framebuffer holds the gamma-encoded terrain). City emissive is carried
/// separately so the WGSL can add it (night) without it being eaten by the political tint.
pub struct TerrainBase {
    pub width: u32,
    pub height: u32,
    /// RGBA8 (sRGB) faithful terrain = atlas-detail · GetOverlay(colormap) · game lighting
    /// for land, or the real water colourmap · depth/shore/ice for sea/lake. α=255.
    pub base_rgba: Vec<u8>,
    /// River mask + colour: RGB = the real `RiverSurface_diffuse_0.dds` colour, A=255 on a
    /// river pixel else 0. The WGSL draws the river line over the composite (land only).
    pub river_rgba: Vec<u8>,
    /// City-emissive mask: R = the colormap alpha (city-lights mask, 0..255), G,B,A unused.
    /// The WGSL adds `CITY_LIGHTS_INTENSITY · mask · nightFactor` at night. [pdxmap:313,385]
    pub emissive_rgba: Vec<u8>,
    /// PER-PIXEL GATE data for the dynamic snow/mud/ice material branches (the inputs the
    /// game's ApplySnow / GetMudColor read as `vPos.y` and `vNormal.y`):
    ///   * R = world height / `GATE_HEIGHT_RANGE` (so a u8 spans 0..`GATE_HEIGHT_RANGE` world
    ///     units; `world_height = heightmap8 · HEIGHT_SCALE`),
    ///   * G = `normal.y` remapped 0..1 (the upward-facing factor for the snow normal gate),
    ///   * B = 255 on land else 0, A = 255.
    /// Sampled per output pixel in the WGSL to gate the per-pixel material exactly as the
    /// shader does (height ≥ SNOW_START_HEIGHT, normal.y ≥ SNOW_NORMAL_START, etc.).
    pub gate_rgba: Vec<u8>,
}

/// World-height per heightmap 8-bit unit: the game's water sits at heightmap≈95.7 = WATER_HEIGHT
/// 9.5, so 1 heightmap unit ≈ 0.0993 world units ≈ 0.1. [pdxwater.shader:281; constants WATER_HEIGHT]
pub const HEIGHT_SCALE: f32 = 0.0993;
/// The world-height range a u8 in `gate_rgba.r` spans (so the WGSL recovers world height as
/// `gate.r · GATE_HEIGHT_RANGE`). 255·HEIGHT_SCALE ≈ 25.3, the full map height range.
pub const GATE_HEIGHT_RANGE: f32 = 25.3;

/// Atlas tiles repeat across the world at `TERRAIN_TILE_FREQ` per the game's `uv2`. The
/// game tiles `uv2 * TERRAIN_TILE_FREQ` (×128) then `* MAP_SIZE_X/MAP_SIZE_Y` for aspect
/// [pdxmap.shader:241-242]. We reproduce the same per-world repeat count.
const ATLAS_TILES_ACROSS_WORLD: f32 = gc::TERRAIN_TILE_FREQ;

/// All the loaded game terrain assets, grouped so [`build_base`] takes one struct.
pub struct TerrainInputs<'a> {
    /// `colormap_rgb_cityemissivemask_a.dds` (RGB = land colourmap, A = city mask).
    pub colormap: &'a ColorMap,
    /// `colormap_water_0.dds` (the real sea colour). Required (faithful water).
    pub water_colormap: &'a WaterColorMap,
    /// indexed `terrain.bmp` (water classification + atlas tile per pixel).
    pub terrain: &'a IndexedBmp,
    /// `world_normal.bmp` (relief normal map).
    pub normal: &'a RgbBmp,
    /// indexed `heightmap.bmp` (8-bit height → water shore/depth).
    pub heightmap: &'a IndexedBmp,
    /// indexed `rivers.bmp` (river line positions).
    pub rivers: &'a IndexedBmp,
    /// `atlas0.dds` ground-detail tiles (required for the faithful overlay base).
    pub atlas: &'a Atlas,
    /// `atlas_normal0.dds` ground-detail normals (relief detail + specular).
    pub atlas_normal: &'a AtlasNormal,
    /// the river surface diffuse colour.
    pub river_surface: &'a RiverSurface,
    /// terrain defs (index→type, index→atlas tile, perm-snow).
    pub defs: &'a TerrainDefs,
}

/// Build the terrain base by translating the game's `PixelShaderTerrain` math 1:1
/// (Codex-verified), at `(out_w,out_h)` = the province-id raster resolution.
///
/// Per land pixel (the game's order, all in LINEAR space):
///   1. `diffuse = atlas_tile_linear` — the close-range ground material (the GetOverlay base).
///   2. `diffuse = GetOverlay(diffuse, colormap_linear, COLORMAP_OVERLAY_STRENGTH=0.75)`.
///   3. relief: `normal` from world_normal; lighting = `ComposeLight` =
///      `(AmbientLight(normal) + sun·NdotL) · diffuse · HdrRange`.
///   4. city-emissive mask = colormap alpha (carried out separately).
/// Per sea/lake pixel: the real `colormap_water_0.dds × WATER_COLOR_LIGHTNESS=0.5`, with a
/// coastal shore fade from the heightmap (`waterShore`) and ice tint where snow data wants
/// it (we use ICE_COLOR at the far-north polar band, the only place vanilla shows ice).
/// The output is stored 8-bit sRGB (gamma-encoded), the form the WGSL political blend reads.
pub fn build_base(out_w: u32, out_h: u32, inp: &TerrainInputs) -> TerrainBase {
    // Sun direction (LIGHT_DIRECTION), normalised; the diffuse term is N·(-L) (light points
    // FROM the sun toward the ground; surface lit by the opposite). [00_graphics.lua:766-768]
    let sun_dir = {
        let l = gc::LIGHT_DIRECTION;
        let n = (l[0] * l[0] + l[1] * l[1] + l[2] * l[2]).sqrt().max(1e-4);
        [-l[0] / n, -l[1] / n, -l[2] / n] // toward-sun unit vector
    };
    // Sun diffuse colour (HSV→linear) × intensity = SunDiffuseIntensity.rgb·a.
    let sun_col = {
        let c = hsv_to_linear_rgb(gc::SUN_DIFFUSE_COLOR_HSV);
        [
            c[0] * gc::SUN_INTENSITY,
            c[1] * gc::SUN_INTENSITY,
            c[2] * gc::SUN_INTENSITY,
        ]
    };
    // Ambient cube faces = the baked DayAmbientMap* constants (already linear).
    let amb: [[f32; 3]; 6] = gc::AMBIENT_DAY;

    let n = (out_w as usize) * (out_h as usize);
    let mut base_rgba = vec![0u8; n * 4];
    let mut river_rgba = vec![0u8; n * 4];
    let mut emissive_rgba = vec![0u8; n * 4];
    let mut gate_rgba = vec![0u8; n * 4];

    let (tw, th) = (inp.terrain.width as usize, inp.terrain.height as usize);
    let (nw, nh) = (inp.normal.width as usize, inp.normal.height as usize);
    let (hw, hh) = (inp.heightmap.width as usize, inp.heightmap.height as usize);
    let (rw, rh) = (inp.rivers.width as usize, inp.rivers.height as usize);

    let tiles_x = ATLAS_TILES_ACROSS_WORLD;
    let tiles_y = ATLAS_TILES_ACROSS_WORLD * (out_h as f32 / out_w as f32);

    for y in 0..(out_h as usize) {
        let ty = (y * th / (out_h as usize)).min(th - 1);
        let fv = (y as f32 + 0.5) / (out_h as f32);
        for x in 0..(out_w as usize) {
            let tx = (x * tw / (out_w as usize)).min(tw - 1);
            let idx = inp.terrain.indices[ty * tw + tx];
            let ttype = inp.defs.index_type(idx);
            let o = (y * (out_w as usize) + x) * 4;
            let fu = (x as f32 + 0.5) / (out_w as f32);

            // relief normal (bilinear, decoded in the game's convention).
            let normal = sample_world_normal(inp.normal, nw, nh, fu, fv);

            // --- per-pixel GATE data for the dynamic snow/mud/ice material (vPos.y, normal.y).
            {
                let hx = (x * hw / (out_w as usize)).min(hw - 1);
                let hy = (y * hh / (out_h as usize)).min(hh - 1);
                let h8 = inp.heightmap.indices[hy * hw + hx] as f32;
                let world_h = h8 * HEIGHT_SCALE; // world height units
                gate_rgba[o] = (world_h / GATE_HEIGHT_RANGE * 255.0).clamp(0.0, 255.0) as u8;
                gate_rgba[o + 1] = ((normal[1] * 0.5 + 0.5) * 255.0).clamp(0.0, 255.0) as u8;
                gate_rgba[o + 2] = if ttype.is_water() { 0 } else { 255 };
                gate_rgba[o + 3] = 255;
            }

            let lin: [f32; 3] = if ttype.is_water() {
                // --- WATER: real water colourmap × 0.5, coastal shore + polar ice ---
                let mut wc = inp.water_colormap.sample_linear(fu, fv);
                // waterShore from heightmap: height/(95.7/255); shore = saturate((h-0.954)*25).
                let hx = (x * hw / (out_w as usize)).min(hw - 1);
                let hy = (y * hh / (out_h as usize)).min(hh - 1);
                let h8 = inp.heightmap.indices[hy * hw + hx] as f32 / 255.0;
                let water_height = h8 / (95.7 / 255.0);
                let shore = ((water_height - 0.954) * 25.0).clamp(0.0, 1.0);
                // coast: blend toward a lighter shallow tint near the shoreline.
                let shallow = [wc[0] * 1.6 + 0.02, wc[1] * 1.5 + 0.03, wc[2] * 1.4 + 0.04];
                for i in 0..3 {
                    wc[i] = wc[i] + (shallow[i] - wc[i]) * shore;
                }
                // polar ice band (the only place vanilla ApplyIce shows ice): far north/south.
                let polar = (fv - 0.93).max(0.07 - fv).max(0.0) / 0.07;
                let ice_fade = polar.clamp(0.0, 1.0) * 0.6;
                for i in 0..3 {
                    wc[i] = wc[i] + (gc::ICE_COLOR[i] - wc[i]) * ice_fade;
                }
                // the deliberate dev "Darkness" offset: lerp(water, black, 0.2) — applied to
                // ALL water in ALL mapmodes to keep the sea its expected darkness after a
                // gradient-border brightness fix. [pdxwater.shader:360-361]
                for i in 0..3 {
                    wc[i] *= 1.0 - gc::WATER_DARKNESS;
                }
                wc
            } else {
                // --- LAND: atlas diffuse, GetOverlay(colormap), then game lighting ---
                // 1) atlas tile material (linear) = diffuse base + atlas tile normal/spec.
                let ti = inp.defs.index_atlas(idx);
                let (mut diffuse, shade_normal, vspec) = if ti == NO_ATLAS {
                    ([0.5, 0.5, 0.5], normal, 0.0)
                } else {
                    let tu = fu * tiles_x;
                    let tv = fv * tiles_y;
                    let d = inp.atlas.tile_linear(ti, tu, tv);
                    // atlas tile normal rotated into the world relief (game: normal =
                    // RotateVectorByVector(world_normal, terrain_normal)). [pdxmap:306]
                    let (tn, spec) = inp.atlas_normal.tile_normal_spec(ti, tu, tv);
                    let rotated = normalize3(rotate_vector_by_vector(normal, tn));
                    (d, rotated, spec)
                };
                // 2) colormap as the OVERLAY over the atlas (the game's macro land colour).
                let cm = inp.colormap.sample(fu, fv);
                let cm_lin = [
                    srgb_to_linear(cm[0]),
                    srgb_to_linear(cm[1]),
                    srgb_to_linear(cm[2]),
                ];
                diffuse = get_overlay(diffuse, cm_lin, gc::COLORMAP_OVERLAY_STRENGTH);

                // 2b) permanent snow: perm_snow terrain (polar / high-mountain caps) the
                // game always draws white. SNOW_COLOR over the diffuse where the terrain
                // index is perm_snow AND above SNOW_START_HEIGHT (heightmap), gated by the
                // upward-facing normal (SNOW_NORMAL_START). [standardfuncsgfx ApplySnow]
                if inp.defs.index_perm_snow(idx) {
                    let hx = (x * hw / (out_w as usize)).min(hw - 1);
                    let hy = (y * hh / (out_h as usize)).min(hh - 1);
                    let world_h = inp.heightmap.indices[hy * hw + hx] as f32 / 255.0 * 25.5;
                    let h_fade = (world_h - gc::SNOW_START_HEIGHT).clamp(0.0, 1.0);
                    let n_fade = ((shade_normal[1] - 0.7) * 5.0).clamp(0.0, 1.0);
                    let snow = (h_fade * n_fade).clamp(0.0, 1.0);
                    for i in 0..3 {
                        diffuse[i] = diffuse[i] + (gc::SNOW_COLOR[i] - diffuse[i]) * snow;
                    }
                }

                // 3) lighting: ComposeLight = (ambient + sun·NdotL) · diffuse · HdrRange,
                //    plus a Blinn-Phong specular highlight from the atlas spec (game:
                //    SpecRemapped = vSpec*vSpec*0.4; specular added on top). [pdxmap:335-373]
                let ndotl = (shade_normal[0] * sun_dir[0]
                    + shade_normal[1] * sun_dir[1]
                    + shade_normal[2] * sun_dir[2])
                    .max(0.0);
                let ambient = ambient_light(shade_normal, &amb);
                // half-vector specular toward a top-down viewer (the political camera looks
                // straight down: view dir ≈ +Y). H = normalize(toSun + view).
                let view = [0.0f32, 1.0, 0.0];
                let half = normalize3([
                    sun_dir[0] + view[0],
                    sun_dir[1] + view[1],
                    sun_dir[2] + view[2],
                ]);
                let ndoth = (shade_normal[0] * half[0]
                    + shade_normal[1] * half[1]
                    + shade_normal[2] * half[2])
                    .max(0.0);
                let spec_remapped = vspec * vspec * 0.4;
                let spec = spec_remapped * ndoth.powf(32.0);
                let mut out = [0.0f32; 3];
                for i in 0..3 {
                    let light = ambient[i] + sun_col[i] * ndotl;
                    out[i] = light * diffuse[i] * gc::HDR_RANGE + sun_col[i] * spec;
                }
                out
            };

            // store as 8-bit sRGB (gamma-encoded), the framebuffer form the WGSL reads.
            base_rgba[o] = linear_to_srgb8(lin[0]);
            base_rgba[o + 1] = linear_to_srgb8(lin[1]);
            base_rgba[o + 2] = linear_to_srgb8(lin[2]);
            base_rgba[o + 3] = 255;

            // city emissive mask = colormap alpha (land only; sea has no city mask).
            if !ttype.is_water() {
                let a = inp.colormap.alpha(fu, fv);
                emissive_rgba[o] = a;
            }

            // river line (land only): real river surface colour.
            let rx = (x * rw / (out_w as usize)).min(rw - 1);
            let ry = (y * rh / (out_h as usize)).min(rh - 1);
            if is_river_index(inp.rivers.indices[ry * rw + rx]) {
                river_rgba[o] = inp.river_surface.color[0];
                river_rgba[o + 1] = inp.river_surface.color[1];
                river_rgba[o + 2] = inp.river_surface.color[2];
                river_rgba[o + 3] = 255;
            }
        }
    }

    TerrainBase {
        width: out_w,
        height: out_h,
        base_rgba,
        river_rgba,
        emissive_rgba,
        gate_rgba,
    }
}

/// Bilinearly sample `world_normal.bmp` at (u,v) → a unit normal (game convention).
fn sample_world_normal(normal: &RgbBmp, nw: usize, nh: usize, u: f32, v: f32) -> [f32; 3] {
    if nw == 0 || nh == 0 {
        return [0.0, 1.0, 0.0];
    }
    let fx = (u * nw as f32 - 0.5).clamp(0.0, nw as f32 - 1.0);
    let fy = (v * nh as f32 - 0.5).clamp(0.0, nh as f32 - 1.0);
    let x0 = fx.floor() as usize;
    let y0 = fy.floor() as usize;
    let x1 = (x0 + 1).min(nw - 1);
    let y1 = (y0 + 1).min(nh - 1);
    let dx = fx - x0 as f32;
    let dy = fy - y0 as f32;
    let s = |x: usize, y: usize| -> [f32; 3] {
        let p = (y * nw + x) * 3;
        decode_world_normal(normal.rgb[p], normal.rgb[p + 1], normal.rgb[p + 2])
    };
    let (a, b, c, d) = (s(x0, y0), s(x1, y0), s(x0, y1), s(x1, y1));
    let mut out = [0.0f32; 3];
    for i in 0..3 {
        let top = a[i] * (1.0 - dx) + b[i] * dx;
        let bot = c[i] * (1.0 - dx) + d[i] * dx;
        out[i] = top * (1.0 - dy) + bot * dy;
    }
    let len = (out[0] * out[0] + out[1] * out[1] + out[2] * out[2])
        .sqrt()
        .max(1e-4);
    [out[0] / len, out[1] / len, out[2] / len]
}

// ---------------------------------------------------------------------------
// Minimal Clausewitz-script block helpers (only enough to read 00_terrain.txt).
// ---------------------------------------------------------------------------

/// Extract the brace-balanced body of `key = { … }` (first occurrence). Returns the
/// inner text (without the outer braces). Comments are not stripped here — the callers
/// only read numbers/identifiers, and `#` comments never appear inside the data we read.
fn extract_block<'a>(txt: &'a str, key: &str) -> Option<&'a str> {
    let mut search_from = 0;
    loop {
        let rel = txt[search_from..].find(key)?;
        let kpos = search_from + rel;
        // ensure it's a token boundary (preceded by whitespace/start, followed by ws/=)
        let before_ok = kpos == 0 || txt.as_bytes()[kpos - 1].is_ascii_whitespace();
        let after = kpos + key.len();
        let after_ok = txt[after..]
            .chars()
            .next()
            .map(|c| c.is_whitespace() || c == '=')
            .unwrap_or(false);
        if before_ok && after_ok {
            // find the opening brace after the key.
            if let Some(brel) = txt[after..].find('{') {
                let bopen = after + brel;
                let bytes = txt.as_bytes();
                let mut depth = 0i32;
                let mut i = bopen;
                while i < bytes.len() {
                    match bytes[i] {
                        b'{' => depth += 1,
                        b'}' => {
                            depth -= 1;
                            if depth == 0 {
                                return Some(&txt[bopen + 1..i]);
                            }
                        }
                        _ => {}
                    }
                    i += 1;
                }
                return None;
            }
        }
        search_from = kpos + key.len();
    }
}

/// Iterate `name = { body }` pairs at the TOP level of `txt` (one nesting level deep).
/// Yields `(name, body)` with `body` the inner brace-balanced text.
fn iter_named_blocks(txt: &str) -> Vec<(&str, &str)> {
    let bytes = txt.as_bytes();
    let mut out = Vec::new();
    let mut i = 0usize;
    while i < bytes.len() {
        // find an identifier token
        if bytes[i].is_ascii_alphanumeric() || bytes[i] == b'_' {
            let start = i;
            while i < bytes.len() && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'_') {
                i += 1;
            }
            let name = &txt[start..i];
            // skip ws
            let mut j = i;
            while j < bytes.len() && bytes[j].is_ascii_whitespace() {
                j += 1;
            }
            if j < bytes.len() && bytes[j] == b'=' {
                j += 1;
                while j < bytes.len() && bytes[j].is_ascii_whitespace() {
                    j += 1;
                }
                if j < bytes.len() && bytes[j] == b'{' {
                    // brace-match
                    let bopen = j;
                    let mut depth = 0i32;
                    let mut k = bopen;
                    while k < bytes.len() {
                        match bytes[k] {
                            b'{' => depth += 1,
                            b'}' => {
                                depth -= 1;
                                if depth == 0 {
                                    break;
                                }
                            }
                            _ => {}
                        }
                        k += 1;
                    }
                    if k < bytes.len() {
                        out.push((name, &txt[bopen + 1..k]));
                        i = k + 1;
                        continue;
                    }
                }
            }
        } else {
            i += 1;
        }
    }
    out
}

/// Parse the FIRST `color = { r g b }` triplet inside a block body.
fn parse_color_triplet(body: &str) -> Option<[u8; 3]> {
    let cpos = find_token(body, "color")?;
    let after = &body[cpos..];
    let bopen = after.find('{')?;
    let bclose = after[bopen..].find('}')? + bopen;
    let nums: Vec<u8> = after[bopen + 1..bclose]
        .split_whitespace()
        .filter_map(|t| t.parse::<i64>().ok())
        .map(|v| v.clamp(0, 255) as u8)
        .collect();
    if nums.len() >= 3 {
        Some([nums[0], nums[1], nums[2]])
    } else {
        None
    }
}

/// Parse a `color = { N }` SINGLE-number block body → the palette index N.
fn parse_color_single_index(body: &str) -> Option<u8> {
    let cpos = find_token(body, "color")?;
    let after = &body[cpos..];
    let bopen = after.find('{')?;
    let bclose = after[bopen..].find('}')? + bopen;
    let nums: Vec<i64> = after[bopen + 1..bclose]
        .split_whitespace()
        .filter_map(|t| t.parse::<i64>().ok())
        .collect();
    if nums.len() == 1 && (0..=255).contains(&nums[0]) {
        Some(nums[0] as u8)
    } else {
        None
    }
}

/// Parse a `key = N` integer scalar inside a block body (e.g. `texture = 4`). Returns the
/// first integer following the token's `=`. Used for the atlas `texture` index.
fn parse_scalar(body: &str, key: &str) -> Option<i64> {
    let kpos = find_token(body, key)?;
    let after = &body[kpos + key.len()..];
    // skip whitespace and the `=`.
    let rest = after.trim_start();
    let rest = rest.strip_prefix('=').unwrap_or(rest).trim_start();
    rest.split_whitespace().next()?.parse::<i64>().ok()
}

/// Find `token` as a standalone word in `s`, returning its byte offset.
fn find_token(s: &str, token: &str) -> Option<usize> {
    let mut from = 0;
    loop {
        let rel = s[from..].find(token)?;
        let pos = from + rel;
        let before_ok = pos == 0 || !s.as_bytes()[pos - 1].is_ascii_alphanumeric();
        let after_idx = pos + token.len();
        let after_ok = s[after_idx..]
            .chars()
            .next()
            .map(|c| !c.is_ascii_alphanumeric() && c != '_')
            .unwrap_or(true);
        if before_ok && after_ok {
            return Some(pos);
        }
        from = pos + token.len();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"
categories = {
    ocean = { color = { 40 83 176 } is_water = yes }
    lakes = { color = { 58 91 255 } is_water = yes }
    forest = { color = { 89 199 85 } }
    plains = { color = { 255 129 66 } }
    urban  = { color = { 120 120 120 } }
    desert = { color = { 255 127 0 } }
    mountain = { color = { 157 192 208 } }
    hills = { color = { 248 255 153 } }
    marsh = { color = { 76 96 35 } }
    jungle = { color = { 127 191 0 } }
}
terrain = {
    terrain_0  = { type = plains   color = { 0 } texture = 1 }
    terrain_1  = { type = forest   color = { 1 } texture = 4 }
    desert_m   = { type = hills    color = { 2 } texture = 3 }
    desert     = { type = desert   color = { 3 } texture = 9 }
    forest_13  = { type = urban    color = { 13 } texture = 10 spawn_city = yes }
    forest_14  = { type = lakes    color = { 14 } texture = 255 }
    ocean_15   = { type = ocean    color = { 15 } texture = 9 }
    snow_16    = { type = mountain color = { 16 } texture = 11 perm_snow = yes }
}
"#;

    #[test]
    fn parse_terrain_defs_index_to_type() {
        let d = TerrainDefs::parse(SAMPLE);
        // palette index -> terrain type (from the `terrain` block).
        assert_eq!(d.index_type(0), TerrainType::Plains);
        assert_eq!(d.index_type(1), TerrainType::Forest);
        assert_eq!(d.index_type(2), TerrainType::Hills);
        assert_eq!(d.index_type(3), TerrainType::Desert);
        assert_eq!(d.index_type(13), TerrainType::Urban);
        assert_eq!(d.index_type(14), TerrainType::Lakes);
        assert_eq!(d.index_type(15), TerrainType::Ocean);
        // an index not listed defaults to plains (never invented).
        assert_eq!(d.index_type(200), TerrainType::Plains);
    }

    #[test]
    fn parse_terrain_defs_index_to_atlas_tile() {
        let d = TerrainDefs::parse(SAMPLE);
        // `texture = N` -> the atlas tile index (0..=15), per palette index.
        assert_eq!(d.index_atlas(0), 1); // terrain_0 texture = 1
        assert_eq!(d.index_atlas(1), 4); // terrain_1 texture = 4
        assert_eq!(d.index_atlas(2), 3); // desert_m texture = 3
        assert_eq!(d.index_atlas(3), 9); // desert   texture = 9
        assert_eq!(d.index_atlas(13), 10); // forest_13 texture = 10
                                           // texture = 255 (lakes) is out of the 0..16 atlas range -> NO_ATLAS.
        assert_eq!(d.index_atlas(14), NO_ATLAS);
        // an index not listed -> NO_ATLAS (never an invented tile).
        assert_eq!(d.index_atlas(200), NO_ATLAS);
    }

    /// `perm_snow = yes` is parsed per palette index; others are false.
    #[test]
    fn parse_terrain_defs_perm_snow() {
        let d = TerrainDefs::parse(SAMPLE);
        assert!(d.index_perm_snow(16)); // snow_16 perm_snow = yes
        assert!(!d.index_perm_snow(0)); // terrain_0 has none
        assert!(!d.index_perm_snow(3)); // desert has none
        assert!(!d.index_perm_snow(200)); // unlisted -> false
    }

    /// `rotate_vector_by_vector(v, up=+Y)` with the game's convention: v.y is the up
    /// component, so rotating a straight-up tangent normal by an up axis preserves up.
    #[test]
    fn rotate_normal_basis_preserves_up() {
        // A flat tangent-space normal (0,1,0) rotated by world-up (0,1,0) stays ~up.
        let r = rotate_vector_by_vector([0.0, 1.0, 0.0], [0.0, 1.0, 0.0]);
        let n = normalize3(r);
        assert!(n[1] > 0.99, "up preserved {n:?}");
        // and the result is unit length.
        let len = (r[0] * r[0] + r[1] * r[1] + r[2] * r[2]).sqrt();
        assert!((len - 1.0).abs() < 1e-3, "unit length {len}");
    }

    /// The colourmap sampler: bilinear, pixel-centre, clamp-to-edge — the v2 base layer.
    #[test]
    fn colormap_sample_corners_and_centre() {
        // 2x2 colourmap: TL=red, TR=green, BL=blue, BR=white (top-left origin).
        let rgba = vec![
            255, 0, 0, 255, /* TL */ 0, 255, 0, 255, /* TR */
            0, 0, 255, 255, /* BL */ 255, 255, 255, 255, /* BR */
        ];
        let cm = ColorMap::from_rgba(2, 2, rgba);
        // pixel-centre UVs land exactly on each texel centre (clamped): corners exact.
        assert_eq!(cm.sample(0.25, 0.25), [255, 0, 0]); // TL centre
        assert_eq!(cm.sample(0.75, 0.25), [0, 255, 0]); // TR centre
        assert_eq!(cm.sample(0.25, 0.75), [0, 0, 255]); // BL centre
        assert_eq!(cm.sample(0.75, 0.75), [255, 255, 255]); // BR centre
                                                            // the exact centre (0.5,0.5) bilerps all four equally -> ~[128,128,128].
        let mid = cm.sample(0.5, 0.5);
        for c in mid {
            assert!((120..=135).contains(&c), "centre bilerp {mid:?}");
        }
        // out-of-the-grid UV clamps to an edge texel (never panics / wraps).
        assert_eq!(cm.sample(0.0, 0.0), [255, 0, 0]); // clamps to TL
        assert_eq!(cm.sample(1.0, 1.0), [255, 255, 255]); // clamps to BR
    }

    /// The atlas detail modulation is MEAN-PRESERVING: a flat (single-colour) tile yields
    /// a factor of exactly 1.0 everywhere (no brightness shift), and a varying tile yields
    /// factors that straddle 1.0. This is the no-overwhelm / no-colour-shift guarantee.
    #[test]
    fn atlas_detail_is_mean_preserving() {
        // 8x8 atlas (tile = 2x2, 4x4 grid). Tile 0 is flat grey -> factor == 1.0.
        let w = 8u32;
        let h = 8u32;
        let mut rgba = vec![0u8; (w * h * 4) as usize];
        for px in rgba.chunks_exact_mut(4) {
            px.copy_from_slice(&[128, 128, 128, 255]);
        }
        let a = Atlas::from_rgba(w, h, rgba);
        for &(tu, tv) in &[(0.0f32, 0.0f32), (0.3, 0.7), (0.9, 0.1)] {
            let f = a.detail_factor(0, tu, tv, 0.15);
            assert!((f - 1.0).abs() < 1e-3, "flat tile factor {f} != 1.0");
        }
        // an out-of-range tile index is a no-op (factor 1.0), never a panic.
        assert_eq!(a.detail_factor(99, 0.5, 0.5, 0.15), 1.0);
    }

    /// A varying atlas tile produces a non-trivial modulation that stays bounded and
    /// centred near 1.0 (averaged over the tile), so it grains the base without shifting
    /// its mean colour.
    #[test]
    fn atlas_detail_varies_but_stays_centred() {
        // 4x4 atlas (tile = 1x1, so each tile is a single texel). Make tile 0's single
        // texel bright and tile-grid means consistent: use a 8x8 with a checker tile 0.
        let w = 8u32;
        let h = 8u32;
        let mut rgba = vec![0u8; (w * h * 4) as usize];
        // tile 0 occupies x:0..2, y:0..2. Checker: (0,0)&(1,1)=white, others=black.
        let set = |rgba: &mut [u8], x: u32, y: u32, v: u8| {
            let o = ((y * w + x) * 4) as usize;
            rgba[o..o + 4].copy_from_slice(&[v, v, v, 255]);
        };
        set(&mut rgba, 0, 0, 255);
        set(&mut rgba, 1, 1, 255);
        // (1,0) and (0,1) stay black.
        let a = Atlas::from_rgba(w, h, rgba);
        let bright = a.detail_factor(0, 0.0, 0.0, 0.5); // white texel -> > 1
        let dark = a.detail_factor(0, 0.5, 0.0, 0.5); // black texel -> < 1
        assert!(bright > 1.0, "bright {bright}");
        assert!(dark < 1.0, "dark {dark}");
        // bounded
        assert!((0.4..=1.6).contains(&bright) && (0.4..=1.6).contains(&dark));
    }

    #[test]
    fn type_colors_from_categories() {
        let d = TerrainDefs::parse(SAMPLE);
        assert_eq!(d.type_color(TerrainType::Forest), [89, 199, 85]);
        assert_eq!(d.type_color(TerrainType::Plains), [255, 129, 66]);
        assert_eq!(d.type_color(TerrainType::Urban), [120, 120, 120]);
        assert_eq!(d.type_color(TerrainType::Mountain), [157, 192, 208]);
    }

    #[test]
    fn palette_maps_water_indices_to_water_color() {
        let d = TerrainDefs::parse(SAMPLE);
        let p = d.palette();
        // index 15 = ocean -> WATER_COLOR; index 14 = lakes -> LAKE_COLOR.
        assert_eq!(p[15], WATER_COLOR);
        assert_eq!(p[14], LAKE_COLOR);
        // index 1 = forest -> the forest category colour.
        assert_eq!(p[1], [89, 199, 85]);
    }

    #[test]
    fn water_type_flag() {
        assert!(TerrainType::Ocean.is_water());
        assert!(TerrainType::Lakes.is_water());
        assert!(!TerrainType::Plains.is_water());
        assert!(!TerrainType::Forest.is_water());
    }

    #[test]
    fn river_index_classification() {
        // 0/1/2 = land + markers, 3..=11 = river, 12+/254/255 = not a river.
        assert!(!is_river_index(0));
        assert!(!is_river_index(1));
        assert!(!is_river_index(2));
        assert!(is_river_index(3));
        assert!(is_river_index(6));
        assert!(is_river_index(11));
        assert!(!is_river_index(12));
        assert!(!is_river_index(254));
        assert!(!is_river_index(255));
    }

    /// Color-space round-trip: srgb_to_linear ∘ linear_to_srgb8 is (near) identity, and
    /// the curve uses the GAME's gamma (pow 2.2 / 0.45), not the sRGB piecewise curve.
    #[test]
    fn color_space_roundtrip_and_curve() {
        for c in [0u8, 1, 50, 128, 200, 255] {
            let back = linear_to_srgb8(srgb_to_linear(c));
            assert!(
                (back as i32 - c as i32).abs() <= 1,
                "roundtrip {c} -> {back}"
            );
        }
        // mid-grey 188 ≈ linear 0.5 under gamma 2.2 (the game's curve): (188/255)^2.2 ≈ 0.5.
        let mid = srgb_to_linear(188);
        assert!((mid - 0.5).abs() < 0.03, "gamma-2.2 mid {mid}");
    }

    /// `get_overlay` matches the game's GetOverlay: pct=0 is a pure pass-through of the
    /// base; and the overlay-blend IDENTITY point is when the overlay is exactly 0.5 in
    /// GAMMA space (overlay-linear = 0.5^2.2): there `2·0.5·cg = cg`, so the gamma result
    /// equals the base's gamma → the linear result equals the base. (Confirms the math is
    /// done in gamma space with the game's exact curve.)
    #[test]
    fn get_overlay_identity_cases() {
        let base = [0.2f32, 0.5, 0.8];
        // pct = 0 -> base unchanged.
        let z = get_overlay(base, [0.9, 0.1, 0.4], 0.0);
        for i in 0..3 {
            assert!((z[i] - base[i]).abs() < 1e-5, "pct0 {z:?}");
        }
        // overlay = exactly gamma-0.5 (= linear 0.5^2.2) is the overlay-blend identity:
        // full-strength overlay leaves the base unchanged.
        let g = 0.5f32.powf(gc::GAMMA_DECODE); // gamma 0.5 in linear space
        let m = get_overlay(base, [g, g, g], 1.0);
        // within the gamma round-trip error (pow(pow(x,0.45),2.2) is not bit-exact).
        for i in 0..3 {
            assert!(
                (m[i] - base[i]).abs() < 0.01,
                "gamma-0.5 overlay identity {m:?} vs {base:?}"
            );
        }
        // a brighter overlay (gamma 0.75) LIGHTENS the base (screen side of overlay).
        let b = 0.75f32.powf(gc::GAMMA_DECODE);
        let lighter = get_overlay(base, [b, b, b], 1.0);
        for i in 0..3 {
            assert!(
                lighter[i] >= base[i] - 1e-4,
                "brighter overlay lightens {lighter:?}"
            );
        }
    }

    /// HSV→linear-RGB for a pure-value grey (S=0): R=G=B=V^2.2; and the documented sun
    /// HSV {0.14,0,1} (S=0 → white V=1) decodes to linear white.
    #[test]
    fn hsv_to_linear_grey_and_sun() {
        let grey = hsv_to_linear_rgb([0.0, 0.0, 0.5]);
        let expect = 0.5f32.powf(gc::GAMMA_DECODE);
        for c in grey {
            assert!((c - expect).abs() < 1e-4, "grey {grey:?}");
        }
        let sun = hsv_to_linear_rgb(gc::SUN_DIFFUSE_COLOR_HSV); // S=0 -> white
        for c in sun {
            assert!((c - 1.0).abs() < 1e-4, "sun white {sun:?}");
        }
    }

    /// `ambient_light` picks the +face for a positive-axis normal and the -face for a
    /// negative one, weighted by the squared component (sums to the face for a pure axis).
    #[test]
    fn ambient_light_axis_selection() {
        // 6 distinct faces.
        let amb = [
            [0.1, 0.0, 0.0], // +X
            [0.0, 0.1, 0.0], // -X
            [0.2, 0.2, 0.2], // +Y
            [0.3, 0.0, 0.0], // -Y
            [0.0, 0.0, 0.4], // +Z
            [0.0, 0.4, 0.0], // -Z
        ];
        // normal = +Z -> picks the +Z face exactly (sq.z = 1).
        let c = ambient_light([0.0, 0.0, 1.0], &amb);
        assert!((c[2] - 0.4).abs() < 1e-6 && c[0] < 1e-6, "ambient +Z {c:?}");
        // normal = -X -> picks the -X face.
        let c = ambient_light([-1.0, 0.0, 0.0], &amb);
        assert!((c[1] - 0.1).abs() < 1e-6, "ambient -X {c:?}");
    }

    #[test]
    fn indexed_bmp_roundtrip_bottom_up() {
        // Build a tiny 2x2 8-bit BMP (bottom-up) with a 40-byte header and verify the
        // rows are de-flipped to top-left origin. Palette is irrelevant to indices.
        let w = 2u32;
        let h = 2u32;
        let stride = ((w as usize) + 3) & !3; // = 4
        let palette_bytes = 256 * 4;
        let dataoff = 14 + 40 + palette_bytes;
        let mut data = vec![0u8; dataoff + stride * (h as usize)];
        data[0..2].copy_from_slice(b"BM");
        data[10..14].copy_from_slice(&(dataoff as u32).to_le_bytes());
        data[14..18].copy_from_slice(&40u32.to_le_bytes());
        data[18..22].copy_from_slice(&(w as i32).to_le_bytes());
        data[22..26].copy_from_slice(&(h as i32).to_le_bytes()); // positive => bottom-up
        data[28..30].copy_from_slice(&8u16.to_le_bytes());
        // bottom row (stored first) = [10, 11], top row (stored second) = [20, 21]
        data[dataoff] = 10;
        data[dataoff + 1] = 11;
        data[dataoff + stride] = 20;
        data[dataoff + stride + 1] = 21;
        let bmp = IndexedBmp::parse(&data).unwrap();
        assert_eq!(bmp.width, 2);
        assert_eq!(bmp.height, 2);
        // top-left origin: output row 0 = the BMP's TOP row = [20,21].
        assert_eq!(&bmp.indices[0..2], &[20, 21]);
        assert_eq!(&bmp.indices[2..4], &[10, 11]);
    }
}
