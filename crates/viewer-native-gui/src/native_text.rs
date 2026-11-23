
use std::collections::BTreeMap;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Glyph {
    pub ch: char,
    pub advance: i32,
    pub width: i32,
    pub height: i32,
    pub xoffset: i32,
    pub yoffset: i32,
}

pub trait Glyphs {
    fn own(&self, ch: char) -> Option<Glyph>;
    fn kerning(&self, a: char, b: char) -> i32;
    fn line_height(&self) -> i32;

    fn glyph(&self, ch: char) -> Option<Glyph> {
        self.own(ch)
            .or_else(|| (u32::from(ch) > 0x1F).then(|| self.own('?')).flatten())
    }
}

pub fn decode(bytes: &[u8]) -> Option<Vec<char>> {
    const OFFSETS: [u32; 6] = [
        0x0000_0000,
        0x0000_3080,
        0x000E_2080,
        0x03C8_2080,
        0xFA08_2080,
        0x8208_2080,
    ];
    let trail = |byte: u8| match byte {
        0x00..=0xBF => 0,
        0xC0..=0xDF => 1,
        0xE0..=0xEF => 2,
        0xF0..=0xF7 => 3,
        0xF8..=0xFB => 4,
        0xFC..=0xFF => 5,
    };
    let continuation = |byte: u8| (0x80..=0xBF).contains(&byte);
    let mut out = Vec::with_capacity(bytes.len());
    let mut at = 0;
    while at < bytes.len() {
        let lead = bytes[at];
        let extra = trail(lead);
        if extra >= bytes.len() - at {
            return None;
        }
        if extra >= 4 {
            return None;
        }
        if extra == 3 && !continuation(bytes[at + 3]) {
            return None;
        }
        if extra >= 2 && !continuation(bytes[at + 2]) {
            return None;
        }
        if extra >= 1 {
            let second = bytes[at + 1];
            let legal = second <= 0xBF
                && match lead {
                    0xE0 => second >= 0xA0,
                    0xED => second <= 0x9F,
                    0xF0 => second >= 0x90,
                    0xF4 => second <= 0x8F,
                    _ => second >= 0x80,
                };
            if !legal {
                return None;
            }
        }
        if (0x80..0xC2).contains(&lead) || lead > 0xF4 {
            return None;
        }
        let mut code = 0u32;
        for (index, byte) in bytes[at..=at + extra].iter().enumerate() {
            if index > 0 {
                code = code.wrapping_shl(6);
            }
            code = code.wrapping_add(u32::from(*byte));
        }
        let code = code.wrapping_sub(OFFSETS[extra]);
        out.push(char::from_u32(code)?);
        at += extra + 1;
    }
    let counted = bytes.iter().filter(|byte| !continuation(**byte)).count();
    out.resize(counted.max(out.len()), '\0');
    Some(out)
}

pub const COLOR: char = '\u{11}';
pub const FLAG: char = '\u{12}';
pub const RESOLVED_FLAG: char = '\u{1f}';
pub const ICON: char = '\u{13}';
pub const CURRENCY: char = '\u{14}';
pub const NUMBER: char = '\u{1a}';
pub const CURRENCY_ADVANCE: i32 = 16;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Special {
    Color(Option<char>),
    Flag(String),
    Icon(String),
    Currency,
    Number(String),
}

pub fn special(text: &[char]) -> Option<(usize, Special)> {
    let rest = &text[1..];
    Some(match *text.first()? {
        COLOR => (1 + rest.len().min(1), Special::Color(rest.first().copied())),
        FLAG => {
            let taken = rest.len().min(3);
            (1 + taken, Special::Flag(rest[..taken].iter().collect()))
        }
        ICON => {
            let name: String = rest
                .iter()
                .take(128)
                .take_while(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '_' | '|'))
                .collect();
            let length = name.chars().count();
            (
                1 + length + usize::from(rest.len() > length),
                Special::Icon(name),
            )
        }
        CURRENCY => (1, Special::Currency),
        NUMBER => {
            let taken = rest.len().min(2);
            (1 + taken, Special::Number(rest[..taken].iter().collect()))
        }
        _ => return None,
    })
}

type KeyResolver<'a> = dyn Fn(&[char]) -> Option<usize> + 'a;

fn sequence(text: &[char], resolved_key: Option<&KeyResolver<'_>>) -> Option<(usize, Special)> {
    if let Some(resolved_key) = resolved_key
        && text.first() == Some(&RESOLVED_FLAG)
    {
        let rest = &text[1..];
        let length = resolved_key(rest).unwrap_or_else(|| {
            rest.iter()
                .take_while(|ch| ch.is_ascii_alphanumeric() || **ch == '_')
                .count()
        });
        return Some((1 + length, Special::Flag(rest[..length].iter().collect())));
    }
    special(text)
}

pub type IconSprite = ([i32; 2], i32);

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TextIcon {
    pub sprite: Vec<u8>,
    pub texture: [i32; 2],
    pub frames: i32,
    pub frame: i32,
}

impl TextIcon {
    pub fn advance(&self) -> Result<i32, String> {
        if self.frames == 0 {
            return Err(format!(
                "shows the inline icon {}, whose zero noOfFrames makes its advance divide by zero: the game ends on the division",
                String::from_utf8_lossy(&self.sprite)
            ));
        }
        Ok(((self.texture[0] as u32) / (self.frames as u32)) as i32)
    }

    pub fn quad(&self) -> ([i32; 2], [f32; 4]) {
        let step = 1.0f32 / self.frames as f32;
        let u0 = self.frame as f32 * step;
        let u1 = (self.frame.wrapping_add(1) as f32 * step).min(1.0);
        (
            [self.texture[0].wrapping_div(self.frames), self.texture[1]],
            [u0, 0.0, u1, 1.0],
        )
    }
}

pub fn find_icon(name: &str, sprite: &dyn Fn(&[u8]) -> Option<IconSprite>) -> Option<TextIcon> {
    let (names, frame) = content_reader_localisation::localisation::icon_lookup(name);
    names.into_iter().find_map(|found| {
        let (texture, frames) = sprite(&found)?;
        Some(TextIcon {
            sprite: found,
            texture,
            frames,
            frame: frame.unwrap_or(0),
        })
    })
}

fn special_advance(
    special: &Special,
    sprite: &dyn Fn(&[u8]) -> Option<IconSprite>,
    icons_add_height: bool,
) -> Result<(i32, Option<i32>), String> {
    Ok(match special {
        Special::Color(_) => (0, None),
        Special::Flag(_) => (12, None),
        Special::Icon(name) if name.is_empty() => (0, icons_add_height.then_some(0)),
        Special::Icon(name) => match find_icon(name, sprite) {
            Some(icon) => (icon.advance()?, icons_add_height.then_some(icon.texture[1])),
            None => (0, icons_add_height.then_some(0)),
        },
        Special::Currency => (CURRENCY_ADVANCE, None),
        Special::Number(_) => (8, None),
    })
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Limits {
    pub width: i32,
    pub height: i32,
    pub border: [i32; 2],
    pub newlines: bool,
    pub preserve_indent: bool,
    pub ellipsis: bool,
    pub multiline: bool,
    pub special: bool,
    pub icons_add_height: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Laid {
    pub text: Vec<char>,
    pub widths: Vec<i32>,
    pub heights: Vec<i32>,
    pub truncated: bool,
    resolved_flags: bool,
    resolved_keys: Vec<Vec<char>>,
}

struct Layout<'a> {
    glyphs: &'a dyn Glyphs,
    limits: Limits,
    width: i32,
    height: i32,
    base: i32,
    space_advance: i32,
    out: Vec<char>,
    word: Vec<char>,
    line_x: i32,
    word_w: i32,
    trail: i32,
    line_h: i32,
    word_h: i32,
    glyph_h: i32,
    widths: Vec<i32>,
    heights: Vec<i32>,
    truncated: bool,
}

impl Layout<'_> {
    fn soft_line_feed(&mut self) -> i32 {
        if !self.limits.newlines {
            return 0;
        }
        let count = if self.limits.preserve_indent {
            let start = self
                .out
                .iter()
                .rposition(|ch| *ch == '\n')
                .map_or(0, |i| i + 1);
            let count = self.out[start..]
                .iter()
                .take_while(|ch| **ch == ' ')
                .count();
            if (count as i32).wrapping_mul(self.space_advance) < self.width {
                count
            } else {
                0
            }
        } else {
            0
        };
        self.out.push('\n');
        self.out.extend(std::iter::repeat_n(' ', count));
        (count as i32).wrapping_mul(self.space_advance)
    }

    fn room(&self) -> bool {
        let used: i32 = self
            .heights
            .iter()
            .fold(0, |sum, height| sum.wrapping_add(*height));
        self.limits.multiline
            && self.height > self.glyph_h.wrapping_add(self.line_h).wrapping_add(used)
    }

    fn overflow(&mut self) -> bool {
        if !self.room() {
            self.truncated = true;
            return false;
        }
        if self.line_x == self.word_w {
            self.out.append(&mut self.word);
            let indent = self.soft_line_feed();
            self.widths.push(self.line_x);
            self.heights.push(self.line_h.max(self.word_h));
            self.line_x = indent;
            self.word_w = 0;
            self.line_h = self.base;
            self.word_h = self.base;
        } else {
            let indent = self.soft_line_feed();
            self.out.append(&mut self.word);
            self.widths.push(self.line_x - self.word_w - self.trail);
            self.heights.push(self.line_h);
            self.word_w = self.word_w.wrapping_add(indent);
            self.line_x = self.word_w;
            self.line_h = self.word_h;
        }
        self.trail = 0;
        true
    }

    fn advance(&mut self, advance: i32) {
        self.word_w += advance;
        self.line_x += advance;
        self.word_h = self.word_h.max(self.glyph_h);
        self.glyph_h = self.base;
    }
}

pub fn lay_out(
    glyphs: &dyn Glyphs,
    sprite: &dyn Fn(&[u8]) -> Option<IconSprite>,
    text: &[char],
    limits: Limits,
) -> Result<Laid, String> {
    layout(glyphs, sprite, text, limits, None)
}

pub fn lay_out_resolved_flags(
    glyphs: &dyn Glyphs,
    sprite: &dyn Fn(&[u8]) -> Option<IconSprite>,
    text: &[char],
    limits: Limits,
    resolved_key: &dyn Fn(&[char]) -> Option<usize>,
) -> Result<Laid, String> {
    layout(glyphs, sprite, text, limits, Some(resolved_key))
}

fn layout(
    glyphs: &dyn Glyphs,
    sprite: &dyn Fn(&[u8]) -> Option<IconSprite>,
    text: &[char],
    limits: Limits,
    resolved_key: Option<&KeyResolver<'_>>,
) -> Result<Laid, String> {
    let base = glyphs.line_height();
    let mut layout = Layout {
        glyphs,
        limits,
        width: if limits.width > 0 {
            limits.width
        } else {
            i32::MAX
        }
        .wrapping_sub(limits.border[0].wrapping_mul(2)),
        height: limits.height.wrapping_sub(limits.border[1].wrapping_mul(2)),
        base,
        space_advance: glyphs.glyph(' ').map_or(0, |glyph| glyph.advance),
        out: Vec::with_capacity(text.len()),
        word: Vec::new(),
        line_x: 0,
        word_w: 0,
        trail: 0,
        line_h: base,
        word_h: base,
        glyph_h: base,
        widths: Vec::new(),
        heights: Vec::new(),
        truncated: false,
    };
    let mut resolved_keys = Vec::new();
    let mut previous: Option<char> = None;
    let mut index = 0;
    while index < text.len() && layout.widths.len() < 255 {
        let ch = text[index];
        if limits.special
            && let Some((length, special)) = sequence(&text[index..], resolved_key)
        {
            let (advance, height) = special_advance(&special, sprite, limits.icons_add_height)?;
            if layout.width <= layout.line_x.wrapping_add(advance) && !layout.overflow() {
                break;
            }
            layout.word.extend_from_slice(&text[index..index + length]);
            if resolved_key.is_some() && ch == RESOLVED_FLAG {
                let key = text[index + 1..index + length].to_vec();
                if !resolved_keys.contains(&key) {
                    resolved_keys.push(key);
                }
            }
            if let Some(height) = height {
                layout.glyph_h = height;
            }
            layout.advance(advance);
            previous = None;
            index += length;
            continue;
        }
        if let Some(glyph) = layout.glyphs.glyph(ch) {
            let kerning = previous.map_or(0, |previous| layout.glyphs.kerning(previous, ch));
            let advance = glyph.advance + kerning;
            if (glyph.height != 0 || layout.word_w == 0)
                && layout.width <= layout.line_x.wrapping_add(advance)
                && !layout.overflow()
            {
                break;
            }
            layout.word.push(ch);
            layout.advance(advance);
            if glyph.height == 0 {
                layout.out.append(&mut layout.word);
                layout.line_h = layout.line_h.max(layout.word_h);
                layout.word_w = 0;
                layout.word_h = base;
                layout.trail = advance;
            }
            previous = Some(glyph.ch);
        } else if matches!(ch, '\n' | '\r') {
            if !layout.room() {
                layout.truncated = true;
                break;
            }
            layout.out.append(&mut layout.word);
            layout.out.push(ch);
            layout.widths.push(layout.line_x);
            layout.heights.push(layout.line_h.max(layout.word_h));
            layout.line_x = 0;
            layout.word_w = 0;
            layout.line_h = base;
            layout.word_h = base;
            layout.trail = 0;
            previous = None;
        } else {
            layout.word.push(ch);
            previous = None;
        }
        index += 1;
    }
    layout.out.append(&mut layout.word);
    layout.line_h = layout.line_h.max(layout.word_h);
    let line_x = layout.line_x;
    let mut out = layout.out;
    if layout.truncated && limits.ellipsis {
        ellipsis(glyphs, &mut out, layout.width.wrapping_sub(line_x));
    }
    let mut widths = layout.widths;
    let mut heights = layout.heights;
    widths.push(line_x);
    heights.push(layout.line_h);
    Ok(Laid {
        text: out,
        widths,
        heights,
        truncated: layout.truncated,
        resolved_flags: resolved_key.is_some(),
        resolved_keys,
    })
}

fn ellipsis(glyphs: &dyn Glyphs, out: &mut Vec<char>, room: i32) {
    if out.len() <= 2 {
        return;
    }
    let Some(dot) = glyphs.glyph('.') else {
        return;
    };
    let dot_advance = dot.advance;
    let mut room = room;
    let mut need = 3 * (dot_advance + 1) - glyphs.kerning('.', '.');
    let mut replaced = 0;
    while replaced < 3 && room < need {
        let at = out.len() - 1 - replaced;
        room += glyphs.glyph(out[at]).map_or(0, |glyph| glyph.advance) - dot_advance;
        need -= dot_advance;
        out[at] = '.';
        replaced += 1;
    }
    out.extend(std::iter::repeat_n('.', 3 - replaced));
}

pub struct Placement<'a> {
    pub top: i32,
    pub start: &'a dyn Fn(usize, i32) -> i32,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Piece {
    Glyph {
        ch: char,
        at: [i32; 2],
        color: [f32; 4],
    },
    Icon {
        sprite: Vec<u8>,
        at: [i32; 2],
        size: [i32; 2],
        uv: [f32; 4],
    },
    Undefined(String),
    Flag {
        tag: String,
        at: [i32; 2],
        resolved: bool,
    },
}

pub struct Colors<'a> {
    pub own: [f32; 4],
    pub font: &'a BTreeMap<u8, [f32; 4]>,
    pub global: &'a BTreeMap<u8, [f32; 4]>,
}

impl Colors<'_> {
    pub fn lookup(&self, code: char) -> Option<[f32; 4]> {
        let byte = (u32::from(code) & 0xFF) as u8;
        self.font
            .get(&byte)
            .or_else(|| self.global.get(&byte))
            .copied()
    }
}

pub fn packed(color: [f32; 4]) -> [f32; 4] {
    color.map(|value| ((value * 255.0) as i64 & 0xFF) as f32 / 255.0)
}

pub fn draw(
    glyphs: &dyn Glyphs,
    sprite: &dyn Fn(&[u8]) -> Option<IconSprite>,
    laid: &Laid,
    special: bool,
    placement: &Placement,
    colors: &Colors,
) -> Result<Vec<Piece>, String> {
    let line_height = glyphs.line_height();
    let height = |line: usize| laid.heights.get(line).copied().unwrap_or(line_height);
    let width = |line: usize| laid.widths.get(line).copied().unwrap_or(0);
    let mut pieces = Vec::new();
    let mut line = 0;
    let mut top = placement.top;
    let mut pen = (placement.start)(0, width(0));
    let mut previous: Option<char> = None;
    let mut color = packed(colors.own);
    let mut stack: Vec<[f32; 4]> = Vec::new();
    let text = &laid.text;
    let resolved_key = |text: &[char]| {
        laid.resolved_keys
            .iter()
            .filter(|key| !key.is_empty() && text.starts_with(key))
            .map(Vec::len)
            .max()
    };
    let mut index = 0;
    while index < text.len() {
        let ch = text[index];
        if ch == '\n' {
            top += height(line);
            line += 1;
            pen = (placement.start)(line, width(line));
            previous = None;
            index += 1;
            continue;
        }
        if special
            && let Some((length, sequence)) = sequence(
                &text[index..],
                laid.resolved_flags
                    .then_some(&resolved_key as &dyn Fn(&[char]) -> Option<usize>),
            )
        {
            match &sequence {
                Special::Color(Some('!')) if stack.len() > 1 => {
                    color = stack[stack.len() - 2];
                    stack.pop();
                }
                Special::Color(Some(code)) => {
                    let found = if *code == '!' {
                        Some(colors.own)
                    } else {
                        colors.lookup(*code)
                    };
                    if let Some(found) = found {
                        color = packed(found);
                        stack.push(color);
                    }
                }
                Special::Color(None) => {}
                Special::Icon(name) if name.is_empty() => {}
                Special::Icon(name) => match find_icon(name, sprite) {
                    Some(icon) => {
                        let advance = icon.advance()?;
                        let (size, uv) = icon.quad();
                        pieces.push(Piece::Icon {
                            at: [pen, top + height(line) / 2 - size[1] / 2],
                            sprite: icon.sprite,
                            size,
                            uv,
                        });
                        pen += advance;
                    }
                    None => pieces.push(Piece::Undefined(name.clone())),
                },
                Special::Flag(tag) => {
                    if tag.chars().count() > 2 || (laid.resolved_flags && ch == RESOLVED_FLAG) {
                        pieces.push(Piece::Flag {
                            tag: tag.clone(),
                            at: [pen, top + 5 - (line_height / 2 - height(line) / 2)],
                            resolved: laid.resolved_flags && ch == RESOLVED_FLAG,
                        });
                    }
                    pen += 12;
                }
                Special::Currency | Special::Number(_) => {
                    return Err(
                        "shows a currency or number sprite (0x14, 0x1A), whose record reads an uninitialised allocation: the game reads uninitialised memory"
                            .into(),
                    );
                }
            }
            previous = None;
            index += length;
            continue;
        }
        if let Some(glyph) = glyphs.glyph(ch) {
            pen += previous.map_or(0, |previous| glyphs.kerning(previous, ch));
            if glyph.height >= 1 {
                pieces.push(Piece::Glyph {
                    ch: glyph.ch,
                    at: [
                        pen + glyph.xoffset,
                        top + glyph.yoffset - (line_height / 2 - height(line) / 2),
                    ],
                    color,
                });
            }
            pen += glyph.advance;
            previous = Some(glyph.ch);
        }
        index += 1;
    }
    Ok(pieces)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Page {
    pub texture: usize,
    pub size: [i32; 2],
}

#[derive(Clone, Debug, PartialEq)]
pub struct Font {
    pub facts: content_reader_gui::gui::native::fonts::FontFacts,
    pub glyphs: content_reader_gui::gui::native::fonts::GlyphSet,
    pub pages: Vec<Option<Page>>,
    pub refusals: Vec<(super::native_gui::RefusalKind, String)>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct GlyphQuad {
    pub page: Page,
    pub size: [i32; 2],
    pub uv: [f32; 4],
    pub null_page: Option<Vec<u8>>,
}

impl Font {
    pub fn name(&self) -> String {
        String::from_utf8_lossy(&self.facts.name).into_owned()
    }

    pub fn quad(&self, ch: char) -> Option<GlyphQuad> {
        let glyph = self.glyphs.glyphs.get(&u32::from(ch))?;
        let index = glyph.page as usize;
        let (page, null_page) = match self.pages.get(index).copied().flatten() {
            Some(page) => (page, None),
            None => (
                Page {
                    texture: super::sprites::NO_TEXTURE,
                    size: [0, 0],
                },
                Some(self.glyphs.pages.get(index).cloned().unwrap_or_default()),
            ),
        };
        let [w, h] = page.size.map(|value| value as f32);
        Some(GlyphQuad {
            page,
            size: [glyph.width, glyph.height],
            uv: [
                glyph.x as f32 / w,
                glyph.y as f32 / h,
                glyph.x.wrapping_add(glyph.width) as f32 / w,
                glyph.y.wrapping_add(glyph.height) as f32 / h,
            ],
            null_page,
        })
    }
}

impl Glyphs for Font {
    fn own(&self, ch: char) -> Option<Glyph> {
        let glyph = self.glyphs.glyphs.get(&u32::from(ch))?;
        Some(Glyph {
            ch,
            advance: glyph.xadvance,
            width: glyph.width,
            height: glyph.height,
            xoffset: glyph.xoffset,
            yoffset: glyph.yoffset,
        })
    }

    fn kerning(&self, a: char, b: char) -> i32 {
        self.glyphs
            .glyphs
            .get(&u32::from(a))
            .and_then(|glyph| glyph.kernings.get(&u32::from(b)))
            .copied()
            .unwrap_or(0)
    }

    fn line_height(&self) -> i32 {
        self.glyphs.line_height.unwrap_or(0)
    }
}

#[cfg(any(test, feature = "test-support"))]
pub fn monospace(name: &str, width: i32, height: i32) -> Font {
    use content_reader_gui::gui::native::fonts::{FontFacts, Glyph as Record, GlyphSet};
    let glyphs = (0x20u32..0x7F)
        .map(|code| {
            (
                code,
                Record {
                    page: 0,
                    x: (code as i32 - 0x20) * width,
                    y: 0,
                    width,
                    height: if code == 0x20 { 0 } else { height },
                    xoffset: 0,
                    yoffset: 0,
                    xadvance: width,
                    kernings: BTreeMap::new(),
                },
            )
        })
        .collect();
    Font {
        facts: FontFacts {
            name: name.as_bytes().to_vec(),
            color: 0xFFFF_FFFF,
            border_color: 0,
            files: vec![b"mono".to_vec()],
            icons_add_height: false,
            icon_scale: 1.0,
            text_colors: BTreeMap::new(),
            offsets: [0; 4],
            viewport: [None; 2],
            aux_color: [1.0; 4],
            refusals: Vec::new(),
        },
        glyphs: GlyphSet {
            line_height: Some(height),
            base: Some(height),
            pages: vec![b"mono.tga".to_vec()],
            glyphs,
            missing: Vec::new(),
        },
        pages: vec![Some(Page {
            texture: 0,
            size: [95 * width, height],
        })],
        refusals: Vec::new(),
    }
}

#[cfg(test)]
mod tests;

#[cfg(test)]
mod validation_tests {
    use content_reader_localisation::localisation::icon_lookup;
    use content_reader_localisation::localisation::{Argument, Localisation};
    use viewer_foundation::tooltip_text::{TextIcons, builtin, displayable, displayable_flags, token};
    struct Icons;
    impl TextIcons for Icons {
        fn icon(&self, name: &str) -> Option<(u32, i64)> {
            name.starts_with("strip").then(|| {
                (
                    3,
                    i64::from(icon_lookup(name).1.unwrap_or(0)),
                )
            })
        }
    }

    #[test]
    fn carrier_flags_facade_preserves_other_text_validation() {
        let flag = "\u{12}SYN\u{11}HCountry\u{11}!:\n".to_owned();
        assert!(
            displayable(&Icons, flag.clone())
                .unwrap_err()
                .contains("inline flag")
        );
        assert_eq!(displayable_flags(&Icons, flag.clone()), Ok(flag));
        for text in ["\u{13}strip|4 ", "\u{14}", "\u{1a}x"] {
            assert_eq!(
                displayable_flags(&Icons, text.into()),
                displayable(&Icons, text.into()),
            );
            assert!(displayable_flags(&Icons, text.into()).is_err());
        }
        assert_eq!(
            displayable_flags(&Icons, "\u{13}missing ".into()),
            Ok("\u{13}missing ".into())
        );
    }

    #[test]
    fn localised_text_is_native_and_refused_by_name() {
        let localisation: Localisation = [
            ("BATTLE_ATTACK", "§YAngriff§! $NESTED$"),
            ("NESTED", "[x] 100§§"),
            ("SKILL", "$SKILL|Y$"),
            ("FLAG", "@GER"),
            ("ICON", "£strip|3 £strip|4 £missing"),
            ("SPACE", "a¤b"),
        ]
        .into_iter()
        .map(|(key, value)| (key.to_owned(), value.to_owned()))
        .collect();
        assert_eq!(
            builtin(&Icons, &localisation, "BATTLE_ATTACK", &[]).as_deref(),
            Ok("\u{11}YAngriff\u{11}! [x] 100§")
        );
        assert_eq!(
            builtin(
                &Icons,
                &localisation,
                "SKILL",
                &[("SKILL", Argument::Int32(4))]
            )
            .as_deref(),
            Ok("\u{11}Y4\u{11}!")
        );
        assert_eq!(
            builtin(&Icons, &localisation, "BUTTON_DISMISS", &[]).as_deref(),
            Ok("BUTTON_DISMISS")
        );
        assert_eq!(
            token(&Icons, &localisation, b"BUTTON_DISMISS\0x").as_deref(),
            Ok("BUTTON_DISMISS")
        );
        assert_eq!(token(&Icons, &localisation, b"").as_deref(), Ok(""));
        assert_eq!(
            token(&Icons, &localisation, b"FLAG"),
            Err("localisation FLAG holds the inline flag 0x12 \"GER\" (an authored `@`), which the viewer does not draw".into())
        );
        assert_eq!(
            token(&Icons, &localisation, b"ICON"),
            Err("localisation ICON holds the icon 0x13 \"strip|4\" (an authored `£`), which selects frame index 3 of a 3-frame sprite, outside it".into())
        );
        assert!(
            token(&Icons, &localisation, b"SPACE")
                .unwrap_err()
                .contains("0x14")
        );
    }
}
