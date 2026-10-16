//! Bitmap-font text layout and drawing for [`super::native_gui`].
//!
//! Layout uses integer arithmetic with division truncating toward zero. Words break
//! at zero-height glyphs, including spaces; a single-word line can break within the
//! word. Trailing spaces contribute width. Line feeds and carriage returns end lines,
//! and the layout holds at most 255 lines.
//!
//! Drawing uses line alignment, glyph offsets and kerning, preserving text colour
//! stacks and inline icons. Missing printable glyphs use the question-mark fallback;
//! control codepoints have no glyph or width.

use std::collections::BTreeMap;

/// One glyph as the layout measures it: the codepoint drawn (a missing one's `?`), its
/// advance, its box and offsets, in font pixels.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Glyph {
    pub ch: char,
    pub advance: i32,
    pub width: i32,
    pub height: i32,
    pub xoffset: i32,
    pub yoffset: i32,
}

/// A font's glyphs as the game's text code reads them.
pub trait Glyphs {
    /// The glyph `ch` draws, before the `?` fallback.
    fn own(&self, ch: char) -> Option<Glyph>;
    /// The kerning after `a` before `b`.
    fn kerning(&self, a: char, b: char) -> i32;
    /// `common lineHeight` of the first file.
    fn line_height(&self) -> i32;

    /// the glyph, else `?`'s for a codepoint above 0x1F.
    fn glyph(&self, ch: char) -> Option<Glyph> {
        self.own(ch)
            .or_else(|| (u32::from(ch) > 0x1F).then(|| self.own('?')).flatten())
    }
}

/// Decode UTF-8 codepoints with the established conversion and error policy.
/// The output allocation counts leading bytes. Invalid input returns `None`;
/// no usable codepoints leave empty text.
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
        // Validate trailing bytes in reverse order, then the byte immediately after the lead.
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
        // Strict: a surrogate fails; nothing past U+10FFFF passes the checks above.
        out.push(char::from_u32(code)?);
        at += extra + 1;
    }
    let counted = bytes.iter().filter(|byte| !continuation(**byte)).count();
    out.resize(counted.max(out.len()), '\0');
    Some(out)
}

/// The special sequences the game reads, by their control codepoint.
pub const COLOR: char = '\u{11}';
pub const FLAG: char = '\u{12}';
/// Viewer-resolved flag key, parsed only for producer-bound text.
pub const RESOLVED_FLAG: char = '\u{1f}';
pub const ICON: char = '\u{13}';
pub const CURRENCY: char = '\u{14}';
pub const NUMBER: char = '\u{1a}';
/// The layout's advance for the 0x14 control, whatever the font.
pub const CURRENCY_ADVANCE: i32 = 16;

/// One special sequence at the start of `text`: how many codepoints it takes, and what it is.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Special {
    /// `0x11 X`: the colour code `X`.
    Color(Option<char>),
    /// `0x12` and up to three codepoints: a country's flag, 12 wide.
    Flag(String),
    /// `0x13`, a name of `0-9 A-Z a-z _ |` (up to 128) and the codepoint that ends it:
    /// `GFX_<name>`'s frame.
    Icon(String),
    /// `0x14`: 16 wide.
    Currency,
    /// `0x1A` and two digits: 8 wide.
    Number(String),
}

/// The special sequence starting at `text[0]`, if its first codepoint opens one.
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
            // The codepoint after the name ends it and is taken with it.
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

/// The length of the resolved flag key the text starts with, when it names one.
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

/// Inline sprite geometry uses its loaded texture dimensions, frame count
/// and selected frame, preserving signed division and zero-frame refusal.
pub type IconSprite = ([i32; 2], i32);

/// An inline icon the content defines: the sprite found, its texture size and frames, and the
/// frame index (0 when the name has no `|`, the draw's own).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TextIcon {
    pub sprite: Vec<u8>,
    pub texture: [i32; 2],
    pub frames: i32,
    pub frame: i32,
}

impl TextIcon {
    /// Divide width by frame count after converting each to unsigned 32 bits.
    /// A zero frame count divides by zero, which ends the game.
    pub fn advance(&self) -> Result<i32, String> {
        if self.frames == 0 {
            return Err(format!(
                "shows the inline icon {}, whose zero noOfFrames makes its advance divide by zero: the game ends on the division",
                String::from_utf8_lossy(&self.sprite)
            ));
        }
        Ok(((self.texture[0] as u32) / (self.frames as u32)) as i32)
    }

    /// The quad: `width / frames` (signed) by the texture's height, at the frame's
    /// coordinates along `u` (inline icons never read `framedirectiony`): `step = 1.0f / n`,
    /// `u0 = frame * step`, `u1 = min((frame + 1) * step, 1.0)`, `v` 0 to 1, the frame not
    /// clamped.
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

/// The inline icon `name` shows, if the content defines it ([`icon_lookup`]).
pub fn find_icon(name: &str, sprite: &dyn Fn(&[u8]) -> Option<IconSprite>) -> Option<TextIcon> {
    let (names, frame) = icon_lookup(name);
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

/// A special sequence's width and word height: an icon uses its advance (none
/// when absent from content) and, under `icons_add_height`, its texture's height.
/// A flag is 12 wide, a currency sign 16 and a number 8.
fn special_advance(
    special: &Special,
    sprite: &dyn Fn(&[u8]) -> Option<IconSprite>,
    icons_add_height: bool,
) -> Result<(i32, Option<i32>), String> {
    Ok(match special {
        Special::Color(_) => (0, None),
        Special::Flag(_) => (12, None),
        // The game skips lookup for an empty parsed name, even if content defines GFX_.
        Special::Icon(name) if name.is_empty() => (0, icons_add_height.then_some(0)),
        Special::Icon(name) => match find_icon(name, sprite) {
            Some(icon) => (icon.advance()?, icons_add_height.then_some(icon.texture[1])),
            None => (0, icons_add_height.then_some(0)),
        },
        Special::Currency => (CURRENCY_ADVANCE, None),
        Special::Number(_) => (8, None),
    })
}

/// Parse the inline icon name and optional frame selector. Preserve empty-name
/// and missing-sprite behavior; the frame suffix does not establish a name.
pub fn icon_lookup(name: &str) -> ([Vec<u8>; 2], Option<i32>) {
    let (stem, suffix) = match name.split_once('|') {
        Some((stem, suffix)) => (stem, Some(suffix)),
        None => (name, None),
    };
    let frame = suffix.map(|suffix| {
        let mut value: i32 = 0;
        for byte in suffix.bytes() {
            if !byte.is_ascii_digit() {
                return 0;
            }
            value = value.wrapping_mul(10).wrapping_add(i32::from(byte - b'0'));
        }
        if value < 0 { 0 } else { value - 1 }
    });
    // The name's codepoints are ASCII (`special` takes `0-9 A-Z a-z _ |` only), so its bytes are
    // the bytes the draw hands the lookup.
    (
        [format!("GFX_{stem}").into_bytes(), stem.as_bytes().to_vec()],
        frame,
    )
}

/// What a layout is run with.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Limits {
    /// The box width; 0 or less is no limit (`0x7FFFFFFF`).
    pub width: i32,
    pub height: i32,
    pub border: [i32; 2],
    /// Writes a line feed where a line breaks (the draw's layout).
    pub newlines: bool,
    /// Copies literal leading spaces after a soft line feed; tooltip
    /// prewrapping enables this, ordinary preparation and drawing do not.
    pub preserve_indent: bool,
    /// Ends cut text with three dots (`truncate`).
    pub ellipsis: bool,
    pub multiline: bool,
    /// Reads the special sequences (`ignore_special_chars = no`).
    pub special: bool,
    /// With the font's `icons_add_height`, an inline icon raises its line to the
    /// texture's height.
    pub icons_add_height: bool,
}

/// A layout: the text as written out (cut, with line feeds when asked), each line's width and
/// height, and whether the text was cut.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Laid {
    pub text: Vec<char>,
    pub widths: Vec<i32>,
    pub heights: Vec<i32>,
    pub truncated: bool,
    resolved_flags: bool,
    resolved_keys: Vec<Vec<char>>,
}

/// The state.
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
    /// Count only U+0020 after the last LF, accepting the count only
    /// when its signed, wrapping width is strictly below the available width.
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

    /// Whether a new line may start: not when single-line, nor when it would reach the height
    /// limit.
    fn room(&self) -> bool {
        let used: i32 = self
            .heights
            .iter()
            .fold(0, |sum, height| sum.wrapping_add(*height));
        self.limits.multiline
            && self.height > self.glyph_h.wrapping_add(self.line_h).wrapping_add(used)
    }

    /// An overflow: `false` when the text is cut here, else the line broken
    ///
    fn overflow(&mut self) -> bool {
        if !self.room() {
            self.truncated = true;
            return false;
        }
        if self.line_x == self.word_w {
            // The line is the word: it stays, and the codepoint starts the next line.
            self.out.append(&mut self.word);
            let indent = self.soft_line_feed();
            self.widths.push(self.line_x);
            self.heights.push(self.line_h.max(self.word_h));
            // Keep the copied spaces in the pen, then clears the new word.
            self.line_x = indent;
            self.word_w = 0;
            self.line_h = self.base;
            self.word_h = self.base;
        } else {
            // The pending word moves to the next line; the width drops the last space's advance.
            let indent = self.soft_line_feed();
            self.out.append(&mut self.word);
            self.widths.push(self.line_x - self.word_w - self.trail);
            self.heights.push(self.line_h);
            // In this branch the copied spaces belong to the pending word's width.
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

/// Lay out codepoints with the resolved font and inline-sprite lookup.
/// Zero frame counts refuse the division; unavailable sprites contribute
/// only their specified fallback geometry.
pub fn lay_out(
    glyphs: &dyn Glyphs,
    sprite: &dyn Fn(&[u8]) -> Option<IconSprite>,
    text: &[char],
    limits: Limits,
) -> Result<Laid, String> {
    layout(glyphs, sprite, text, limits, None)
}

/// Producer-bound combat text: U+001F then its selected Normal-atlas key. The character after
/// the key remains text; unknown keys still reach the caller's counted resource refusal.
/// Authored text uses [`lay_out`]. Flags keep the native advance and draw
/// placement.
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
            // The game looks the original codepoint up in the previous glyph's table.
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
                // A break opportunity: the word is written out.
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

/// with more than two codepoints written, up to three of the last are turned
/// into `.` until three dots fit in `room` (each dot's advance plus one, less a dot pair's
/// kerning), and the rest of the three appended. A font without a `.` is not read (the game
/// reads the missing glyph).
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

/// Where each line of a drawn text starts and where its first line's top is: `start(line,
/// width)` is a line's pen x.
pub struct Placement<'a> {
    pub top: i32,
    pub start: &'a dyn Fn(usize, i32) -> i32,
}

/// One drawn piece of a text.
#[derive(Clone, Debug, PartialEq)]
pub enum Piece {
    /// A glyph's quad top-left, straight RGBA.
    Glyph {
        ch: char,
        at: [i32; 2],
        color: [f32; 4],
    },
    /// An inline icon: its sprite's texture at the frame's raw coordinates, `size` at `at`.
    Icon {
        sprite: Vec<u8>,
        at: [i32; 2],
        size: [i32; 2],
        uv: [f32; 4],
    },
    /// An icon the content does not define (`text_icon_lookup` finds nothing): no width, drawn
    /// as nothing.
    Undefined(String),
    /// A flag : the three-codepoint tag's small
    /// flag, 10 by 7 at `at`.
    Flag {
        tag: String,
        at: [i32; 2],
        /// The producer already resolved an atlas key, rather than a native country tag.
        resolved: bool,
    },
}

/// The colours a text draws with: the text's own (its object colour), the font's `textcolors`
/// and the global table.
pub struct Colors<'a> {
    pub own: [f32; 4],
    pub font: &'a BTreeMap<u8, [f32; 4]>,
    pub global: &'a BTreeMap<u8, [f32; 4]>,
}

impl Colors<'_> {
    /// The lookup of a code: the font's table, then the global one, by its low
    /// byte.
    pub fn lookup(&self, code: char) -> Option<[f32; 4]> {
        let byte = (u32::from(code) & 0xFF) as u8;
        self.font
            .get(&byte)
            .or_else(|| self.global.get(&byte))
            .copied()
    }
}

/// a colour packed to bytes, `f * 255`
/// truncated.
pub fn packed(color: [f32; 4]) -> [f32; 4] {
    color.map(|value| ((value * 255.0) as i64 & 0xFF) as f32 / 255.0)
}

/// Draw laid-out lines from their alignment positions, preserving glyph offsets,
/// kerning, colour stacks and inline artwork. Each draw starts with an empty
/// colour stack. Inline flags use three codepoints and their dedicated geometry.
/// Currency and numeric sprite records remain uninitialised and refuse drawing.
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
                    // "Couldnt find texticon": no advance.
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

/// Loaded font page texture and the extent used for glyph UVs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Page {
    /// The caller's texture id.
    pub texture: usize,
    /// The texture extent used to normalise the glyph's box.
    pub size: [i32; 2],
}

/// A resolved font with the selected language family and loaded glyph pages.
/// The registry supplies fallback font selection; each page retains its loaded
/// texture extent and independent read failures.
#[derive(Clone, Debug, PartialEq)]
pub struct Font {
    /// The registered font's facts: its name, `color`, `textcolors`, `icons_add_height`.
    pub facts: installed_content::gui::native::fonts::FontFacts,
    /// The selected family's glyphs.
    pub glyphs: installed_content::gui::native::fonts::GlyphSet,
    /// Missing font-page textures retain a zero-by-zero extent and null binding.
    /// Their glyphs remain in layout but draw no visible pixels.
    pub pages: Vec<Option<Page>>,
    /// Why a text in this font cannot be drawn as the game draws it: the reader's refusals of
    /// its declaration ([`installed_content::gui::native::fonts::FontFacts::refusals`]), a `.fnt` the
    /// layers cannot address, a page in a format this port has not implemented. A layout
    /// refuses a text in it by these reasons.
    pub refusals: Vec<(super::native_gui::RefusalKind, String)>,
}

/// A glyph's quad as the game writes it : its box in
/// font pixels and its texture coordinates, the box over the page's extent (in `f32`, so a null
/// page's 0 by 0 gives infinite or NaN coordinates), `v` running down.
#[derive(Clone, Debug, PartialEq)]
pub struct GlyphQuad {
    pub page: Page,
    pub size: [i32; 2],
    pub uv: [f32; 4],
    /// The page's texture name when its texture did not load: the quad binds the null texture
    /// ([`super::sprites::NO_TEXTURE`]), which a caller notes.
    pub null_page: Option<Vec<u8>>,
}

impl Font {
    /// The registered name, lossy for display.
    pub fn name(&self) -> String {
        String::from_utf8_lossy(&self.facts.name).into_owned()
    }

    /// The quad of the glyph drawn for `ch` (already the drawn codepoint, a fallback's `?`):
    /// `x / w`, `y / h`, `(x + width) / w`, `(y + height) / h` in `f32`, over the null texture's
    /// 0 by 0 when its page did not load. A glyph's page is the index of the family file it was
    /// read from ([`installed_content::gui::native::fonts::Glyph::page`]), which always has a record.
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
            // The font loader already stored the space height as zero.
            height: glyph.height,
            xoffset: glyph.xoffset,
            yoffset: glyph.yoffset,
        })
    }

    /// the first pair of `b` in `a`'s list, from any file loaded so far.
    fn kerning(&self, a: char, b: char) -> i32 {
        self.glyphs
            .glyphs
            .get(&u32::from(a))
            .and_then(|glyph| glyph.kernings.get(&u32::from(b)))
            .copied()
            .unwrap_or(0)
    }

    /// `lineHeight` of the family's first file, or 0 when that file does not load.
    fn line_height(&self) -> i32 {
        self.glyphs.line_height.unwrap_or(0)
    }
}

/// A font of every printable ASCII glyph `width` wide and `height` tall on one page (the space
/// `load_font`'s height 0), for tests.
#[cfg(any(test, feature = "test-support"))]
pub fn monospace(name: &str, width: i32, height: i32) -> Font {
    use installed_content::gui::native::fonts::{FontFacts, Glyph as Record, GlyphSet};
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
