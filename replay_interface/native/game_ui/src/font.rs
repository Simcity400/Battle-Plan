use crate::assets::{normalize_rel_path, AssetResolver};
use crate::dds::RgbaImage;
use crate::{ErrorKind, GameUiError, Result};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;

#[derive(Clone, Debug, PartialEq)]
pub struct BitmapFont {
    pub name: String,
    pub source_file: String,
    pub info_size: i32,
    pub info_padding: [i32; 4],
    pub info_spacing: [i32; 2],
    pub outline: i32,
    pub line_height: i32,
    pub base: i32,
    pub scale_w: usize,
    pub scale_h: usize,
    pub glyph_channel: Option<usize>,
    pub pages: Vec<FontPage>,
    pub chars: BTreeMap<u32, FontChar>,
    pub kernings: BTreeMap<(u32, u32), i32>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FontPage {
    pub id: u32,
    pub file: String,
    pub image: RgbaImage,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FontChar {
    pub id: u32,
    pub x: usize,
    pub y: usize,
    pub width: usize,
    pub height: usize,
    pub xoffset: i32,
    pub yoffset: i32,
    pub xadvance: i32,
    pub page: u32,
    pub chnl: u32,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TextLayout {
    pub quads: Vec<GlyphQuad>,
    pub missing_glyphs: BTreeSet<char>,
    pub width: i32,
    pub height: i32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GlyphQuad {
    pub ch: char,
    pub char_index: usize,
    pub id: u32,
    pub pen_x: i32,
    pub pen_y: i32,
    pub baseline_y: i32,
    pub x: i32,
    pub y: i32,
    pub width: usize,
    pub height: usize,
    pub xadvance: i32,
    pub kerning: i32,
    pub page: u32,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TextRenderReport {
    pub missing_glyphs: BTreeSet<char>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ColorizedText {
    pub text: String,
    pub colors: Vec<[u8; 4]>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TextAlign {
    Left,
    Center,
    Right,
}

impl TextAlign {
    pub fn parse(value: Option<&str>) -> Self {
        match value.unwrap_or("left").to_ascii_lowercase().as_str() {
            "center" | "centre" => Self::Center,
            "right" => Self::Right,
            _ => Self::Left,
        }
    }
}

impl BitmapFont {
    pub fn load(resolver: &AssetResolver, name: &str) -> Result<Self> {
        let rel = format!("gfx/fonts/{name}.fnt");
        let resolved = resolver.resolve_file(&rel)?;
        let bytes = fs::read(&resolved.path)?;
        let parsed = parse_bmfont_bytes(name, &resolved.rel_path, &bytes)?;
        parsed.load_pages(resolver)
    }

    pub fn printable_ascii_coverage(&self) -> BTreeSet<char> {
        (32u8..=126)
            .filter_map(|byte| {
                let ch = char::from(byte);
                self.chars.contains_key(&u32::from(byte)).then_some(ch)
            })
            .collect()
    }

    pub fn missing_chars(&self, text: &str) -> BTreeSet<char> {
        strip_color_codes(text)
            .chars()
            .filter(|ch| !ch.is_control() && !self.chars.contains_key(&(*ch as u32)))
            .collect()
    }

    pub fn measure_text(&self, text: &str) -> i32 {
        self.layout_text(0, 0, text).width
    }

    pub fn layout_text(&self, x: i32, y: i32, text: &str) -> TextLayout {
        let mut layout = TextLayout::default();
        let mut pen_x = x;
        let mut pen_y = y;
        let mut line_start_x = x;
        let mut max_line_width = 0i32;
        let mut line_count = 1i32;
        let mut prev = None;

        for (char_index, ch) in strip_color_codes(text).chars().enumerate() {
            if ch == '\n' {
                max_line_width = max_line_width.max(pen_x - line_start_x);
                pen_x = x;
                pen_y += self.line_height.max(0);
                line_start_x = x;
                line_count += 1;
                prev = None;
                continue;
            }
            let id = ch as u32;
            let kerning = prev
                .and_then(|prev_id| self.kernings.get(&(prev_id, id)).copied())
                .unwrap_or(0);
            pen_x += kerning;
            let Some(font_char) = self.chars.get(&id) else {
                if !ch.is_control() {
                    layout.missing_glyphs.insert(ch);
                }
                prev = Some(id);
                continue;
            };
            layout.quads.push(GlyphQuad {
                ch,
                char_index,
                id,
                pen_x,
                pen_y,
                baseline_y: pen_y + self.base,
                x: pen_x + font_char.xoffset,
                y: pen_y + font_char.yoffset,
                width: font_char.width,
                height: font_char.height,
                xadvance: font_char.xadvance,
                kerning,
                page: font_char.page,
            });
            pen_x += font_char.xadvance;
            prev = Some(id);
        }

        max_line_width = max_line_width.max(pen_x - line_start_x);
        layout.width = max_line_width.max(0);
        layout.height = (line_count * self.line_height.max(0)).max(0);
        layout
    }

    pub fn draw_text(
        &self,
        image: &mut RgbaImage,
        x: i32,
        y: i32,
        max_width: Option<i32>,
        align: TextAlign,
        text: &str,
        color: [u8; 4],
    ) -> TextRenderReport {
        let rich = colorize_hoi4_text(text, color);
        let measured = self.measure_text(&rich.text);
        let x = match (max_width, align) {
            (Some(max_width), TextAlign::Center) => x + (max_width - measured) / 2,
            (Some(max_width), TextAlign::Right) => x + max_width - measured,
            _ => x,
        };

        let layout = self.layout_text(x, y, &rich.text);
        for quad in &layout.quads {
            if quad.width > 0 && quad.height > 0 {
                let Some(font_char) = self.chars.get(&quad.id) else {
                    continue;
                };
                if let Some(page) = self.pages.iter().find(|page| page.id == font_char.page) {
                    let tint = rich.colors.get(quad.char_index).copied().unwrap_or(color);
                    blit_glyph(
                        image,
                        &page.image,
                        quad.x,
                        quad.y,
                        font_char,
                        tint,
                        self.glyph_channel,
                    );
                }
            }
        }
        TextRenderReport {
            missing_glyphs: layout.missing_glyphs,
        }
    }
}

pub fn strip_color_codes(text: &str) -> String {
    if text.contains('\u{00a7}') || text.contains('\u{00c2}') {
        return colorize_hoi4_text(text, [0, 0, 0, 0]).text;
    }
    let mut out = String::new();
    let mut chars = text.chars();
    while let Some(ch) = chars.next() {
        if ch == '§' {
            let _ = chars.next();
        } else {
            out.push(ch);
        }
    }
    out
}

pub fn colorize_hoi4_text(text: &str, default_color: [u8; 4]) -> ColorizedText {
    let mut out = String::new();
    let mut colors = Vec::new();
    let mut current = default_color;
    let mut chars = text.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch == '\u{00a7}' || (ch == '\u{00c2}' && chars.peek() == Some(&'\u{00a7}')) {
            if ch == '\u{00c2}' {
                let _ = chars.next();
            }
            let Some(code) = chars.next() else {
                continue;
            };
            current = match code.to_ascii_uppercase() {
                'G' => [134, 210, 120, default_color[3]],
                'R' => [210, 80, 72, default_color[3]],
                'Y' => [245, 213, 101, default_color[3]],
                '!' => default_color,
                _ => current,
            };
            continue;
        }
        out.push(ch);
        colors.push(current);
    }
    ColorizedText { text: out, colors }
}

fn blit_glyph(
    dst: &mut RgbaImage,
    page: &RgbaImage,
    x: i32,
    y: i32,
    font_char: &FontChar,
    color: [u8; 4],
    glyph_channel: Option<usize>,
) {
    for gy in 0..font_char.height {
        let dy = y + gy as i32;
        if dy < 0 || dy >= dst.height as i32 {
            continue;
        }
        for gx in 0..font_char.width {
            let dx = x + gx as i32;
            if dx < 0 || dx >= dst.width as i32 {
                continue;
            }
            let sx = font_char.x + gx;
            let sy = font_char.y + gy;
            if sx >= page.width || sy >= page.height {
                continue;
            }
            let src = (sy * page.width + sx) * 4;
            let alpha = glyph_alpha(&page.pixels[src..src + 4], color[3], glyph_channel);
            if alpha == 0 {
                continue;
            }
            let src_rgba = [color[0], color[1], color[2], alpha];
            blend_pixel(dst, dx as usize, dy as usize, src_rgba);
        }
    }
}

fn glyph_alpha(src: &[u8], tint_alpha: u8, glyph_channel: Option<usize>) -> u8 {
    let coverage = if let Some(channel) = glyph_channel {
        if channel == 3 {
            src[channel]
        } else {
            255 - src[channel]
        }
    } else if src[3] < 255 {
        src[3]
    } else {
        src[0].max(src[1]).max(src[2])
    };
    ((u16::from(coverage) * u16::from(tint_alpha) + 127) / 255) as u8
}

fn blend_pixel(dst: &mut RgbaImage, x: usize, y: usize, src: [u8; 4]) {
    let offset = (y * dst.width + x) * 4;
    let dst_px = [
        dst.pixels[offset],
        dst.pixels[offset + 1],
        dst.pixels[offset + 2],
        dst.pixels[offset + 3],
    ];
    let out = alpha_blend(dst_px, src);
    dst.pixels[offset..offset + 4].copy_from_slice(&out);
}

pub fn alpha_blend(dst: [u8; 4], src: [u8; 4]) -> [u8; 4] {
    let sa = u32::from(src[3]);
    if sa == 0 {
        return dst;
    }
    let da = u32::from(dst[3]);
    let inv_sa = 255 - sa;
    let out_a = sa + (da * inv_sa + 127) / 255;
    if out_a == 0 {
        return [0, 0, 0, 0];
    }
    let mut out = [0u8; 4];
    for channel in 0..3 {
        let sr = u32::from(src[channel]);
        let dr = u32::from(dst[channel]);
        let premul = sr * sa + (dr * da * inv_sa + 127) / 255;
        out[channel] = ((premul + out_a / 2) / out_a).min(255) as u8;
    }
    out[3] = out_a.min(255) as u8;
    out
}

#[derive(Clone, Debug)]
struct ParsedFont {
    name: String,
    source_file: String,
    info_size: i32,
    info_padding: [i32; 4],
    info_spacing: [i32; 2],
    outline: i32,
    line_height: i32,
    base: i32,
    scale_w: usize,
    scale_h: usize,
    glyph_channel: Option<usize>,
    page_files: BTreeMap<u32, String>,
    chars: BTreeMap<u32, FontChar>,
    kernings: BTreeMap<(u32, u32), i32>,
}

impl ParsedFont {
    fn load_pages(self, resolver: &AssetResolver) -> Result<BitmapFont> {
        let mut pages = Vec::new();
        let page_files = if self.page_files.is_empty() {
            inferred_page_files(&self.name, resolver)?
        } else {
            self.page_files
        };
        for (id, file) in page_files {
            let rel = if normalize_rel_path(&file).contains('/') {
                normalize_rel_path(&file)
            } else {
                normalize_rel_path(&format!("gfx/fonts/{file}"))
            };
            let decoded = resolver
                .decode_texture(&rel)
                .or_else(|err| decode_inferred_page(resolver, &self.name, id).map_err(|_| err))?;
            pages.push(FontPage {
                id,
                file: decoded.rel_path,
                image: decoded.image,
            });
        }
        Ok(BitmapFont {
            name: self.name,
            source_file: self.source_file,
            info_size: self.info_size,
            info_padding: self.info_padding,
            info_spacing: self.info_spacing,
            outline: self.outline,
            line_height: self.line_height,
            base: self.base,
            scale_w: self.scale_w,
            scale_h: self.scale_h,
            glyph_channel: self.glyph_channel,
            pages,
            chars: self.chars,
            kernings: self.kernings,
        })
    }
}

fn decode_inferred_page(
    resolver: &AssetResolver,
    name: &str,
    id: u32,
) -> Result<crate::assets::DecodedTexture> {
    for candidate in [
        format!("gfx/fonts/{name}.dds"),
        format!("gfx/fonts/{name}.tga"),
        format!("gfx/fonts/{name}_{id}.dds"),
        format!("gfx/fonts/{name}_{id}.tga"),
    ] {
        if resolver.resolve_optional_file(&candidate)?.is_some() {
            return resolver.decode_texture(&candidate);
        }
    }
    Err(GameUiError::missing(format!(
        "no inferred texture page found for font {name} page {id}"
    )))
}

fn parse_bmfont_bytes(name: &str, source_file: &str, bytes: &[u8]) -> Result<ParsedFont> {
    if bytes.get(0..3) == Some(b"BMF") {
        parse_binary_bmfont(name, source_file, bytes)
    } else {
        parse_text_bmfont(name, source_file, bytes)
    }
}

fn parse_text_bmfont(name: &str, source_file: &str, bytes: &[u8]) -> Result<ParsedFont> {
    let text = crate::paradox_text::decode_text(bytes);
    let mut parsed = ParsedFont {
        name: name.to_string(),
        source_file: source_file.to_string(),
        info_size: 0,
        info_padding: [0; 4],
        info_spacing: [0; 2],
        outline: 0,
        line_height: 0,
        base: 0,
        scale_w: 0,
        scale_h: 0,
        glyph_channel: None,
        page_files: BTreeMap::new(),
        chars: BTreeMap::new(),
        kernings: BTreeMap::new(),
    };
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let Some((kind, rest)) = line.split_once(char::is_whitespace) else {
            continue;
        };
        let fields = parse_kv_fields(rest);
        match kind {
            "info" => {
                parsed.info_size = field_i32(&fields, "size").unwrap_or(0);
                parsed.info_padding = field_i32_array::<4>(&fields, "padding").unwrap_or([0; 4]);
                parsed.info_spacing = field_i32_array::<2>(&fields, "spacing").unwrap_or([0; 2]);
                parsed.outline = field_i32(&fields, "outline").unwrap_or(0);
            }
            "common" => {
                parsed.line_height = field_i32(&fields, "lineHeight").unwrap_or(0);
                parsed.base = field_i32(&fields, "base").unwrap_or(0);
                parsed.scale_w = field_usize(&fields, "scaleW").unwrap_or(0);
                parsed.scale_h = field_usize(&fields, "scaleH").unwrap_or(0);
                parsed.glyph_channel = glyph_channel_from_common(
                    field_u32(&fields, "redChnl"),
                    field_u32(&fields, "greenChnl"),
                    field_u32(&fields, "blueChnl"),
                    field_u32(&fields, "alphaChnl"),
                );
            }
            "page" => {
                if let (Some(id), Some(file)) = (field_u32(&fields, "id"), fields.get("file")) {
                    parsed.page_files.insert(id, file.to_string());
                }
            }
            "char" => {
                if let Some(font_char) = parse_text_char(&fields) {
                    parsed.chars.insert(font_char.id, font_char);
                }
            }
            "kerning" => {
                if let (Some(first), Some(second), Some(amount)) = (
                    field_u32(&fields, "first"),
                    field_u32(&fields, "second"),
                    field_i32(&fields, "amount"),
                ) {
                    parsed.kernings.insert((first, second), amount);
                }
            }
            _ => {}
        }
    }
    Ok(parsed)
}

fn parse_text_char(fields: &BTreeMap<String, String>) -> Option<FontChar> {
    Some(FontChar {
        id: field_u32(fields, "id")?,
        x: field_usize(fields, "x")?,
        y: field_usize(fields, "y")?,
        width: field_usize(fields, "width")?,
        height: field_usize(fields, "height")?,
        xoffset: field_i32(fields, "xoffset")?,
        yoffset: field_i32(fields, "yoffset")?,
        xadvance: field_i32(fields, "xadvance")?,
        page: field_u32(fields, "page").unwrap_or(0),
        chnl: field_u32(fields, "chnl").unwrap_or(15),
    })
}

fn parse_kv_fields(text: &str) -> BTreeMap<String, String> {
    let mut fields = BTreeMap::new();
    let mut pos = 0usize;
    let bytes = text.as_bytes();
    while pos < bytes.len() {
        while pos < bytes.len() && bytes[pos].is_ascii_whitespace() {
            pos += 1;
        }
        let key_start = pos;
        while pos < bytes.len() && bytes[pos] != b'=' && !bytes[pos].is_ascii_whitespace() {
            pos += 1;
        }
        if pos >= bytes.len() || bytes[pos] != b'=' {
            while pos < bytes.len() && !bytes[pos].is_ascii_whitespace() {
                pos += 1;
            }
            continue;
        }
        let key = &text[key_start..pos];
        pos += 1;
        let value = if pos < bytes.len() && bytes[pos] == b'"' {
            pos += 1;
            let start = pos;
            while pos < bytes.len() && bytes[pos] != b'"' {
                pos += 1;
            }
            let value = text[start..pos].to_string();
            if pos < bytes.len() {
                pos += 1;
            }
            value
        } else {
            let start = pos;
            while pos < bytes.len() && !bytes[pos].is_ascii_whitespace() {
                pos += 1;
            }
            text[start..pos].to_string()
        };
        fields.insert(key.to_string(), value);
    }
    fields
}

fn parse_binary_bmfont(name: &str, source_file: &str, bytes: &[u8]) -> Result<ParsedFont> {
    if bytes.len() < 4 {
        return Err(GameUiError::parse("binary BMFont header is truncated"));
    }
    let mut parsed = ParsedFont {
        name: name.to_string(),
        source_file: source_file.to_string(),
        info_size: 0,
        info_padding: [0; 4],
        info_spacing: [0; 2],
        outline: 0,
        line_height: 0,
        base: 0,
        scale_w: 0,
        scale_h: 0,
        glyph_channel: None,
        page_files: BTreeMap::new(),
        chars: BTreeMap::new(),
        kernings: BTreeMap::new(),
    };
    let mut pos = 4usize;
    while pos < bytes.len() {
        if pos + 5 > bytes.len() {
            return Err(GameUiError::parse(
                "binary BMFont block header is truncated",
            ));
        }
        let block_type = bytes[pos];
        let size = u32::from_le_bytes(
            bytes[pos + 1..pos + 5]
                .try_into()
                .expect("slice length checked"),
        ) as usize;
        pos += 5;
        let block = bytes.get(pos..pos + size).ok_or_else(|| {
            GameUiError::parse(format!("binary BMFont block {block_type} is truncated"))
        })?;
        match block_type {
            1 => {
                if block.len() >= 2 {
                    parsed.info_size = i16::from_le_bytes([block[0], block[1]]) as i32;
                }
                if block.len() >= 11 {
                    parsed.info_padding = [
                        i32::from(block[7]),
                        i32::from(block[8]),
                        i32::from(block[9]),
                        i32::from(block[10]),
                    ];
                }
                if block.len() >= 13 {
                    parsed.info_spacing = [i32::from(block[11]), i32::from(block[12])];
                }
                if block.len() >= 14 {
                    parsed.outline = i32::from(block[13]);
                }
            }
            2 => {
                if block.len() >= 15 {
                    parsed.line_height = u16::from_le_bytes([block[0], block[1]]) as i32;
                    parsed.base = u16::from_le_bytes([block[2], block[3]]) as i32;
                    parsed.scale_w = u16::from_le_bytes([block[4], block[5]]) as usize;
                    parsed.scale_h = u16::from_le_bytes([block[6], block[7]]) as usize;
                    parsed.glyph_channel = glyph_channel_from_common(
                        Some(u32::from(block[12])),
                        Some(u32::from(block[13])),
                        Some(u32::from(block[14])),
                        Some(u32::from(block[11])),
                    );
                }
            }
            3 => parse_binary_pages(block, &mut parsed.page_files),
            4 => parse_binary_chars(block, &mut parsed.chars)?,
            5 => parse_binary_kernings(block, &mut parsed.kernings)?,
            _ => {}
        }
        pos += size;
    }
    Ok(parsed)
}

fn parse_binary_pages(block: &[u8], out: &mut BTreeMap<u32, String>) {
    for (idx, page) in block
        .split(|byte| *byte == 0)
        .filter(|page| !page.is_empty())
        .enumerate()
    {
        out.insert(idx as u32, String::from_utf8_lossy(page).into_owned());
    }
}

fn parse_binary_chars(block: &[u8], out: &mut BTreeMap<u32, FontChar>) -> Result<()> {
    if block.len() % 20 != 0 {
        return Err(GameUiError::parse(
            "binary BMFont chars block has invalid length",
        ));
    }
    for chunk in block.chunks_exact(20) {
        let font_char = FontChar {
            id: le_u32(chunk, 0),
            x: le_u16(chunk, 4) as usize,
            y: le_u16(chunk, 6) as usize,
            width: le_u16(chunk, 8) as usize,
            height: le_u16(chunk, 10) as usize,
            xoffset: le_i16(chunk, 12) as i32,
            yoffset: le_i16(chunk, 14) as i32,
            xadvance: le_i16(chunk, 16) as i32,
            page: u32::from(chunk[18]),
            chnl: u32::from(chunk[19]),
        };
        out.insert(font_char.id, font_char);
    }
    Ok(())
}

fn parse_binary_kernings(block: &[u8], out: &mut BTreeMap<(u32, u32), i32>) -> Result<()> {
    if block.len() % 10 != 0 {
        return Err(GameUiError::parse(
            "binary BMFont kernings block has invalid length",
        ));
    }
    for chunk in block.chunks_exact(10) {
        out.insert(
            (le_u32(chunk, 0), le_u32(chunk, 4)),
            le_i16(chunk, 8) as i32,
        );
    }
    Ok(())
}

fn inferred_page_files(name: &str, resolver: &AssetResolver) -> Result<BTreeMap<u32, String>> {
    for candidate in [
        format!("gfx/fonts/{name}.dds"),
        format!("gfx/fonts/{name}.tga"),
        format!("gfx/fonts/{name}_0.dds"),
        format!("gfx/fonts/{name}_0.tga"),
    ] {
        if resolver.resolve_optional_file(&candidate)?.is_some() {
            return Ok(BTreeMap::from([(0, candidate)]));
        }
    }
    Err(GameUiError::missing(format!(
        "no texture page found for font {name}"
    )))
}

fn field_i32(fields: &BTreeMap<String, String>, key: &str) -> Option<i32> {
    fields.get(key)?.trim_end_matches(',').parse().ok()
}

fn field_u32(fields: &BTreeMap<String, String>, key: &str) -> Option<u32> {
    fields.get(key)?.trim_end_matches(',').parse().ok()
}

fn field_usize(fields: &BTreeMap<String, String>, key: &str) -> Option<usize> {
    fields.get(key)?.trim_end_matches(',').parse().ok()
}

fn field_i32_array<const N: usize>(
    fields: &BTreeMap<String, String>,
    key: &str,
) -> Option<[i32; N]> {
    let values = fields
        .get(key)?
        .split(',')
        .map(|value| value.trim().parse::<i32>().ok())
        .collect::<Option<Vec<_>>>()?;
    values.try_into().ok()
}

fn glyph_channel_from_common(
    red: Option<u32>,
    green: Option<u32>,
    blue: Option<u32>,
    alpha: Option<u32>,
) -> Option<usize> {
    [(alpha, 3usize), (red, 0), (green, 1), (blue, 2)]
        .into_iter()
        .find_map(|(value, channel)| (value == Some(0)).then_some(channel))
}

fn le_u16(data: &[u8], offset: usize) -> u16 {
    u16::from_le_bytes([data[offset], data[offset + 1]])
}

fn le_i16(data: &[u8], offset: usize) -> i16 {
    i16::from_le_bytes([data[offset], data[offset + 1]])
}

fn le_u32(data: &[u8], offset: usize) -> u32 {
    u32::from_le_bytes(
        data[offset..offset + 4]
            .try_into()
            .expect("slice length checked"),
    )
}

pub fn blank_image(width: usize, height: usize) -> RgbaImage {
    RgbaImage {
        width,
        height,
        pixels: vec![0; width * height * 4],
    }
}

pub fn solid_image(width: usize, height: usize, rgba: [u8; 4]) -> RgbaImage {
    let mut image = blank_image(width, height);
    for pixel in image.pixels.chunks_exact_mut(4) {
        pixel.copy_from_slice(&rgba);
    }
    image
}

pub fn validate_ascii(font: &BitmapFont) -> Result<()> {
    let missing = (32u8..=126)
        .filter(|byte| !font.chars.contains_key(&u32::from(*byte)))
        .map(char::from)
        .collect::<String>();
    if missing.is_empty() {
        Ok(())
    } else {
        Err(GameUiError::new(
            ErrorKind::Missing,
            format!(
                "font {} is missing printable ASCII glyphs: {missing}",
                font.name
            ),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strips_hoi4_color_codes() {
        assert_eq!(
            strip_color_codes("\u{00a7}Y12\u{00a7}! / \u{00a7}R0\u{00a7}!"),
            "12 / 0"
        );
    }

    #[test]
    fn maps_hoi4_color_codes_to_per_glyph_tints() {
        let default = [221, 214, 190, 255];
        let colorized = colorize_hoi4_text(
            "\u{00a7}GFull\u{00a7}! \u{00a7}Y74%\u{00a7}! \u{00a7}R0\u{00a7}!",
            default,
        );
        assert_eq!(colorized.text, "Full 74% 0");
        assert_eq!(colorized.colors[0], [134, 210, 120, 255]);
        assert_eq!(colorized.colors[5], [245, 213, 101, 255]);
        assert_eq!(colorized.colors[9], [210, 80, 72, 255]);
    }

    #[test]
    fn parses_text_char_line() {
        let fields = parse_kv_fields(
            r#"id=65 x=1 y=2 width=3 height=4 xoffset=-1 yoffset=6 xadvance=7 page=0 chnl=15"#,
        );
        let ch = parse_text_char(&fields).unwrap();
        assert_eq!(ch.id, 65);
        assert_eq!(ch.xoffset, -1);
        assert_eq!(ch.xadvance, 7);
    }

    #[test]
    fn parses_text_info_metrics() {
        let fields =
            parse_kv_fields(r#"face="Ubuntu" size=18 padding=2,2,2,2 spacing=1,1 outline=0"#);
        assert_eq!(field_i32_array::<4>(&fields, "padding"), Some([2, 2, 2, 2]));
        assert_eq!(field_i32_array::<2>(&fields, "spacing"), Some([1, 1]));
        assert_eq!(field_i32(&fields, "outline"), Some(0));
    }

    #[test]
    fn selected_rgb_glyph_channel_is_inverted() {
        assert_eq!(glyph_alpha(&[0, 255, 255, 255], 255, Some(0)), 255);
        assert_eq!(glyph_alpha(&[255, 0, 0, 255], 255, Some(0)), 0);
        assert_eq!(glyph_alpha(&[0, 0, 0, 128], 255, Some(3)), 128);
    }

    #[test]
    fn date_layout_advances_by_xadvance_and_kerning() {
        let font = test_font_for_date();
        let text = "9 Nov 1943, 17:00";
        let mut image = blank_image(256, 48);
        let report = font.draw_text(
            &mut image,
            0,
            0,
            None,
            TextAlign::Left,
            text,
            [255, 255, 255, 255],
        );
        assert!(report.missing_glyphs.is_empty());

        let layout = font.layout_text(0, 0, text);
        assert_eq!(layout.quads.len(), text.chars().count());
        assert!(layout.missing_glyphs.is_empty());

        let mut expected_pen = 0i32;
        let mut expected_width = 0i32;
        let mut prev = None;
        let mut previous_pen = None;
        for quad in &layout.quads {
            let kerning = prev
                .and_then(|prev_id| font.kernings.get(&(prev_id, quad.id)).copied())
                .unwrap_or(0);
            expected_pen += kerning;
            assert_eq!(quad.kerning, kerning);
            assert_eq!(quad.pen_x, expected_pen);
            if let Some(previous_pen) = previous_pen {
                assert!(
                    quad.pen_x > previous_pen,
                    "pen did not advance before glyph {:?}",
                    quad.ch
                );
            }

            let glyph = font.chars.get(&quad.id).unwrap();
            assert_eq!(quad.x, quad.pen_x + glyph.xoffset);
            assert_eq!(quad.y, quad.pen_y + glyph.yoffset);
            assert_eq!(quad.baseline_y, quad.pen_y + font.base);
            expected_pen += glyph.xadvance;
            expected_width += kerning + glyph.xadvance;
            previous_pen = Some(quad.pen_x);
            prev = Some(quad.id);
        }

        assert_eq!(layout.width, expected_width);
        assert_eq!(font.measure_text(text), expected_width);
    }

    fn test_font_for_date() -> BitmapFont {
        let text = "9 Nov 1943, 17:00";
        let mut chars = BTreeMap::new();
        for ch in text.chars() {
            let id = ch as u32;
            chars.entry(id).or_insert(FontChar {
                id,
                x: 0,
                y: 0,
                width: 9,
                height: 12,
                xoffset: -2,
                yoffset: 3,
                xadvance: if ch == ' ' { 4 } else { 10 },
                page: 0,
                chnl: 15,
            });
        }
        chars.insert(
            '1' as u32,
            FontChar {
                id: '1' as u32,
                x: 0,
                y: 0,
                width: 8,
                height: 12,
                xoffset: -1,
                yoffset: 3,
                xadvance: 9,
                page: 0,
                chnl: 15,
            },
        );
        let mut pixels = vec![0u8; 16 * 16 * 4];
        for px in pixels.chunks_exact_mut(4) {
            px.copy_from_slice(&[255, 255, 255, 255]);
        }
        BitmapFont {
            name: "test_date".to_string(),
            source_file: "synthetic".to_string(),
            info_size: 18,
            info_padding: [2, 2, 2, 2],
            info_spacing: [1, 1],
            outline: 0,
            line_height: 18,
            base: 15,
            scale_w: 16,
            scale_h: 16,
            glyph_channel: None,
            pages: vec![FontPage {
                id: 0,
                file: "synthetic".to_string(),
                image: RgbaImage {
                    width: 16,
                    height: 16,
                    pixels,
                },
            }],
            chars,
            kernings: BTreeMap::from([(('1' as u32, '9' as u32), -1)]),
        }
    }
}
