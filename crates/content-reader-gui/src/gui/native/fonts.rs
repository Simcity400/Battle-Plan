#[cfg(test)]
use content_reader_layers::textures::texture_candidates;

use std::collections::BTreeMap;

use content_reader_parser::parser::read_bool;

use super::{
    Cause, Refusal,
    reader::{FILE, Reader, keyword_of, read_string, record},
    spill::{self, Layout},
};

pub type TextColors = BTreeMap<u8, [f32; 4]>;

#[derive(Clone, Debug, PartialEq)]
pub struct FontFacts {
    pub name: Vec<u8>,
    pub color: u32,
    pub border_color: u32,
    pub files: Vec<Vec<u8>>,
    pub icons_add_height: bool,
    pub icon_scale: f32,
    pub text_colors: TextColors,
    pub offsets: [i32; 4],
    pub viewport: [Option<i32>; 2],
    pub aux_color: [f32; 4],
    pub refusals: Vec<Refusal>,
}

impl FontFacts {
    pub fn cursor_offset(&self) -> [i32; 2] {
        [self.offsets[0], self.offsets[1]]
    }

    pub fn selection_offset(&self) -> [i32; 2] {
        [self.offsets[2], self.offsets[3]]
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct FontOverride {
    pub name: Vec<u8>,
    pub files: Vec<Vec<u8>>,
    pub languages: Vec<Vec<u8>>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum FontEvent {
    Font(FontFacts),
    Override(FontOverride),
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct FontFamily {
    pub files: Vec<Vec<u8>>,
    pub languages: Vec<Vec<u8>>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Font {
    pub facts: FontFacts,
    pub families: Vec<FontFamily>,
}

impl Font {
    pub fn family(&self, language: &[u8]) -> &FontFamily {
        self.families
            .iter()
            .find(|family| family.languages.iter().any(|own| own == language))
            .unwrap_or(&self.families[0])
    }
}

fn hash61(name: &[u8]) -> u32 {
    content_reader_parser::parser::c_string(name)
        .iter()
        .fold(0u32, |hash, byte| {
            hash.wrapping_add(*byte as i8 as i32 as u32)
                .wrapping_mul(61)
        })
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Fonts {
    fonts: Vec<Font>,
    buckets: BTreeMap<u32, Vec<usize>>,
}

const BUCKETS: u32 = 0x1FF;

impl Fonts {
    pub fn new<'a>(events: impl IntoIterator<Item = &'a FontEvent>) -> Self {
        let mut fonts = Self::default();
        for event in events {
            match event {
                FontEvent::Font(facts) => {
                    let index = fonts.fonts.len();
                    fonts.fonts.push(Font {
                        facts: facts.clone(),
                        families: vec![FontFamily {
                            files: facts.files.clone(),
                            languages: Vec::new(),
                        }],
                    });
                    fonts
                        .buckets
                        .entry(hash61(&facts.name) % BUCKETS)
                        .or_default()
                        .push(index);
                }
                FontEvent::Override(replace) => {
                    if let Some(index) = fonts.index_or_fallback(&replace.name) {
                        fonts.fonts[index].families.push(FontFamily {
                            files: replace.files.clone(),
                            languages: replace.languages.clone(),
                        });
                    }
                }
            }
        }
        fonts
    }

    pub fn get(&self, name: &[u8]) -> Option<&Font> {
        self.index(name).map(|index| &self.fonts[index])
    }

    pub fn get_or_fallback(&self, name: &[u8]) -> Option<&Font> {
        self.index_or_fallback(name).map(|index| &self.fonts[index])
    }

    fn index(&self, name: &[u8]) -> Option<usize> {
        let name = content_reader_parser::parser::c_string(name);
        self.buckets
            .get(&(hash61(name) % BUCKETS))?
            .iter()
            .rev()
            .copied()
            .find(|index| content_reader_parser::parser::c_string(&self.fonts[*index].facts.name) == name)
    }

    fn index_or_fallback(&self, name: &[u8]) -> Option<usize> {
        self.index(name).or_else(|| {
            self.buckets
                .values()
                .find_map(|bucket| bucket.last().copied())
        })
    }

    pub fn fonts(&self) -> &[Font] {
        &self.fonts
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Glyph {
    pub page: u32,
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
    pub xoffset: i32,
    pub yoffset: i32,
    pub xadvance: i32,
    pub kernings: BTreeMap<u32, i32>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct GlyphSet {
    pub line_height: Option<i32>,
    pub base: Option<i32>,
    pub pages: Vec<Vec<u8>>,
    pub glyphs: BTreeMap<u32, Glyph>,
    pub missing: Vec<String>,
}

pub fn load_glyphs<B: AsRef<[u8]> + std::ops::Deref<Target = [u8]> + Into<content_reader_image::Bytes>>(family: &FontFamily, read: &dyn Fn(&[u8]) -> Option<B>) -> GlyphSet {
    let mut set = GlyphSet::default();
    for (page, entry) in family.files.iter().enumerate() {
        let mut texture = entry.clone();
        texture.extend_from_slice(b".tga");
        set.pages.push(texture);
        let mut file = entry.clone();
        file.extend_from_slice(b".fnt");
        let Some(bytes) = read(&file) else {
            set.missing
                .push(String::from_utf8_lossy(&file).into_owned());
            continue;
        };
        read_fnt(&bytes, page as u32, &mut set);
    }
    set
}

fn read_fnt(bytes: &[u8], page: u32, set: &mut GlyphSet) {
    let mut reader = Reader::new(
        content_reader_script::native::lex(bytes, content_reader_script::native::Mode::KeepMark),
        0,
    );
    let number = |reader: &Reader| content_reader_parser::fixed5::decimal_i32(&reader.value.text);
    loop {
        let token = reader.peek();
        if token.is_end() {
            break;
        }
        let word = token.c_str().to_ascii_lowercase();
        match word.as_slice() {
            b"info" | b"common" | b"page" | b"chars" => {
                reader.read();
            }
            b"padding" | b"spacing" => {
                let count = if word == b"padding" { 9 } else { 5 };
                for _ in 0..count {
                    reader.read();
                }
            }
            b"lineheight" | b"base" => {
                reader.key_operator_value();
                let value = number(&reader);
                let slot = if word == b"base" {
                    &mut set.base
                } else {
                    &mut set.line_height
                };
                if page == 0 {
                    *slot = Some(value);
                }
                if word == b"base" {
                    reader.key_operator_value();
                }
            }
            b"char" => {
                reader.read();
                break;
            }
            _ => {
                reader.key_operator_value();
            }
        }
    }
    while reader.current.c_str() == b"char" {
        if reader.peek().is_end() {
            return;
        }
        let mut values = [0i32; 8];
        for value in &mut values {
            reader.key_operator_value();
            *value = number(&reader);
        }
        let [id, x, y, width, height, xoffset, yoffset, xadvance] = values;
        let id = id as u32;
        if id != 10 {
            set.glyphs.entry(id).or_insert(Glyph {
                page,
                x,
                y,
                width,
                height: if id == 0x20 { 0 } else { height },
                xoffset,
                yoffset,
                xadvance,
                kernings: BTreeMap::new(),
            });
        }
        loop {
            let token = reader.read_resolved();
            if token.c_str() == b"char" || token.c_str() == b"kerning" {
                break;
            }
            if reader.peek().is_end() {
                return;
            }
        }
    }
    while reader.current.c_str() == b"kerning" {
        if reader.peek().is_end() {
            return;
        }
        let mut values = [0i32; 3];
        for value in &mut values {
            reader.key_operator_value();
            *value = number(&reader);
        }
        let [first, second, amount] = values;
        if let Some(glyph) = set.glyphs.get_mut(&(first as u32)) {
            glyph.kernings.entry(second as u32).or_insert(amount);
        }
        reader.read();
    }
}

struct FontBytes<'a> {
    font: &'a mut FontFacts,
    image: &'a mut FontImage,
}

const FILES: spill::VectorSite = spill::VectorSite {
    what: "the font's file list",
    append: "font reader (string vector append)",
    readers: "graphics resource initialization (font family append, family 0)",
    destructor: "font destructor",
};

#[derive(Clone, Debug)]
struct FontImage {
    files: spill::Vector<Vec<u8>>,
    entries: Vec<(u8, [f32; 4])>,
    zero: bool,
    writer: &'static str,
}

impl Default for FontImage {
    fn default() -> Self {
        Self {
            files: spill::Vector::new(FILES),
            entries: vec![(0, [1.0; 4]); 256],
            zero: false,
            writer: "cursor_offset",
        }
    }
}

impl FontImage {
    fn table(&self) -> TextColors {
        self.entries
            .iter()
            .enumerate()
            .filter(|(code, (flag, _))| *flag != 0 || (*code == 0 && self.zero))
            .map(|(code, (_, color))| (code as u8, *color))
            .collect()
    }
}

impl Layout for FontBytes<'_> {
    fn end(&self) -> (usize, String) {
        spill::heap_end(0x3620, "graphics file reader")
    }

    fn write(&mut self, offset: usize, bits: u32) -> Result<(), String> {
        let font = &mut *self.font;
        let float = f32::from_bits(bits);
        match offset {
            0x30..=0x3C => font.offsets[(offset - 0x30) / 4] = bits as i32,
            0x40 | 0x44 => {
                return Err(
                    "the font's lost-device interface vtable (font virtual-function table at \
                 the image's load address, which the replay does not hold), registered for the \
                 renderer to call through at every device reset"
                        .into(),
                );
            }
            0x48 | 0x4C => {
                return Err(
                    "the font's graphics context pointer, which the glyph loader reads when \
                 the font's files load: a heap address the replay does not hold"
                        .into(),
                );
            }
            0x50 | 0x54 => font.viewport[(offset - 0x50) / 4] = Some(bits as i32),
            0x58 => font.color = bits,
            0x60 => font.border_color = bits,
            0x5C..=0xBC => {}
            0xC0..=0xCC => font.aux_color[(offset - 0xC0) / 4] = float,
            0xD0..=0xE4 => self
                .image
                .files
                .write(offset - 0xD0, bits, self.image.writer)?,
            0xE8..=0xF4 if bits == 0 => {}
            0xE8..=0xFC => {
                return Err(
                    "the font's families vector, through which the graphics resource \
                 initialization appends family 0 at the font's `}`: a null block, a count past \
                 its capacity, or an allocator at an address the replay does not hold"
                        .into(),
                );
            }
            0x100..=0x10C => {}
            0x110..=0x310C => {
                let (entry, at) = ((offset - 0x110) / 0x30, (offset - 0x110) % 0x30);
                let (code, color) = &mut self.image.entries[entry];
                match at {
                    0 => *code = bits.to_le_bytes()[0],
                    0x20..=0x2C => color[(at - 0x20) / 4] = float,
                    _ => {}
                }
            }
            0x3110 => font.icons_add_height = bits.to_le_bytes()[0] != 0,
            0x3114 => font.icon_scale = float,
            0x3118..=0x3614 => {
                match (offset - 0x3118) % 0x20 {
                    0 if bits != 0 => {
                        return Err("a glyph cache record's text key, which the font draw \
                         matches with a drawn text's hash until the slot ages out on the game's \
                         render clock, drawing nothing for a null buffer: a clock the replay does \
                         not hold"
                            .into());
                    }
                    0x18 | 0x1C if bits != 0 => {
                        return Err("a glyph cache record's vertex buffer, which font \
                         destruction and glyph vertex buffer release free, and drawing binds: \
                         bits that are no buffer"
                            .into());
                    }
                    _ => {}
                }
            }
            _ => {}
        }
        Ok(())
    }
}

impl Reader<'_> {
    pub(super) fn font(&mut self) -> FontFacts {
        let mut font = FontFacts {
            name: Vec::new(),
            color: 0,
            border_color: 0,
            files: Vec::new(),
            icons_add_height: false,
            icon_scale: 1.0,
            text_colors: TextColors::new(),
            offsets: [0; 4],
            viewport: [None; 2],
            aux_color: [1.0; 4],
            refusals: Vec::new(),
        };
        let mut image = FontImage::default();
        while self.next_key() {
            let text = self.value.text.clone();
            let word = keyword_of(&self.key.clone()).unwrap_or_default();
            match word {
                "name" => font.name = read_string(&text),
                "color" => font.color = content_reader_parser::crt::scan_x32(&text).stored().unwrap_or(font.color),
                "border_color" => {
                    font.border_color = content_reader_parser::crt::scan_x32(&text)
                        .stored()
                        .unwrap_or(font.border_color);
                }
                "fontname" => self.skip_reported(),
                "colorcodes" | "color_override" => self.skip_current_block(),
                "path" => {
                    if let Err(reason) = image.files.append(read_string(&text)) {
                        record(&mut font.refusals, FILE, word, reason, Cause::Unread);
                    }
                }
                "icons_add_height" => {
                    font.icons_add_height = read_bool(&text).unwrap_or(font.icons_add_height);
                }
                "icon_scale" => {
                    font.icon_scale = content_reader_parser::crt::scan_f32(&text).unwrap_or(font.icon_scale)
                }
                "textcolors" => {
                    let mut refusals = Vec::new();
                    let mut table = image.table();
                    let before = table.clone();
                    self.text_colors(&mut table, &mut refusals);
                    for (code, color) in table {
                        if before.get(&code) != Some(&color) {
                            image.entries[usize::from(code)] = (code, color);
                            image.zero |= code == 0;
                        }
                    }
                    font.refusals.extend(refusals);
                }
                "fontfiles" => {
                    let names = self.string_list();
                    let result = image.files.clear("font reader").and_then(|()| {
                        names
                            .into_iter()
                            .try_for_each(|name| image.files.append(name))
                    });
                    if let Err(reason) = result {
                        record(&mut font.refusals, FILE, word, reason, Cause::Unread);
                    }
                }
                "cursor_offset" | "selection_offset" => {
                    let start = if word == "cursor_offset" { 0x30 } else { 0x38 };
                    let mut written = Vec::new();
                    let result =
                        self.components(|index, value| written.push((index, value as u32)));
                    image.writer = if word == "cursor_offset" {
                        "cursor_offset"
                    } else {
                        "selection_offset"
                    };
                    let mut bytes = FontBytes {
                        font: &mut font,
                        image: &mut image,
                    };
                    if let Err(reason) =
                        spill::spill(&mut bytes, "integer component reader", start, written)
                    {
                        record(&mut font.refusals, FILE, word, reason, Cause::Unread);
                    }
                    if let Err(reason) = result {
                        self.poison(&mut font.refusals, FILE, word, &reason);
                    }
                }
                _ => self.unexpected(),
            }
        }
        match image.files.visible() {
            Ok(files) => font.files = files,
            Err(reason) => record(
                &mut font.refusals,
                FILE,
                image.writer,
                reason,
                Cause::Unread,
            ),
        }
        font.text_colors = image.table();
        let name = String::from_utf8_lossy(&font.name).into_owned();
        for refusal in &mut font.refusals {
            refusal.path.clone_from(&name);
        }
        font
    }

    pub(super) fn font_override(&mut self) -> FontOverride {
        let mut replace = FontOverride::default();
        while self.next_key() {
            let word = keyword_of(&self.key.clone()).unwrap_or_default();
            match word {
                "name" => replace.name = read_string(&self.value.text),
                "fontfiles" => replace.files.extend(self.string_list()),
                "languages" => replace.languages.extend(self.string_list()),
                _ => {}
            }
        }
        replace
    }

    pub(crate) fn string_list(&mut self) -> Vec<Vec<u8>> {
        let mut list = Vec::new();
        loop {
            if self.peek().ends_block() {
                self.read();
                return list;
            }
            let token = self.read_resolved();
            list.push(read_string(&token.text));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gui::native::sprites::read_sprites;

    fn events(bytes: &[u8]) -> Vec<FontEvent> {
        read_sprites(bytes).fonts
    }

    #[test]
    fn a_font_skips_its_deprecated_keys_as_the_game_does() {
        content_reader_script::test_game::enter();
        let font = |text: &[u8]| -> FontFacts {
            let read = events(text);
            let Some(FontEvent::Font(font)) = read.into_iter().next() else {
                panic!("no font");
            };
            font
        };
        let scalar = font(
            b"bitmapfonts = { bitmapfont = { name = \"f\" fontName=\"old\" path=\"a.fnt\" \
              color=0xffffffff } }",
        );
        assert_eq!(scalar.files, [b"a.fnt".to_vec()]);
        assert_eq!(scalar.color, 0xFFFF_FFFF);
        let nested = font(
            b"@o = {\nbitmapfonts = { bitmapfont = { name = \"f\" fontName = { a = { @o } } } \
              path = \"b.fnt\" } } }",
        );
        assert_eq!(nested.files, [b"b.fnt".to_vec()]);
        let raw = font(
            b"@o = {\nbitmapfonts = { bitmapfont = { name = \"f\" colorCodes = { a = { @o } } \
              path = \"c.fnt\" } }",
        );
        assert_eq!(raw.files, [b"c.fnt".to_vec()]);
    }

    #[test]
    fn a_font_keeps_its_ordered_file_operations() {
        content_reader_script::test_game::enter();
        let read = events(
            b"bitmapfonts = { bitmapfont = { name = \"f\" path = \"a\" fontfiles = { \"b\" \"c\" } \
              path = \"d\" color = 0xff102030 border_color = 0x80000000 icons_add_height = yes \
              icon_scale = 0.5 cursor_offset = { 1 2 } selection_offset = { x = 3 y = 4 } \
              textcolors = { Y = { 0 0 0 } Y = { 1 1 1 } } fontName = { x } } }",
        );
        let FontEvent::Font(font) = &read[0] else {
            panic!("{read:?}");
        };
        assert_eq!(font.files, [b"b".to_vec(), b"c".to_vec(), b"d".to_vec()]);
        assert_eq!((font.color, font.border_color), (0xFF10_2030, 0x8000_0000));
        assert!(font.icons_add_height);
        assert_eq!(font.icon_scale, 0.5);
        assert_eq!(
            (font.cursor_offset(), font.selection_offset()),
            ([1, 2], [3, 4])
        );
        assert_eq!(font.text_colors[&b'Y'], [0.0, 0.0, 0.0, 1.0]);
        assert!(font.refusals.is_empty(), "{:?}", font.refusals);
    }

    #[test]
    fn offset_components_land_on_the_font() {
        content_reader_script::test_game::enter();
        let font = |text: &str| {
            let read = events(
                format!("bitmapfonts = {{ bitmapfont = {{ name = \"f\" {text} }} }}").as_bytes(),
            );
            let FontEvent::Font(font) = &read[0] else {
                panic!("{read:?}");
            };
            font.clone()
        };
        let four = font("cursor_offset = { 1 2 3 4 }");
        assert!(four.refusals.is_empty(), "{:?}", four.refusals);
        assert_eq!(
            (four.cursor_offset(), four.selection_offset()),
            ([1, 2], [3, 4])
        );
        let vtable = font("selection_offset = { 1 2 3 }");
        assert!(
            vtable.refusals[0]
                .reason
                .contains("lost-device interface vtable"),
            "{:?}",
            vtable.refusals
        );
        let graphics = font("cursor_offset = { 1 2 3 4 0x 0x 5 }");
        assert!(
            graphics.refusals[0].reason.contains("glyph loader"),
            "{:?}",
            graphics.refusals
        );
    }

    #[test]
    fn offset_components_reach_every_field_of_the_font() {
        content_reader_script::test_game::enter();
        let font = |before: &str, offsets: &[(usize, &str)], after: &str| {
            let last = offsets.iter().map(|(at, _)| (at - 0x30) / 4).max().unwrap();
            let mut components = vec!["-"; last + 1];
            for (at, value) in offsets {
                components[(at - 0x30) / 4] = value;
            }
            let read = events(
                format!(
                    "bitmapfonts = {{ bitmapfont = {{ name = \"f\" {before} cursor_offset = {{ {} }} \
                     {after} }} }}",
                    components.join(" ")
                )
                .as_bytes(),
            );
            let FontEvent::Font(font) = &read[0] else {
                panic!("{read:?}");
            };
            font.clone()
        };
        let one = 1.0f32.to_bits().to_string();
        let half = 0.5f32.to_bits().to_string();
        let fields = font(
            "path = \"a\" path = \"b\" textcolors = { G = { 0 1 0 } }",
            &[
                (0x50, "800"),
                (0x54, "600"),
                (0x58, "0x10"),
                (0x60, "7"),
                (0xC0, &half),
                (0xDC, "1"),
                (0x110 + 0x52 * 0x30, "82"),
                (0x110 + 0x52 * 0x30 + 0x20, &half),
                (0x3110, "1"),
                (0x3114, &one),
                (0x3118 + 0x8, "5"),
                (0x3618, "9"),
            ],
            "",
        );
        assert!(fields.refusals.is_empty(), "{:?}", fields.refusals);
        assert_eq!(fields.viewport, [Some(800), Some(600)]);
        assert_eq!((fields.color, fields.border_color), (16, 7));
        assert_eq!(fields.aux_color, [0.5, 1.0, 1.0, 1.0]);
        assert_eq!(fields.files, [b"a".to_vec()]);
        assert_eq!(fields.text_colors[&b'R'], [0.5, 1.0, 1.0, 1.0]);
        assert_eq!(fields.text_colors[&b'G'], [0.0, 1.0, 0.0, 1.0]);
        assert!(fields.icons_add_height);
        assert_eq!(fields.icon_scale, 1.0);
        let kept = font(
            "",
            &[(0x110 + 0x52 * 0x30, "82")],
            "textcolors = { R = { 255 0 0 } }",
        );
        assert_eq!(kept.text_colors[&b'R'], [1.0; 4]);
        let null = font("", &[(0xD8, "3")], "path = \"x\"");
        assert!(
            null.refusals[0].reason.contains("the font's file list"),
            "{:?}",
            null.refusals
        );
        let families = font("", &[(0xF4, "1")], "");
        assert!(
            families.refusals[0]
                .reason
                .contains("the font's families vector")
        );
        let key = font("", &[(0x3118 + 0x20 * 3, "1")], "");
        assert!(key.refusals[0].reason.contains("render clock"));
        let buffer = font("", &[(0x3118 + 0x18, "1")], "");
        assert!(
            buffer.refusals[0]
                .reason
                .contains("glyph vertex buffer release")
        );
        let past = font("", &[(0x3620, "1")], "");
        assert!(past.refusals[0].reason.contains("heap overflow"));
    }

    #[test]
    fn overrides_and_fallbacks_follow_the_registry() {
        content_reader_script::test_game::enter();
        let fonts = Fonts::new(&events(
            b"bitmapfonts = { bitmapfont = { name = \"f\" path = \"one\" } \
              bitmapfont_override = { name = \"f\" fontfiles = { \"kr\" } languages = { \"l_korean\" } path = \"x\" } \
              bitmapfont = { name = \"f\" path = \"two\" } \
              bitmapfont_override = { name = \"missing\" fontfiles = { \"fb\" } languages = { \"l_x\" } } }",
        ));
        let latest = fonts.get(b"f").unwrap();
        assert_eq!(latest.facts.files, [b"two".to_vec()]);
        assert_eq!(fonts.fonts()[0].families.len(), 2);
        assert_eq!(latest.families.len(), 2);
        assert_eq!(fonts.fonts()[0].family(b"l_korean").files, [b"kr".to_vec()]);
        assert_eq!(
            fonts.fonts()[0].family(b"l_english").files,
            [b"one".to_vec()]
        );
        assert_eq!(
            fonts.get_or_fallback(b"nothing").unwrap().facts.name,
            b"f".to_vec()
        );
        assert_eq!(hash61(b"a"), 97 * 61);
    }

    #[test]
    fn glyphs_load_in_token_order() {
        content_reader_script::test_game::enter();
        let files: BTreeMap<&[u8], &[u8]> = BTreeMap::from([
            (
                &b"a.fnt"[..],
                &b"info face=\"x\" size=14\ncommon lineHeight=18 base=14 scaleW=256 scaleH=256 pages=1\n\
                   page id=0 file=\"a.tga\"\nchars count=2\n\
                   char id=65 x=1 y=2 width=3 height=4 xoffset=-1 yoffset=5 xadvance=6 page=0 chnl=15\n\
                   char id=32 x=0 y=0 width=0 height=9 xoffset=0 yoffset=0 xadvance=4 page=0 chnl=15\n\
                   kernings count=1\nkerning first=65 second=66 amount=-2\n"[..],
            ),
            (
                &b"b.fnt"[..],
                &b"common lineHeight=99 base=1\nchar id=65 x=9 y=9 width=9 height=9 xoffset=9 yoffset=9 xadvance=9\n\
                   char id=66 x=7 y=7 width=7 height=7 xoffset=7 yoffset=7 xadvance=7\n"[..],
            ),
            (&b"d\x80.fnt"[..], &b"common lineHeight=77 base=7\n"[..]),
        ]);
        let set = load_glyphs(
            &FontFamily {
                files: vec![
                    b"a".to_vec(),
                    b"b".to_vec(),
                    b"c".to_vec(),
                    b"d\x80".to_vec(),
                ],
                languages: Vec::new(),
            },
            &|path| files.get(path).map(|bytes| bytes.to_vec()),
        );
        assert_eq!((set.line_height, set.base), (Some(18), Some(14)));
        assert_eq!(set.pages.len(), 4);
        assert_eq!(set.missing, ["c.fnt"]);
        let a = &set.glyphs[&65];
        assert_eq!((a.x, a.xoffset, a.xadvance, a.page), (1, -1, 6, 0));
        assert_eq!(a.kernings[&66], -2);
        assert_eq!(set.glyphs[&32].height, 0);
        assert_eq!(set.glyphs[&66].page, 1);
        assert_eq!(
            texture_candidates(b"gfx\\fonts\\a.tga"),
            [b"gfx/fonts/a.dds", b"gfx/fonts/a.tga", b"gfx/fonts/a.tga"]
        );
    }

    #[test]
    fn glyphs_keep_negative_ids_and_skip_trailing_pairs() {
        content_reader_script::test_game::enter();
        let fnt: &[u8] = b"info face=\"Monofonto\" size=-20\n\
            common lineHeight=24 base=20 scaleW=1024 scaleH=512 pages=1\n\
            char id=-1   x=530 y=150 width=9 height=24 xoffset=0 yoffset=0 xadvance=10 page=0 chnl=15\n\
            char id=65   x=1 y=2 width=3 height=4 xoffset=0 yoffset=0 xadvance=5 page=0 chnl=15\n\
            \t\t\tpage=0\n\t\t\tpage=0\n";
        let set = load_glyphs(
            &FontFamily {
                files: vec![b"f".to_vec()],
                languages: Vec::new(),
            },
            &|path| (path == b"f.fnt").then(|| fnt.to_vec()),
        );
        assert_eq!((set.line_height, set.base), (Some(24), Some(20)));
        assert_eq!(set.glyphs.len(), 2);
        assert_eq!(set.glyphs[&u32::MAX].xadvance, 10);
        assert_eq!(set.glyphs[&65].xadvance, 5);
        assert!(set.missing.is_empty());
    }
}
