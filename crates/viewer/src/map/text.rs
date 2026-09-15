//! Bitmap fonts and the screen-space text layout the map's labels use.
//!
//! The fonts are the ones `interface/core.gfx` declares: `tahoma_60`
//! (`gfx/fonts/hoi_mapfont4`, the country-name font), `hoi_20bs` (the
//! victory-point and capital labels), `Main_14` (unit-counter counts) and
//! `hoi_16mbs` (base counts). `gfx/fonts` is outside the coverage policy, so
//! [`installed_content::fonts::load_font`] reads the `.fnt` and its page through
//! `ResolvedLayers::read_uncovered` (`RENDERING.md` section 4).
//!
//! # No code-side outline
//!
//! R2 sections 4.6 and 5.3: `hoi_20bs` and `tahoma_60` declare `color` but no
//! `border_color`, and `mapname.shader` applies neither a vertex nor a constant
//! colour. The dark edge around on-map text is baked into the atlas page — the
//! `A` box of `hoi_mapfont4` is a black body at alpha 128 with a white halo at
//! alpha 70, and the `B` box of `hoi_20bs` is a white body at alpha 255 with a
//! black rim. Nothing here adds an outline. Screen-space text multiplies the
//! authored page by the definition's `color`; world-space `mapname.shader`
//! ignores that definition colour and samples the RGBA page directly.
//!
//! # Colour codes
//!
//! Localisation keeps the game's `§Y…§!` runs (`RENDERING.md` section 4), and
//! the engine strips them when it lays text out. [`strip_color_codes`] does the
//! same and keeps the text. The per-code colours in the `textcolors` blocks are
//! not applied: every map label the viewer draws is a plain name, and applying
//! them would need a per-glyph colour the batch does not carry yet.

use std::{collections::BTreeMap, io};

use installed_content::{
    ResolvedLayers,
    fonts::{self, BitmapFont, FontDefinition},
    image::dds,
};

use super::names::{GlyphBox, GlyphMetrics};

/// `interface/core.gfx:1678-1688`: the map-name font, `gfx/fonts/hoi_mapfont4`.
pub const MAP_FONT: &str = "tahoma_60";
/// `core.gfx:1232-1247`: the victory-point and capital label font.
pub const LABEL_FONT: &str = "hoi_20bs";
/// `core.gfx:870-875`: `gfx/fonts/garamond_14`, the unit-counter count font.
pub const COUNTER_FONT: &str = "Main_14";
/// `core.gfx:1093-1111`: the base and wing count font.
pub const BASE_FONT: &str = "hoi_16mbs";

/// The language the viewer resolves localisation in.
///
/// The replay records no language setting, so the reference profile's English
/// is used. [`installed_content::localisation::read`] takes the bare name, which is
/// what a file's `l_english:` header carries after its `l_` prefix.
pub const LANGUAGE: &str = "english";

/// The same language as a `bitmapfont_override` block spells it.
pub const FONT_LANGUAGE: &str = "l_english";

/// The `§` a colour run opens with, and `§!` closes with.
const COLOR_CODE: char = '\u{a7}';

/// One loaded bitmap font: its metrics, its page textures and its colour.
pub struct Font {
    /// The `interface/*.gfx` name, such as `hoi_20bs`.
    pub name: String,
    /// The stem the `.fnt` was loaded from, for reporting.
    pub stem: String,
    font: BitmapFont,
    /// Page texture ids, in `.fnt` page order; see [`super::sprites::MapIcons`].
    pages: Vec<usize>,
    /// Atlas size the `.fnt` declares, which its glyph boxes are measured in.
    atlas: [f32; 2],
    /// The definition's `color`, straight RGBA; white when it declares none.
    color: [f32; 4],
}

impl Font {
    /// The page texture id a glyph of `ch` is drawn from.
    pub fn page_of(&self, ch: char) -> Option<usize> {
        let glyph = self.font.glyphs.get(&(ch as u32))?;
        self.pages.get(glyph.page as usize).copied()
    }

    /// The first page texture id; every shipped map font has exactly one page.
    pub fn first_page(&self) -> Option<usize> {
        self.pages.first().copied()
    }

    /// The definition's colour as straight RGBA in 0..1.
    pub fn color(&self) -> [f32; 4] {
        self.color
    }

    /// The `.fnt` `lineHeight`.
    pub fn line_height(&self) -> f32 {
        self.font.line_height as f32
    }

    /// Page count, for reporting.
    pub fn page_count(&self) -> usize {
        self.pages.len()
    }

    /// Glyph count, for reporting.
    pub fn glyph_count(&self) -> usize {
        self.font.glyphs.len()
    }

    /// One line naming the font and the `.fnt` header it was loaded from.
    pub fn summary(&self) -> String {
        format!(
            "{} ({}): lineHeight {}, base {}, {} glyphs on {} page(s)",
            self.name,
            self.stem,
            self.line_height(),
            GlyphMetrics::base(self),
            self.glyph_count(),
            self.page_count()
        )
    }
}

impl GlyphMetrics for Font {
    fn advance(&self, ch: char) -> Option<f32> {
        self.font
            .glyphs
            .get(&(ch as u32))
            .map(|glyph| glyph.xadvance as f32)
    }

    fn line_height(&self) -> f32 {
        self.font.line_height as f32
    }

    fn base(&self) -> f32 {
        self.font.base as f32
    }

    fn glyph(&self, ch: char) -> Option<GlyphBox> {
        let glyph = self.font.glyphs.get(&(ch as u32))?;
        Some(GlyphBox {
            width: glyph.width as f32,
            height: glyph.height as f32,
            xoffset: glyph.xoffset as f32,
            yoffset: glyph.yoffset as f32,
            xadvance: glyph.xadvance as f32,
            uv: [
                glyph.x as f32 / self.atlas[0],
                glyph.y as f32 / self.atlas[1],
                (glyph.x + glyph.width) as f32 / self.atlas[0],
                (glyph.y + glyph.height) as f32 / self.atlas[1],
            ],
        })
    }

    fn kerning(&self, a: char, b: char) -> f32 {
        self.font
            .kernings
            .get(&(a as u32, b as u32))
            .map_or(0.0, |amount| *amount as f32)
    }
}

/// Loads one named font and appends its pages to `images`.
///
/// Returns a named error rather than panicking when `interface/*.gfx` declares
/// no such font; a missing `.fnt` or page is reported by the loader itself.
pub fn load(
    layers: &ResolvedLayers,
    definitions: &BTreeMap<String, FontDefinition>,
    name: &str,
    images: &mut Vec<dds::Texture>,
) -> io::Result<Font> {
    let definition = definitions
        .get(name)
        .ok_or_else(|| invalid(format!("interface/*.gfx declares no bitmapfont {name:?}")))?;
    // `files_for` picks the language override; English keeps the base chain.
    let stem = definition
        .files_for(FONT_LANGUAGE)
        .first()
        .cloned()
        .ok_or_else(|| invalid(format!("bitmapfont {name:?} names no font file")))?;
    let loaded = fonts::load_font_file(layers, &stem)?;
    if loaded.font.glyphs.is_empty() {
        return Err(invalid(format!("{stem}.fnt declares no characters")));
    }
    let mut pages = Vec::with_capacity(loaded.pages.len());
    for page in &loaded.pages {
        pages.push(images.len());
        images.push(page.texture.clone());
    }
    let color = definition
        .color
        .as_ref()
        .and_then(fonts::FontColor::rgba)
        .map_or([1.0; 4], |rgba| rgba.map(|channel| channel as f32 / 255.0));
    let atlas = [
        nonzero(
            loaded.font.scale.0,
            loaded.pages.first().map(|p| p.texture.width),
        ),
        nonzero(
            loaded.font.scale.1,
            loaded.pages.first().map(|p| p.texture.height),
        ),
    ];
    Ok(Font {
        name: name.to_owned(),
        stem: loaded.stem,
        font: loaded.font,
        pages,
        atlas,
        color,
    })
}

/// The `.fnt` atlas size, falling back to the page texture when it is zero.
fn nonzero(declared: u32, page: Option<u32>) -> f32 {
    match (declared, page) {
        (0, Some(size)) if size > 0 => size as f32,
        (0, _) => 1.0,
        (size, _) => size as f32,
    }
}

/// The four fonts the map passes draw with.
pub struct Fonts {
    /// `tahoma_60` = `hoi_mapfont4`, the country and state name font.
    pub map: Font,
    /// `hoi_20bs`, the victory-point and capital label font.
    pub label: Font,
    /// `Main_14`, the unit-counter count font.
    pub counter: Font,
    /// `hoi_16mbs`, the air and naval base count font.
    pub base: Font,
}

impl Fonts {
    /// Loads all four, appending their pages to `images`.
    pub fn load(layers: &ResolvedLayers, images: &mut Vec<dds::Texture>) -> io::Result<Self> {
        let definitions = fonts::read_definitions(layers)?;
        Ok(Self {
            map: load(layers, &definitions, MAP_FONT, images)?,
            label: load(layers, &definitions, LABEL_FONT, images)?,
            counter: load(layers, &definitions, COUNTER_FONT, images)?,
            base: load(layers, &definitions, BASE_FONT, images)?,
        })
    }

    /// One summary line per font, in load order.
    pub fn summary(&self) -> Vec<String> {
        [&self.map, &self.label, &self.counter, &self.base]
            .into_iter()
            .map(Font::summary)
            .collect()
    }
}

/// Horizontal alignment, as an `instantTextboxType`'s `format` declares it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Align {
    #[default]
    Left,
    Centre,
    Right,
}

impl Align {
    /// Reads a `.gui` `format` value; an absent or unknown word stays left.
    ///
    /// The shipped files write `left`, `centre`, `center` and `right`, quoted
    /// or bare, in either case.
    pub fn parse(format: Option<&str>) -> Self {
        match format.map(str::to_ascii_lowercase).as_deref() {
            Some("centre" | "center") => Align::Centre,
            Some("right") => Align::Right,
            _ => Align::Left,
        }
    }
}

/// One laid-out glyph in device pixels.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TextGlyph {
    /// Page texture id the quad samples.
    pub page: usize,
    /// `x, y, width, height` in device pixels, `y` down from the top.
    pub rect: [f32; 4],
    /// Normalised atlas rectangle `[u0, v0, u1, v1]`.
    pub uv: [f32; 4],
}

/// Strips the game's `§X…§!` colour runs, keeping the text between them.
///
/// `§` followed by any one character opens a run and `§!` closes it; a trailing
/// `§` with nothing after it is dropped. The engine's own layout does the same
/// before it measures, so the width this module reports matches the engine's.
pub fn strip_color_codes(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars();
    while let Some(ch) = chars.next() {
        if ch == COLOR_CODE {
            // Both the opening `§X` and the closing `§!` consume one character.
            chars.next();
        } else {
            out.push(ch);
        }
    }
    out
}

/// Width of `text` in font pixels, with the colour codes stripped.
///
/// Characters the font has no glyph for are dropped, which is what
/// [`super::names::place_name`] does, so a measured width and a laid-out width
/// always agree.
pub fn measure(font: &Font, text: &str) -> f32 {
    let mut width = 0.0f32;
    let mut previous: Option<char> = None;
    for ch in strip_color_codes(text).chars() {
        let Some(advance) = GlyphMetrics::advance(font, ch) else {
            continue;
        };
        if let Some(p) = previous {
            width += GlyphMetrics::kerning(font, p, ch);
        }
        width += advance;
        previous = Some(ch);
    }
    width
}

/// Lays one line out at `origin`, the top-left of the text box in device pixels.
///
/// `max_width` is the `.gui` `maxWidth`, which is both the box the run is
/// aligned inside and the cut: a glyph whose advance would carry the pen past
/// it is dropped, together with everything after it. The engine ellipsises
/// instead, but no label the viewer draws reaches the limit (the widest
/// `VICTORY_POINTS_*` string is far inside `hoi_20bs`'s 149 px box), so a plain
/// cut is enough and is stated here rather than guessed at.
///
/// With no `maxWidth` there is no box, so a centred run is centred on `origin`
/// itself and a right-aligned one ends there.
///
/// Glyph `y` follows BMFont: `yoffset` is measured from the top of the line
/// box, which is `origin.y`, not from the baseline.
pub fn layout(
    font: &Font,
    text: &str,
    origin: [f32; 2],
    align: Align,
    max_width: Option<f32>,
) -> Vec<TextGlyph> {
    let stripped = strip_color_codes(text);
    let width = measure(font, &stripped);
    let limit = max_width.unwrap_or(f32::INFINITY);
    // The slack the run has inside its box, never negative: an over-long run
    // starts at the box's left edge and is cut at its right one.
    let slack = (max_width.unwrap_or(0.0) - width).max(0.0);
    let start = match align {
        Align::Left => origin[0],
        Align::Centre => origin[0] + slack * 0.5,
        Align::Right => origin[0] + slack,
    };
    let mut out = Vec::new();
    let mut pen = 0.0f32;
    let mut previous: Option<char> = None;
    for ch in stripped.chars() {
        let Some(advance) = GlyphMetrics::advance(font, ch) else {
            continue;
        };
        if let Some(p) = previous {
            pen += GlyphMetrics::kerning(font, p, ch);
        }
        if pen + advance > limit {
            break;
        }
        if let (Some(glyph), Some(page)) = (GlyphMetrics::glyph(font, ch), font.page_of(ch))
            && glyph.width > 0.0
            && glyph.height > 0.0
        {
            out.push(TextGlyph {
                page,
                rect: [
                    start + pen + glyph.xoffset,
                    origin[1] + glyph.yoffset,
                    glyph.width,
                    glyph.height,
                ],
                uv: glyph.uv,
            });
        }
        pen += advance;
        previous = Some(ch);
    }
    out
}

fn invalid(message: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message.into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use installed_content::fonts::Glyph;

    /// A synthetic three-character font: `A` and `B` 10 wide, space 4 wide.
    fn font() -> Font {
        let mut glyphs = BTreeMap::new();
        glyphs.insert(
            'A' as u32,
            Glyph {
                x: 0,
                y: 0,
                width: 8,
                height: 12,
                xoffset: 1,
                yoffset: 2,
                xadvance: 10,
                page: 0,
                channel: 15,
            },
        );
        glyphs.insert(
            'B' as u32,
            Glyph {
                x: 8,
                y: 0,
                width: 8,
                height: 12,
                xoffset: 0,
                yoffset: 2,
                xadvance: 10,
                page: 0,
                channel: 15,
            },
        );
        glyphs.insert(
            ' ' as u32,
            Glyph {
                xadvance: 4,
                page: 0,
                ..Default::default()
            },
        );
        Font {
            name: "test".into(),
            stem: "test".into(),
            font: BitmapFont {
                face: "test".into(),
                size: 20,
                unicode: true,
                line_height: 20,
                base: 16,
                scale: (64, 32),
                page_count: 1,
                pages: vec!["test_0.dds".into()],
                glyphs,
                kernings: [(('A' as u32, 'B' as u32), -2)].into_iter().collect(),
                unparsed_lines: Vec::new(),
            },
            pages: vec![7],
            atlas: [64.0, 32.0],
            color: [1.0, 1.0, 1.0, 1.0],
        }
    }

    #[test]
    fn colour_codes_are_stripped_and_the_text_between_them_kept() {
        assert_eq!(strip_color_codes("§YBerlin§!"), "Berlin");
        assert_eq!(strip_color_codes("a§Gb§!c"), "abc");
        assert_eq!(strip_color_codes("plain"), "plain");
        // A dangling code at the end of the string drops the marker only.
        assert_eq!(strip_color_codes("tail§"), "tail");
        assert_eq!(strip_color_codes("§!§!"), "");
    }

    #[test]
    fn the_advance_sums_the_glyph_steps_and_the_kerning_pairs() {
        let font = font();
        // A then B is 10 + (-2 kerning) + 10.
        assert_eq!(measure(&font, "AB"), 18.0);
        assert_eq!(measure(&font, "BA"), 20.0);
        assert_eq!(measure(&font, "A B"), 24.0);
        // Characters the font has no box for are dropped, not measured as zero.
        assert_eq!(measure(&font, "AZB"), 18.0);
        // Colour codes never reach the measure.
        assert_eq!(measure(&font, "§YAB§!"), 18.0);
    }

    #[test]
    fn alignment_places_the_run_inside_the_declared_box() {
        let font = font();
        let left = layout(&font, "AB", [100.0, 50.0], Align::Left, Some(149.0));
        assert_eq!(left.len(), 2);
        assert_eq!(left[0].page, 7);
        // `xoffset` 1 on `A`, and `yoffset` 2 below the top of the line box.
        assert_eq!(left[0].rect, [101.0, 52.0, 8.0, 12.0]);
        // `B` starts one advance plus the kerning further on, with xoffset 0.
        assert_eq!(left[1].rect[0], 108.0);
        // The atlas rectangle is normalised by the `.fnt` page size.
        assert_eq!(left[0].uv, [0.0, 0.0, 8.0 / 64.0, 12.0 / 32.0]);

        // Centre and right place the run inside the `maxWidth` box.
        let centre = layout(&font, "AB", [100.0, 50.0], Align::Centre, Some(50.0));
        assert_eq!(centre[0].rect[0], 100.0 + (50.0 - 18.0) * 0.5 + 1.0);
        let right = layout(&font, "AB", [100.0, 50.0], Align::Right, Some(50.0));
        assert_eq!(right[0].rect[0], 100.0 + 50.0 - 18.0 + 1.0);
        // A run wider than the box starts at its left edge whatever the format,
        // and is cut at the first glyph that would not fit.
        let tight = layout(&font, "AB", [100.0, 50.0], Align::Centre, Some(10.0));
        assert_eq!(tight.len(), 1);
        assert_eq!(tight[0].rect[0], 101.0);
        // With no box there is nothing to centre inside, so the origin is used.
        let free = layout(&font, "AB", [100.0, 50.0], Align::Centre, None);
        assert_eq!(free[0].rect[0], 101.0);
    }

    #[test]
    fn the_max_width_cuts_the_run_and_empty_glyphs_draw_nothing() {
        let font = font();
        let cut = layout(&font, "ABAB", [0.0, 0.0], Align::Left, Some(20.0));
        assert_eq!(cut.len(), 2);
        // The space has an empty box, so it advances the pen but emits no quad.
        let spaced = layout(&font, "A B", [0.0, 0.0], Align::Left, None);
        assert_eq!(spaced.len(), 2);
        assert_eq!(spaced[1].rect[0], 14.0);
    }

    #[test]
    fn the_gui_format_word_selects_the_alignment() {
        assert_eq!(Align::parse(Some("left")), Align::Left);
        assert_eq!(Align::parse(Some("centre")), Align::Centre);
        assert_eq!(Align::parse(Some("center")), Align::Centre);
        assert_eq!(Align::parse(Some("RIGHT")), Align::Right);
        assert_eq!(Align::parse(None), Align::Left);
        assert_eq!(Align::parse(Some("justified")), Align::Left);
    }

    #[test]
    fn the_glyph_metrics_trait_reports_the_fnt_fields() {
        let font = font();
        assert_eq!(GlyphMetrics::line_height(&font), 20.0);
        assert_eq!(GlyphMetrics::base(&font), 16.0);
        assert_eq!(GlyphMetrics::advance(&font, 'A'), Some(10.0));
        assert_eq!(GlyphMetrics::advance(&font, 'Z'), None);
        assert_eq!(GlyphMetrics::glyph(&font, 'Z'), None);
        assert_eq!(GlyphMetrics::kerning(&font, 'A', 'B'), -2.0);
        assert_eq!(GlyphMetrics::kerning(&font, 'B', 'A'), 0.0);
        assert_eq!(font.page_of('A'), Some(7));
        assert_eq!(font.page_of('Z'), None);
    }
}
