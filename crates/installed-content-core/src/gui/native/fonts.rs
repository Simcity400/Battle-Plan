//! The game's bitmap fonts as read and loaded: `bitmapfont` facts, overrides, the 511-bucket
//! font registry and its fallback, language families and glyphs from each family's files.
//!
//! The file list keeps `path` and `fontfiles` in order: each `path` appends, while each `fontfiles`
//! clears and replaces the list. At the font's `}`, the list becomes family 0 with no languages. A
//! `bitmapfont_override` appends a family to the font found by lookup and fallback at the time of
//! reading; repeated `fontfiles` and `languages` lists concatenate. At each `.gfx` file's end,
//! every font selects the first family containing the game's language byte for byte, else family 0,
//! and loads that family's files in order.

use std::collections::BTreeMap;

use crate::parser::read_bool;

use super::{
    Cause, Refusal,
    reader::{FILE, Reader, keyword_of, read_string, record},
    spill::{self, Layout},
};

/// A text colour table, by the code's byte: straight RGBA.
pub type TextColors = BTreeMap<u8, [f32; 4]>;

/// The stored facts and constructor defaults of one `bitmapfont`.
#[derive(Clone, Debug, PartialEq)]
pub struct FontFacts {
    /// `name`: the last assignment.
    pub name: Vec<u8>,
    /// `color` (0): `%x`, `0xAARRGGBB`, unpacked as the text object's color. A font without one
    /// draws (0, 0, 0, 0).
    pub color: u32,
    /// `border_color` (0): `%x`, written raw as each glyph vertex's second color.
    pub border_color: u32,
    /// The file list: each `path` appended, each `fontfiles` replacing it.
    pub files: Vec<Vec<u8>>,
    /// `icons_add_height`: raises a line to an inline icon's height.
    pub icons_add_height: bool,
    /// `icon_scale` (`%f`, 1.0).
    pub icon_scale: f32,
    /// The font's own `textcolors`: the first colour of each code (code 0 the last).
    pub text_colors: TextColors,
    /// `cursor_offset` and `selection_offset`, integer component reader: the four `i32`s from the
    /// cursor offset on.
    pub offsets: [i32; 4],
    /// The viewport size the font draw divides by (`2 / w`, `2 / h`): the constructor copies the
    /// graphics context's render size (as it is when the font is built) and nothing rewrites it;
    /// `None` keeps that copy, `Some` is what an offset's component wrote.
    pub viewport: [Option<i32>; 2],
    /// The auxiliary color uploaded as `ModColor`, initially (1, 1, 1, 1). An offset component can
    /// write it. Text object rendering sets its own auxiliary color before drawing and restores
    /// white afterwards, so a written color reaches only a text sprite bake occurring before any
    /// text object has drawn with the font.
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

/// A `bitmapfont_override` as read.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct FontOverride {
    /// `name`.
    pub name: Vec<u8>,
    /// `fontfiles` and `languages`: string list reader, each repeat appending.
    pub files: Vec<Vec<u8>>,
    pub languages: Vec<Vec<u8>>,
}

/// A `.gfx` file's font declarations, in reading order.
#[derive(Clone, Debug, PartialEq)]
pub enum FontEvent {
    Font(FontFacts),
    Override(FontOverride),
}

/// One font family: the files it loads and the languages that select it (none for the font's
/// own family 0).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct FontFamily {
    pub files: Vec<Vec<u8>>,
    pub languages: Vec<Vec<u8>>,
}

/// A registered font: its facts and its families in order.
#[derive(Clone, Debug, PartialEq)]
pub struct Font {
    pub facts: FontFacts,
    pub families: Vec<FontFamily>,
}

impl Font {
    /// The first family containing `language` byte for byte, else family 0.
    pub fn family(&self, language: &[u8]) -> &FontFamily {
        self.families
            .iter()
            .find(|family| family.languages.iter().any(|own| own == language))
            .unwrap_or(&self.families[0])
    }
}

/// Font name hash: start at 0, add each byte up to the first NUL as a signed char, then
/// multiply by 61, wrapping at 32 bits.
fn hash61(name: &[u8]) -> u32 {
    crate::parser::c_string(name)
        .iter()
        .fold(0u32, |hash, byte| {
            hash.wrapping_add(*byte as i8 as i32 as u32)
                .wrapping_mul(61)
        })
}

/// The font registry the graphics context keeps (511 buckets), each font pushed on its bucket's
/// front once read.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Fonts {
    fonts: Vec<Font>,
    /// Each bucket's fonts, most recently pushed last.
    buckets: BTreeMap<u32, Vec<usize>>,
}

/// The font registry's initial bucket count, never grown.
const BUCKETS: u32 = 0x1FF;

impl Fonts {
    /// Registers the font events of `files` in the game's load order.
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

    /// The latest font of `name`, compared byte for byte.
    pub fn get(&self, name: &[u8]) -> Option<&Font> {
        self.index(name).map(|index| &self.fonts[index])
    }

    /// The latest font of `name`; if missing, the most recently pushed font in the lowest
    /// nonempty bucket. `None` when no fonts are registered.
    pub fn get_or_fallback(&self, name: &[u8]) -> Option<&Font> {
        self.index_or_fallback(name).map(|index| &self.fonts[index])
    }

    fn index(&self, name: &[u8]) -> Option<usize> {
        let name = crate::parser::c_string(name);
        self.buckets
            .get(&(hash61(name) % BUCKETS))?
            .iter()
            .rev()
            .copied()
            .find(|index| crate::parser::c_string(&self.fonts[*index].facts.name) == name)
    }

    fn index_or_fallback(&self, name: &[u8]) -> Option<usize> {
        self.index(name).or_else(|| {
            self.buckets
                .values()
                .find_map(|bucket| bucket.last().copied())
        })
    }

    /// Every registered font, in reading order.
    pub fn fonts(&self) -> &[Font] {
        &self.fonts
    }
}

/// The stored facts of one glyph.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Glyph {
    /// The index of the family file it came from: that file's page.
    pub page: u32,
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
    pub xoffset: i32,
    pub yoffset: i32,
    pub xadvance: i32,
    /// `(second, amount)`, the first pair of each second kept.
    pub kernings: BTreeMap<u32, i32>,
}

/// A family's glyphs from each file in order, with the first file holding a codepoint and a
/// kerning pair winning.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct GlyphSet {
    /// `lineHeight` and `base`, from the first file only.
    pub line_height: Option<i32>,
    pub base: Option<i32>,
    /// Each file's page texture as registered: the entry and `.tga` (resolved through
    /// [`texture_candidates`]); a page is appended even when its `.fnt` is missing.
    pub pages: Vec<Vec<u8>>,
    pub glyphs: BTreeMap<u32, Glyph>,
    /// The missing `.fnt` paths, retained as text for messages.
    pub missing: Vec<String>,
}

/// The paths texture extension resolver tries for a texture name, as bytes: `\`
/// made `/`, then, cut at its last `.`, the `.dds`, the `.tga`, then the name as it is; none for
/// a name without a `.`. A caller opens each by its bytes
/// ([`crate::ResolvedLayers::read_uncovered_path`]).
pub fn texture_candidates(name: &[u8]) -> Vec<Vec<u8>> {
    let name: Vec<u8> = crate::parser::c_string(name)
        .iter()
        .map(|byte| if *byte == b'\\' { b'/' } else { *byte })
        .collect();
    let Some(dot) = name.iter().rposition(|byte| *byte == b'.') else {
        return Vec::new();
    };
    let base = &name[..dot];
    vec![
        [base, b".dds"].concat(),
        [base, b".tga"].concat(),
        name.clone(),
    ]
}

/// Loads `family`'s glyphs, reading each `.fnt` by its bytes through `read`.
pub fn load_glyphs(family: &FontFamily, read: &dyn Fn(&[u8]) -> Option<Vec<u8>>) -> GlyphSet {
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

/// Read one `.fnt` token by token; line boundaries do not affect the lexer.
fn read_fnt(bytes: &[u8], page: u32, set: &mut GlyphSet) {
    let mut reader = Reader::new(
        crate::parser::native::lex(bytes, crate::parser::native::Mode::KeepMark),
        0,
    );
    let number = |reader: &Reader| crate::fixed5::decimal_i32(&reader.value.text);
    // The header compares each peeked token without ASCII case sensitivity.
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
                    // Falls through to one more key and value (normally `scaleW`).
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
    // The `char` records: the current token must be exactly `char`.
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
        // To the next `char` or `kerning`, or the end.
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

/// The bitmap font's 0x3620 bytes as offset components write into them from the cursor offset on.
struct FontBytes<'a> {
    font: &'a mut FontFacts,
    image: &'a mut FontImage,
}

/// The file list's vector, initially empty.
const FILES: spill::VectorSite = spill::VectorSite {
    what: "the font's file list",
    append: "font reader (string vector append)",
    readers: "graphics resource initialization (font family append, family 0)",
    destructor: "font destructor",
};

/// A `bitmapfont`'s state beyond its named fields during reading: the file vector, text color
/// entries (0x30 bytes each, after the active family: code, then color, then RGBA) and the key
/// whose components wrote them.
#[derive(Clone, Debug)]
struct FontImage {
    files: spill::Vector<Vec<u8>>,
    /// Each entry's code byte and RGBA.
    entries: Vec<(u8, [f32; 4])>,
    /// Whether `textcolors` assigned code 0 (its code byte stays 0).
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
    /// The table exposed by [`FontFacts::text_colors`]: entries with a set code byte, which
    /// The text color lookup uses, and code 0 once assigned.
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
            // `cursor_offset` and `selection_offset`.
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
            // The saved text color and the current text colors, rewritten by color setting or
            // restoration before draws read them. The words between the border color and the
            // current colors have no constructor write or reader. The three color objects'
            // virtual-function tables and padding are unused.
            0x5C..=0xBC => {}
            0xC0..=0xCC => font.aux_color[(offset - 0xC0) / 4] = float,
            0xD0..=0xE4 => self
                .image
                .files
                .write(offset - 0xD0, bits, self.image.writer)?,
            // The families' vector (data, capacity, count and allocator allocator) is empty while
            // the font is read. At `}`, resource initialization appends family 0; changed bits can
            // send that append through null, past capacity or through an unavailable allocator
            // address.
            0xE8..=0xF4 if bits == 0 => {}
            0xE8..=0xFC => {
                return Err(
                    "the font's families vector, through which the graphics resource \
                 initialization appends family 0 at the font's `}`: a null block, a count past \
                 its capacity, or an allocator at an address the replay does not hold"
                        .into(),
                );
            }
            // The active family, selected and loaded before any draw; then padding.
            0x100..=0x10C => {}
            0x110..=0x310C => {
                let (entry, at) = ((offset - 0x110) / 0x30, (offset - 0x110) % 0x30);
                let (code, color) = &mut self.image.entries[entry];
                match at {
                    0 => *code = bits.to_le_bytes()[0],
                    0x20..=0x2C => color[(at - 0x20) / 4] = float,
                    // Padding and the color object's vtable and unwritten field, which nothing reads.
                    _ => {}
                }
            }
            0x3110 => font.icons_add_height = bits.to_le_bytes()[0] != 0,
            0x3114 => font.icon_scale = float,
            0x3118..=0x3614 => {
                // The forty glyph buffer cache records (0x20 bytes: text key, last use, texture,
                // vertex count, vertex buffer), zero until the first draw.
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
                    // The last use only picks the slot a miss reuses; the texture and count are
                    // read only past a non-null buffer.
                    _ => {}
                }
            }
            // Padding to the object's end.
            _ => {}
        }
        Ok(())
    }
}

impl Reader<'_> {
    /// Read one `bitmapfont`, falling back to common font fields.
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
                // `%x` into the field: a scan matching nothing reports an error and skips
                // nothing.
                "color" => font.color = crate::crt::scan_x32(&text).stored().unwrap_or(font.color),
                "border_color" => {
                    font.border_color = crate::crt::scan_x32(&text)
                        .stored()
                        .unwrap_or(font.border_color);
                }
                // The deprecated `fontName` key skips a block with variable resolution.
                "fontname" => self.skip_reported(),
                // Deprecated: skip the raw block.
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
                    font.icon_scale = crate::crt::scan_f32(&text).unwrap_or(font.icon_scale)
                }
                // Read text color entries: retain a set code byte, or store a new code byte and
                // its color.
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
                // Destroy the counted strings, then read the replacement list.
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
                // Read integer components into the cursor or selection offset, including spills
                // beyond them ([`FontBytes`]).
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
        // At `}`, copy the file list into family 0.
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

    /// Read one `bitmapfont_override`: `name`, then append `fontfiles` and `languages`. Other
    /// keys return without skipping anything.
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

    /// Read a string list without checking its value. Consume and resolve every token as a
    /// string, including `=` and `{`, until consuming a peeked `}` or EOF.
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

    /// `fontName` skips a block with variable resolution. Scalars skip nothing, so later keys
    /// are read; blocks skip through `}`, and a reference to a stored `{` starts nesting.
    /// `colorCodes` skips the raw value block, using braces as written.
    #[test]
    fn a_font_skips_its_deprecated_keys_as_the_game_does() {
        engine_executable::executable_tables::initialize_for_tests()
            .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
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

    /// `path` appends and `fontfiles` replaces, in order; scalars take their value only, and a
    /// `{` under `path` is its text, the block read on as keys.
    #[test]
    fn a_font_keeps_its_ordered_file_operations() {
        engine_executable::executable_tables::initialize_for_tests()
            .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
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

    /// The integer component reader writes the offsets from the cursor offset on: a
    /// `cursor_offset`'s third and fourth components are the selection offset; after them come the
    /// lost-device interface's vtable and the graphics pointer, which the game calls and reads
    /// through.
    #[test]
    fn offset_components_land_on_the_font() {
        engine_executable::executable_tables::initialize_for_tests()
            .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
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

    /// Past the two pointers (skipped by components `%i` stores nothing for), the components
    /// land on the font's own fields: the draw's viewport, the colours, the file list's count,
    /// a text colour entry, the icon keys, the glyph cache, then the object's end.
    #[test]
    fn offset_components_reach_every_field_of_the_font() {
        engine_executable::executable_tables::initialize_for_tests()
            .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
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
        // A written code byte keeps its entry from a later `textcolors`.
        let kept = font(
            "",
            &[(0x110 + 0x52 * 0x30, "82")],
            "textcolors = { R = { 255 0 0 } }",
        );
        assert_eq!(kept.text_colors[&b'R'], [1.0; 4]);
        // A later `path` through a written capacity copies through the null block.
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

    /// The registry: the latest font of a name, the fallback of the lowest non-empty bucket,
    /// overrides appended to the font found when read, and the family a language selects.
    #[test]
    fn overrides_and_fallbacks_follow_the_registry() {
        engine_executable::executable_tables::initialize_for_tests()
            .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
        let fonts = Fonts::new(&events(
            b"bitmapfonts = { bitmapfont = { name = \"f\" path = \"one\" } \
              bitmapfont_override = { name = \"f\" fontfiles = { \"kr\" } languages = { \"l_korean\" } path = \"x\" } \
              bitmapfont = { name = \"f\" path = \"two\" } \
              bitmapfont_override = { name = \"missing\" fontfiles = { \"fb\" } languages = { \"l_x\" } } }",
        ));
        let latest = fonts.get(b"f").unwrap();
        assert_eq!(latest.facts.files, [b"two".to_vec()]);
        // The first override went to the first `f`; the second to the fallback, which is the
        // latest `f` (the only bucket).
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

    /// The glyph token walk: header keywords, `base` consuming the next pair, positional `char`
    /// values, the first file winning per codepoint and pair, and `kerning` records.
    #[test]
    fn glyphs_load_in_token_order() {
        engine_executable::executable_tables::initialize_for_tests()
            .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
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
        // A file whose path is not UTF-8 is read by its bytes, as every other.
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

    /// Forms accepted by the token reader and refused by a line-based BMFont reader: a negative
    /// `char id` is kept under its unsigned value, and trailing pairs such as `page=0` are
    /// passed over while walking to the next `char` or `kerning`.
    #[test]
    fn glyphs_keep_negative_ids_and_skip_trailing_pairs() {
        engine_executable::executable_tables::initialize_for_tests()
            .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
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
