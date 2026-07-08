//! the game bitmap font loading for replay-app overlay text.
//!
//! Grounding (UI_SOURCE_OF_TRUTH_REPORT: fonts resolve through
//! `interface/core.gfx` / `replay_interface/game_assets/fonts.json`): a font
//! NAME is resolved through the committed `fonts.json` map to its `.fnt`
//! file(s) under the real game install, and the glyph atlas pages are the
//! game's own textures.
//!
//! `game_ui` owns BMFont parsing and DDS/TGA page decoding. This module
//! converts the decoded pages into the replay renderer's white-RGB,
//! coverage-alpha atlas format before the D3D overlay uploads them.

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

use game_ui::assets::normalize_rel_path;
use game_ui::dds::{self, RgbaImage};
use game_ui::tga;
use map_viewer::GameFileResolver;

use crate::Result;

/// The font the overlay renders with.
///
/// The in-game clock/date readout (`interface/ingameclock.gui` ->
/// `time_text`, extracted to `game_assets/gui/ingame_clock.json`) uses this
/// bitmap font. Its pages ship as DDS, so loading goes through `game_ui`.
pub const OVERLAY_FONT_NAME: &str = "hoi_18mbs";

/// Fallback font when the real overlay font cannot be loaded.
///
/// This is the former overlay proxy: a real core.gfx font
/// (`gfx/fonts/garamond_16`, TGA-backed) resolved through fonts.json. It is
/// logged as a fallback whenever it becomes active.
pub const OVERLAY_PROXY_FONT_NAME: &str = "ToolTip_Font";

const DEFAULT_VANILLA_GAME_DIR: &str = r"X:\SteamLibrary\steamapps\common\Hearts of Iron IV";

/// One glyph from the BMFont `char` table. Coordinates are texels in the
/// owning page; offsets/advance are pixels relative to the pen position at
/// the line top (BMFont semantics).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Glyph {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
    pub xoffset: i32,
    pub yoffset: i32,
    pub xadvance: i32,
    /// Index into [`Font::pages`] (already offset for multi-.fnt fonts).
    pub page: usize,
}

/// A decoded glyph atlas page: RGBA8, rows top-down.
#[derive(Clone)]
pub struct FontPage {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
    /// SHIPPED mip levels 1.. (each `(width, height, rgba)`), decoded from the
    /// font DDS's own mip chain. The engine samples the map-label atlas with
    /// trilinear filtering + a -0.4 LOD bias (`gfx/FX/mapname.shader`
    /// Samplers; frame4700 binds `hoi_mapfont4` with mips=4), so mid-zoom
    /// titles blend mips 1-2 — uploading mip 0 alone rendered them sharper
    /// and darker-cored than the engine. Empty for TGA pages / compressed
    /// chains (single-mip upload, the pre-mip behaviour).
    pub mips: Vec<(u32, u32, Vec<u8>)>,
}

impl std::fmt::Debug for FontPage {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Compact by hand: the pixel plane is large.
        formatter
            .debug_struct("FontPage")
            .field("width", &self.width)
            .field("height", &self.height)
            .field("rgba_len", &self.rgba.len())
            .field("mip_levels", &self.mips.len())
            .finish()
    }
}

/// How a font's atlas pages encode glyph pixels, deciding how
/// [`coverage_atlas_rgba`] converts them for the overlay shader.
///
/// the game map/UI fonts (`hoi_mapfont4`, `hoi_18mbs`) ship **pre-coloured**
/// atlases: each glyph is a full-colour image (dark body + light rim baked in)
/// whose ALPHA channel carries coverage (verified by decoding the DDS — glyph
/// interior `rgb≈black, a≈0.5`, edge `rgb≈white, a≈0.3`, background `a=0`). The
/// game's `gfx/FX/mapname.shader` draws that atlas straight (`tex2D` then
/// `a *= transparency`, src_alpha/inv_src_alpha), so the correct replay of a map
/// label preserves the atlas RGBA rather than collapsing it to a white coverage
/// mask. Coverage mode is the older single-channel behaviour, kept byte-for-byte
/// for callers that opt out (the default overlay path).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AtlasMode {
    /// White RGB, alpha = extracted glyph coverage (legacy overlay behaviour).
    Coverage,
    /// Native atlas RGBA passed straight through (pre-coloured, outlined glyphs).
    PreservedColor,
}

/// A loaded game font: merged glyph/kerning tables plus decoded pages.
#[derive(Clone)]
pub struct Font {
    pub name: String,
    /// How the atlas pages were converted (see [`AtlasMode`]).
    pub atlas_mode: AtlasMode,
    /// Line height in pixels (BMFont `common lineHeight`).
    pub line_height: i32,
    /// Baseline offset from the line top (BMFont `common base`). Layout
    /// anchors at the line top so this is unused today; kept because it is
    /// real glyph-metric data future parity work (e.g. baseline-aligned
    /// mixed fonts) needs.
    #[cfg_attr(not(test), allow(dead_code))]
    pub base: i32,
    /// Default text color from `fonts.json` (`0xAARRGGBB`).
    pub default_color: u32,
    glyphs: HashMap<u32, Glyph>,
    kernings: HashMap<(u32, u32), i32>,
    pub pages: Vec<FontPage>,
}

impl std::fmt::Debug for Font {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Compact by hand: the page pixel planes are large.
        formatter
            .debug_struct("Font")
            .field("name", &self.name)
            .field("line_height", &self.line_height)
            .field("base", &self.base)
            .field(
                "default_color",
                &format_args!("{:#010x}", self.default_color),
            )
            .field("glyph_count", &self.glyphs.len())
            .field("kerning_count", &self.kernings.len())
            .field("page_count", &self.pages.len())
            .finish()
    }
}

/// One positioned glyph quad: screen-pixel rect + normalized UV rect.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GlyphQuad {
    pub page: usize,
    pub x0: f32,
    pub y0: f32,
    pub x1: f32,
    pub y1: f32,
    pub u0: f32,
    pub v0: f32,
    pub u1: f32,
    pub v1: f32,
}

/// Result of laying out one line of text.
pub struct TextLayout {
    pub quads: Vec<GlyphQuad>,
    /// Characters with no glyph in the font. The font's missing-glyph
    /// behavior is skip-and-record: no advance, no substitute box.
    pub missing: Vec<char>,
    /// Total pen advance in pixels (kerning included).
    pub advance: f32,
}

/// Font NAME -> files/color, resolved from the committed `fonts.json`
/// (extracted from `interface/core.gfx`). Language overrides are ignored:
/// the replay surface renders the English-default font set.
#[derive(Clone, Debug, PartialEq)]
pub struct FontSpec {
    pub name: String,
    /// Game-relative `.fnt` paths without extension (e.g.
    /// "gfx/fonts/garamond_16").
    pub files: Vec<String>,
    /// `0xAARRGGBB` default text color from core.gfx.
    pub color: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OverlayFontKind {
    Real,
    Proxy,
}

#[derive(Clone, Debug)]
pub struct OverlayFontSelection {
    pub font: Font,
    pub font_name: &'static str,
    pub kind: OverlayFontKind,
    pub root: PathBuf,
    pub fallback_reason: Option<String>,
}

/// The committed font map, relative to this crate (same convention as the
/// default bundle dir): `replay_interface/game_assets/fonts.json`.
pub fn default_fonts_json_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("game_assets")
        .join("fonts.json")
}

/// Resolve `font_name` through the committed fonts.json and load it from
/// the game install at `game_root` with a vanilla-only resolver.
pub fn load_game_font(game_root: &Path, font_name: &str) -> Result<Font> {
    let resolver = GameFileResolver::vanilla(game_root.to_path_buf())?;
    load_game_font_from_resolver(&resolver, font_name)
}

pub fn load_game_font_from_resolver(resolver: &GameFileResolver, font_name: &str) -> Result<Font> {
    load_game_font_with_mode(resolver, font_name, AtlasMode::Coverage)
}

/// Load a font preserving its atlas's native RGBA (see [`AtlasMode`]). Used by
/// the map-label path so pre-coloured, outlined map glyphs render like the
/// game's `mapname` effect instead of as a flat coverage mask. The default
/// overlay path keeps [`AtlasMode::Coverage`] so its dumps stay byte-identical.
pub fn load_game_font_preserving_color(
    resolver: &GameFileResolver,
    font_name: &str,
) -> Result<Font> {
    load_game_font_with_mode(resolver, font_name, AtlasMode::PreservedColor)
}

fn load_game_font_with_mode(
    resolver: &GameFileResolver,
    font_name: &str,
    atlas_mode: AtlasMode,
) -> Result<Font> {
    let fonts_json = default_fonts_json_path();
    let bytes = fs::read(&fonts_json)
        .map_err(|err| format!("missing or unreadable {}: {err}", fonts_json.display()))?;
    let spec = font_spec_from_json(&bytes, font_name, &fonts_json.display().to_string())?;
    Font::load_from_resolver_with_mode(resolver, &spec, atlas_mode)
}

pub fn load_overlay_font(resolver: Option<&GameFileResolver>) -> Result<OverlayFontSelection> {
    if let Some(resolver) = resolver {
        match load_game_font_from_resolver(resolver, OVERLAY_FONT_NAME) {
            Ok(font) => {
                return Ok(OverlayFontSelection {
                    font,
                    font_name: OVERLAY_FONT_NAME,
                    kind: OverlayFontKind::Real,
                    root: resolver.game_dir().to_path_buf(),
                    fallback_reason: None,
                });
            }
            Err(err) => {
                let reason = format!("failed to load real font {OVERLAY_FONT_NAME:?}: {err}");
                return load_proxy_overlay_font(resolver, Some(reason));
            }
        }
    }

    let Some(root) = default_vanilla_game_dir_if_present() else {
        return Err(format!(
            "no game install given; pass --game <the game install dir> or set BATTLE_PLAN_VANILLA; \
             proxy fallback root {DEFAULT_VANILLA_GAME_DIR:?} is not present"
        )
        .into());
    };
    let resolver = GameFileResolver::vanilla(root)?;
    load_proxy_overlay_font(
        &resolver,
        Some("no game install given; using the TGA proxy fallback".to_string()),
    )
}

fn load_proxy_overlay_font(
    resolver: &GameFileResolver,
    fallback_reason: Option<String>,
) -> Result<OverlayFontSelection> {
    let font = load_game_font_from_resolver(resolver, OVERLAY_PROXY_FONT_NAME).map_err(|err| {
        format!(
            "failed to load proxy font {OVERLAY_PROXY_FONT_NAME:?} from {}: {err}",
            resolver.game_dir().display()
        )
    })?;
    Ok(OverlayFontSelection {
        font,
        font_name: OVERLAY_PROXY_FONT_NAME,
        kind: OverlayFontKind::Proxy,
        root: resolver.game_dir().to_path_buf(),
        fallback_reason,
    })
}

fn default_vanilla_game_dir_if_present() -> Option<PathBuf> {
    let root = PathBuf::from(DEFAULT_VANILLA_GAME_DIR);
    root.is_dir().then_some(root)
}

/// Parse a fonts.json document and pull out one font's spec.
pub fn font_spec_from_json(bytes: &[u8], font_name: &str, source: &str) -> Result<FontSpec> {
    let value: serde_json::Value = serde_json::from_slice(bytes)
        .map_err(|err| format!("{source} is not valid JSON: {err}"))?;
    let fonts = value
        .get("fonts")
        .and_then(serde_json::Value::as_object)
        .ok_or_else(|| format!("{source} has no \"fonts\" object"))?;
    let entry = fonts
        .get(font_name)
        .ok_or_else(|| format!("font {font_name:?} is not defined in {source}"))?;
    let path = entry.get("path").and_then(serde_json::Value::as_str);
    let fontfiles: Vec<String> = entry
        .get("fontfiles")
        .and_then(serde_json::Value::as_array)
        .map(|values| {
            values
                .iter()
                .filter_map(serde_json::Value::as_str)
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default();
    // core.gfx fonts declare either a single `path` or a `fontfiles` list
    // (latin + cyrillic pieces merged into one glyph table).
    let files = match path {
        Some(path) => vec![path.to_string()],
        None => fontfiles,
    };
    if files.is_empty() {
        return Err(format!("font {font_name:?} in {source} declares no font files").into());
    }
    let color_text = entry
        .get("color")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| format!("font {font_name:?} in {source} has no color string"))?;
    let color = parse_argb_color(color_text)
        .map_err(|err| format!("font {font_name:?} in {source}: {err}"))?;
    Ok(FontSpec {
        name: font_name.to_string(),
        files,
        color,
    })
}

/// Parse a core.gfx `0xAARRGGBB` color string.
fn parse_argb_color(text: &str) -> Result<u32> {
    let hex = text
        .strip_prefix("0x")
        .or_else(|| text.strip_prefix("0X"))
        .ok_or_else(|| format!("color {text:?} does not start with 0x"))?;
    if hex.len() != 8 {
        return Err(format!("color {text:?} is not 8 hex digits (AARRGGBB)").into());
    }
    u32::from_str_radix(hex, 16)
        .map_err(|err| format!("color {text:?} is not hexadecimal: {err}").into())
}

impl Font {
    /// Load and merge the spec's `.fnt` files through `game_ui` (first file
    /// wins for duplicated glyph ids / kerning pairs, matching a
    /// latin-first merge). The mod-aware resolver supplies the BMFont and
    /// page file paths; `game_ui` supplies DDS/TGA page decoding.
    pub fn load(game_root: &Path, spec: &FontSpec) -> Result<Font> {
        let resolver = GameFileResolver::vanilla(game_root.to_path_buf())?;
        Self::load_from_resolver(&resolver, spec)
    }

    pub fn load_from_resolver(resolver: &GameFileResolver, spec: &FontSpec) -> Result<Font> {
        Self::load_from_resolver_with_mode(resolver, spec, AtlasMode::Coverage)
    }

    pub fn load_from_resolver_with_mode(
        resolver: &GameFileResolver,
        spec: &FontSpec,
        atlas_mode: AtlasMode,
    ) -> Result<Font> {
        let mut glyphs: HashMap<u32, Glyph> = HashMap::new();
        let mut kernings: HashMap<(u32, u32), i32> = HashMap::new();
        let mut pages: Vec<FontPage> = Vec::new();
        let mut line_height: Option<i32> = None;
        let mut base: Option<i32> = None;

        for relative in &spec.files {
            let font_name = game_ui_font_name(relative)?;
            let bitmap = load_bitmap_font_from_resolver(resolver, &font_name).map_err(|err| {
                format!(
                    "font {:?}: failed to load {} through game resolver: {err}",
                    spec.name, relative
                )
            })?;
            let page_base = pages.len();
            let mut page_indices: HashMap<u32, usize> = HashMap::new();
            for page in &bitmap.pages {
                let page_index = page_base + page_indices.len();
                page_indices.insert(page.id, page_index);
                let mut mips = Vec::with_capacity(page.mip_images.len());
                for level in &page.mip_images {
                    mips.push((
                        usize_to_u32(level.width, "font page mip width")?,
                        usize_to_u32(level.height, "font page mip height")?,
                        atlas_rgba(level, bitmap.glyph_channel, atlas_mode),
                    ));
                }
                pages.push(FontPage {
                    width: usize_to_u32(page.image.width, "font page width")?,
                    height: usize_to_u32(page.image.height, "font page height")?,
                    rgba: atlas_rgba(&page.image, bitmap.glyph_channel, atlas_mode),
                    mips,
                });
            }
            for (id, font_char) in &bitmap.chars {
                let declared_page = u32::try_from(font_char.page).map_err(|_| {
                    format!(
                        "{}: char id={id} page {} does not fit in u32",
                        bitmap.source_file, font_char.page
                    )
                })?;
                let page = *page_indices.get(&declared_page).ok_or_else(|| {
                    format!(
                        "{}: char id={id} references missing page {}",
                        bitmap.source_file, declared_page
                    )
                })?;
                let page_image = &pages[page];
                if font_char.x + font_char.width > page_image.width
                    || font_char.y + font_char.height > page_image.height
                {
                    return Err(format!(
                        "{}: char id={id} rect {}x{}+{}+{} exceeds page {} ({}x{})",
                        bitmap.source_file,
                        font_char.width,
                        font_char.height,
                        font_char.x,
                        font_char.y,
                        declared_page,
                        page_image.width,
                        page_image.height
                    )
                    .into());
                }
                let mut glyph = *font_char;
                glyph.page = page;
                glyphs.entry(*id).or_insert(glyph);
            }
            for (pair, amount) in &bitmap.kernings {
                kernings.entry(*pair).or_insert(*amount);
            }
            line_height.get_or_insert(bitmap.line_height);
            base.get_or_insert(bitmap.base);
        }

        Ok(Font {
            name: spec.name.clone(),
            atlas_mode,
            line_height: line_height.ok_or("font has no .fnt files")?,
            base: base.unwrap_or(0),
            default_color: spec.color,
            glyphs,
            kernings,
            pages,
        })
    }

    pub fn glyph_count(&self) -> usize {
        self.glyphs.len()
    }

    #[cfg_attr(not(test), allow(dead_code))]
    pub fn kerning_count(&self) -> usize {
        self.kernings.len()
    }

    /// Capital-letter glyph height in font px — the modal height of the
    /// present `A`..`Z` glyphs, falling back to the line height when the font
    /// has no capitals.
    ///
    /// NAME2: the engine's country-name size law (LAB3, fitted on decoded
    /// engine glyph quads) is expressed in CAP heights — the quad heights of
    /// capital letters — while BMFont's `lineHeight` includes ascent/descent
    /// padding (`hoi_mapfont4`: capitals are 55 px in an 84 px line). Scaling
    /// a label by `world_height / line_height` therefore renders capitals at
    /// only ~65% of the law's size; consumers of the cap-height law must
    /// divide by THIS metric. Derived from the loaded font, so mods with
    /// different metrics are honoured.
    pub fn cap_height(&self) -> i32 {
        let mut counts: HashMap<u32, u32> = HashMap::new();
        for id in b'A'..=b'Z' {
            if let Some(glyph) = self.glyphs.get(&u32::from(id)) {
                if glyph.height > 0 {
                    *counts.entry(glyph.height).or_insert(0) += 1;
                }
            }
        }
        counts
            .into_iter()
            .max_by_key(|&(height, count)| (count, height))
            .map(|(height, _)| height as i32)
            .unwrap_or(self.line_height)
    }

    /// Lay out one line of text with the pen starting at (`origin_x`,
    /// `origin_y`) = the line's top-left. Kerning pairs apply between
    /// consecutive present glyphs; characters without a glyph are skipped
    /// and recorded (never a substitute box).
    pub fn layout_text(&self, origin_x: f32, origin_y: f32, text: &str) -> TextLayout {
        self.layout_text_tracked(origin_x, origin_y, text, 0.0)
    }

    /// [`Self::layout_text`] with uniform additive letter TRACKING: `tracking`
    /// font px are added to the pen after EVERY character slot (spaces
    /// included — a word gap spans two slots and receives two increments).
    ///
    /// NAME2 (ENGINE-MEASURED, session_MAPNAME_7B decoded mapname VBs): the
    /// engine justifies short country names across their baseline exactly this
    /// way — per-gap pen deltas exceed the font xadvance by a CONSTANT per
    /// label (IRAN +61 fnt px/gap, FRANCE +12, a tiny island UNITED KINGDOM
    /// +121 whose across-the-space D->K gap measures +239 ≈ 2 increments,
    /// proving the additive-per-slot form), independent of glyph widths
    /// (additive, not a multiplicative stretch), and never negative (long
    /// names lay out at natural advances, residuals ±2 px).
    pub fn layout_text_tracked(
        &self,
        origin_x: f32,
        origin_y: f32,
        text: &str,
        tracking: f32,
    ) -> TextLayout {
        let mut quads = Vec::new();
        let mut missing = Vec::new();
        let mut pen = origin_x;
        let mut previous: Option<u32> = None;
        for ch in text.chars() {
            // Glyph ids in the game fonts are single-byte ANSI or Unicode code
            // points depending on the export; both agree with `char as u32`
            // for ASCII, which is all the overlay emits today.
            let id = ch as u32;
            let Some(glyph) = self.glyphs.get(&id) else {
                missing.push(ch);
                previous = None;
                continue;
            };
            if let Some(previous) = previous {
                if let Some(amount) = self.kernings.get(&(previous, id)) {
                    pen += *amount as f32;
                }
            }
            if glyph.width > 0 && glyph.height > 0 {
                let page = &self.pages[glyph.page];
                let x0 = pen + glyph.xoffset as f32;
                let y0 = origin_y + glyph.yoffset as f32;
                quads.push(GlyphQuad {
                    page: glyph.page,
                    x0,
                    y0,
                    x1: x0 + glyph.width as f32,
                    y1: y0 + glyph.height as f32,
                    u0: glyph.x as f32 / page.width as f32,
                    v0: glyph.y as f32 / page.height as f32,
                    u1: (glyph.x + glyph.width) as f32 / page.width as f32,
                    v1: (glyph.y + glyph.height) as f32 / page.height as f32,
                });
            }
            pen += glyph.xadvance as f32 + tracking;
            previous = Some(id);
        }
        TextLayout {
            quads,
            missing,
            advance: pen - origin_x,
        }
    }

    /// Measure one line: (total advance, line height) in pixels.
    pub fn measure(&self, text: &str) -> (u32, u32) {
        let layout = self.layout_text(0.0, 0.0, text);
        (
            layout.advance.max(0.0).ceil() as u32,
            self.line_height.max(0) as u32,
        )
    }
}

fn game_ui_font_name(relative: &str) -> Result<String> {
    let normalized = normalize_rel_path(relative);
    let without_ext = normalized.strip_suffix(".fnt").unwrap_or(&normalized);
    let name = without_ext
        .strip_prefix("gfx/fonts/")
        .unwrap_or(without_ext)
        .to_string();
    if name.is_empty() {
        Err(format!("font path {relative:?} does not name a font").into())
    } else {
        Ok(name)
    }
}

/// Convert a decoded atlas page for the overlay shader per [`AtlasMode`].
///
/// * [`AtlasMode::Coverage`] collapses to white RGB + extracted coverage alpha
///   (the legacy overlay behaviour, unchanged so its dumps stay byte-identical).
/// * [`AtlasMode::PreservedColor`] copies the decoded RGBA straight through, so
///   the pre-coloured, outlined map-label glyphs keep the dark body / light rim
///   the game bakes into `hoi_mapfont4` (rendered exactly like `mapname.shader`:
///   sample the texel, modulate alpha by the per-label transparency).
fn atlas_rgba(image: &RgbaImage, glyph_channel: Option<usize>, mode: AtlasMode) -> Vec<u8> {
    match mode {
        AtlasMode::Coverage => coverage_atlas_rgba(image, glyph_channel),
        // `RgbaImage` is already the R,G,B,A byte order the overlay uploads.
        AtlasMode::PreservedColor => image.pixels.clone(),
    }
}

fn coverage_atlas_rgba(image: &RgbaImage, glyph_channel: Option<usize>) -> Vec<u8> {
    let mut out = Vec::with_capacity(image.pixels.len());
    for pixel in image.pixels.chunks_exact(4) {
        let alpha = glyph_coverage(pixel, glyph_channel);
        out.extend_from_slice(&[0xFF, 0xFF, 0xFF, alpha]);
    }
    out
}

fn glyph_coverage(pixel: &[u8], glyph_channel: Option<usize>) -> u8 {
    if let Some(channel) = glyph_channel {
        if channel == 3 {
            return pixel[3];
        }
        return 255 - pixel[channel.min(2)];
    }
    if pixel[3] < 255 {
        pixel[3]
    } else {
        pixel[0].max(pixel[1]).max(pixel[2])
    }
}

fn usize_to_u32(value: usize, what: &str) -> Result<u32> {
    u32::try_from(value).map_err(|_| format!("{what} {value} does not fit in u32").into())
}

#[derive(Debug)]
struct LoadedBitmapPage {
    id: u32,
    image: RgbaImage,
    /// Decoded DDS mip levels 1.. (see [`FontPage::mips`]); empty for TGA.
    mip_images: Vec<RgbaImage>,
}

#[derive(Debug)]
struct LoadedBitmapFont {
    source_file: String,
    line_height: i32,
    base: i32,
    glyph_channel: Option<usize>,
    pages: Vec<LoadedBitmapPage>,
    chars: HashMap<u32, Glyph>,
    kernings: HashMap<(u32, u32), i32>,
}

fn load_bitmap_font_from_resolver(
    resolver: &GameFileResolver,
    name: &str,
) -> Result<LoadedBitmapFont> {
    let rel = format!("gfx/fonts/{name}.fnt");
    let fnt_path = resolver.resolve_required(&rel, "font file")?;
    let bytes = fs::read(&fnt_path).map_err(|err| {
        format!(
            "failed to read font file {rel} resolved to {}: {err}",
            fnt_path.display()
        )
    })?;
    let source_file = fnt_path.display().to_string();
    let parsed = parse_fnt_text(&bytes, &source_file)?;
    let pages = load_bitmap_font_pages_from_resolver(resolver, name, &parsed)?;
    Ok(LoadedBitmapFont {
        source_file,
        line_height: parsed.line_height,
        base: parsed.base,
        glyph_channel: parsed.glyph_channel,
        pages,
        chars: parsed.glyphs.into_iter().collect(),
        kernings: parsed
            .kernings
            .into_iter()
            .map(|(first, second, amount)| ((first, second), amount))
            .collect(),
    })
}

fn load_bitmap_font_pages_from_resolver(
    resolver: &GameFileResolver,
    name: &str,
    parsed: &ParsedFnt,
) -> Result<Vec<LoadedBitmapPage>> {
    let mut pages = Vec::new();
    for (page_id, declared) in parsed.page_files.iter().enumerate() {
        let page = resolve_bitmap_font_page(resolver, name, page_id as u32, declared.as_deref())?;
        pages.push(page);
    }
    Ok(pages)
}

fn resolve_bitmap_font_page(
    resolver: &GameFileResolver,
    name: &str,
    page_id: u32,
    declared: Option<&str>,
) -> Result<LoadedBitmapPage> {
    if let Some(declared) = declared {
        let rel = font_page_declared_rel(declared);
        if let Some(path) = resolver.resolve(&rel)? {
            return decode_resolved_font_page(page_id, rel, path);
        }
    }

    for rel in inferred_font_page_candidates(name, page_id) {
        if let Some(path) = resolver.resolve(&rel)? {
            return decode_resolved_font_page(page_id, rel, path);
        }
    }
    Err(format!("no texture page found for font {name} page {page_id}").into())
}

fn font_page_declared_rel(declared: &str) -> String {
    let rel = normalize_rel_path(declared);
    if rel.contains('/') {
        rel
    } else {
        normalize_rel_path(&format!("gfx/fonts/{rel}"))
    }
}

fn inferred_font_page_candidates(name: &str, page_id: u32) -> [String; 4] {
    [
        format!("gfx/fonts/{name}.dds"),
        format!("gfx/fonts/{name}.tga"),
        format!("gfx/fonts/{name}_{page_id}.dds"),
        format!("gfx/fonts/{name}_{page_id}.tga"),
    ]
}

fn decode_resolved_font_page(
    page_id: u32,
    rel_path: String,
    path: PathBuf,
) -> Result<LoadedBitmapPage> {
    let ext = path
        .extension()
        .and_then(|value| value.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    let (image, mip_images) = match ext.as_str() {
        "dds" => {
            let bytes = std::fs::read(&path).map_err(|err| {
                format!(
                    "failed to read font page {rel_path} resolved to {}: {err}",
                    path.display()
                )
            })?;
            // Prefer the SHIPPED mip chain (see FontPage::mips); compressed
            // chains fall back to the historical mip-0 decode.
            match dds::decode_mip_chain_bytes(&bytes) {
                Ok(mut levels) if !levels.is_empty() => {
                    let image = levels.remove(0);
                    (image, levels)
                }
                _ => {
                    let image = dds::decode_bytes(&bytes)
                        .map_err(|err| {
                            format!(
                                "failed to decode font page {rel_path} resolved to {}: {err}",
                                path.display()
                            )
                        })?
                        .image;
                    (image, Vec::new())
                }
            }
        }
        "tga" => {
            let image = tga::decode_file(&path)
                .map_err(|err| {
                    format!(
                        "failed to decode font page {rel_path} resolved to {}: {err}",
                        path.display()
                    )
                })?
                .image;
            (image, Vec::new())
        }
        _ => {
            return Err(format!(
                "page {page_id} {rel_path} resolved to {} has unsupported texture extension .{ext}",
                path.display()
            )
            .into())
        }
    };
    Ok(LoadedBitmapPage {
        id: page_id,
        image,
        mip_images,
    })
}

/// Parsed contents of one BMFont **text** `.fnt` file.
#[allow(dead_code)]
#[derive(Debug)]
struct ParsedFnt {
    line_height: i32,
    base: i32,
    scale_w: u32,
    scale_h: u32,
    glyph_channel: Option<usize>,
    /// Declared page file names by page id; `None` for pages the file never
    /// declares (garamond-style exports have no `page` lines at all).
    page_files: Vec<Option<String>>,
    glyphs: Vec<(u32, Glyph)>,
    kernings: Vec<(u32, u32, i32)>,
}

/// Parse the AngelCode BMFont text format. the game ships only this format;
/// the binary format ("BMF\x03") fails closed.
fn parse_fnt_text(bytes: &[u8], source: &str) -> Result<ParsedFnt> {
    if bytes.starts_with(b"BMF") {
        return Err(format!(
            "{source} is a binary BMFont file; the game ships the text format and \
             binary decoding is not implemented (fail closed, no substitute)"
        )
        .into());
    }
    // Face names may be non-UTF8 single-byte text; we never need them, so a
    // lossy decode is safe for the numeric fields we do read.
    let text = String::from_utf8_lossy(bytes);

    let mut common: Option<(i32, i32, u32, u32, usize, Option<usize>)> = None;
    let mut page_files: Vec<(u32, String)> = Vec::new();
    let mut glyphs = Vec::new();
    let mut kernings = Vec::new();
    let mut saw_info = false;

    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let (tag, fields) = parse_line_fields(line);
        match tag.as_str() {
            "info" => saw_info = true,
            "common" => {
                common = Some((
                    required_int(&fields, "lineHeight", source)? as i32,
                    optional_int(&fields, "base").unwrap_or(0) as i32,
                    required_int(&fields, "scaleW", source)? as u32,
                    required_int(&fields, "scaleH", source)? as u32,
                    required_int(&fields, "pages", source)? as usize,
                    bmfont_glyph_channel_from_common(&fields),
                ));
            }
            "page" => {
                let id = required_int(&fields, "id", source)? as u32;
                let file = fields
                    .get("file")
                    .cloned()
                    .ok_or_else(|| format!("{source}: page line has no file attribute"))?;
                page_files.push((id, file));
            }
            "char" => {
                let id = required_int(&fields, "id", source)?;
                if id < 0 {
                    // BMFont uses id=-1 for the "missing char" glyph; the
                    // fail-closed policy is skip-and-record, so drop it.
                    continue;
                }
                glyphs.push((
                    id as u32,
                    Glyph {
                        x: required_int(&fields, "x", source)? as u32,
                        y: required_int(&fields, "y", source)? as u32,
                        width: required_int(&fields, "width", source)? as u32,
                        height: required_int(&fields, "height", source)? as u32,
                        xoffset: optional_int(&fields, "xoffset").unwrap_or(0) as i32,
                        yoffset: optional_int(&fields, "yoffset").unwrap_or(0) as i32,
                        xadvance: required_int(&fields, "xadvance", source)? as i32,
                        page: optional_int(&fields, "page").unwrap_or(0) as usize,
                    },
                ));
            }
            "kerning" => {
                kernings.push((
                    required_int(&fields, "first", source)? as u32,
                    required_int(&fields, "second", source)? as u32,
                    required_int(&fields, "amount", source)? as i32,
                ));
            }
            // "chars"/"kernings" count lines and anything unknown: ignored.
            _ => {}
        }
    }

    let (line_height, base, scale_w, scale_h, page_count, glyph_channel) =
        common.ok_or_else(|| format!("{source} has no \"common\" line; not a BMFont text file"))?;
    if !saw_info {
        return Err(format!("{source} has no \"info\" line; not a BMFont text file").into());
    }
    if page_count == 0 {
        return Err(format!("{source} declares pages=0").into());
    }
    let mut files: Vec<Option<String>> = vec![None; page_count];
    for (id, file) in page_files {
        let slot = files
            .get_mut(id as usize)
            .ok_or_else(|| format!("{source}: page id={id} but common declares {page_count}"))?;
        *slot = Some(file);
    }
    Ok(ParsedFnt {
        line_height,
        base,
        scale_w,
        scale_h,
        glyph_channel,
        page_files: files,
        glyphs,
        kernings,
    })
}

fn bmfont_glyph_channel_from_common(fields: &HashMap<String, String>) -> Option<usize> {
    [
        ("alphaChnl", 3usize),
        ("redChnl", 0usize),
        ("greenChnl", 1usize),
        ("blueChnl", 2usize),
    ]
    .into_iter()
    .find_map(|(key, channel)| (optional_int(fields, key) == Some(0)).then_some(channel))
}

/// Split a BMFont line into its tag and key=value fields. Values may be
/// quoted and contain spaces (`face="Century Gothic"`).
fn parse_line_fields(line: &str) -> (String, HashMap<String, String>) {
    let mut tokens: Vec<String> = Vec::new();
    let mut current = String::new();
    let mut in_quotes = false;
    for ch in line.chars() {
        match ch {
            '"' => in_quotes = !in_quotes,
            c if c.is_whitespace() && !in_quotes => {
                if !current.is_empty() {
                    tokens.push(std::mem::take(&mut current));
                }
            }
            c => current.push(c),
        }
    }
    if !current.is_empty() {
        tokens.push(current);
    }
    let mut iter = tokens.into_iter();
    let tag = iter.next().unwrap_or_default();
    let mut fields = HashMap::new();
    for token in iter {
        if let Some((key, value)) = token.split_once('=') {
            fields.insert(key.to_string(), value.to_string());
        }
    }
    (tag, fields)
}

fn optional_int(fields: &HashMap<String, String>, key: &str) -> Option<i64> {
    fields.get(key).and_then(|value| value.parse::<i64>().ok())
}

fn required_int(fields: &HashMap<String, String>, key: &str, source: &str) -> Result<i64> {
    fields
        .get(key)
        .ok_or_else(|| format!("{source}: missing {key}= attribute"))?
        .parse::<i64>()
        .map_err(|err| format!("{source}: {key}= is not an integer: {err}").into())
}

/// Resolve the on-disk texture for one page of `fnt_path`.
///
/// Real-install quirks this must handle (verified): garamond `.fnt` files
/// declare no page file at all, and `cg_16b.fnt` declares `cg_16d.tga`
/// which does not exist while `cg_16b.tga` does — so the declared name is
/// tried first and `<fnt stem>.tga` second. A page that only exists as DDS
/// (e.g. the clock font's `hoi_18mbs_0.dds`) fails closed by name.
#[allow(dead_code)]
fn resolve_page_path(fnt_path: &Path, declared: Option<&str>, page_id: usize) -> Result<PathBuf> {
    let dir = fnt_path.parent().unwrap_or_else(|| Path::new(""));
    let stem = fnt_path
        .file_stem()
        .and_then(|stem| stem.to_str())
        .ok_or_else(|| format!("{} has no file stem", fnt_path.display()))?;
    let mut candidates: Vec<PathBuf> = Vec::new();
    if let Some(declared) = declared {
        candidates.push(dir.join(declared));
    }
    let implicit = dir.join(format!("{stem}.tga"));
    if !candidates.contains(&implicit) {
        candidates.push(implicit);
    }
    for candidate in &candidates {
        if !candidate.is_file() {
            continue;
        }
        let extension = candidate
            .extension()
            .and_then(|ext| ext.to_str())
            .map(str::to_ascii_lowercase)
            .unwrap_or_default();
        match extension.as_str() {
            "tga" => return Ok(candidate.clone()),
            "dds" => {
                return Err(format!(
                    "page {page_id} of {} only ships as DDS ({}); DDS decoding is a later \
                     slice, so this font cannot be rendered yet (fail closed, no substitute)",
                    fnt_path.display(),
                    candidate.display()
                )
                .into())
            }
            _ => {
                return Err(format!(
                    "page {page_id} of {} has unsupported texture format: {}",
                    fnt_path.display(),
                    candidate.display()
                )
                .into())
            }
        }
    }
    // No candidate exists as-is; a DDS twin means "DDS-only font".
    let dds = dir.join(format!("{stem}.dds"));
    if dds.is_file() {
        return Err(format!(
            "page {page_id} of {} only ships as DDS ({}); DDS decoding is a later slice, \
             so this font cannot be rendered yet (fail closed, no substitute)",
            fnt_path.display(),
            dds.display()
        )
        .into());
    }
    Err(format!(
        "no page texture found for page {page_id} of {}: tried {}",
        fnt_path.display(),
        candidates
            .iter()
            .map(|candidate| candidate.display().to_string())
            .collect::<Vec<_>>()
            .join(", ")
    )
    .into())
}

/// Decode a TGA into RGBA8 rows top-down.
///
/// Supports what the real the game font pages are (verified against the
/// install): truecolor type 2 (uncompressed) and type 10 (RLE), 24- or
/// 32-bit, either vertical origin, with the optional TGA v2 footer
/// ("TRUEVISION-XFILE.") ignored. Anything else (colormapped, grayscale
/// type 3) fails closed naming the file.
pub fn decode_tga(bytes: &[u8], source: &str) -> Result<FontPage> {
    if bytes.len() < 18 {
        return Err(format!("{source}: TGA shorter than its 18-byte header").into());
    }
    let id_length = bytes[0] as usize;
    let colormap_type = bytes[1];
    let image_type = bytes[2];
    let width = u32::from(u16::from_le_bytes([bytes[12], bytes[13]]));
    let height = u32::from(u16::from_le_bytes([bytes[14], bytes[15]]));
    let bpp = bytes[16];
    let descriptor = bytes[17];

    if colormap_type != 0 {
        return Err(format!("{source}: colormapped TGA (type {image_type}) is unsupported").into());
    }
    let rle = match image_type {
        2 => false,
        10 => true,
        other => {
            return Err(format!(
                "{source}: TGA image type {other} is unsupported (only truecolor 2/10)"
            )
            .into())
        }
    };
    if bpp != 24 && bpp != 32 {
        return Err(format!("{source}: {bpp}-bit TGA is unsupported (only 24/32)").into());
    }
    if descriptor & 0x10 != 0 {
        return Err(format!("{source}: right-to-left TGA rows are unsupported").into());
    }
    if width == 0 || height == 0 {
        return Err(format!("{source}: TGA has empty dimensions {width}x{height}").into());
    }
    let top_down = descriptor & 0x20 != 0;
    let bytes_per_pixel = usize::from(bpp / 8);
    let pixel_count = width as usize * height as usize;
    let data = &bytes[18 + id_length.min(bytes.len() - 18)..];

    // Decode in file order first (bottom-up files get flipped after).
    let mut rgba = vec![0u8; pixel_count * 4];
    let mut write_pixel = |index: usize, pixel: &[u8]| {
        let out = &mut rgba[index * 4..index * 4 + 4];
        out[0] = pixel[2]; // BGR(A) -> RGBA
        out[1] = pixel[1];
        out[2] = pixel[0];
        out[3] = if bytes_per_pixel == 4 { pixel[3] } else { 0xFF };
    };
    if !rle {
        let needed = pixel_count * bytes_per_pixel;
        if data.len() < needed {
            return Err(format!(
                "{source}: TGA pixel data is {} bytes, expected {needed}",
                data.len()
            )
            .into());
        }
        for index in 0..pixel_count {
            write_pixel(index, &data[index * bytes_per_pixel..]);
        }
    } else {
        let mut offset = 0usize;
        let mut index = 0usize;
        while index < pixel_count {
            let header = *data
                .get(offset)
                .ok_or_else(|| format!("{source}: truncated TGA RLE stream"))?;
            offset += 1;
            let run = usize::from(header & 0x7F) + 1;
            if header & 0x80 != 0 {
                let end = offset + bytes_per_pixel;
                if end > data.len() {
                    return Err(format!("{source}: truncated TGA RLE packet").into());
                }
                let pixel = &data[offset..end];
                for _ in 0..run {
                    if index >= pixel_count {
                        return Err(format!("{source}: TGA RLE overruns the image").into());
                    }
                    write_pixel(index, pixel);
                    index += 1;
                }
                offset = end;
            } else {
                let end = offset + run * bytes_per_pixel;
                if end > data.len() {
                    return Err(format!("{source}: truncated TGA raw packet").into());
                }
                for pixel in data[offset..end].chunks_exact(bytes_per_pixel) {
                    if index >= pixel_count {
                        return Err(format!("{source}: TGA raw packet overruns the image").into());
                    }
                    write_pixel(index, pixel);
                    index += 1;
                }
                offset = end;
            }
        }
    }

    if !top_down {
        let row_bytes = width as usize * 4;
        let mut flipped = vec![0u8; rgba.len()];
        for y in 0..height as usize {
            let src = (height as usize - 1 - y) * row_bytes;
            flipped[y * row_bytes..(y + 1) * row_bytes]
                .copy_from_slice(&rgba[src..src + row_bytes]);
        }
        rgba = flipped;
    }
    Ok(FontPage {
        width,
        height,
        rgba,
        mips: Vec::new(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Synthetic garamond-style .fnt: no page/count lines, ANSI ids,
    /// trailing spaces — the oldest export style the game ships.
    const GARAMOND_STYLE_FNT: &str = "\
info face=\"Test Face\" size=16 bold=0 italic=0 charset=\"ANSI\" stretchH=100 smooth=0 aa=1 padding=2,2,2,2 spacing=1,1
common lineHeight=16 base=12 scaleW=8 scaleH=8 pages=1
char id=32   x=0     y=0     width=1     height=0     xoffset=0     yoffset=16    xadvance=4     page=0
char id=65   x=0     y=0     width=4     height=6     xoffset=1     yoffset=2     xadvance=5     page=0
char id=86   x=4     y=0     width=3     height=6     xoffset=0     yoffset=2     xadvance=4     page=0
kerning first=65  second=86 amount=-2
";

    /// Synthetic cg-style .fnt: page/chars-count lines, quoted face with a
    /// space, unicode ids — the newer export style.
    const CG_STYLE_FNT: &str = "\
info face=\"Century Gothic\" size=16 bold=1 italic=0 charset=\"\" unicode=1 stretchH=100 smooth=1 aa=2 padding=3,3,3,3 spacing=1,1 outline=0
common lineHeight=16 base=13 scaleW=8 scaleH=8 pages=1 packed=0 alphaChnl=1 redChnl=0 greenChnl=0 blueChnl=0
page id=0 file=\"declared_page.tga\"
chars count=1
char id=66 x=1 y=2 width=3 height=4 xoffset=0 yoffset=1 xadvance=4 page=0 chnl=15
kernings count=0
";

    fn synthetic_page() -> FontPage {
        FontPage {
            width: 8,
            height: 8,
            rgba: vec![0xFF; 8 * 8 * 4],
            mips: Vec::new(),
        }
    }

    fn font_from(parsed: ParsedFnt, color: u32) -> Font {
        Font {
            name: "test".to_string(),
            atlas_mode: AtlasMode::Coverage,
            line_height: parsed.line_height,
            base: parsed.base,
            default_color: color,
            glyphs: parsed
                .glyphs
                .into_iter()
                .map(|(id, glyph)| (id, Glyph { ..glyph }))
                .collect(),
            kernings: parsed
                .kernings
                .into_iter()
                .map(|(first, second, amount)| ((first, second), amount))
                .collect(),
            pages: vec![synthetic_page()],
        }
    }

    /// A 2x2 test image as (x, y) -> RGBA: distinct corner colors.
    fn test_pixels() -> [[u8; 4]; 4] {
        [
            [255, 0, 0, 255],  // (0,0) top-left red
            [0, 255, 0, 128],  // (1,0) top-right green
            [0, 0, 255, 255],  // (0,1) bottom-left blue
            [255, 255, 0, 64], // (1,1) bottom-right yellow
        ]
    }

    fn tga_header(image_type: u8, bpp: u8, descriptor: u8) -> Vec<u8> {
        let mut header = vec![0u8; 18];
        header[2] = image_type;
        header[12] = 2; // width 2
        header[14] = 2; // height 2
        header[16] = bpp;
        header[17] = descriptor;
        header
    }

    fn write_font_tga(path: &Path, rgba: [u8; 4]) {
        let mut tga = vec![0u8; 18];
        tga[2] = 2;
        tga[12] = 8;
        tga[14] = 8;
        tga[16] = 32;
        tga[17] = 0x20;
        for _ in 0..(8 * 8) {
            tga.extend_from_slice(&[rgba[2], rgba[1], rgba[0], rgba[3]]);
        }
        fs::create_dir_all(path.parent().expect("font tga parent")).expect("create fonts dir");
        fs::write(path, tga).expect("write tga");
    }

    fn write_font(root: &Path, name: &str, text: &str, page_rgba: [u8; 4]) {
        let fonts_dir = root.join("gfx").join("fonts");
        fs::create_dir_all(&fonts_dir).expect("create fonts dir");
        fs::write(fonts_dir.join(format!("{name}.fnt")), text).expect("write fnt");
        write_font_tga(&fonts_dir.join(format!("{name}.tga")), page_rgba);
    }

    #[test]
    fn fnt_text_parses_garamond_style_without_page_lines() {
        let parsed = parse_fnt_text(GARAMOND_STYLE_FNT.as_bytes(), "test.fnt").expect("parses");
        assert_eq!(parsed.line_height, 16);
        assert_eq!(parsed.base, 12);
        assert_eq!(parsed.scale_w, 8);
        assert_eq!(parsed.page_files, vec![None]);
        assert_eq!(parsed.glyphs.len(), 3);
        let (id, a) = &parsed.glyphs[1];
        assert_eq!(*id, 65);
        assert_eq!(
            *a,
            Glyph {
                x: 0,
                y: 0,
                width: 4,
                height: 6,
                xoffset: 1,
                yoffset: 2,
                xadvance: 5,
                page: 0,
            }
        );
        assert_eq!(parsed.kernings, vec![(65, 86, -2)]);
    }

    #[test]
    fn fnt_text_parses_cg_style_with_page_and_count_lines() {
        let parsed = parse_fnt_text(CG_STYLE_FNT.as_bytes(), "test.fnt").expect("parses");
        assert_eq!(
            parsed.page_files,
            vec![Some("declared_page.tga".to_string())]
        );
        assert_eq!(parsed.glyphs.len(), 1);
        assert_eq!(parsed.glyphs[0].0, 66);
        assert!(parsed.kernings.is_empty());
    }

    #[test]
    fn fnt_binary_format_fails_closed() {
        let err = parse_fnt_text(b"BMF\x03\x00\x00\x00", "bin.fnt").expect_err("must fail");
        assert!(err.to_string().contains("binary BMFont"), "{err}");
    }

    #[test]
    fn fnt_without_common_line_fails_closed() {
        let err = parse_fnt_text(b"info face=\"x\" size=16", "bad.fnt").expect_err("must fail");
        assert!(err.to_string().contains("common"), "{err}");
    }

    #[test]
    fn tga_decodes_32bit_uncompressed_bottom_up() {
        let pixels = test_pixels();
        // Bottom-up file order: bottom row first.
        let mut bytes = tga_header(2, 32, 0x08);
        for [r, g, b, a] in [pixels[2], pixels[3], pixels[0], pixels[1]] {
            bytes.extend_from_slice(&[b, g, r, a]); // BGRA on disk
        }
        let page = decode_tga(&bytes, "test.tga").expect("decodes");
        assert_eq!((page.width, page.height), (2, 2));
        // Output is top-down RGBA.
        assert_eq!(&page.rgba[0..4], &pixels[0]);
        assert_eq!(&page.rgba[4..8], &pixels[1]);
        assert_eq!(&page.rgba[8..12], &pixels[2]);
        assert_eq!(&page.rgba[12..16], &pixels[3]);
    }

    #[test]
    fn tga_decodes_24bit_uncompressed_top_down() {
        let mut bytes = tga_header(2, 24, 0x20);
        for [r, g, b, _] in test_pixels() {
            bytes.extend_from_slice(&[b, g, r]);
        }
        let page = decode_tga(&bytes, "test.tga").expect("decodes");
        for (index, [r, g, b, _]) in test_pixels().iter().enumerate() {
            assert_eq!(
                &page.rgba[index * 4..index * 4 + 4],
                &[*r, *g, *b, 0xFF],
                "pixel {index}"
            );
        }
    }

    #[test]
    fn tga_decodes_32bit_rle_and_ignores_v2_footer() {
        let pixels = test_pixels();
        let mut bytes = tga_header(10, 32, 0x20);
        // RLE packet: (0,0) repeated twice, then a raw packet of 2 pixels.
        bytes.push(0x81);
        bytes.extend_from_slice(&[pixels[0][2], pixels[0][1], pixels[0][0], pixels[0][3]]);
        bytes.push(0x01);
        for [r, g, b, a] in [pixels[2], pixels[3]] {
            bytes.extend_from_slice(&[b, g, r, a]);
        }
        // TGA v2 footer (present on the real garamond pages): ignored.
        bytes.extend_from_slice(&[0u8; 8]);
        bytes.extend_from_slice(b"TRUEVISION-XFILE.\0");
        let page = decode_tga(&bytes, "test.tga").expect("decodes");
        assert_eq!(&page.rgba[0..4], &pixels[0]);
        assert_eq!(&page.rgba[4..8], &pixels[0]);
        assert_eq!(&page.rgba[8..12], &pixels[2]);
        assert_eq!(&page.rgba[12..16], &pixels[3]);
    }

    #[test]
    fn tga_rejects_grayscale_type_closed() {
        let bytes = tga_header(3, 8, 0x20);
        let err = decode_tga(&bytes, "gray.tga").expect_err("must fail");
        assert!(err.to_string().contains("image type 3"), "{err}");
    }

    #[test]
    fn tga_rejects_truncated_pixel_data() {
        let mut bytes = tga_header(2, 32, 0x20);
        bytes.extend_from_slice(&[0u8; 7]); // less than 2x2x4
        let err = decode_tga(&bytes, "short.tga").expect_err("must fail");
        assert!(err.to_string().contains("expected 16"), "{err}");
    }

    #[test]
    fn tga_rejects_truncated_rle_stream() {
        let mut bytes = tga_header(10, 32, 0x20);
        bytes.push(0x83); // RLE run of 4 but no pixel follows
        let err = decode_tga(&bytes, "short_rle.tga").expect_err("must fail");
        assert!(err.to_string().contains("truncated"), "{err}");
    }

    /// A tiny pre-coloured atlas like `hoi_mapfont4`: glyph interior is dark
    /// (rgb≈0) with high coverage alpha, edge is light (rgb≈255) with partial
    /// alpha, background has zero alpha. Two pixels are enough to pin the modes.
    fn precolored_page() -> RgbaImage {
        RgbaImage {
            width: 2,
            height: 1,
            // pixel 0: dark glyph body (rgb 10, a 200); pixel 1: transparent bg
            // (rgb 140, a 0) — the grey the old "255-R" law turned into a box.
            pixels: vec![10, 10, 10, 200, 140, 140, 140, 0],
        }
    }

    #[test]
    fn preserved_color_mode_passes_atlas_rgba_through() {
        let page = precolored_page();
        let out = atlas_rgba(&page, Some(0), AtlasMode::PreservedColor);
        // Native RGBA preserved verbatim: the dark body keeps its colour and
        // coverage, and the zero-alpha background stays fully transparent (no
        // black box).
        assert_eq!(out, vec![10, 10, 10, 200, 140, 140, 140, 0]);
    }

    #[test]
    fn coverage_mode_is_unchanged_for_the_overlay_path() {
        // The legacy coverage law (redChnl=0 -> "255 - R") must stay bit-for-bit
        // so the overlay clock dumps remain byte-identical; this documents the
        // exact bug the map path avoids (the transparent grey background becomes
        // a semi-opaque box: 255 - 140 = 115).
        let page = precolored_page();
        let out = atlas_rgba(&page, Some(0), AtlasMode::Coverage);
        assert_eq!(out, vec![0xFF, 0xFF, 0xFF, 245, 0xFF, 0xFF, 0xFF, 115]);
    }

    const FONTS_JSON_FIXTURE: &[u8] = br#"{
        "fonts": {
            "PathFont": { "name": "PathFont", "path": "gfx/fonts/one", "fontfiles": [], "color": "0xffffffff" },
            "ListFont": { "name": "ListFont", "path": null, "fontfiles": ["gfx/fonts/a", "gfx/fonts/b"], "color": "0xff7799ff" },
            "BadColor": { "name": "BadColor", "path": "gfx/fonts/one", "fontfiles": [], "color": "0x4ffffffff" },
            "NoFiles": { "name": "NoFiles", "path": null, "fontfiles": [], "color": "0xffffffff" }
        }
    }"#;

    #[test]
    fn font_spec_resolves_single_path_fonts() {
        let spec = font_spec_from_json(FONTS_JSON_FIXTURE, "PathFont", "fixture").expect("spec");
        assert_eq!(spec.files, vec!["gfx/fonts/one".to_string()]);
        assert_eq!(spec.color, 0xFFFF_FFFF);
    }

    #[test]
    fn font_spec_resolves_fontfiles_lists() {
        let spec = font_spec_from_json(FONTS_JSON_FIXTURE, "ListFont", "fixture").expect("spec");
        assert_eq!(
            spec.files,
            vec!["gfx/fonts/a".to_string(), "gfx/fonts/b".to_string()]
        );
        assert_eq!(spec.color, 0xFF77_99FF);
    }

    #[test]
    fn font_spec_unknown_name_fails_closed() {
        let err = font_spec_from_json(FONTS_JSON_FIXTURE, "Nope", "fixture").expect_err("fails");
        assert!(err.to_string().contains("\"Nope\""), "{err}");
    }

    #[test]
    fn font_spec_malformed_color_fails_closed() {
        // fonts.json really contains a 9-digit color ("0x4ffffffff" on
        // hoi_22chat); requesting such a font must fail loudly, not guess.
        let err =
            font_spec_from_json(FONTS_JSON_FIXTURE, "BadColor", "fixture").expect_err("fails");
        assert!(err.to_string().contains("8 hex digits"), "{err}");
    }

    #[test]
    fn font_spec_without_files_fails_closed() {
        let err = font_spec_from_json(FONTS_JSON_FIXTURE, "NoFiles", "fixture").expect_err("fails");
        assert!(err.to_string().contains("no font files"), "{err}");
    }

    #[test]
    fn committed_fonts_json_resolves_the_overlay_font() {
        // The committed game_assets/fonts.json must keep resolving the
        // font the overlay uses (no game install required for this).
        let path = default_fonts_json_path();
        let bytes = fs::read(&path).expect("committed fonts.json exists");
        let spec = font_spec_from_json(&bytes, OVERLAY_FONT_NAME, "fonts.json")
            .expect("overlay font resolves");
        assert_eq!(
            spec.files,
            vec![
                "gfx/fonts/hoi_18mbs".to_string(),
                "gfx/fonts/hoi_18mbs_cryllic".to_string()
            ]
        );
        assert_eq!(spec.color, 0xFFFF_FFFF);
    }

    #[test]
    fn committed_fonts_json_resolves_the_proxy_font() {
        let path = default_fonts_json_path();
        let bytes = fs::read(&path).expect("committed fonts.json exists");
        let spec = font_spec_from_json(&bytes, OVERLAY_PROXY_FONT_NAME, "fonts.json")
            .expect("proxy font resolves");
        assert_eq!(spec.files, vec!["gfx/fonts/garamond_16".to_string()]);
        assert_eq!(spec.color, 0xFFFF_FFFF);
    }

    #[test]
    fn measure_applies_advances_and_kerning() {
        let parsed = parse_fnt_text(GARAMOND_STYLE_FNT.as_bytes(), "test.fnt").expect("parses");
        let font = font_from(parsed, 0xFFFF_FFFF);
        // "AV": 5 (A advance) - 2 (kerning) + 4 (V advance) = 7.
        assert_eq!(font.measure("AV"), (7, 16));
        // Kerning does not apply across a missing glyph.
        assert_eq!(font.measure("A V"), (5 + 4 + 4, 16));
        assert_eq!(font.measure(""), (0, 16));
    }

    #[test]
    fn layout_skips_and_records_missing_glyphs() {
        let parsed = parse_fnt_text(GARAMOND_STYLE_FNT.as_bytes(), "test.fnt").expect("parses");
        let font = font_from(parsed, 0xFFFF_FFFF);
        let layout = font.layout_text(0.0, 0.0, "AZV");
        // Z has no glyph: skipped with no advance, recorded, and it breaks
        // the A-V kerning pair (pen = 5 + 4 = 9).
        assert_eq!(layout.missing, vec!['Z']);
        assert_eq!(layout.quads.len(), 2);
        assert_eq!(layout.advance, 9.0);
    }

    #[test]
    fn cap_height_is_the_modal_capital_glyph_height() {
        let parsed = parse_fnt_text(GARAMOND_STYLE_FNT.as_bytes(), "test.fnt").expect("parses");
        let font = font_from(parsed, 0xFFFF_FFFF);
        // A (65) and V (86) both have height 6 -> cap height 6, NOT the
        // 16 px line height (the NAME2 engine size law is in cap heights).
        assert_eq!(font.cap_height(), 6);
    }

    #[test]
    fn tracked_layout_adds_uniform_advance_per_character_slot() {
        // NAME2 engine law shape: uniform ADDITIVE tracking per character
        // slot, spaces included (the captured across-the-space gap measures
        // two increments).
        let parsed = parse_fnt_text(GARAMOND_STYLE_FNT.as_bytes(), "test.fnt").expect("parses");
        let font = font_from(parsed, 0xFFFF_FFFF);
        let layout = font.layout_text_tracked(0.0, 0.0, "A AV", 10.0);
        // Pen: A(5)+10 | space(4)+10 | A(5)+10 kern(A,V)-2 | V(4)+10.
        assert_eq!(layout.quads.len(), 3);
        let a2 = &layout.quads[1];
        // Second A pen = 5+10+4+10 = 29; xoffset 1 -> x0 = 30.
        assert_eq!(a2.x0, 30.0);
        let v = &layout.quads[2];
        // V pen = 29+5+10-2 = 42; xoffset 0 -> x0 = 42.
        assert_eq!(v.x0, 42.0);
        assert_eq!(layout.advance, 5.0 + 4.0 + 5.0 + 4.0 + 4.0 * 10.0 - 2.0);
        // tracking = 0 reproduces the natural layout exactly.
        let natural = font.layout_text(0.0, 0.0, "A AV");
        let tracked_zero = font.layout_text_tracked(0.0, 0.0, "A AV", 0.0);
        assert_eq!(natural.advance, tracked_zero.advance);
        assert_eq!(natural.quads.len(), tracked_zero.quads.len());
    }

    #[test]
    fn layout_positions_quads_with_offsets_and_uvs() {
        let parsed = parse_fnt_text(GARAMOND_STYLE_FNT.as_bytes(), "test.fnt").expect("parses");
        let font = font_from(parsed, 0xFFFF_FFFF);
        let layout = font.layout_text(10.0, 20.0, "AV");
        let a = &layout.quads[0];
        assert_eq!((a.x0, a.y0, a.x1, a.y1), (11.0, 22.0, 15.0, 28.0));
        assert_eq!((a.u0, a.v0, a.u1, a.v1), (0.0, 0.0, 0.5, 0.75));
        let v = &layout.quads[1];
        // Pen after A = 10+5, kerning -2, xoffset 0 -> x0 = 13.
        assert_eq!((v.x0, v.y0), (13.0, 22.0));
        // Space glyph (width 1, height 0) produces no quad but advances.
        let with_space = font.layout_text(0.0, 0.0, "A A");
        assert_eq!(with_space.quads.len(), 2);
        assert_eq!(with_space.advance, 5.0 + 4.0 + 5.0);
    }

    #[test]
    fn load_merges_fnt_files_and_decodes_tga_pages() {
        let dir = crate::bundle::tests::unique_temp_dir("font_load");
        let fonts_dir = dir.join("gfx").join("fonts");
        fs::create_dir_all(&fonts_dir).expect("create fonts dir");
        fs::write(fonts_dir.join("testfont.fnt"), GARAMOND_STYLE_FNT).expect("write fnt");
        // 8x8 32-bit top-down TGA, all white.
        let mut tga = vec![0u8; 18];
        tga[2] = 2;
        tga[12] = 8;
        tga[14] = 8;
        tga[16] = 32;
        tga[17] = 0x20;
        tga.extend(std::iter::repeat(0xFFu8).take(8 * 8 * 4));
        fs::write(fonts_dir.join("testfont.tga"), &tga).expect("write tga");
        let spec = FontSpec {
            name: "testfont".to_string(),
            files: vec!["gfx/fonts/testfont".to_string()],
            color: 0xFF11_2233,
        };
        let font = Font::load(&dir, &spec).expect("font loads");
        assert_eq!(font.line_height, 16);
        assert_eq!(font.default_color, 0xFF11_2233);
        assert_eq!(font.pages.len(), 1);
        assert_eq!(font.measure("AV"), (7, 16));
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn load_reports_invalid_dds_pages() {
        let dir = crate::bundle::tests::unique_temp_dir("font_dds");
        let fonts_dir = dir.join("gfx").join("fonts");
        fs::create_dir_all(&fonts_dir).expect("create fonts dir");
        fs::write(fonts_dir.join("ddsfont.fnt"), GARAMOND_STYLE_FNT).expect("write fnt");
        fs::write(fonts_dir.join("ddsfont.dds"), b"DDS fake").expect("write dds");
        let spec = FontSpec {
            name: "ddsfont".to_string(),
            files: vec!["gfx/fonts/ddsfont".to_string()],
            color: 0xFFFF_FFFF,
        };
        let err = Font::load(&dir, &spec).expect_err("invalid DDS must fail closed");
        let message = err.to_string();
        assert!(message.contains("ddsfont.dds"), "{message}");
        assert!(message.contains("DDS"), "{message}");
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn load_falls_back_to_stem_tga_when_declared_page_is_missing() {
        // Real-install case: cg_16b.fnt declares cg_16d.tga which does not
        // exist; the shipped texture is cg_16b.tga.
        let dir = crate::bundle::tests::unique_temp_dir("font_fallback");
        let fonts_dir = dir.join("gfx").join("fonts");
        fs::create_dir_all(&fonts_dir).expect("create fonts dir");
        fs::write(fonts_dir.join("cgfont.fnt"), CG_STYLE_FNT).expect("write fnt");
        let mut tga = vec![0u8; 18];
        tga[2] = 2;
        tga[12] = 8;
        tga[14] = 8;
        tga[16] = 32;
        tga[17] = 0x20;
        tga.extend(std::iter::repeat(0xFFu8).take(8 * 8 * 4));
        fs::write(fonts_dir.join("cgfont.tga"), &tga).expect("write tga");
        let spec = FontSpec {
            name: "cgfont".to_string(),
            files: vec!["gfx/fonts/cgfont".to_string()],
            color: 0xFFFF_FFFF,
        };
        let font = Font::load(&dir, &spec).expect("falls back to stem tga");
        assert_eq!(font.pages.len(), 1);
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn load_from_resolver_uses_mod_fnt_override() {
        let dir = crate::bundle::tests::unique_temp_dir("font_mod_fnt");
        let vanilla = dir.join("vanilla");
        let mod_root = dir.join("mod");
        write_font(
            &vanilla,
            "testfont",
            GARAMOND_STYLE_FNT,
            [255, 255, 255, 255],
        );
        let mod_fnt = GARAMOND_STYLE_FNT.replace("common lineHeight=16", "common lineHeight=22");
        write_font(&mod_root, "testfont", &mod_fnt, [255, 255, 255, 255]);
        let resolver = GameFileResolver::with_mod_paths(vanilla, vec![mod_root]).expect("resolver");
        let spec = FontSpec {
            name: "testfont".to_string(),
            files: vec!["gfx/fonts/testfont".to_string()],
            color: 0xFFFF_FFFF,
        };

        let font = Font::load_from_resolver(&resolver, &spec).expect("font loads");

        assert_eq!(font.line_height, 22);
        assert_eq!(resolver.stats().overridden_file_count, 2);
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn load_from_resolver_uses_mod_page_override_with_vanilla_fnt() {
        let dir = crate::bundle::tests::unique_temp_dir("font_mod_page");
        let vanilla = dir.join("vanilla");
        let mod_root = dir.join("mod");
        write_font(
            &vanilla,
            "testfont",
            GARAMOND_STYLE_FNT,
            [255, 255, 255, 255],
        );
        write_font_tga(
            &mod_root.join("gfx").join("fonts").join("testfont.tga"),
            [255, 255, 255, 64],
        );
        let resolver = GameFileResolver::with_mod_paths(vanilla, vec![mod_root]).expect("resolver");
        let spec = FontSpec {
            name: "testfont".to_string(),
            files: vec!["gfx/fonts/testfont".to_string()],
            color: 0xFFFF_FFFF,
        };

        let font = Font::load_from_resolver(&resolver, &spec).expect("font loads");

        assert_eq!(&font.pages[0].rgba[0..4], &[0xFF, 0xFF, 0xFF, 64]);
        assert_eq!(resolver.stats().overridden_file_count, 1);
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn load_from_resolver_bad_mod_fnt_names_mod_file_without_vanilla_fallback() {
        let dir = crate::bundle::tests::unique_temp_dir("font_bad_mod");
        let vanilla = dir.join("vanilla");
        let mod_root = dir.join("mod");
        write_font(
            &vanilla,
            "testfont",
            GARAMOND_STYLE_FNT,
            [255, 255, 255, 255],
        );
        let mod_fonts = mod_root.join("gfx").join("fonts");
        fs::create_dir_all(&mod_fonts).expect("create mod fonts dir");
        fs::write(mod_fonts.join("testfont.fnt"), "not a font").expect("write bad fnt");
        let resolver =
            GameFileResolver::with_mod_paths(vanilla, vec![mod_root.clone()]).expect("resolver");
        let spec = FontSpec {
            name: "testfont".to_string(),
            files: vec!["gfx/fonts/testfont".to_string()],
            color: 0xFFFF_FFFF,
        };

        let err = Font::load_from_resolver(&resolver, &spec).expect_err("bad mod fails");
        let message = err.to_string();

        assert!(
            message.contains(&mod_root.display().to_string()),
            "{message}"
        );
        assert!(message.contains("testfont.fnt"), "{message}");
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn hoi_18mbs_loads_from_real_install_when_present() {
        let Some(root) = real_install_root_if_present() else {
            eprintln!("skipping real-install font test: {DEFAULT_VANILLA_GAME_DIR} not present");
            return;
        };
        let font = load_game_font(&root, OVERLAY_FONT_NAME).expect("hoi_18mbs loads");
        assert_eq!(font.name, OVERLAY_FONT_NAME);
        assert_eq!(font.line_height, 18);
        assert_eq!(font.base, 15);
        assert!(font.glyph_count() > 0);
        assert_eq!(
            font.kerning_count(),
            real_install_kerning_count(&root, &["hoi_18mbs", "hoi_18mbs_cryllic"])
        );
        assert!(!font.pages.is_empty());
        assert!(
            font.pages.iter().all(|page| page.width > 0
                && page.height > 0
                && page.rgba.len() == page.width as usize * page.height as usize * 4),
            "{:?}",
            font.pages
        );
        assert!(
            font.pages
                .iter()
                .any(|page| page.rgba.chunks_exact(4).any(|pixel| pixel[3] > 0)),
            "decoded pages contain no glyph coverage"
        );

        let missing_ascii = missing_printable_ascii(&font);
        assert!(missing_ascii.is_empty(), "missing ASCII: {missing_ascii}");
        for byte in 32u8..=126 {
            let glyph = font.glyphs.get(&u32::from(byte)).expect("ASCII glyph");
            let page = font.pages.get(glyph.page).expect("decoded glyph page");
            assert!(
                glyph.x + glyph.width <= page.width,
                "glyph {byte} exceeds page width"
            );
            assert!(
                glyph.y + glyph.height <= page.height,
                "glyph {byte} exceeds page height"
            );
        }

        let (width, height) = font.measure("9 Nov 1943, 17:00");
        assert!(width > 0 && height == 18, "{width}x{height}");
    }

    #[test]
    fn overlay_font_without_game_dir_uses_proxy_when_default_vanilla_exists() {
        let Some(_root) = real_install_root_if_present() else {
            eprintln!("skipping proxy fallback test: {DEFAULT_VANILLA_GAME_DIR} not present");
            return;
        };
        let selection = load_overlay_font(None).expect("proxy fallback loads");
        assert_eq!(selection.font_name, OVERLAY_PROXY_FONT_NAME);
        assert_eq!(selection.kind, OverlayFontKind::Proxy);
        assert_eq!(selection.font.name, OVERLAY_PROXY_FONT_NAME);
        assert!(
            selection
                .fallback_reason
                .as_deref()
                .unwrap_or_default()
                .contains("no game install"),
            "{:?}",
            selection.fallback_reason
        );
        assert!(selection.font.glyph_count() > 0);
        assert!(!selection.font.pages.is_empty());
    }

    fn real_install_root_if_present() -> Option<PathBuf> {
        let root = PathBuf::from(DEFAULT_VANILLA_GAME_DIR);
        root.join("gfx")
            .join("fonts")
            .join("hoi_18mbs.fnt")
            .is_file()
            .then_some(root)
    }

    fn missing_printable_ascii(font: &Font) -> String {
        (32u8..=126)
            .filter(|byte| !font.glyphs.contains_key(&u32::from(*byte)))
            .map(char::from)
            .collect()
    }

    fn real_install_kerning_count(root: &Path, font_stems: &[&str]) -> usize {
        font_stems
            .iter()
            .map(|stem| {
                let path = root.join("gfx").join("fonts").join(format!("{stem}.fnt"));
                let text = fs::read_to_string(&path)
                    .unwrap_or_else(|err| panic!("read {}: {err}", path.display()));
                text.lines()
                    .filter(|line| line.trim_start().starts_with("kerning "))
                    .count()
            })
            .sum()
    }
}
