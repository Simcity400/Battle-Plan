
use std::cell::RefCell;
#[cfg(test)]
use std::path::PathBuf;
use std::{
    collections::{BTreeMap, BTreeSet},
    io,
};

use content_reader_layers::ResolvedLayers;
use content_reader_gui::gui::native::{fonts::{self, GlyphSet}, resources};
use std::sync::Arc;
use viewer_gpu::textures::GuiSource;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GlyphBox {
    pub width: f32,
    pub height: f32,
    pub xoffset: f32,
    pub yoffset: f32,
    pub xadvance: f32,
    pub uv: [f32; 4],
}

pub trait GlyphMetrics {
    fn advance(&self, ch: char) -> Option<f32>;

    fn line_height(&self) -> f32;

    fn base(&self) -> f32;

    fn glyph(&self, ch: char) -> Option<GlyphBox>;

    fn kerning(&self, a: char, b: char) -> f32 {
        let _ = (a, b);
        0.0
    }

    fn text_color(&self, code: char) -> Option<[f32; 4]> {
        let _ = code;
        None
    }
}

pub const MAP_FONT: &str = "tahoma_60";
const COLOR_CODE: char = '\u{11}';

type Page = (usize, [f32; 2]);

#[derive(Clone, Debug)]
pub struct Font {
    projection_id: u64,
    pub name: String,
    pub stem: String,
    glyphs: GlyphSet,
    pages: Vec<Option<Page>>,
    pub logged: Vec<String>,
    color: [f32; 4],
    text_colors: BTreeMap<u8, [f32; 4]>,
    global_colors: BTreeMap<u8, [f32; 4]>,
}

fn next_font_id() -> u64 {
    use std::sync::atomic::{AtomicU64, Ordering};
    static NEXT: AtomicU64 = AtomicU64::new(1);
    NEXT.fetch_add(1, Ordering::Relaxed)
}

impl Font {
    pub fn projection_id(&self) -> u64 {
        self.projection_id
    }

    fn empty(name: &str) -> Self {
        Self {
            projection_id: next_font_id(),
            name: name.to_owned(),
            stem: String::new(),
            glyphs: GlyphSet::default(),
            pages: Vec::new(),
            logged: Vec::new(),
            color: [0.0; 4],
            text_colors: BTreeMap::new(),
            global_colors: BTreeMap::new(),
        }
    }

    fn glyph_of(&self, ch: char) -> Option<(&fonts::Glyph, Option<Page>)> {
        let glyph = self.glyphs.glyphs.get(&(ch as u32))?;
        Some((
            glyph,
            self.pages.get(glyph.page as usize).copied().flatten(),
        ))
    }

    pub fn page_of(&self, ch: char) -> Option<usize> {
        self.glyph_of(ch)?.1.map(|(texture, _)| texture)
    }

    pub fn color(&self) -> [f32; 4] {
        self.color
    }

    pub fn text_color(&self, code: char) -> Option<[f32; 4]> {
        let byte = (u32::from(code) & 0xFF) as u8;
        self.text_colors
            .get(&byte)
            .or_else(|| self.global_colors.get(&byte))
            .copied()
    }

    pub fn line_height(&self) -> f32 {
        self.glyphs.line_height.unwrap_or_default() as f32
    }

    pub fn page_count(&self) -> usize {
        self.pages.iter().flatten().count()
    }

    pub fn glyph_count(&self) -> usize {
        self.glyphs.glyphs.len()
    }

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

    fn text_color(&self, code: char) -> Option<[f32; 4]> {
        Font::text_color(self, code)
    }
}

pub fn load(
    layers: &ResolvedLayers,
    interface: &super::sprites::Interface,
    name: &str,
    language: &str,
    images: &mut Vec<GuiSource>,
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
    let family = font.family(format!("l_{language}").as_bytes());
    let failed = RefCell::new(None);
    let recorded = layers.without_overlays();
    let glyphs = fonts::load_glyphs(family, &|path| match layers.read_uncovered_path(path) {
        Ok(bytes) => Some(bytes),
        Err(error) => {
            if let Some(bytes) = recorded
                .as_ref()
                .and_then(|recorded| recorded.read_uncovered_path(path).ok())
            {
                layers.note_overlay_refusal(format!(
                    "font file {} ({error})",
                    String::from_utf8_lossy(path)
                ));
                return Some(bytes);
            }
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
    let shared_layers = Arc::new(layers.clone());
    let shared_recorded = recorded.as_ref().cloned().map(Arc::new);
    let mut pages = Vec::with_capacity(glyphs.pages.len());
    let mut logged = Vec::new();
    for (file, texture) in family.files.iter().zip(&glyphs.pages) {
        let fnt = String::from_utf8_lossy(&[file.as_slice(), b".fnt"].concat()).into_owned();
        if glyphs.missing.contains(&fnt) {
            layers.note_refused(format!("font file {fnt}"));
            logged.push(format!(
                "bitmapfont {name:?}: missing font file {fnt}, whose glyphs it does not have"
            ));
            pages.push(None);
            continue;
        }
        match resources::stack_texture_or_recorded_transient(
            layers,
            recorded.as_ref(),
            texture,
            interface.decode,
        )? {
            Ok(page) => {
                pages.push(Some((
                    images.len(),
                    [page.width as f32, page.height as f32],
                )));
                images.push(GuiSource::deferred_stack(
                    Arc::clone(&shared_layers),
                    shared_recorded.clone(),
                    texture.to_vec(),
                    interface.decode,
                    &page.image,
                ));
            }
            Err(failure) if failure.native_failure() => {
                layers.note_refused(format!("font page {}", String::from_utf8_lossy(texture)));
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
        projection_id: next_font_id(),
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
        text_colors: font.facts.text_colors.clone(),
        global_colors: interface.registry.text_colors.clone(),
    })
}

pub struct Fonts {
    pub map: Font,
    pub counter: Option<Font>,
    arrow_own: Option<Font>,
    arrow_name: Option<String>,
}

#[derive(Clone, Copy, Debug, Default)]
pub struct DeclaredFonts<'a> {
    pub counter: Option<&'a str>,
    pub arrow: Option<&'a str>,
}

impl Fonts {
    pub fn load(
        layers: &ResolvedLayers,
        interface: &super::sprites::Interface,
        language: &str,
        declared: &DeclaredFonts<'_>,
        images: &mut Vec<GuiSource>,
        warnings: &mut Vec<String>,
    ) -> io::Result<Self> {
        let DeclaredFonts { counter, arrow } = *declared;
        let refuse = |what: &str, error: &io::Error| -> io::Result<()> {
            if layers.shows_visual_mods() {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!("{what} font: {error}"),
                ));
            }
            Ok(())
        };
        let undeclared = |what: &str| -> io::Result<()> {
            refuse(
                what,
                &io::Error::new(io::ErrorKind::NotFound, "no bitmapfont is declared"),
            )
        };
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
                refuse("unit counter", &error)?;
                warnings.push(format!("Unit counter counts are not drawn: {error}."));
                None
            }
            None => {
                undeclared("unit counter")?;
                warnings
                    .push("Unit counter counts are not drawn: no bitmapfont is declared.".into());
                None
            }
        };
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
                        refuse("map-name", &error)?;
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
                undeclared("map-name")?;
                warnings.push(format!(
                    "1 map-name font refusal: no bitmapfont is declared for {MAP_FONT}; country and state names are not drawn."
                ));
                Font::empty(MAP_FONT)
            }
        };
        let arrow_name = resolve(arrow);
        let loaded = [Some(&map), counter.as_ref()];
        let (arrow_own, arrow_name) = match arrow_name {
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
                    refuse("battle-plan", &error)?;
                    warnings.push(format!("Battle-plan labels are not drawn: {error}."));
                    (None, None)
                }
            },
            None => {
                undeclared("battle-plan")?;
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

    pub fn arrow(&self) -> Option<&Font> {
        let name = self.arrow_name.as_deref()?;
        self.arrow_own
            .as_ref()
            .or_else(|| find([Some(&self.map), self.counter.as_ref()], name))
    }

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

fn find<'a, const N: usize>(fonts: [Option<&'a Font>; N], name: &str) -> Option<&'a Font> {
    fonts.into_iter().flatten().find(|font| font.name == name)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Align {
    #[default]
    Left,
    Centre,
    Right,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct StyledChar {
    pub ch: char,
    pub color: Option<[f32; 4]>,
    pub reset_kerning: bool,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TextGlyph {
    pub page: usize,
    pub rect: [f32; 4],
    pub uv: [f32; 4],
    pub color: Option<[f32; 4]>,
}

pub fn color_runs(text: &str, lookup: impl Fn(char) -> Option<[f32; 4]>) -> Vec<StyledChar> {
    let mut out = Vec::with_capacity(text.len());
    let mut color: Option<[f32; 4]> = None;
    let mut stack: Vec<Option<[f32; 4]>> = Vec::new();
    let mut reset_kerning = false;
    let mut chars = text.chars();
    while let Some(ch) = chars.next() {
        if ch == COLOR_CODE {
            reset_kerning = true;
            match chars.next() {
                Some('!') if stack.len() > 1 => {
                    color = stack[stack.len() - 2];
                    stack.pop();
                }
                Some('!') => {
                    color = None;
                    stack.push(None);
                }
                Some(code) => {
                    if let Some(found) = lookup(code) {
                        let packed = super::native_text::packed(found);
                        color = Some(packed);
                        stack.push(Some(packed));
                    }
                }
                None => {}
            }
            continue;
        }
        out.push(StyledChar {
            ch,
            color,
            reset_kerning,
        });
        reset_kerning = false;
    }
    out
}

pub fn strip_color_codes(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars();
    while let Some(ch) = chars.next() {
        if ch == COLOR_CODE {
            chars.next();
        } else {
            out.push(ch);
        }
    }
    out
}

pub fn unsupported_control(text: &str) -> Option<String> {
    use content_reader_localisation::{localisation::Piece, localisation::pieces};
    pieces(text)
        .into_iter()
        .find(|piece| !matches!(piece, Piece::Text(_) | Piece::Colour(_) | Piece::Control14))
        .map(|piece| piece.describe())
}

pub fn measure(font: &Font, text: &str) -> f32 {
    let mut width = 0.0f32;
    let mut previous: Option<char> = None;
    for item in color_runs(text, |_| None) {
        if item.reset_kerning {
            previous = None;
        }
        let Some(advance) = GlyphMetrics::advance(font, item.ch) else {
            continue;
        };
        if let Some(p) = previous {
            width += GlyphMetrics::kerning(font, p, item.ch);
        }
        width += advance;
        previous = Some(item.ch);
    }
    width
}

struct FontGlyphs<'a>(&'a Font);

impl super::native_text::Glyphs for FontGlyphs<'_> {
    fn own(&self, ch: char) -> Option<super::native_text::Glyph> {
        let (glyph, _) = self.0.glyph_of(ch)?;
        Some(super::native_text::Glyph {
            ch,
            advance: glyph.xadvance,
            width: glyph.width,
            height: glyph.height,
            xoffset: glyph.xoffset,
            yoffset: glyph.yoffset,
        })
    }

    fn kerning(&self, a: char, b: char) -> i32 {
        GlyphMetrics::kerning(self.0, a, b) as i32
    }

    fn line_height(&self) -> i32 {
        self.0.line_height() as i32
    }
}

pub fn layout(
    font: &Font,
    text: &str,
    origin: [f32; 2],
    align: Align,
    max_width: Option<f32>,
) -> Vec<TextGlyph> {
    layout_truncated(font, text, origin, align, max_width, true)
}

pub fn layout_truncated(
    font: &Font,
    text: &str,
    origin: [f32; 2],
    align: Align,
    max_width: Option<f32>,
    truncate: bool,
) -> Vec<TextGlyph> {
    use super::native_text::{Glyphs as _, Limits, lay_out};
    let glyphs = FontGlyphs(font);
    let chars: Vec<char> = text.chars().collect();
    let (shown, offset) = match max_width {
        Some(limit) => {
            let width = limit as i32;
            if width <= 0 {
                return Vec::new();
            }
            let Ok(prepared) = lay_out(
                &glyphs,
                &|_| None,
                &chars,
                Limits {
                    width,
                    height: i32::MAX,
                    border: [0, 0],
                    newlines: false,
                    preserve_indent: false,
                    ellipsis: truncate,
                    multiline: false,
                    special: true,
                    icons_add_height: false,
                },
            ) else {
                return Vec::new();
            };
            let line_width = lay_out(
                &glyphs,
                &|_| None,
                &prepared.text,
                Limits {
                    width: i32::MAX,
                    height: i32::MAX,
                    border: [0, 0],
                    newlines: false,
                    preserve_indent: false,
                    ellipsis: false,
                    multiline: false,
                    special: true,
                    icons_add_height: false,
                },
            )
            .ok()
            .and_then(|laid| laid.widths.first().copied())
            .unwrap_or(0);
            let offset = match align {
                Align::Left => 0,
                Align::Centre => width / 2 - line_width / 2,
                Align::Right => width - line_width,
            };
            (prepared.text, offset as f32)
        }
        None => (chars, 0.0),
    };
    let bounded = max_width.is_some();
    let start = origin[0] + offset;
    let mut out = Vec::with_capacity(shown.len());
    let mut pen = 0i32;
    let mut previous: Option<char> = None;
    let mut color: Option<[f32; 4]> = None;
    let mut stack: Vec<Option<[f32; 4]>> = Vec::new();
    let mut iter = shown.into_iter();
    while let Some(ch) = iter.next() {
        if ch == COLOR_CODE {
            match iter.next() {
                Some('!') if stack.len() > 1 => {
                    color = stack[stack.len() - 2];
                    stack.pop();
                }
                Some('!') => {
                    color = None;
                    stack.push(None);
                }
                Some(code) => {
                    if let Some(found) = font.text_color(code) {
                        let packed = super::native_text::packed(found);
                        color = Some(packed);
                        stack.push(Some(packed));
                    }
                }
                None => {}
            }
            previous = None;
            continue;
        }
        let resolved = if bounded {
            glyphs.glyph(ch)
        } else {
            glyphs.own(ch)
        };
        if let Some(glyph) = resolved {
            if let Some(p) = previous {
                pen += glyphs.kerning(p, ch);
            }
            if glyph.height > 0
                && let (Some(metrics), Some(page)) =
                    (GlyphMetrics::glyph(font, glyph.ch), font.page_of(glyph.ch))
                && metrics.width > 0.0
                && metrics.height > 0.0
            {
                out.push(TextGlyph {
                    page,
                    rect: [
                        start + (pen + glyph.xoffset) as f32,
                        origin[1] + glyph.yoffset as f32,
                        metrics.width,
                        metrics.height,
                    ],
                    uv: metrics.uv,
                    color,
                });
            }
            pen += glyph.advance;
            previous = Some(glyph.ch);
        }
    }
    out
}

fn invalid(message: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message.into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use content_reader_gui::gui::native::fonts::Glyph;
    use std::collections::BTreeMap;

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
            projection_id: next_font_id(),
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
            text_colors: BTreeMap::new(),
            global_colors: BTreeMap::new(),
        }
    }

    #[test]
    fn a_later_font_file_supplies_the_glyphs_the_first_lacks() {
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

    fn stack(
        name: &str,
        files: &[(&str, &[u8])],
    ) -> (PathBuf, ResolvedLayers, super::super::sprites::Interface) {
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
        let registry = content_reader_gui::gui::native::resources::Registry::load(&layers).unwrap();
        let interface = super::super::sprites::Interface {
            sprites: content_reader_gfx::gfx::SpriteSet::new(&registry.files),
            registry,
            decode: content_reader_image::native::Decode::default(),
        };
        (root, layers, interface)
    }

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
        content_reader_script::test_game::enter();
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

    #[test]
    fn only_a_font_outside_the_covered_root_is_named_unverified() {
        content_reader_script::test_game::enter();
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
    fn visual_mods_that_hide_every_bitmapfont_refuse_their_fonts() {
        content_reader_script::test_game::enter();
        let root = std::env::temp_dir().join(format!(
            "hoi4_viewer_text_hidden_fonts_{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&root);
        let write = |relative: &str, bytes: &[u8]| {
            let path = root.join(relative);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, bytes).unwrap();
        };
        write(
            "base/interface/fonts.gfx",
            b"bitmapfonts = { bitmapfont = { name = f fontfiles = { gfx/fonts/f } } }",
        );
        write("art/interface/fonts.gfx", b"bitmapfonts = { }");
        let art = root.join("art").display().to_string().replace('\\', "/");
        write(
            "user/mod/art.mod",
            format!(
                "name=\"Art\"
path=\"{art}\"
"
            )
            .as_bytes(),
        );
        std::fs::create_dir_all(root.join("steamapps/common/Game")).unwrap();
        let options = content_reader_layers::ResolveOptions {
            game_dir: root.join("steamapps/common/Game"),
            user_dir: root.join("user"),
            cache_dir: root.join("cache"),
        };
        let base = root.join("base");
        let mut layers = ResolvedLayers::from_folders(&[(base.as_path(), &[][..])]).unwrap();
        let selected = [content_reader_layers::content::visual::Choice {
            id: "mod/art.mod".into(),
            name: "Art".into(),
        }];
        let cancel = std::sync::atomic::AtomicBool::new(false);
        content_reader_layers::content::visual::apply(&mut layers, &options, &selected, &[], &cancel)
            .unwrap();
        let presentation = layers.presentation().clone();
        assert!(presentation.shows_visual_mods());
        let registry =
            content_reader_gui::gui::native::resources::Registry::load(&presentation).unwrap();
        assert!(registry.fonts.get_or_fallback(b"").is_none());
        let interface = super::super::sprites::Interface {
            sprites: content_reader_gfx::gfx::SpriteSet::new(&registry.files),
            registry,
            decode: content_reader_image::native::Decode::default(),
        };
        let load = |layers: &ResolvedLayers| {
            Fonts::load(
                layers,
                &interface,
                "english",
                &DeclaredFonts::default(),
                &mut Vec::new(),
                &mut Vec::new(),
            )
        };
        let error = load(&presentation)
            .err()
            .expect("the mods' fonts are refused");
        assert!(
            error.to_string().contains("no bitmapfont is declared"),
            "{error}"
        );
        assert!(!layers.shows_visual_mods() && load(&layers).is_ok());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_refused_map_font_also_names_battle_plan_label_refusal() {
        content_reader_script::test_game::enter();
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

    #[test]
    fn a_missing_family_file_is_skipped_and_named() {
        content_reader_script::test_game::enter();
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

    #[test]
    fn a_page_divides_its_glyphs_by_the_extent_the_game_decodes() {
        content_reader_script::test_game::enter();
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
        assert_eq!((images[0].meta().width, images[0].meta().height), (8, 8));
        assert!(!images[0].is_held());
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn a_refused_font_does_not_load() {
        content_reader_script::test_game::enter();
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
        let stored = |authored: &str| {
            let loc: content_reader_localisation::localisation::Localisation =
                [("K".to_owned(), authored.to_owned())]
                    .into_iter()
                    .collect();
            loc.get("K").unwrap().to_owned()
        };
        assert_eq!(strip_color_codes(&stored("§YBerlin§!")), "Berlin");
        assert_eq!(strip_color_codes(&stored("a§Gb§!c")), "abc");
        assert_eq!(strip_color_codes("plain"), "plain");
        assert_eq!(strip_color_codes(&stored("tail§")), "tail");
        assert_eq!(strip_color_codes(&stored("§!§!")), "");
        assert_eq!(strip_color_codes(&stored("100§§")), "100§");
        assert_eq!(unsupported_control(&stored("§Ya§! b")), None);
        assert_eq!(
            unsupported_control(&stored("£pol_power 5")).as_deref(),
            Some("the icon 0x13 \"pol_power\" (an authored `£`)")
        );
        assert!(unsupported_control(&stored("@GER")).is_some());
        assert_eq!(stored("a¤b"), "a\u{14}b");
        assert_eq!(unsupported_control(&stored("a¤b")), None);
        assert!(unsupported_control(&stored("a¤{12")).is_some());
    }

    #[test]
    fn the_advance_sums_the_glyph_steps_and_the_kerning_pairs() {
        let font = font();
        assert_eq!(measure(&font, "AB"), 18.0);
        assert_eq!(measure(&font, "BA"), 20.0);
        assert_eq!(measure(&font, "A B"), 24.0);
        assert_eq!(measure(&font, "AZB"), 18.0);
        assert_eq!(measure(&font, "\u{11}YAB\u{11}!"), 18.0);
        assert_eq!(measure(&font, "A\u{11}GB\u{11}!"), 20.0);
    }

    #[test]
    fn alignment_places_the_run_inside_the_declared_box() {
        let font = font();
        let left = layout(&font, "AB", [100.0, 50.0], Align::Left, Some(149.0));
        assert_eq!(left.len(), 2);
        assert_eq!(left[0].page, 7);
        assert_eq!(left[0].rect, [101.0, 52.0, 8.0, 12.0]);
        assert_eq!(left[1].rect[0], 108.0);
        assert_eq!(left[0].uv, [0.0, 0.0, 8.0 / 64.0, 12.0 / 32.0]);

        let centre = layout(&font, "AB", [100.0, 50.0], Align::Centre, Some(50.0));
        assert_eq!(centre[0].rect[0], 100.0 + (50.0 - 18.0) * 0.5 + 1.0);
        let right = layout(&font, "AB", [100.0, 50.0], Align::Right, Some(50.0));
        assert_eq!(right[0].rect[0], 100.0 + 50.0 - 18.0 + 1.0);
        let exact_cut = layout(&font, "AB", [100.0, 50.0], Align::Centre, Some(10.0));
        assert!(exact_cut.is_empty());
        let tight = layout(&font, "AB", [100.0, 50.0], Align::Centre, Some(11.0));
        assert_eq!(tight.len(), 1);
        assert_eq!(tight[0].rect[0], 101.0);
        let free = layout(&font, "AB", [100.0, 50.0], Align::Centre, None);
        assert_eq!(free[0].rect[0], 101.0);
    }

    #[test]
    fn the_max_width_cuts_the_run_and_empty_glyphs_draw_nothing() {
        let font = font();
        let cut = layout(&font, "ABAB", [0.0, 0.0], Align::Left, Some(20.0));
        assert_eq!(cut.len(), 2);
        let spaced = layout(&font, "A B", [0.0, 0.0], Align::Left, None);
        assert_eq!(spaced.len(), 2);
        assert_eq!(spaced[1].rect[0], 14.0);
    }

    #[test]
    fn width_limited_layout_draws_native_ellipsis_with_kerning_and_ink_spacing() {
        let mut font = font();
        font.glyphs.glyphs.insert(
            '.' as u32,
            Glyph {
                x: 24,
                y: 0,
                width: 2,
                height: 3,
                xoffset: 1,
                yoffset: 9,
                xadvance: 4,
                page: 0,
                kernings: [('.' as u32, -1)].into_iter().collect(),
            },
        );
        font.glyphs
            .glyphs
            .get_mut(&('B' as u32))
            .unwrap()
            .kernings
            .insert('.' as u32, -1);
        font.glyphs.glyphs.insert(
            'W' as u32,
            Glyph {
                x: 32,
                y: 0,
                width: 18,
                height: 12,
                xoffset: 1,
                yoffset: 2,
                xadvance: 20,
                page: 0,
                kernings: BTreeMap::new(),
            },
        );
        let dot_uv = GlyphMetrics::glyph(&font, '.').unwrap().uv;
        let a_uv = GlyphMetrics::glyph(&font, 'A').unwrap().uv;
        let b_uv = GlyphMetrics::glyph(&font, 'B').unwrap().uv;

        let replaced_two = layout(&font, "ABABAB", [10.0, 20.0], Align::Left, Some(40.0));
        assert_eq!(
            replaced_two.iter().map(|g| g.uv).collect::<Vec<_>>(),
            vec![a_uv, b_uv, dot_uv, dot_uv, dot_uv]
        );
        assert_eq!(
            replaced_two.iter().map(|g| g.rect).collect::<Vec<_>>(),
            vec![
                [11.0, 22.0, 8.0, 12.0],
                [18.0, 22.0, 8.0, 12.0],
                [28.0, 29.0, 2.0, 3.0],
                [31.0, 29.0, 2.0, 3.0],
                [34.0, 29.0, 2.0, 3.0],
            ]
        );

        let appended = layout(&font, "ABAW", [0.0, 0.0], Align::Left, Some(44.0));
        assert_eq!(
            appended.iter().map(|g| g.uv).collect::<Vec<_>>(),
            vec![a_uv, b_uv, a_uv, dot_uv, dot_uv, dot_uv]
        );

        let replaced_one = layout(&font, "ABAW", [0.0, 0.0], Align::Left, Some(40.0));
        assert_eq!(
            replaced_one.iter().map(|g| g.uv).collect::<Vec<_>>(),
            vec![a_uv, b_uv, dot_uv, dot_uv, dot_uv]
        );

        let short = layout(&font, "ABW", [0.0, 0.0], Align::Left, Some(20.0));
        assert_eq!(
            short.iter().map(|g| g.uv).collect::<Vec<_>>(),
            vec![a_uv, b_uv]
        );

        let replaced_three = layout(&font, "A BA", [0.0, 0.0], Align::Left, Some(25.0));
        assert_eq!(
            replaced_three.iter().map(|g| g.uv).collect::<Vec<_>>(),
            vec![dot_uv, dot_uv, dot_uv]
        );
        assert_eq!(
            replaced_three.iter().map(|g| g.rect).collect::<Vec<_>>(),
            vec![
                [1.0, 9.0, 2.0, 3.0],
                [4.0, 9.0, 2.0, 3.0],
                [7.0, 9.0, 2.0, 3.0],
            ]
        );

        let centered = layout(&font, "ABABAB", [10.0, 20.0], Align::Centre, Some(40.0));
        assert_eq!(centered.len(), 5);
        assert_eq!(centered[0].rect[0], 10.0 + 7.0 + 1.0);
        let right = layout(&font, "ABABAB", [10.0, 20.0], Align::Right, Some(40.0));
        assert_eq!(right[0].rect[0], 10.0 + (40.0 - 27.0) + 1.0);

        let mut colored = font.clone();
        let yellow = [0.9, 0.8, 0.1, 1.0];
        colored.text_colors.insert(b'Y', yellow);
        let tinted_replaced = layout(&colored, "A\u{11}YBAW", [0.0, 0.0], Align::Left, Some(44.0));
        let expected_yellow = super::super::native_text::packed(yellow);
        assert_eq!(
            tinted_replaced.iter().map(|g| g.color).collect::<Vec<_>>(),
            vec![
                None,
                Some(expected_yellow),
                Some(expected_yellow),
                Some(expected_yellow),
                Some(expected_yellow),
            ]
        );
        let tinted_appended = layout(&colored, "A\u{11}YBAW", [0.0, 0.0], Align::Left, Some(46.0));
        assert_eq!(
            tinted_appended.iter().map(|g| g.color).collect::<Vec<_>>(),
            vec![
                None,
                Some(expected_yellow),
                Some(expected_yellow),
                Some(expected_yellow),
                Some(expected_yellow),
                Some(expected_yellow),
            ]
        );

        let mut without_dot = font.clone();
        without_dot.glyphs.glyphs.remove(&('.' as u32));
        let no_dot = layout(&without_dot, "ABABAB", [0.0, 0.0], Align::Left, Some(40.0));
        assert_eq!(
            no_dot.iter().map(|g| g.uv).collect::<Vec<_>>(),
            vec![a_uv, b_uv, a_uv, b_uv]
        );

        let unbounded = layout(&font, "ABABAB", [0.0, 0.0], Align::Left, None);
        assert_eq!(
            unbounded.iter().map(|g| g.uv).collect::<Vec<_>>(),
            vec![a_uv, b_uv, a_uv, b_uv, a_uv, b_uv]
        );
    }

    #[test]
    fn native_overflow_controls_fallback_and_integer_centring() {
        let mut font = font();
        let yellow = [0.9, 0.8, 0.1, 1.0];
        font.text_colors.insert(b'Y', yellow);
        font.glyphs.glyphs.insert(
            '.' as u32,
            Glyph {
                x: 24,
                y: 0,
                width: 2,
                height: 3,
                xoffset: 1,
                yoffset: 9,
                xadvance: 4,
                page: 0,
                kernings: [('.' as u32, -1)].into_iter().collect(),
            },
        );
        font.glyphs.glyphs.insert(
            'W' as u32,
            Glyph {
                x: 32,
                y: 0,
                width: 18,
                height: 12,
                xoffset: 1,
                yoffset: 2,
                xadvance: 20,
                page: 0,
                kernings: BTreeMap::new(),
            },
        );
        let a_uv = GlyphMetrics::glyph(&font, 'A').unwrap().uv;
        let b_uv = GlyphMetrics::glyph(&font, 'B').unwrap().uv;
        let dot_uv = GlyphMetrics::glyph(&font, '.').unwrap().uv;
        let expected_yellow = super::super::native_text::packed(yellow);

        let exact_boundary = layout(&font, "AAAA", [0.0, 0.0], Align::Left, Some(40.0));
        assert_eq!(
            exact_boundary.iter().map(|g| g.uv).collect::<Vec<_>>(),
            vec![a_uv, a_uv, dot_uv, dot_uv, dot_uv]
        );

        let no_ellipsis =
            layout_truncated(&font, "AAAA", [0.0, 0.0], Align::Left, Some(40.0), false);
        assert_eq!(
            no_ellipsis.iter().map(|g| g.uv).collect::<Vec<_>>(),
            vec![a_uv, a_uv, a_uv]
        );

        let zero_height_space = layout(&font, "AA B", [0.0, 0.0], Align::Left, Some(24.0));
        assert_eq!(
            zero_height_space.iter().map(|g| g.uv).collect::<Vec<_>>(),
            vec![dot_uv, dot_uv, dot_uv]
        );

        let trailing_color = layout(&font, "ABA\u{11}YW", [0.0, 0.0], Align::Left, Some(44.0));
        assert_eq!(
            trailing_color.iter().map(|g| g.uv).collect::<Vec<_>>(),
            vec![a_uv, b_uv, a_uv, dot_uv, dot_uv, dot_uv]
        );
        assert_eq!(
            trailing_color.iter().map(|g| g.color).collect::<Vec<_>>(),
            vec![
                None,
                None,
                None,
                Some(expected_yellow),
                Some(expected_yellow),
                Some(expected_yellow),
            ]
        );

        let missing_after_color = layout(&font, "A\u{11}YZB", [0.0, 0.0], Align::Left, None);
        assert_eq!(missing_after_color.len(), 2);
        assert_eq!(missing_after_color[1].rect[0], 10.0);

        let odd_centered = layout(&font, "AAAAW", [0.0, 0.0], Align::Centre, Some(41.0));
        assert_eq!(
            odd_centered.iter().map(|g| g.uv).collect::<Vec<_>>(),
            vec![a_uv, a_uv, dot_uv, dot_uv, dot_uv]
        );
        assert_eq!(odd_centered[0].rect[0], 6.0);

        let mut fallback_font = font.clone();
        fallback_font.glyphs.glyphs.remove(&('.' as u32));
        fallback_font.glyphs.glyphs.insert(
            '?' as u32,
            Glyph {
                x: 48,
                y: 0,
                width: 4,
                height: 10,
                xoffset: 0,
                yoffset: 1,
                xadvance: 4,
                page: 0,
                kernings: BTreeMap::new(),
            },
        );
        let q_uv = GlyphMetrics::glyph(&fallback_font, '?').unwrap().uv;
        let fallback_dots = layout(&fallback_font, "AAAA", [0.0, 0.0], Align::Left, Some(40.0));
        assert_eq!(
            fallback_dots.iter().map(|g| g.uv).collect::<Vec<_>>(),
            vec![a_uv, a_uv, q_uv, q_uv, q_uv]
        );
        assert_eq!(measure(&fallback_font, "AZB"), 18.0);
        let unbounded_missing = layout(&fallback_font, "AZB", [0.0, 0.0], Align::Left, None);
        assert_eq!(
            unbounded_missing.iter().map(|g| g.uv).collect::<Vec<_>>(),
            vec![a_uv, b_uv]
        );
        assert_eq!(unbounded_missing[1].rect[0], 8.0);
    }

    #[test]
    fn color_runs_preserve_nested_open_and_close_matching_native_screen_labels() {
        let mut font = font();
        let green = [0.2, 0.8, 0.3, 1.0];
        let yellow = [0.9, 0.8, 0.1, 1.0];
        font.text_colors.insert(b'G', green);
        font.global_colors.insert(b'Y', yellow);
        font.glyphs.glyphs.insert(
            'C' as u32,
            Glyph {
                x: 16,
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
        let loc: content_reader_localisation::localisation::Localisation =
            [("K".to_owned(), "A§YB§GC§!B§!A".to_owned())]
                .into_iter()
                .collect();
        let authored = loc.get("K").unwrap();
        assert_eq!(measure(&font, authored), 50.0);
        let glyphs = layout(&font, authored, [0.0, 0.0], Align::Left, None);
        assert_eq!(glyphs.len(), 5);
        let expected_yellow = super::super::native_text::packed(yellow);
        let expected_green = super::super::native_text::packed(green);
        assert_eq!(
            glyphs.iter().map(|g| g.color).collect::<Vec<_>>(),
            vec![
                None,
                Some(expected_yellow),
                Some(expected_green),
                Some(expected_yellow),
                None,
            ]
        );

        struct NativeGlyphs<'a>(&'a Font);
        impl super::super::native_text::Glyphs for NativeGlyphs<'_> {
            fn own(&self, ch: char) -> Option<super::super::native_text::Glyph> {
                let (glyph, _) = self.0.glyph_of(ch)?;
                Some(super::super::native_text::Glyph {
                    ch,
                    advance: glyph.xadvance,
                    width: glyph.width,
                    height: glyph.height,
                    xoffset: glyph.xoffset,
                    yoffset: glyph.yoffset,
                })
            }
            fn kerning(&self, a: char, b: char) -> i32 {
                GlyphMetrics::kerning(self.0, a, b) as i32
            }
            fn line_height(&self) -> i32 {
                self.0.line_height() as i32
            }
        }
        let own = [0.5, 0.5, 0.5, 0.5];
        let native_glyphs = NativeGlyphs(&font);
        let chars: Vec<char> = authored.chars().collect();
        let laid = super::super::native_text::lay_out(
            &native_glyphs,
            &|_| None,
            &chars,
            super::super::native_text::Limits {
                width: 200,
                height: 40,
                border: [0, 0],
                newlines: false,
                preserve_indent: false,
                ellipsis: false,
                multiline: false,
                special: true,
                icons_add_height: false,
            },
        )
        .unwrap();
        assert_eq!(laid.widths, vec![50]);
        let colors = super::super::native_text::Colors {
            own,
            font: &font.text_colors,
            global: &font.global_colors,
        };
        let pieces = super::super::native_text::draw(
            &native_glyphs,
            &|_| None,
            &laid,
            true,
            &super::super::native_text::Placement {
                top: 0,
                start: &|_, _| 0,
            },
            &colors,
        )
        .unwrap();
        let screen_colors: Vec<[f32; 4]> = pieces
            .into_iter()
            .filter_map(|piece| match piece {
                super::super::native_text::Piece::Glyph { color, .. } => Some(color),
                _ => None,
            })
            .collect();
        let packed_own = super::super::native_text::packed(own);
        let map_colors: Vec<[f32; 4]> = glyphs
            .iter()
            .map(|g| g.color.unwrap_or(packed_own))
            .collect();
        assert_eq!(map_colors, screen_colors);

        let mut batch = super::super::sprites::Batch::default();
        batch.text(&glyphs, packed_own);
        let batch_colors: Vec<[f32; 4]> = batch.quads.iter().map(|q| q.color).collect();
        assert_eq!(batch_colors, screen_colors);
        assert_eq!(
            batch_colors,
            vec![
                packed_own,
                expected_yellow,
                expected_green,
                expected_yellow,
                packed_own,
            ]
        );
    }
}
