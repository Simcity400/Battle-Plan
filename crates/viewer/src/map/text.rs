//! Bitmap fonts and the screen-space text layout the map's labels use.
//!
//! Font declarations and family pages resolve through the recorded content stack. The shared font
//! reader supplies glyph metrics and named verification failures. Outline artwork belongs to the
//! page; this module adds no outline. Screen-space text applies authored font colour, while the
//! installed world-name shader controls its own colour treatment.
//!
//! # Colour codes
//!
//! Localisation stores an authored `§` as the control byte 0x11
//! (`installed_content::localisation`), and the engine's text layout
//! reads 0x11 and the code after it as a colour change, drawing neither.
//! [`strip_color_codes`] removes them and keeps the text. The per-code colours
//! in the `textcolors` blocks are not applied: every map label the viewer
//! draws is a plain name, and applying them would need a per-glyph colour the
//! batch does not carry yet. The other controls (an inline flag, a text icon,
//! 0x14 and 0x1A) are not drawn here; [`unsupported_control`] names the first
//! one so a caller refuses the text rather than drawing what follows it.

use std::{collections::BTreeSet, io};

use installed_content::{
    ResolvedLayers,
    gui::native::{
        fonts::{self, GlyphSet},
        resources,
    },
    image::dds,
};

use super::names::{GlyphBox, GlyphMetrics};

/// `interface/core.gfx`: the map-name font, `gfx/fonts/hoi_mapfont4`.
pub const MAP_FONT: &str = "tahoma_60";
/// The stored control byte 0x11 a colour run opens with, and 0x11 `!` closes
/// with.
const COLOR_CODE: char = '\u{11}';

/// A loaded page: its texture id and the extent its glyph boxes are divided by.
type Page = (usize, [f32; 2]);

/// One loaded bitmap font: its family's glyphs as the game loads them, their page textures and
/// its colour.
#[derive(Clone, Debug)]
pub struct Font {
    /// The `interface/*.gfx` name, such as `hoi_20bs`.
    pub name: String,
    /// The family's first file, for reporting.
    pub stem: String,
    /// Load family files in order, with the first glyph for a codepoint winning.
    glyphs: GlyphSet,
    /// Each family file contributes a page id and decoded extent for glyph UVs. An absent file
    /// contributes no glyphs. An unloaded page retains advance widths but supplies no drawing.
    pages: Vec<Option<Page>>,
    /// What the game logs and reads past while loading the font, for the caller's warnings:
    /// each family file no layer holds ("Missing font file"), and each page that does not load
    /// in the game, with why.
    pub logged: Vec<String>,
    /// The font's `color` (`0xAARRGGBB`, 0 when it declares none), straight RGBA.
    color: [f32; 4],
}

impl Font {
    fn empty(name: &str) -> Self {
        Self {
            name: name.to_owned(),
            stem: String::new(),
            glyphs: GlyphSet::default(),
            pages: Vec::new(),
            logged: Vec::new(),
            color: [0.0; 4],
        }
    }

    /// The glyph `ch` draws and its page, `None` when its page did not load.
    fn glyph_of(&self, ch: char) -> Option<(&fonts::Glyph, Option<Page>)> {
        let glyph = self.glyphs.glyphs.get(&(ch as u32))?;
        Some((
            glyph,
            self.pages.get(glyph.page as usize).copied().flatten(),
        ))
    }

    /// The page texture id a glyph of `ch` is drawn from.
    pub fn page_of(&self, ch: char) -> Option<usize> {
        self.glyph_of(ch)?.1.map(|(texture, _)| texture)
    }

    /// The definition's colour as straight RGBA in 0..1.
    pub fn color(&self) -> [f32; 4] {
        self.color
    }

    /// The first `.fnt`'s `lineHeight`.
    pub fn line_height(&self) -> f32 {
        self.glyphs.line_height.unwrap_or_default() as f32
    }

    /// Page count, for reporting.
    pub fn page_count(&self) -> usize {
        self.pages.iter().flatten().count()
    }

    /// Glyph count, for reporting.
    pub fn glyph_count(&self) -> usize {
        self.glyphs.glyphs.len()
    }

    /// One line naming the font and the `.fnt` header it was loaded from.
    pub fn summary(&self) -> String {
        format!(
            "{} ({}): lineHeight {}, base {}, {} glyphs on {} page(s) in {} file(s)",
            self.name,
            self.stem,
            self.line_height(),
            GlyphMetrics::base(self),
            self.glyph_count(),
            self.page_count(),
            self.pages.len()
        )
    }
}

impl GlyphMetrics for Font {
    fn advance(&self, ch: char) -> Option<f32> {
        self.glyph_of(ch).map(|(glyph, _)| glyph.xadvance as f32)
    }

    fn line_height(&self) -> f32 {
        Font::line_height(self)
    }

    fn base(&self) -> f32 {
        self.glyphs.base.unwrap_or_default() as f32
    }

    fn glyph(&self, ch: char) -> Option<GlyphBox> {
        let (glyph, page) = self.glyph_of(ch)?;
        let Some((_, atlas)) = page else {
            // Its page did not load: the glyph advances and draws nothing.
            return Some(GlyphBox {
                width: 0.0,
                height: 0.0,
                xoffset: glyph.xoffset as f32,
                yoffset: glyph.yoffset as f32,
                xadvance: glyph.xadvance as f32,
                uv: [0.0; 4],
            });
        };
        Some(GlyphBox {
            width: glyph.width as f32,
            height: glyph.height as f32,
            xoffset: glyph.xoffset as f32,
            yoffset: glyph.yoffset as f32,
            xadvance: glyph.xadvance as f32,
            uv: [
                glyph.x as f32 / atlas[0],
                glyph.y as f32 / atlas[1],
                (glyph.x + glyph.width) as f32 / atlas[0],
                (glyph.y + glyph.height) as f32 / atlas[1],
            ],
        })
    }

    fn kerning(&self, a: char, b: char) -> f32 {
        self.glyphs
            .glyphs
            .get(&(a as u32))
            .and_then(|glyph| glyph.kernings.get(&(b as u32)))
            .map_or(0.0, |amount| *amount as f32)
    }
}

/// Loads one named font and appends its pages to `images`.
///
/// Returns a named error rather than panicking when `interface/*.gfx` declares no such font,
/// when the game's reading of its declaration is refused (its recorded reasons), when its family
/// loads no glyph or its first file no line metrics, or when a page's drawing is not determined
/// by the file or the recording ([`resources::NoTexture::native_failure`]). A family file no
/// layer holds contributes nothing, as the game logs it ("Missing font file") and reads on; a
/// page the game itself does not load leaves its glyphs advancing and drawing nothing; both are
/// named in [`Font::logged`]. A read that fails other than by a file's absence is the error.
pub fn load(
    layers: &ResolvedLayers,
    interface: &super::sprites::Interface,
    name: &str,
    language: &str,
    images: &mut Vec<dds::Texture>,
) -> io::Result<Font> {
    let font = interface
        .registry
        .fonts
        .get(name.as_bytes())
        .ok_or_else(|| invalid(format!("interface/*.gfx declares no bitmapfont {name:?}")))?;
    if !font.facts.refusals.is_empty() {
        return Err(invalid(format!(
            "bitmapfont {name:?} is refused: {}",
            font.facts
                .refusals
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>()
                .join("; ")
        )));
    }
    // Use the first bitmap-font override whose languages include the reader's language, otherwise
    // the declared family's own files.
    let family = font.family(format!("l_{language}").as_bytes());
    let failed = std::cell::RefCell::new(None);
    let glyphs = fonts::load_glyphs(family, &|path| match layers.read_uncovered_path(path) {
        Ok(bytes) => Some(bytes),
        Err(error) => {
            if error.kind() != io::ErrorKind::NotFound {
                failed.borrow_mut().get_or_insert(error);
            }
            None
        }
    });
    if let Some(error) = failed.into_inner() {
        return Err(error);
    }
    if glyphs.glyphs.is_empty() {
        return Err(invalid(format!("bitmapfont {name:?} loads no characters")));
    }
    if glyphs.line_height.is_none() || glyphs.base.is_none() {
        return Err(invalid(format!(
            "bitmapfont {name:?}: its first file gives no lineHeight or base"
        )));
    }
    let mut pages = Vec::with_capacity(glyphs.pages.len());
    let mut logged = Vec::new();
    for (file, texture) in family.files.iter().zip(&glyphs.pages) {
        let fnt = String::from_utf8_lossy(&[file.as_slice(), b".fnt"].concat()).into_owned();
        if glyphs.missing.contains(&fnt) {
            logged.push(format!(
                "bitmapfont {name:?}: missing font file {fnt}, whose glyphs it does not have"
            ));
            pages.push(None);
            continue;
        }
        // Register the page texture and use its decoded extent.
        match resources::stack_texture(layers, texture, interface.decode)? {
            Ok(page) => {
                pages.push(Some((
                    images.len(),
                    [page.width as f32, page.height as f32],
                )));
                images.push(page.image);
            }
            Err(failure) if failure.native_failure() => {
                logged.push(format!(
                    "bitmapfont {name:?} page {failure}; its glyphs advance and draw nothing"
                ));
                pages.push(None);
            }
            Err(failure) => {
                return Err(invalid(format!("bitmapfont {name:?} page {failure}")));
            }
        }
    }
    let [blue, green, red, alpha] = font.facts.color.to_le_bytes();
    Ok(Font {
        name: name.to_owned(),
        stem: family
            .files
            .first()
            .map(|file| String::from_utf8_lossy(file).into_owned())
            .unwrap_or_default(),
        glyphs,
        pages,
        logged,
        color: [red, green, blue, alpha].map(|channel| f32::from(channel) / 255.0),
    })
}

/// The fonts the map passes draw with.
pub struct Fonts {
    /// The country and state name font, with the native fallback or an empty refused font.
    pub map: Font,
    /// The unit counter's `count_txt` font from `mapicons.gui` (`Main_14`),
    /// resolved as the engine does; `None` when that font does not load, and
    /// counters then draw no count.
    pub counter: Option<Font>,
    /// The battle-plan label font `gfx/maparrows/maparrows.txt` names in its
    /// `arrowText` block, resolved as `counter` is; loaded only when it is none
    /// of the fonts above. See [`Fonts::arrow`].
    arrow_own: Option<Font>,
    /// The resolved name of the plan label font, when one resolved and loaded.
    arrow_name: Option<String>,
}

/// The font names the content declares for each map text family.
#[derive(Clone, Copy, Debug, Default)]
pub struct DeclaredFonts<'a> {
    /// The unit counter count box's font.
    pub counter: Option<&'a str>,
    /// `maparrows.txt`'s `arrowText` font.
    pub arrow: Option<&'a str>,
}

impl Fonts {
    /// Loads them from the content's registered fonts for `language`, appending their pages to
    /// `images`.
    ///
    /// `counter` is the counter count box's declared font. A name the
    /// registry does not hold, or none at all, takes
    /// the game's fallback
    /// ([`fonts::Fonts::get_or_fallback`]); a font that then fails to load leaves the
    /// count undrawn with a warning rather than failing every map icon.
    ///
    /// Arrow and map-name fonts use the registered fallback. An unreadable fallback withholds text
    /// and reports a counted font refusal.
    pub fn load(
        layers: &ResolvedLayers,
        interface: &super::sprites::Interface,
        language: &str,
        declared: &DeclaredFonts<'_>,
        images: &mut Vec<dds::Texture>,
        warnings: &mut Vec<String>,
    ) -> io::Result<Self> {
        let DeclaredFonts { counter, arrow } = *declared;
        // Use the declared font name, empty when undeclared, otherwise the fallback.
        let resolve = |name: Option<&str>| {
            interface
                .registry
                .fonts
                .get_or_fallback(name.unwrap_or_default().as_bytes())
                .map(|font| String::from_utf8_lossy(&font.facts.name).into_owned())
        };
        let counter = match resolve(counter)
            .map(|name| load(layers, interface, &name, language, images))
        {
            Some(Ok(font)) => Some(font),
            Some(Err(error)) => {
                warnings.push(format!("Unit counter counts are not drawn: {error}."));
                None
            }
            None => {
                warnings
                    .push("Unit counter counts are not drawn: no bitmapfont is declared.".into());
                None
            }
        };
        // Log a missing name and select the first font bucket.
        let map = match resolve(Some(MAP_FONT)) {
            Some(name) => {
                if name != MAP_FONT {
                    warnings.push(format!(
                        "1 map-name font fallback: {MAP_FONT} is absent; using {name}."
                    ));
                }
                let before = images.len();
                let loaded = counter
                    .as_ref()
                    .filter(|font| font.name == name)
                    .cloned()
                    .map(Ok)
                    .unwrap_or_else(|| load(layers, interface, &name, language, images));
                match loaded {
                    Ok(font) => font,
                    Err(error) if error.kind() == io::ErrorKind::InvalidData => {
                        images.truncate(before);
                        warnings.push(format!(
                            "1 map-name font refusal: {name}: {error}; country and state names are not drawn."
                        ));
                        Font::empty(&name)
                    }
                    Err(error) => return Err(error),
                }
            }
            None => {
                warnings.push(format!(
                    "1 map-name font refusal: no bitmapfont is declared for {MAP_FONT}; country and state names are not drawn."
                ));
                Font::empty(MAP_FONT)
            }
        };
        // Loaded last, so the other fonts keep their page ids. An undeclared
        // or unnamed font takes the fallback, as the game does; a name
        // already loaded shares that font's atlas pages.
        let arrow_name = resolve(arrow);
        let loaded = [Some(&map), counter.as_ref()];
        let (arrow_own, arrow_name) = match arrow_name {
            // Share the same registered font; a refused load withholds both label families.
            Some(name) if name == map.name && map.glyph_count() == 0 => {
                warnings.push(format!(
                    "1 battle-plan font refusal: {name} also failed as the map-name font; battle-plan labels are not drawn."
                ));
                (None, None)
            }
            Some(name) if find(loaded, &name).is_some() => (None, Some(name)),
            Some(name) => match load(layers, interface, &name, language, images) {
                Ok(font) => (Some(font), Some(name)),
                Err(error) => {
                    warnings.push(format!("Battle-plan labels are not drawn: {error}."));
                    (None, None)
                }
            },
            None => {
                warnings
                    .push("Battle-plan labels are not drawn: no bitmapfont is declared.".into());
                (None, None)
            }
        };
        let mut noted = BTreeSet::new();
        let mut uncovered = BTreeSet::new();
        for font in [Some(&map), counter.as_ref(), arrow_own.as_ref()]
            .into_iter()
            .flatten()
            .filter(|font| noted.insert(font.name.as_str()))
        {
            warnings.extend(
                font.logged
                    .iter()
                    .map(|note| format!("Map text: {note}, as in the game.")),
            );
            uncovered.extend(
                font.glyphs
                    .pages
                    .iter()
                    .filter(|page| !layers.covered_path(page))
                    .map(|page| {
                        String::from_utf8_lossy(page.strip_suffix(b".tga").unwrap_or(page))
                            .into_owned()
                    }),
            );
        }
        if !uncovered.is_empty() {
            warnings.push(format!(
                "Map text: {} font file(s) are outside the content coverage policy and are not hash-verified: {}.",
                uncovered.len(),
                uncovered.into_iter().collect::<Vec<_>>().join(", ")
            ));
        }
        Ok(Self {
            map,
            counter,
            arrow_own,
            arrow_name,
        })
    }

    /// The battle-plan label font, shared with an already loaded font of the
    /// same name; `None` when it did not load, and plan labels are then not drawn.
    pub fn arrow(&self) -> Option<&Font> {
        let name = self.arrow_name.as_deref()?;
        self.arrow_own
            .as_ref()
            .or_else(|| find([Some(&self.map), self.counter.as_ref()], name))
    }

    /// One summary line per font, in load order.
    pub fn summary(&self) -> Vec<String> {
        [
            Some(&self.map),
            self.counter.as_ref(),
            self.arrow_own.as_ref(),
        ]
        .into_iter()
        .flatten()
        .map(Font::summary)
        .collect()
    }
}

/// The first of `fonts` loaded under `name`.
fn find<'a, const N: usize>(fonts: [Option<&'a Font>; N], name: &str) -> Option<&'a Font> {
    fonts.into_iter().flatten().find(|font| font.name == name)
}

/// Horizontal alignment, as an `instantTextboxType`'s `format` declares it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Align {
    #[default]
    Left,
    Centre,
    Right,
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

/// Strips the game's colour runs, keeping the text between them.
///
/// 0x11 followed by any one character opens a run and 0x11 `!` closes it; a
/// trailing 0x11 with nothing after it is dropped. The engine's own layout
/// does the same before it measures, so the width this module
/// reports matches the engine's.
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

/// The first control in stored `text` that map text does not draw, named for
/// a refusal: every control but a colour code.
pub fn unsupported_control(text: &str) -> Option<String> {
    use installed_content::localisation::{Piece, pieces};
    pieces(text)
        .into_iter()
        .find(|piece| !matches!(piece, Piece::Text(_) | Piece::Colour(_)))
        .map(|piece| piece.describe())
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
/// `max_width` supplies the alignment box and truncation limit. A glyph whose advance would move
/// the pen beyond it, and every following glyph, is dropped. Labels use a plain cut where the
/// game uses ellipsis.
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
    use installed_content::gui::native::fonts::Glyph;
    use std::collections::BTreeMap;

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
                kernings: [('B' as u32, -2)].into_iter().collect(),
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
                kernings: BTreeMap::new(),
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
            glyphs: GlyphSet {
                line_height: Some(20),
                base: Some(16),
                pages: vec![b"test.tga".to_vec()],
                glyphs,
                missing: Vec::new(),
            },
            pages: vec![Some((7, [64.0, 32.0]))],
            logged: Vec::new(),
            color: [1.0, 1.0, 1.0, 1.0],
        }
    }

    #[test]
    fn a_later_font_file_supplies_the_glyphs_the_first_lacks() {
        // Its glyph is on the second file's page.
        let mut font = font();
        font.glyphs.glyphs.insert(
            'Z' as u32,
            Glyph {
                xadvance: 6,
                page: 1,
                ..Default::default()
            },
        );
        font.pages.push(Some((9, [64.0, 32.0])));
        assert_eq!(measure(&font, "AZB"), 26.0);
        assert_eq!(font.page_of('Z'), Some(9));
        assert_eq!(font.page_of('A'), Some(7));
    }

    /// A content stack holding `files`, and the interface read from it.
    fn stack(
        name: &str,
        files: &[(&str, &[u8])],
    ) -> (
        std::path::PathBuf,
        ResolvedLayers,
        super::super::sprites::Interface,
    ) {
        let root =
            std::env::temp_dir().join(format!("hoi4_viewer_text_{name}_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        for (path, bytes) in files {
            let file = root.join(path);
            std::fs::create_dir_all(file.parent().unwrap()).unwrap();
            std::fs::write(&file, bytes).unwrap();
        }
        let layers = ResolvedLayers::from_folders(&[(&root, &[][..])]).unwrap();
        let registry = installed_content::gui::native::resources::Registry::load(&layers).unwrap();
        let interface = super::super::sprites::Interface {
            sprites: installed_content::gfx::SpriteSet::new(&registry.files),
            registry,
            decode: installed_content::image::native::Decode::default(),
        };
        (root, layers, interface)
    }

    /// An 8 x 7 `DXT5` file: four blocks of zeroes.
    fn dxt5_8x7() -> Vec<u8> {
        let mut dds = vec![0u8; 128];
        dds[..4].copy_from_slice(b"DDS ");
        for (offset, value) in [(4, 124), (12, 7), (16, 8), (28, 1), (76, 32), (80, 4)] {
            dds[offset..offset + 4].copy_from_slice(&u32::to_le_bytes(value));
        }
        dds[84..88].copy_from_slice(b"DXT5");
        dds.extend([0u8; 64]);
        dds
    }

    #[test]
    fn map_name_font_uses_native_fallback_and_refuses_missing_glyphs_locally() {
        engine::executable_tables::initialize_for_tests()
            .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
        let dds = dxt5_8x7();
        let (root, layers, interface) = stack(
            "map_fallback",
            &[
                (
                    "interface/fonts.gfx",
                    b"bitmapfonts = { bitmapfont = { name = f fontfiles = { gfx/fonts/f } } }",
                ),
                (
                    "gfx/fonts/f.fnt",
                    b"common lineHeight=8 base=6 scaleW=8 scaleH=8\n\
                      char id=65 x=0 y=0 width=4 height=7 xoffset=0 yoffset=0 xadvance=5\n",
                ),
                ("gfx/fonts/f.dds", &dds),
            ],
        );
        let mut warnings = Vec::new();
        let fonts = Fonts::load(
            &layers,
            &interface,
            "english",
            &DeclaredFonts::default(),
            &mut Vec::new(),
            &mut warnings,
        )
        .unwrap();
        assert_eq!(fonts.map.name, "f");
        assert_eq!(fonts.map.page_of('A'), Some(0));
        assert!(
            warnings
                .iter()
                .any(|why| why.contains("1 map-name font fallback"))
        );
        std::fs::remove_dir_all(root).unwrap();

        let (root, layers, interface) = stack("map_absent", &[]);
        let mut warnings = Vec::new();
        let fonts = Fonts::load(
            &layers,
            &interface,
            "english",
            &DeclaredFonts::default(),
            &mut Vec::new(),
            &mut warnings,
        )
        .unwrap();
        assert_eq!(fonts.map.glyph_count(), 0);
        assert!(
            warnings
                .iter()
                .any(|why| why.contains("1 map-name font refusal"))
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    /// `gfx/fonts` is covered and verified; a font a mod declares elsewhere is named unverified.
    #[test]
    fn only_a_font_outside_the_covered_root_is_named_unverified() {
        engine::executable_tables::initialize_for_tests()
            .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
        let dds = dxt5_8x7();
        for (name, directory, unverified) in [
            ("covered_font", "gfx/fonts", false),
            ("uncovered_font", "gfx/modfonts", true),
        ] {
            let gfx =
                format!("bitmapfonts = {{ bitmapfont = {{ name = f path = {directory}/f }} }}");
            let (root, layers, interface) = stack(
                name,
                &[
                    ("interface/fonts.gfx", gfx.as_bytes()),
                    (
                        &format!("{directory}/f.fnt"),
                        b"common lineHeight=8 base=6 scaleW=8 scaleH=8\n\
                          char id=65 x=0 y=0 width=4 height=7 xoffset=0 yoffset=0 xadvance=5\n",
                    ),
                    (&format!("{directory}/f.dds"), &dds),
                ],
            );
            let mut warnings = Vec::new();
            let fonts = Fonts::load(
                &layers,
                &interface,
                "english",
                &DeclaredFonts::default(),
                &mut Vec::new(),
                &mut warnings,
            )
            .unwrap();
            assert_eq!(fonts.map.page_of('A'), Some(0));
            let named: Vec<_> = warnings
                .iter()
                .filter(|why| why.contains("not hash-verified"))
                .collect();
            if unverified {
                assert_eq!(named.len(), 1, "{warnings:?}");
                assert!(named[0].ends_with("1 font file(s) are outside the content coverage policy and are not hash-verified: gfx/modfonts/f."));
            } else {
                assert!(named.is_empty(), "{warnings:?}");
            }
            std::fs::remove_dir_all(root).unwrap();
        }
    }

    #[test]
    fn a_refused_map_font_also_names_battle_plan_label_refusal() {
        engine::executable_tables::initialize_for_tests()
            .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
        let (root, layers, interface) = stack(
            "map_arrow_refused",
            &[(
                "interface/fonts.gfx",
                b"bitmapfonts = { bitmapfont = { name = f fontfiles = { gfx/fonts/missing } } }",
            )],
        );
        let mut warnings = Vec::new();
        let fonts = Fonts::load(
            &layers,
            &interface,
            "english",
            &DeclaredFonts {
                counter: None,
                arrow: Some("f"),
            },
            &mut Vec::new(),
            &mut warnings,
        )
        .unwrap();
        assert!(fonts.arrow().is_none());
        assert!(
            warnings
                .iter()
                .any(|why| why.contains("1 map-name font refusal"))
        );
        assert!(
            warnings
                .iter()
                .any(|why| why.contains("1 battle-plan font refusal"))
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    /// Absent family files are named and skipped while later files still load.
    #[test]
    fn a_missing_family_file_is_skipped_and_named() {
        engine::executable_tables::initialize_for_tests()
            .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
        let dds = dxt5_8x7();
        let (root, layers, interface) = stack(
            "missing",
            &[
                (
                    "interface/fonts.gfx",
                    b"bitmapfonts = { bitmapfont = { name = \"f\" \
                      fontfiles = { \"gfx/fonts/f\" \"gfx/fonts/gone\" } } }",
                ),
                (
                    "gfx/fonts/f.fnt",
                    b"common lineHeight=8 base=6 scaleW=8 scaleH=8\n\
                      char id=65 x=0 y=0 width=4 height=7 xoffset=0 yoffset=0 xadvance=5\n",
                ),
                ("gfx/fonts/f.dds", &dds),
            ],
        );
        let mut images = Vec::new();
        let font = load(&layers, &interface, "f", "english", &mut images).unwrap();
        assert_eq!(font.page_of('A'), Some(0));
        assert_eq!(font.logged.len(), 1, "{:?}", font.logged);
        assert!(font.logged[0].contains("missing font file gfx/fonts/gone.fnt"));
        std::fs::remove_dir_all(&root).unwrap();
    }

    /// An eight-by-seven DXT5 page decodes to an eight-by-eight block extent. Glyph UVs divide by the
    /// loaded extent rather than authored height.
    #[test]
    fn a_page_divides_its_glyphs_by_the_extent_the_game_decodes() {
        engine::executable_tables::initialize_for_tests()
            .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
        let dds = dxt5_8x7();
        let (root, layers, interface) = stack(
            "dxt",
            &[
                (
                    "interface/fonts.gfx",
                    b"bitmapfonts = { bitmapfont = { name = \"f\" path = \"gfx/fonts/f\" } }",
                ),
                (
                    "gfx/fonts/f.fnt",
                    b"info face=\"f\" size=8\ncommon lineHeight=8 base=6 scaleW=8 scaleH=7 pages=1\n\
                      char id=65 x=0 y=0 width=4 height=7 xoffset=0 yoffset=0 xadvance=5 page=0\n",
                ),
                ("gfx/fonts/f.dds", &dds),
            ],
        );
        let mut images = Vec::new();
        let font = load(&layers, &interface, "f", "english", &mut images).unwrap();
        assert!(font.logged.is_empty(), "{:?}", font.logged);
        let glyph = GlyphMetrics::glyph(&font, 'A').unwrap();
        assert_eq!(glyph.uv, [0.0, 0.0, 0.5, 7.0 / 8.0]);
        assert_eq!((images[0].width, images[0].height), (8, 8));
        std::fs::remove_dir_all(&root).unwrap();
    }

    /// Refuse a declaration with an unsupported text colour code byte, preserving its reason.
    #[test]
    fn a_refused_font_does_not_load() {
        engine::executable_tables::initialize_for_tests()
            .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
        let (root, layers, interface) = stack(
            "refused",
            &[(
                "interface/fonts.gfx",
                b"bitmapfonts = { bitmapfont = { name = \"f\" path = \"gfx/fonts/f\" \
                  textcolors = { \xC3 = { 1 0 0 } } } }",
            )],
        );
        let error = load(&layers, &interface, "f", "english", &mut Vec::new())
            .err()
            .unwrap()
            .to_string();
        assert!(error.contains("bitmapfont \"f\" is refused"), "{error}");
        assert!(error.contains("code byte 0xc3"), "{error}");
        std::fs::remove_dir_all(&root).unwrap();
    }

    /// A page the game does not load leaves its glyphs advancing and drawing nothing.
    #[test]
    fn a_glyph_of_an_unloaded_page_advances_and_draws_nothing() {
        let mut font = font();
        font.pages[0] = None;
        assert_eq!(font.page_of('A'), None);
        assert_eq!(measure(&font, "AB"), 18.0);
        let glyph = GlyphMetrics::glyph(&font, 'A').unwrap();
        assert_eq!(
            (glyph.width, glyph.height, glyph.xadvance),
            (0.0, 0.0, 10.0)
        );
    }

    /// The synthetic font under `name`, its one page at texture id `page`.
    fn named(name: &str, page: usize) -> Font {
        let mut font = font();
        font.name = name.into();
        font.pages = vec![Some((page, [64.0, 32.0]))];
        font
    }

    #[test]
    fn a_plan_label_font_of_a_loaded_name_shares_that_font() {
        let fonts = |arrow_own: Option<Font>, arrow_name: &str| Fonts {
            map: named("tahoma_60", 0),
            counter: Some(named("Main_14", 2)),
            arrow_own,
            arrow_name: Some(arrow_name.into()),
        };
        let shared = fonts(None, "tahoma_60");
        assert_eq!(shared.arrow().map(|font| font.page_of('A')), Some(Some(0)));
        // A shared font adds no summary line: its pages were loaded once.
        assert_eq!(shared.summary().len(), 2);
        let own = fonts(Some(named("plan_font", 5)), "plan_font");
        assert_eq!(own.arrow().map(|font| font.page_of('A')), Some(Some(5)));
        assert_eq!(own.summary().len(), 3);
        let missing = Fonts {
            arrow_name: None,
            ..fonts(None, "tahoma_60")
        };
        assert!(missing.arrow().is_none());
    }

    #[test]
    fn colour_codes_are_stripped_and_the_text_between_them_kept() {
        // Values as the loader stores them.
        let stored = |authored: &str| {
            let loc: installed_content::localisation::Localisation =
                [("K".to_owned(), authored.to_owned())]
                    .into_iter()
                    .collect();
            loc.get("K").unwrap().to_owned()
        };
        assert_eq!(strip_color_codes(&stored("§YBerlin§!")), "Berlin");
        assert_eq!(strip_color_codes(&stored("a§Gb§!c")), "abc");
        assert_eq!(strip_color_codes("plain"), "plain");
        // A dangling code at the end of the string drops the marker only.
        assert_eq!(strip_color_codes(&stored("tail§")), "tail");
        assert_eq!(strip_color_codes(&stored("§!§!")), "");
        // A doubled `§` is stored as the character, which is drawn.
        assert_eq!(strip_color_codes(&stored("100§§")), "100§");
        assert_eq!(unsupported_control(&stored("§Ya§! b")), None);
        assert_eq!(
            unsupported_control(&stored("£pol_power 5")).as_deref(),
            Some("the icon 0x13 \"pol_power\" (an authored `£`)")
        );
        assert!(unsupported_control(&stored("@GER")).is_some());
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
        assert_eq!(measure(&font, "\u{11}YAB\u{11}!"), 18.0);
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
        // The atlas rectangle is normalised by the page texture's size.
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
