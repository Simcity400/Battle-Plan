//! Bitmap fonts: the `bitmapfonts` blocks of `interface/*.gfx` and the BMFont
//! `.fnt` files they name.
//!
//! `gfx/fonts` is outside the coverage policy, so the pages and the `.fnt`
//! files are read through [`ResolvedLayers::read_uncovered`] and are not
//! hash-verified.

use std::{collections::BTreeMap, io};

use crate::{
    ResolvedLayers,
    fields::{block, invalid, key_str, last, text},
    gfx::{content_path, gfx_paths},
    image::{
        dds::{self, Texture},
        tga,
    },
    parser,
};

/// One glyph's place on its page and its metrics, in BMFont's own units.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Glyph {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
    pub xoffset: i32,
    pub yoffset: i32,
    pub xadvance: i32,
    pub page: u32,
    /// `chnl`; 15 when the glyph uses every channel. Absent lines report 15.
    pub channel: u32,
}

/// A parsed BMFont text descriptor.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct BitmapFont {
    pub face: String,
    /// `info size`, negative when the authoring tool matched by cell height.
    pub size: i32,
    /// True when `info unicode=1`, so glyph ids are Unicode code points.
    pub unicode: bool,
    pub line_height: u32,
    pub base: u32,
    /// `common scaleW` and `scaleH`, the page dimensions the file assumes.
    pub scale: (u32, u32),
    /// Page count from `common pages`; at least one.
    pub page_count: u32,
    /// File names from the `page` lines, empty when the file has none.
    /// `hoi_mapfont4.fnt` names a page that is not shipped under that name.
    pub pages: Vec<String>,
    pub glyphs: BTreeMap<u32, Glyph>,
    /// `(first, second)` to the advance adjustment.
    pub kernings: BTreeMap<(u32, u32), i32>,
    /// One-based numbers of lines that are neither a comment nor a record this
    /// reader knows. `tweakergui_assets/gfx/fonts/tweaker_hoi_18mbs.fnt` ends
    /// with fifty lines holding nothing but `page=0`.
    pub unparsed_lines: Vec<u32>,
}

/// Splits one BMFont line into its tag and `key=value` pairs.
fn tagged(line: &str) -> Option<(&str, Vec<(&str, &str)>)> {
    let line = line.trim();
    let mut rest = line;
    let tag_end = rest.find(char::is_whitespace).unwrap_or(rest.len());
    let tag = &rest[..tag_end];
    if tag.is_empty() || tag.contains('=') {
        // A line that opens with `key=value` names no record.
        return None;
    }
    rest = &rest[tag_end..];
    let mut pairs = Vec::new();
    let bytes = rest.as_bytes();
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index].is_ascii_whitespace() {
            index += 1;
            continue;
        }
        let start = index;
        while index < bytes.len() && bytes[index] != b'=' && !bytes[index].is_ascii_whitespace() {
            index += 1;
        }
        let key = &rest[start..index];
        if index >= bytes.len() || bytes[index] != b'=' {
            // A bare word with no value carries nothing a reader needs.
            continue;
        }
        index += 1;
        let value_start = index;
        if index < bytes.len() && bytes[index] == b'"' {
            index += 1;
            let quoted_start = index;
            while index < bytes.len() && bytes[index] != b'"' {
                index += 1;
            }
            pairs.push((key, &rest[quoted_start..index]));
            // An unterminated quote simply runs to the end of the line.
            index = (index + 1).min(bytes.len());
            continue;
        }
        while index < bytes.len() && !bytes[index].is_ascii_whitespace() {
            index += 1;
        }
        pairs.push((key, &rest[value_start..index]));
    }
    Some((tag, pairs))
}

fn lookup<'a>(pairs: &[(&'a str, &'a str)], key: &str) -> Option<&'a str> {
    pairs
        .iter()
        .find(|(name, _)| *name == key)
        .map(|(_, value)| *value)
}

fn number<T: std::str::FromStr>(
    pairs: &[(&str, &str)],
    key: &str,
    tag: &str,
    line: usize,
) -> io::Result<T> {
    let value = lookup(pairs, key)
        .ok_or_else(|| invalid(format!(".fnt line {line}: {tag} has no {key}")))?;
    value.parse().map_err(|_| {
        invalid(format!(
            ".fnt line {line}: {tag} {key} is not a number: {value:?}"
        ))
    })
}

fn optional<T: std::str::FromStr>(
    pairs: &[(&str, &str)],
    key: &str,
    default: T,
    tag: &str,
    line: usize,
) -> io::Result<T> {
    match lookup(pairs, key) {
        None => Ok(default),
        Some(value) => value.parse().map_err(|_| {
            invalid(format!(
                ".fnt line {line}: {tag} {key} is not a number: {value:?}"
            ))
        }),
    }
}

/// Parses the text BMFont format the game ships under `gfx/fonts`.
///
/// The `chars count` and `kernings count` lines are optional, as are the
/// `page` lines: `garamond_14.fnt` has neither a count nor a page line.
pub fn parse_fnt(input: &str) -> io::Result<BitmapFont> {
    let mut font = BitmapFont {
        page_count: 1,
        ..BitmapFont::default()
    };
    let mut seen_info = false;
    let mut seen_common = false;
    for (index, raw) in input.lines().enumerate() {
        let line = index + 1;
        let trimmed = raw.trim();
        // The shipped files comment glyphs out with a leading `#`.
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        let Some((tag, pairs)) = tagged(raw) else {
            font.unparsed_lines.push(line as u32);
            continue;
        };
        match tag {
            "info" => {
                font.face = lookup(&pairs, "face").unwrap_or_default().to_owned();
                font.size = optional(&pairs, "size", 0, tag, line)?;
                font.unicode = optional::<u32>(&pairs, "unicode", 0, tag, line)? != 0;
                seen_info = true;
            }
            "common" => {
                font.line_height = number(&pairs, "lineHeight", tag, line)?;
                font.base = number(&pairs, "base", tag, line)?;
                font.scale = (
                    number(&pairs, "scaleW", tag, line)?,
                    number(&pairs, "scaleH", tag, line)?,
                );
                font.page_count = optional::<u32>(&pairs, "pages", 1, tag, line)?.max(1);
                seen_common = true;
            }
            "page" => {
                let id: usize = number(&pairs, "id", tag, line)?;
                let file = lookup(&pairs, "file")
                    .ok_or_else(|| invalid(format!(".fnt line {line}: page has no file")))?;
                if font.pages.len() <= id {
                    font.pages.resize(id + 1, String::new());
                }
                font.pages[id] = file.to_owned();
            }
            "char" => {
                let id: u32 = number(&pairs, "id", tag, line)?;
                let glyph = Glyph {
                    x: number(&pairs, "x", tag, line)?,
                    y: number(&pairs, "y", tag, line)?,
                    width: number(&pairs, "width", tag, line)?,
                    height: number(&pairs, "height", tag, line)?,
                    xoffset: number(&pairs, "xoffset", tag, line)?,
                    yoffset: number(&pairs, "yoffset", tag, line)?,
                    xadvance: number(&pairs, "xadvance", tag, line)?,
                    page: optional(&pairs, "page", 0, tag, line)?,
                    channel: optional(&pairs, "chnl", 15, tag, line)?,
                };
                if font.glyphs.insert(id, glyph).is_some() {
                    return Err(invalid(format!(".fnt line {line}: glyph {id} is repeated")));
                }
            }
            "kerning" => {
                let first: u32 = number(&pairs, "first", tag, line)?;
                let second: u32 = number(&pairs, "second", tag, line)?;
                font.kernings
                    .insert((first, second), number(&pairs, "amount", tag, line)?);
            }
            // `chars` and `kernings` only repeat a count the entries carry.
            "chars" | "kernings" => {}
            // An unknown record is recorded, not refused: the tweaker font ends
            // with junk lines and a later BMFont tag must not lose a whole font.
            _ => font.unparsed_lines.push(line as u32),
        }
    }
    if !seen_info || !seen_common {
        return Err(invalid(".fnt needs both an info and a common line"));
    }
    if font.glyphs.is_empty() {
        return Err(invalid(".fnt declares no glyphs"));
    }
    Ok(font)
}

/// An authored font colour.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FontColor {
    /// `0xAARRGGBB`, or `None` when the literal does not fit in 32 bits. The
    /// shipped `hoi_22chat` writes `0x4ffffffff`, which no colour holds, and
    /// guessing which eight of its nine digits were meant would be an invention.
    pub argb: Option<u32>,
    /// The literal exactly as the file writes it.
    pub authored: String,
}

impl FontColor {
    /// The colour as straight 8-bit R, G, B, A, when it fits in 32 bits.
    pub fn rgba(&self) -> Option<[u8; 4]> {
        self.argb.map(|argb| {
            [
                (argb >> 16) as u8,
                (argb >> 8) as u8,
                argb as u8,
                (argb >> 24) as u8,
            ]
        })
    }
}

fn color(value: &parser::Value, what: &str) -> io::Result<FontColor> {
    let scalar = text(value, what)?;
    let digits = scalar
        .strip_prefix("0x")
        .or_else(|| scalar.strip_prefix("0X"))
        .ok_or_else(|| invalid(format!("{what} is not an 0xAARRGGBB colour: {scalar:?}")))?;
    if digits.is_empty() || !digits.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(invalid(format!(
            "{what} is not an 0xAARRGGBB colour: {scalar:?}"
        )));
    }
    Ok(FontColor {
        argb: u32::from_str_radix(digits, 16).ok(),
        authored: scalar.to_owned(),
    })
}

/// One `bitmapfont_override` block: the files to use for named languages.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FontOverride {
    /// `l_english` style names; empty when the block names none.
    pub languages: Vec<String>,
    pub path: Option<String>,
    pub fontfiles: Vec<String>,
    pub color: Option<FontColor>,
    pub border_color: Option<FontColor>,
}

/// One named font as `interface/*.gfx` declares it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FontDefinition {
    pub name: String,
    /// `path`, when the block names a single file.
    pub path: Option<String>,
    /// `fontfiles`, the fallback chain, in authored order.
    pub fontfiles: Vec<String>,
    pub color: Option<FontColor>,
    pub border_color: Option<FontColor>,
    pub overrides: Vec<FontOverride>,
    /// The `interface/*.gfx` file the definition came from.
    pub source: String,
}

impl FontDefinition {
    /// The file stems to load, highest priority first: `path`, else `fontfiles`.
    pub fn files(&self) -> Vec<String> {
        match &self.path {
            Some(path) => vec![path.clone()],
            None => self.fontfiles.clone(),
        }
    }

    /// The files to use for a language, falling back to the base declaration.
    pub fn files_for(&self, language: &str) -> Vec<String> {
        for entry in &self.overrides {
            if entry.languages.iter().any(|name| name == language) {
                return match &entry.path {
                    Some(path) => vec![path.clone()],
                    None if !entry.fontfiles.is_empty() => entry.fontfiles.clone(),
                    None => self.files(),
                };
            }
        }
        self.files()
    }
}

fn read_files(value: &parser::Value, what: &str) -> io::Result<Vec<String>> {
    let mut files = Vec::new();
    for item in block(value, what)? {
        if item.key.is_some() {
            return Err(invalid(format!("{what} holds a keyed entry")));
        }
        files.push(content_path(text(&item.value, what)?));
    }
    Ok(files)
}

fn read_override(items: &[parser::Item], name: &str) -> io::Result<FontOverride> {
    Ok(FontOverride {
        languages: match last(items, b"languages") {
            Some(value) => read_files(value, &format!("{name} languages"))?,
            None => Vec::new(),
        },
        path: match last(items, b"path") {
            Some(value) => Some(content_path(text(value, &format!("{name} path"))?)),
            None => None,
        },
        fontfiles: match last(items, b"fontfiles") {
            Some(value) => read_files(value, &format!("{name} fontfiles"))?,
            None => Vec::new(),
        },
        color: match last(items, b"color") {
            Some(value) => Some(color(value, &format!("{name} color"))?),
            None => None,
        },
        border_color: match last(items, b"border_color") {
            Some(value) => Some(color(value, &format!("{name} border_color"))?),
            None => None,
        },
    })
}

/// The `§X` colour codes a `bitmapfonts` block declares, keyed by their letter.
pub type TextColors = BTreeMap<char, [u8; 3]>;

/// Font definitions and `§X` colours parsed out of one `.gfx` file.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FontFile {
    pub definitions: Vec<FontDefinition>,
    /// `(font name, override)` in source order. `interface/core_chinese.gfx`
    /// overrides fonts `interface/core.gfx` declares, so an override is only
    /// matched to its base once every file has been read.
    pub overrides: Vec<(String, FontOverride)>,
    pub text_colors: TextColors,
}

/// Parses every `bitmapfonts` block of one `interface/*.gfx` file.
pub fn parse_definitions(input: &[u8], source: &str) -> io::Result<FontFile> {
    let (document, _) = crate::gfx::parse_interface(input)?;
    let mut file = FontFile::default();
    for root in crate::fields::all(&document.items, b"bitmapfonts") {
        for item in block(root, "bitmapfonts")? {
            let key = key_str(item, "bitmapfonts")?;
            match key {
                "bitmapfont" | "bitmapfont_override" => {}
                "textcolors" => {
                    for entry in block(&item.value, "textcolors")? {
                        let code = key_str(entry, "textcolors")?;
                        let mut characters = code.chars();
                        let (Some(letter), None) = (characters.next(), characters.next()) else {
                            return Err(invalid(format!(
                                "{source}: text colour code {code:?} is not one character"
                            )));
                        };
                        file.text_colors
                            .insert(letter, crate::map::parse_color(&entry.value)?);
                    }
                    continue;
                }
                // `interface/core.gfx` never closes its `bitmapfonts` block
                // before declaring fifteen sprites, so the block holds entries
                // that have nothing to do with fonts. They are left to `gfx`.
                _ => continue,
            }
            let items = block(&item.value, key)?;
            let name = text(
                last(items, b"name")
                    .ok_or_else(|| invalid(format!("{source}: {key} has no name")))?,
                "font name",
            )?
            .to_owned();
            if key == "bitmapfont_override" {
                let entry = read_override(items, &name)?;
                file.overrides.push((name, entry));
                continue;
            }
            let base = read_override(items, &name)?;
            file.definitions.push(FontDefinition {
                name,
                path: base.path,
                fontfiles: base.fontfiles,
                color: base.color,
                border_color: base.border_color,
                overrides: Vec::new(),
                source: source.to_owned(),
            });
        }
    }
    Ok(file)
}

/// Reads every `bitmapfonts` block in `interface/*.gfx` across the layers.
///
/// A name declared by more than one file keeps the later file's definition,
/// which is the layer order followed by the path in byte order.
pub fn read_definitions(layers: &ResolvedLayers) -> io::Result<BTreeMap<String, FontDefinition>> {
    let mut definitions: BTreeMap<String, FontDefinition> = BTreeMap::new();
    let mut overrides = Vec::new();
    for path in gfx_paths(layers) {
        let file = parse_definitions(&layers.read(&path)?, &path)?;
        for definition in file.definitions {
            definitions.insert(definition.name.clone(), definition);
        }
        overrides.extend(file.overrides);
    }
    for (name, entry) in overrides {
        let definition = definitions
            .get_mut(&name)
            .ok_or_else(|| invalid(format!("bitmapfont_override {name:?} has no bitmapfont")))?;
        definition.overrides.push(entry);
    }
    Ok(definitions)
}

/// Reads every `textcolors` block in `interface/*.gfx` across the layers.
pub fn read_text_colors(layers: &ResolvedLayers) -> io::Result<TextColors> {
    let mut colors = TextColors::new();
    for path in gfx_paths(layers) {
        colors.extend(parse_definitions(&layers.read(&path)?, &path)?.text_colors);
    }
    Ok(colors)
}

/// One page texture of a loaded font, and where it was found.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FontPage {
    pub path: String,
    pub texture: Texture,
}

/// A `.fnt` descriptor together with its page textures.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LoadedFont {
    /// The stem the font was loaded from, without an extension.
    pub stem: String,
    pub font: BitmapFont,
    pub pages: Vec<FontPage>,
}

fn decode_page(path: &str, bytes: &[u8]) -> io::Result<Texture> {
    if path.to_ascii_lowercase().ends_with(".dds") {
        dds::decode(bytes)
    } else {
        tga::decode(bytes).map(Texture::from_rgba8)
    }
    .map_err(|error| invalid(format!("{path}: {error}")))
}

/// Candidate page files for page `index`, highest priority first.
fn page_candidates(stem: &str, font: &BitmapFont, index: usize) -> Vec<String> {
    let mut candidates = Vec::new();
    if let Some(declared) = font.pages.get(index).filter(|name| !name.is_empty()) {
        let directory = stem.rsplit_once('/').map_or("", |(head, _)| head);
        candidates.push(if directory.is_empty() {
            declared.clone()
        } else {
            format!("{directory}/{declared}")
        });
    }
    let indexed = format!("{stem}_{index}");
    let bases = if font.page_count > 1 {
        vec![indexed]
    } else {
        vec![stem.to_owned(), indexed]
    };
    for base in bases {
        candidates.push(format!("{base}.dds"));
        candidates.push(format!("{base}.tga"));
    }
    candidates
}

/// Loads `<stem>.fnt` and its page textures from the resolved layers.
///
/// Page files are looked for under the name the `.fnt` declares, then
/// `<stem>_<page>.dds` or `.tga`, then `<stem>.dds` or `.tga` for a single-page
/// font. `hoi_mapfont4.fnt` names `Hoi4_mapfont_tahoma84_merged_0.dds`, which
/// the game does not ship, so the stem fallback is what actually resolves it.
pub fn load_font_file(layers: &ResolvedLayers, stem: &str) -> io::Result<LoadedFont> {
    let descriptor = format!("{stem}.fnt");
    let bytes = layers.read_uncovered(&descriptor)?;
    let text = std::str::from_utf8(bytes.strip_prefix(b"\xef\xbb\xbf").unwrap_or(&bytes))
        .map_err(|_| invalid(format!("{descriptor} is not UTF-8")))?;
    let font = parse_fnt(text).map_err(|error| invalid(format!("{descriptor}: {error}")))?;
    let mut pages = Vec::new();
    for index in 0..font.page_count as usize {
        let candidates = page_candidates(stem, &font, index);
        let mut found = None;
        for candidate in &candidates {
            match layers.read_uncovered(candidate) {
                Ok(bytes) => {
                    found = Some(FontPage {
                        path: candidate.clone(),
                        texture: decode_page(candidate, &bytes)?,
                    });
                    break;
                }
                Err(error) if error.kind() == io::ErrorKind::NotFound => {}
                Err(error) => return Err(error),
            }
        }
        pages.push(found.ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::NotFound,
                format!("{descriptor} page {index} is not among {candidates:?}"),
            )
        })?);
    }
    Ok(LoadedFont {
        stem: stem.to_owned(),
        font,
        pages,
    })
}

/// Loads the primary file of a font definition: `path`, else the first
/// `fontfiles` entry. Use [`load_font_file`] for the rest of the chain.
pub fn load_font(layers: &ResolvedLayers, definition: &FontDefinition) -> io::Result<LoadedFont> {
    let stem = definition.files().first().cloned().ok_or_else(|| {
        invalid(format!(
            "font {:?} declares neither path nor fontfiles",
            definition.name
        ))
    })?;
    load_font_file(layers, &stem)
}

#[cfg(test)]
mod tests {
    use super::*;

    const FNT: &str = "info face=\"Tahoma\" size=84 bold=1 charset=\"\" unicode=1 padding=2,2,2,2\n\
        common lineHeight=84 base=70 scaleW=2048 scaleH=1024 pages=1 packed=0\n\
        page id=0 file=\"Hoi4_mapfont_tahoma84_merged_0.dds\"\n\
        chars count=2\n\
        char id=32   x=2034  y=31  width=7  height=5  xoffset=-3 yoffset=81 xadvance=20 page=0 chnl=15\n\
        char id=33   x=1097  y=647 width=18 height=55 xoffset=3  yoffset=17 xadvance=24 page=0 chnl=15\n\
        kernings count=1\n\
        kerning first=146 second=100 amount=-1\n";

    #[test]
    fn a_full_descriptor_keeps_its_face_metrics_glyphs_and_kernings() {
        let font = parse_fnt(FNT).unwrap();
        assert_eq!(font.face, "Tahoma");
        assert_eq!(font.size, 84);
        assert!(font.unicode);
        assert_eq!(font.line_height, 84);
        assert_eq!(font.base, 70);
        assert_eq!(font.scale, (2048, 1024));
        assert_eq!(font.page_count, 1);
        assert_eq!(font.pages, vec!["Hoi4_mapfont_tahoma84_merged_0.dds"]);
        assert_eq!(font.glyphs.len(), 2);
        assert_eq!(
            font.glyphs[&33],
            Glyph {
                x: 1097,
                y: 647,
                width: 18,
                height: 55,
                xoffset: 3,
                yoffset: 17,
                xadvance: 24,
                page: 0,
                channel: 15,
            }
        );
        assert_eq!(font.kernings[&(146, 100)], -1);
    }

    #[test]
    fn a_descriptor_without_page_count_or_channel_lines_still_parses() {
        // garamond_14.fnt has no page line, no counts and no chnl column.
        let font = parse_fnt(
            "info face=\"Garamond\" size=14 charset=\"ANSI\" stretchH=100\n\
             common lineHeight=14 base=11 scaleW=256 scaleH=256 pages=1\n\
             char id=32 x=0 y=0 width=1 height=0 xoffset=0 yoffset=14 xadvance=3 page=0 \n\
             kerning first=146 second=115 amount=-1  \n",
        )
        .unwrap();
        assert!(!font.unicode);
        assert!(font.pages.is_empty());
        assert_eq!(font.glyphs[&32].channel, 15);
        assert_eq!(font.kernings.len(), 1);
    }

    #[test]
    fn malformed_descriptors_name_their_line() {
        for (input, message) in [
            (
                "common lineHeight=1 base=1 scaleW=1 scaleH=1\nchar id=1 x=0 y=0 width=0 height=0 xoffset=0 yoffset=0 xadvance=0\n",
                "info and a common",
            ),
            (
                "info face=\"a\"\nchar id=1 x=0 y=0 width=0 height=0 xoffset=0 yoffset=0 xadvance=0\n",
                "info and a common",
            ),
            (
                "info face=\"a\"\ncommon lineHeight=1 base=1 scaleW=1\n",
                "has no scaleH",
            ),
            (
                "info face=\"a\"\ncommon lineHeight=1 base=1 scaleW=1 scaleH=1\n",
                "no glyphs",
            ),
            (
                "info face=\"a\"\ncommon lineHeight=1 base=1 scaleW=1 scaleH=1\nchar id=x x=0 y=0 width=0 height=0 xoffset=0 yoffset=0 xadvance=0\n",
                "id is not a number",
            ),
            (
                "info face=\"a\"\ncommon lineHeight=1 base=1 scaleW=1 scaleH=1\nchar id=1 y=0 width=0 height=0 xoffset=0 yoffset=0 xadvance=0\n",
                "char has no x",
            ),
            (
                "info face=\"a\"\ncommon lineHeight=1 base=1 scaleW=1 scaleH=1\npage id=0\n",
                "page has no file",
            ),
        ] {
            let error = parse_fnt(input).unwrap_err().to_string();
            assert!(error.contains(message), "{error} lacks {message}");
        }
        let repeated = FNT.replace("id=33", "id=32");
        assert!(
            parse_fnt(&repeated)
                .unwrap_err()
                .to_string()
                .contains("glyph 32 is repeated")
        );
    }

    #[test]
    fn an_unterminated_quote_and_a_bare_word_do_not_break_the_line() {
        let font = parse_fnt(
            "info face=\"a size=1 charset=\"\" bold
             common lineHeight=1 base=1 scaleW=1 scaleH=1
             char id=32 x=0 y=0 width=0 height=0 xoffset=0 yoffset=0 xadvance=0
",
        )
        .unwrap();
        // The quote swallows the rest of the line, so no size is declared.
        assert_eq!(font.face, "a size=1 charset=");
        assert_eq!(font.size, 0);
        assert_eq!(font.glyphs.len(), 1);
        for line in ["info face=\"", "info face=", "info ="] {
            let input = format!(
                "{line}
common lineHeight=1 base=1 scaleW=1 scaleH=1
                 char id=32 x=0 y=0 width=0 height=0 xoffset=0 yoffset=0 xadvance=0
"
            );
            assert!(parse_fnt(&input).is_ok(), "{line}");
        }
    }

    #[test]
    fn commented_glyphs_are_skipped_and_junk_lines_are_recorded() {
        // korean/hoi_18.fnt comments glyphs out; the tweaker font trails junk.
        let font = parse_fnt(
            "info face=\"a\" size=1\ncommon lineHeight=1 base=1 scaleW=1 scaleH=1\n\
             char id=32 x=0 y=0 width=0 height=0 xoffset=0 yoffset=0 xadvance=0 page=0\n\
             #char id=33 x=0 y=0 width=0 height=0 xoffset=0 yoffset=0 xadvance=0 page=0\n\
             \t\t\tpage=0\n\
             futureTag id=1\n",
        )
        .unwrap();
        assert_eq!(font.glyphs.len(), 1);
        assert!(!font.glyphs.contains_key(&33));
        assert_eq!(font.unparsed_lines, vec![5, 6]);
    }

    #[test]
    fn page_candidates_prefer_the_declared_name_then_the_stem() {
        let font = parse_fnt(FNT).unwrap();
        assert_eq!(
            page_candidates("gfx/fonts/hoi_mapfont4", &font, 0),
            vec![
                "gfx/fonts/Hoi4_mapfont_tahoma84_merged_0.dds",
                "gfx/fonts/hoi_mapfont4.dds",
                "gfx/fonts/hoi_mapfont4.tga",
                "gfx/fonts/hoi_mapfont4_0.dds",
                "gfx/fonts/hoi_mapfont4_0.tga",
            ]
        );
        // A two-page font only looks for the numbered files.
        let two = parse_fnt(&FNT.replace("pages=1", "pages=2")).unwrap();
        assert_eq!(
            page_candidates("gfx/fonts/japanese/hoi_mapfont4", &two, 1),
            vec![
                "gfx/fonts/japanese/hoi_mapfont4_1.dds",
                "gfx/fonts/japanese/hoi_mapfont4_1.tga",
            ]
        );
    }

    const GFX: &[u8] = br#"
bitmapfonts = {
    textcolors = {
        C = { 35 206 255 }
        b = { 0 0 0 }
    }
    bitmapfont = {
        name = "Main_14"
        path = "gfx/fonts/garamond_14"
        color = 0xffffffff
        border_color = 0x00000000
    }
    bitmapfont_override = {
        name = "Main_14"
        fontfiles = { "gfx/fonts/japanese/garamond_14" }
        languages = { "l_japanese" }
    }
    bitmapfont = {
        name = "AnimatedMapText"
        fontfiles = { "gfx/fonts/hoi_18" "gfx/fonts/hoi_18_cryllic" }
        color = 0xff7799ff
    }
}
"#;

    #[test]
    fn definitions_keep_paths_file_lists_colours_and_their_overrides() {
        let file = parse_definitions(GFX, "interface/core.gfx").unwrap();
        assert_eq!(file.text_colors[&'C'], [35, 206, 255]);
        assert_eq!(file.text_colors[&'b'], [0, 0, 0]);
        let main = &file.definitions[0];
        assert_eq!(main.name, "Main_14");
        assert_eq!(main.path.as_deref(), Some("gfx/fonts/garamond_14"));
        assert_eq!(main.color.as_ref().unwrap().argb, Some(0xffff_ffff));
        assert_eq!(main.border_color.as_ref().unwrap().argb, Some(0));
        assert_eq!(main.source, "interface/core.gfx");
        // An override is matched to its base only once every file is read,
        // because interface/core_chinese.gfx overrides fonts declared elsewhere.
        assert!(main.overrides.is_empty());
        assert_eq!(file.overrides.len(), 1);
        assert_eq!(file.overrides[0].0, "Main_14");
        assert_eq!(file.overrides[0].1.languages, vec!["l_japanese"]);
        let mut matched = main.clone();
        matched.overrides.push(file.overrides[0].1.clone());
        assert_eq!(
            matched.files_for("l_japanese"),
            vec!["gfx/fonts/japanese/garamond_14"]
        );
        assert_eq!(
            matched.files_for("l_english"),
            vec!["gfx/fonts/garamond_14"]
        );
        let animated = &file.definitions[1];
        assert_eq!(animated.path, None);
        assert_eq!(
            animated.files(),
            vec!["gfx/fonts/hoi_18", "gfx/fonts/hoi_18_cryllic"]
        );
        assert_eq!(
            animated.color.as_ref().unwrap().rgba(),
            Some([0x77, 0x99, 0xff, 0xff])
        );
        // interface/core.gfx writes nine hex digits for hoi_22chat.
        let wide = parse_definitions(
            br#"bitmapfonts = { bitmapfont = { name = "hoi_22chat" color = 0x4ffffffff } }"#,
            "interface/core.gfx",
        )
        .unwrap();
        let color = wide.definitions[0].color.as_ref().unwrap();
        assert_eq!(color.argb, None);
        assert_eq!(color.rgba(), None);
        assert_eq!(color.authored, "0x4ffffffff");
    }

    #[test]
    fn malformed_font_blocks_are_named() {
        for (input, message) in [
            (
                br#"bitmapfonts = { bitmapfont = { path = "x" } }"#.as_slice(),
                "bitmapfont has no name",
            ),
            (
                br#"bitmapfonts = { bitmapfont = { name = "x" color = zzz } }"#,
                "0xAARRGGBB",
            ),
            (
                br#"bitmapfonts = { bitmapfont = { name = "x" color = 0xzz } }"#,
                "0xAARRGGBB",
            ),
            (
                br#"bitmapfonts = { bitmapfont = { name = "x" color = 0x } }"#,
                "0xAARRGGBB",
            ),
            (
                br#"bitmapfonts = { textcolors = { CC = { 1 2 3 } } }"#,
                "not one character",
            ),
            (
                br#"bitmapfonts = { bitmapfont = { name = "x" fontfiles = { a = b } } }"#,
                "keyed entry",
            ),
        ] {
            let error = parse_definitions(input, "interface/test.gfx")
                .unwrap_err()
                .to_string();
            assert!(error.contains(message), "{error} lacks {message}");
        }
    }

    #[test]
    fn entries_that_are_not_fonts_inside_the_block_are_left_alone() {
        // interface/core.gfx declares sprites inside its bitmapfonts block.
        let file = parse_definitions(
            br#"bitmapfonts = {
                    spriteType = { name = "GFX_warning_11x24" texturefile = "a.dds" }
                    bitmapfont = { name = "Main_14" path = "gfx/fonts/garamond_14" }
                }"#,
            "interface/core.gfx",
        )
        .unwrap();
        assert_eq!(file.definitions.len(), 1);
        assert_eq!(file.definitions[0].name, "Main_14");
    }

    #[test]
    fn a_file_with_no_bitmapfonts_block_yields_nothing() {
        let file =
            parse_definitions(b"spriteTypes = { spriteType = { name = \"x\" } }", "a.gfx").unwrap();
        assert!(file.definitions.is_empty());
        assert!(file.text_colors.is_empty());
    }
}
