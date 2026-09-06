
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

use game_ui::assets::normalize_rel_path;
use game_ui::dds::{self, RgbaImage};
use game_ui::tga;
use game_ui::ErrorKind;
use map_viewer::{GameFileResolver, LabelTextMetrics, MapLabelStraightLayoutStation};

use crate::Result;
use portable_content_manifest::paradox_text::{
    decode_text, destructive_event_count, diagnose_text_decode,
};

fn reject_font_decode_damage(bytes: &[u8], source: &str) -> Result<()> {
    let events = destructive_event_count(&diagnose_text_decode(bytes));
    if events != 0 {
        return Err(format!("{source}: decode destroyed {events} byte sequences").into());
    }
    Ok(())
}

pub const OVERLAY_FONT_NAME: &str = "hoi_18mbs";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Glyph {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
    pub xoffset: i32,
    pub yoffset: i32,
    pub xadvance: i32,
    pub page: usize,
}

#[derive(Clone)]
pub struct FontPage {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
    pub mips: Vec<(u32, u32, Vec<u8>)>,
}

impl std::fmt::Debug for FontPage {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("FontPage")
            .field("width", &self.width)
            .field("height", &self.height)
            .field("rgba_len", &self.rgba.len())
            .field("mip_levels", &self.mips.len())
            .finish()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AtlasMode {
    Coverage,
    PreservedColor,
}

#[derive(Clone)]
pub struct Font {
    pub name: String,
    pub line_height: i32,
    #[cfg_attr(not(test), allow(dead_code))]
    pub base: i32,
    pub default_color: u32,
    glyphs: HashMap<u32, Glyph>,
    kernings: HashMap<(u32, u32), i32>,
    pub pages: Vec<FontPage>,
}

impl std::fmt::Debug for Font {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
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

pub struct TextLayout {
    pub quads: Vec<GlyphQuad>,
    pub missing: Vec<char>,
    pub advance: f32,
}

#[derive(Clone, Debug, PartialEq)]
pub struct FontSpec {
    pub name: String,
    pub files: Vec<String>,
    pub color: u32,
}

#[derive(Clone, Debug)]
pub struct OverlayFontSelection {
    pub font: Font,
    pub font_name: &'static str,
    pub root: PathBuf,
}

#[cfg(test)]
pub fn load_game_font(game_root: &Path, font_name: &str) -> Result<Font> {
    let resolver = GameFileResolver::vanilla(game_root.to_path_buf())?;
    load_game_font_from_resolver(&resolver, font_name)
}

pub fn load_game_font_from_resolver(resolver: &GameFileResolver, font_name: &str) -> Result<Font> {
    load_game_font_with_mode(resolver, font_name, AtlasMode::Coverage)
}

pub fn load_game_font_for_language_from_resolver(
    resolver: &GameFileResolver,
    font_name: &str,
    language: &str,
) -> Result<Font> {
    let spec = game_font_spec(resolver, font_name, Some(language))?;
    Font::load_from_resolver_with_mode(resolver, &spec, AtlasMode::Coverage)
}

pub fn game_font_dependency_paths(
    resolver: &GameFileResolver,
    font_name: &str,
) -> Result<Vec<String>> {
    game_font_dependency_paths_for_language(resolver, font_name, None)
}

pub fn game_font_dependency_paths_for_language(
    resolver: &GameFileResolver,
    font_name: &str,
    language: Option<&str>,
) -> Result<Vec<String>> {
    let core_gfx_rel = "interface/core.gfx";
    let spec = game_font_spec(resolver, font_name, language)?;

    let mut paths = vec![core_gfx_rel.to_string()];
    for relative in &spec.files {
        let resolved_name = game_ui_font_name(relative)?;
        let descriptor = format!("gfx/fonts/{resolved_name}.fnt");
        let descriptor_path = resolver.resolve_required(&descriptor, "font file")?;
        let descriptor_bytes = resolver.read_required_bytes(&descriptor, "bitmap font")?;
        let parsed = parse_fnt_text(&descriptor_bytes, &descriptor)?;
        paths.push(descriptor);
        for (page_id, declared) in parsed.page_files.iter().enumerate() {
            let (page, _) = resolve_bitmap_font_page_path(
                resolver,
                &resolved_name,
                u32::try_from(page_id)?,
                declared.as_deref(),
            )?;
            paths.push(page);
        }
    }
    Ok(paths)
}

fn game_font_spec(
    resolver: &GameFileResolver,
    font_name: &str,
    language: Option<&str>,
) -> Result<FontSpec> {
    let core_gfx_rel = "interface/core.gfx";
    let core_gfx_path = resolver.resolve_required(core_gfx_rel, "core.gfx font registry")?;
    let core_bytes = resolver.read_required_bytes(core_gfx_rel, "bitmap font")?;
    reject_font_decode_damage(&core_bytes, core_gfx_rel)?;
    let core_text = decode_text(&core_bytes);
    let source = format!("{core_gfx_rel} ({})", core_gfx_path.display());
    match language {
        Some(language) => font_spec_from_core_gfx_for_language(&core_text, font_name, language, &source),
        None => font_spec_from_core_gfx(&core_text, font_name, &source),
    }
}

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
    let core_gfx_rel = "interface/core.gfx";
    let core_gfx_path = resolver.resolve_required(core_gfx_rel, "core.gfx font registry")?;
    let bytes = resolver.read_required_bytes(core_gfx_rel, "bitmap font").map_err(|err| {
        format!(
            "failed to read {core_gfx_rel} resolved to {}: {err}",
            core_gfx_path.display()
        )
    })?;
    reject_font_decode_damage(&bytes, &core_gfx_rel)?;
    let text = decode_text(&bytes);
    let spec = font_spec_from_core_gfx(
        &text,
        font_name,
        &format!("{core_gfx_rel} ({})", core_gfx_path.display()),
    )?;
    Font::load_from_resolver_with_mode(resolver, &spec, atlas_mode)
}

pub fn load_overlay_font(resolver: &GameFileResolver) -> Result<OverlayFontSelection> {
    let font = load_game_font_from_resolver(resolver, OVERLAY_FONT_NAME).map_err(|err| {
        format!(
            "failed to load real overlay font {OVERLAY_FONT_NAME:?} from {}: {err}",
            resolver.game_dir().display()
        )
    })?;
    Ok(OverlayFontSelection {
        font,
        font_name: OVERLAY_FONT_NAME,
        root: resolver.game_dir().to_path_buf(),
    })
}

pub fn font_spec_from_core_gfx(text: &str, font_name: &str, source: &str) -> Result<FontSpec> {
    let tokens = lex_pdx_tokens(text);
    let Some((start, end)) = find_assigned_block(&tokens, 0, tokens.len(), "bitmapfonts") else {
        return Err(format!("{source} has no bitmapfonts block").into());
    };
    let mut index = start;
    while index < end {
        if tokens[index].eq_ignore_ascii_case("bitmapfont") {
            if let Some((block_start, block_end)) =
                assigned_block_after_key(&tokens, index, end, "bitmapfont")
            {
                let block_tokens = &tokens[block_start..block_end];
                if font_block_name(block_tokens).as_deref() == Some(font_name) {
                    return font_spec_from_core_gfx_block(block_tokens, font_name, source);
                }
                index = block_end + 1;
                continue;
            }
        }
        index += 1;
    }
    Err(format!("font {font_name:?} is not defined as bitmapfont in {source}").into())
}

pub fn font_spec_from_core_gfx_for_language(
    text: &str,
    font_name: &str,
    language: &str,
    source: &str,
) -> Result<FontSpec> {
    let base = font_spec_from_core_gfx(text, font_name, source)?;
    let mut canonical = language.trim().to_ascii_lowercase();
    if !canonical.starts_with("l_") {
        canonical = format!("l_{canonical}");
    }
    let tokens = lex_pdx_tokens(text);
    let Some((start, end)) = find_assigned_block(&tokens, 0, tokens.len(), "bitmapfonts") else {
        return Err(format!("{source} has no bitmapfonts block").into());
    };
    let mut selected_files = None;
    let mut index = start;
    while index < end {
        if tokens[index].eq_ignore_ascii_case("bitmapfont_override") {
            if let Some((block_start, block_end)) =
                assigned_block_after_key(&tokens, index, end, "bitmapfont_override")
            {
                let block = &tokens[block_start..block_end];
                if font_block_name(block).as_deref() == Some(font_name)
                    && font_block_list(block, "languages")
                        .iter()
                        .any(|value| value.eq_ignore_ascii_case(&canonical))
                {
                    let files = font_block_files(block);
                    if files.is_empty() {
                        return Err(format!(
                            "font override {font_name:?} for {canonical} in {source} has no files"
                        )
                        .into());
                    }
                    if selected_files.replace(files).is_some() {
                        return Err(format!(
                            "font override {font_name:?} for {canonical} is ambiguous in {source}"
                        )
                        .into());
                    }
                }
                index = block_end + 1;
                continue;
            }
        }
        index += 1;
    }
    Ok(FontSpec {
        files: selected_files.unwrap_or(base.files),
        ..base
    })
}

fn font_block_list(tokens: &[String], key: &str) -> Vec<String> {
    let mut index = 0;
    while index + 2 < tokens.len() {
        if tokens[index].eq_ignore_ascii_case(key)
            && tokens[index + 1] == "="
            && tokens[index + 2] == "{"
        {
            if let Some((start, end)) = block_after_open_brace(tokens, index + 2, tokens.len()) {
                return tokens[start..end]
                    .iter()
                    .filter(|value| !matches!(value.as_str(), "=" | "{" | "}"))
                    .cloned()
                    .collect();
            }
        }
        index += 1;
    }
    Vec::new()
}

fn font_block_files(tokens: &[String]) -> Vec<String> {
    let mut index = 0;
    while index + 2 < tokens.len() {
        if tokens[index].eq_ignore_ascii_case("path") && tokens[index + 1] == "=" {
            return vec![normalize_rel_path(&tokens[index + 2])];
        }
        index += 1;
    }
    font_block_list(tokens, "fontfiles")
        .into_iter()
        .map(|value| normalize_rel_path(&value))
        .collect()
}

fn font_block_name(tokens: &[String]) -> Option<String> {
    let mut index = 0;
    while index + 2 < tokens.len() {
        if tokens[index].eq_ignore_ascii_case("name") && tokens[index + 1] == "=" {
            return Some(tokens[index + 2].clone());
        }
        index += 1;
    }
    None
}

fn font_spec_from_core_gfx_block(
    tokens: &[String],
    wanted_font_name: &str,
    source: &str,
) -> Result<FontSpec> {
    let mut name = None;
    let mut path = None;
    let mut fontfiles = Vec::new();
    let mut color = None;
    let mut index = 0;
    while index < tokens.len() {
        let key = tokens[index].as_str();
        if index + 2 >= tokens.len() || tokens[index + 1] != "=" {
            index += 1;
            continue;
        }
        match key.to_ascii_lowercase().as_str() {
            "name" => {
                name = Some(tokens[index + 2].clone());
                index += 3;
            }
            "path" => {
                path = Some(normalize_rel_path(&tokens[index + 2]));
                index += 3;
            }
            "color" => {
                color = Some(tokens[index + 2].clone());
                index += 3;
            }
            "fontfiles" if tokens[index + 2] == "{" => {
                let (list_start, list_end) =
                    block_after_open_brace(tokens, index + 2, tokens.len()).ok_or_else(|| {
                        format!(
                            "font {wanted_font_name:?} in {source}: unterminated fontfiles block"
                        )
                    })?;
                fontfiles.extend(
                    tokens[list_start..list_end]
                        .iter()
                        .filter(|token| token.as_str() != "=")
                        .filter(|token| token.as_str() != "{" && token.as_str() != "}")
                        .map(|token| normalize_rel_path(token)),
                );
                index = list_end + 1;
            }
            _ => index += 3,
        }
    }
    let name = name.ok_or_else(|| format!("bitmapfont in {source} has no name"))?;
    let files = match path {
        Some(path) => vec![path],
        None => fontfiles,
    };
    if files.is_empty() {
        return Err(format!("font {name:?} in {source} declares no font files").into());
    }
    let color_text =
        color.ok_or_else(|| format!("font {name:?} in {source} has no color string"))?;
    let color =
        parse_argb_color(&color_text).map_err(|err| format!("font {name:?} in {source}: {err}"))?;
    Ok(FontSpec { name, files, color })
}

fn assigned_block_after_key(
    tokens: &[String],
    key_index: usize,
    search_end: usize,
    key: &str,
) -> Option<(usize, usize)> {
    (tokens.get(key_index)?.eq_ignore_ascii_case(key)
        && tokens.get(key_index + 1)? == "="
        && tokens.get(key_index + 2)? == "{")
        .then(|| block_after_open_brace(tokens, key_index + 2, search_end))
        .flatten()
}

fn find_assigned_block(
    tokens: &[String],
    search_start: usize,
    search_end: usize,
    key: &str,
) -> Option<(usize, usize)> {
    let mut index = search_start;
    while index + 2 < search_end {
        if let Some(block) = assigned_block_after_key(tokens, index, search_end, key) {
            return Some(block);
        }
        index += 1;
    }
    None
}

fn block_after_open_brace(
    tokens: &[String],
    open_brace_index: usize,
    search_end: usize,
) -> Option<(usize, usize)> {
    if tokens.get(open_brace_index)? != "{" {
        return None;
    }
    let mut depth = 1usize;
    let mut index = open_brace_index + 1;
    while index < search_end {
        match tokens[index].as_str() {
            "{" => depth += 1,
            "}" => {
                depth = depth.checked_sub(1)?;
                if depth == 0 {
                    return Some((open_brace_index + 1, index));
                }
            }
            _ => {}
        }
        index += 1;
    }
    None
}

fn lex_pdx_tokens(text: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let mut current = String::new();
    let mut chars = text.chars().peekable();
    let mut in_quotes = false;
    while let Some(ch) = chars.next() {
        if in_quotes {
            match ch {
                '"' => {
                    tokens.push(std::mem::take(&mut current));
                    in_quotes = false;
                }
                '\\' => {
                    if let Some(next) = chars.next() {
                        current.push(next);
                    }
                }
                _ => current.push(ch),
            }
            continue;
        }
        match ch {
            '"' => {
                if !current.is_empty() {
                    tokens.push(std::mem::take(&mut current));
                }
                in_quotes = true;
            }
            '#' => {
                if !current.is_empty() {
                    tokens.push(std::mem::take(&mut current));
                }
                for next in chars.by_ref() {
                    if next == '\n' {
                        break;
                    }
                }
            }
            '=' | '{' | '}' => {
                if !current.is_empty() {
                    tokens.push(std::mem::take(&mut current));
                }
                tokens.push(ch.to_string());
            }
            c if c.is_whitespace() => {
                if !current.is_empty() {
                    tokens.push(std::mem::take(&mut current));
                }
            }
            _ => current.push(ch),
        }
    }
    if !current.is_empty() {
        tokens.push(current);
    }
    tokens
}

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
    #[cfg(test)]
    pub fn load(game_root: &Path, spec: &FontSpec) -> Result<Font> {
        let resolver = GameFileResolver::vanilla(game_root.to_path_buf())?;
        Self::load_from_resolver(&resolver, spec)
    }

    #[cfg(test)]
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
            line_height: line_height.ok_or("font has no .fnt files")?,
            base: base.unwrap_or(0),
            default_color: spec.color,
            glyphs,
            kernings,
            pages,
        })
    }

    #[cfg(test)]
    pub fn glyph_count(&self) -> usize {
        self.glyphs.len()
    }

    #[cfg_attr(not(test), allow(dead_code))]
    pub fn kerning_count(&self) -> usize {
        self.kernings.len()
    }

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

    pub fn label_text_metrics(&self, text: &str) -> Result<LabelTextMetrics> {
        let mut advances = Vec::with_capacity(text.chars().count());
        let mut straight_layout_stations = Vec::with_capacity(text.chars().count());
        let mut missing = Vec::new();
        let mut pen = 0.0f32;
        let mut previous: Option<u32> = None;
        for (slot_index, ch) in text.chars().enumerate() {
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
                straight_layout_stations.push(MapLabelStraightLayoutStation {
                    slot_index,
                    x: pen + glyph.xoffset as f32 + glyph.width as f32 * 0.5,
                    y: glyph.yoffset as f32,
                });
            }
            advances.push((glyph.xadvance as f32, ch == ' '));
            pen += glyph.xadvance as f32;
            previous = Some(id);
        }
        if !missing.is_empty() {
            return Err(format!(
                "font {} is missing glyphs needed for map label {:?}: {:?}",
                self.name, text, missing
            )
            .into());
        }
        let space_advance_px = self
            .glyphs
            .get(&(' ' as u32))
            .map(|glyph| glyph.xadvance as f32)
            .ok_or_else(|| {
                format!(
                    "font {} has no space glyph for map label tracking",
                    self.name
                )
            })?;
        Ok(LabelTextMetrics {
            advances,
            line_height_px: self.line_height as f32,
            space_advance_px,
            straight_layout_stations,
        })
    }

    pub fn layout_text(&self, origin_x: f32, origin_y: f32, text: &str) -> TextLayout {
        self.layout_text_tracked(origin_x, origin_y, text, 0.0)
    }

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

fn atlas_rgba(image: &RgbaImage, glyph_channel: Option<usize>, mode: AtlasMode) -> Vec<u8> {
    match mode {
        AtlasMode::Coverage => coverage_atlas_rgba(image, glyph_channel),
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
    let bytes = resolver.read_required_bytes(&rel, "bitmap font").map_err(|err| {
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
    let (rel, path) = resolve_bitmap_font_page_path(resolver, name, page_id, declared)?;
    decode_resolved_font_page(resolver, page_id, rel, path)
}

fn resolve_bitmap_font_page_path(
    resolver: &GameFileResolver,
    name: &str,
    page_id: u32,
    _declared: Option<&str>,
) -> Result<(String, PathBuf)> {
    if page_id != 0 {
        return Err(format!("font {name} has no engine atlas page {page_id}").into());
    }
    for rel in portable_content_manifest::bitmap_font_texture_candidates(&format!("gfx/fonts/{name}")) {
        if resolver.has_content_admission() && resolver.admitted_used_content_key(&rel)?.is_none() {
            continue;
        }
        if let Some(path) = resolver.resolve(&rel)? {
            return Ok((rel, path));
        }
    }
    Err(format!("no engine stem texture found for font {name}").into())
}

fn decode_resolved_font_page(
    resolver: &GameFileResolver,
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
            let bytes = resolver.read_required_bytes(&rel_path, "font page").map_err(|err| {
                format!(
                    "failed to read font page {rel_path} resolved to {}: {err}",
                    path.display()
                )
            })?;
            match dds::decode_mip_chain_bytes(&bytes) {
                Ok(mut levels) if !levels.is_empty() => {
                    let image = levels.remove(0);
                    (image, levels)
                }
                Ok(_) => {
                    return Err(format!(
                        "font page {rel_path} resolved to {} decoded to an empty mip chain",
                        path.display()
                    )
                    .into())
                }
                Err(err) if matches!(err.kind, ErrorKind::UnsupportedDdsFormat(_)) => {
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
                Err(err) => {
                    return Err(format!(
                        "failed to decode the shipped mip chain of font page {rel_path} \
                         resolved to {}: {err}",
                        path.display()
                    )
                    .into())
                }
            }
        }
        "tga" => {
            let bytes = resolver.read_required_bytes(&rel_path, "font page")?;
            let image = tga::decode_bytes(&bytes)
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

#[allow(dead_code)]
#[derive(Debug)]
struct ParsedFnt {
    line_height: i32,
    base: i32,
    scale_w: u32,
    scale_h: u32,
    glyph_channel: Option<usize>,
    page_files: Vec<Option<String>>,
    glyphs: Vec<(u32, Glyph)>,
    kernings: Vec<(u32, u32, i32)>,
}

fn parse_fnt_text(bytes: &[u8], source: &str) -> Result<ParsedFnt> {
    if bytes.starts_with(b"BMF") {
        return Err(format!(
            "{source} is a binary BMFont file; the game ships the text format and \
             binary decoding is not implemented (fail closed, no substitute)"
        )
        .into());
    }
    reject_font_decode_damage(bytes, source)?;
    let text = decode_text(bytes);

    let mut common: Option<(i32, i32, u32, u32, usize, Option<usize>)> = None;
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
                    required_unsigned(&fields, "lineHeight", source, MAX_GLYPH_EXTENT)? as i32,
                    signed_i32(
                        optional_int(&fields, "base").unwrap_or(0),
                        "base",
                        source,
                    )?,
                    required_unsigned(&fields, "scaleW", source, MAX_GLYPH_EXTENT)? as u32,
                    required_unsigned(&fields, "scaleH", source, MAX_GLYPH_EXTENT)? as u32,
                    1,
                    bmfont_glyph_channel_from_common(&fields),
                ));
            }
            "char" => {
                let id = required_int(&fields, "id", source)?;
                if id < 0 {
                    continue;
                }
                let id = u32::try_from(id)
                    .map_err(|_| format!("{source}: char id={id} does not fit in u32"))?;
                glyphs.push((
                    id,
                    Glyph {
                        x: required_unsigned(&fields, "x", source, MAX_GLYPH_EXTENT)? as u32,
                        y: required_unsigned(&fields, "y", source, MAX_GLYPH_EXTENT)? as u32,
                        width: required_unsigned(&fields, "width", source, MAX_GLYPH_EXTENT)?
                            as u32,
                        height: required_unsigned(&fields, "height", source, MAX_GLYPH_EXTENT)?
                            as u32,
                        xoffset: signed_i32(
                            optional_int(&fields, "xoffset").unwrap_or(0),
                            "xoffset",
                            source,
                        )?,
                        yoffset: signed_i32(
                            optional_int(&fields, "yoffset").unwrap_or(0),
                            "yoffset",
                            source,
                        )?,
                        xadvance: signed_i32(
                            required_int(&fields, "xadvance", source)?,
                            "xadvance",
                            source,
                        )?,
                        page: 0,
                    },
                ));
            }
            "kerning" => {
                let first = required_int(&fields, "first", source)?;
                let second = required_int(&fields, "second", source)?;
                let amount = required_int(&fields, "amount", source)?;
                kernings.push((
                    u32::try_from(first)
                        .map_err(|_| format!("{source}: kerning first={first} is not a u32"))?,
                    u32::try_from(second)
                        .map_err(|_| format!("{source}: kerning second={second} is not a u32"))?,
                    signed_i32(amount, "kerning amount", source)?,
                ));
            }
            _ => {}
        }
    }

    let (line_height, base, scale_w, scale_h, _page_count, glyph_channel) =
        common.ok_or_else(|| format!("{source} has no \"common\" line; not a BMFont text file"))?;
    if !saw_info {
        return Err(format!("{source} has no \"info\" line; not a BMFont text file").into());
    }
    Ok(ParsedFnt {
        line_height,
        base,
        scale_w,
        scale_h,
        glyph_channel,
        page_files: vec![None],
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

/// One-axis bound on a glyph rect or atlas extent, mirroring
/// `game_ui::font::MAX_GLYPH_EXTENT` (and `dds::DDS_MAX_PIXELS` as a single
/// axis). Keeping both `x` and `width` under 2^26 also makes `x + width` at
/// [`Font::load_from_resolver_with_mode`] provably non-overflowing in `u32`.
const MAX_GLYPH_EXTENT: i64 = 1 << 26;
/// Page count ceiling: the AngelCode BINARY common block stores `pages` as a
/// `u16`, so this is the format's own field width, not an invented limit.

fn required_unsigned(
    fields: &HashMap<String, String>,
    key: &str,
    source: &str,
    max: i64,
) -> Result<i64> {
    let value = required_int(fields, key, source)?;
    if value < 0 || value > max {
        return Err(format!("{source}: {key}={value} is outside the accepted range 0..={max}").into());
    }
    Ok(value)
}

/// As [`required_unsigned`], for a field that is legitimately signed
/// (`xoffset`, `yoffset`, `xadvance`, kerning `amount`): only the cast to
/// `i32` needs bounding.
fn signed_i32(value: i64, key: &str, source: &str) -> Result<i32> {
    i32::try_from(value)
        .map_err(|_| format!("{source}: {key}={value} does not fit in i32").into())
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
#[cfg(test)]
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
    // Same cap as `game_ui::tga::TGA_MAX_PIXELS` / `dds::DDS_MAX_PIXELS`: the
    // `vec![0u8; pixel_count * 4]` below is header-derived, and 65535x65535
    // asks for 17 GB. This decoder is test-only, but it is a sibling of the
    // production one and must not model a different allocation policy.
    const MAX_PIXELS: u64 = 1 << 26;
    if u64::from(width) * u64::from(height) > MAX_PIXELS {
        return Err(format!(
            "{source}: TGA dimensions {width}x{height} exceed the {MAX_PIXELS}-pixel cap"
        )
        .into());
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

    #[test]
    fn bitmap_font_rendering_uses_the_engine_stem_despite_page_metadata() {
        let root = crate::bundle::tests::unique_temp_dir("font-engine-stem");
        let text = CG_STYLE_FNT.replace("pages=1", "pages=2").replace("page=0", "page=1")
            + "\npage id=1 file=\"other.tga\"\n";
        write_font(&root, "probe", &text, [10, 20, 30, 255]);
        write_font_tga(&root.join("gfx/fonts/declared_page.tga"), [90, 80, 70, 255]);
        use portable_content_manifest::*;
        let bytes = fs::read(root.join("gfx/fonts/probe.tga")).unwrap();
        let hash = snapshot::sha256::Sha256::hex(&bytes);
        let required = UsedContentKey {
            effective_vfs_manifest_sha256: "11".repeat(32),
            stored_game_relative_path: "gfx/fonts/probe.tga".into(),
            normalized_game_relative_path: "gfx/fonts/probe.tga".into(),
            winning_layer_ordinal: 0, layer_kind: ContentLayerKind::Base, layer_id: "base".into(),
            source_sha256: hash.clone(), payload_sha256: hash, payload_bytes: bytes.len() as u64,
        };
        let resolver = admitted_font_resolver(&root, required);
        assert!(resolver.has_content_admission());
        assert!(resolver.resolve("gfx/fonts/probe.dds").is_err());
        let parsed = parse_fnt_text(text.as_bytes(), "probe.fnt").unwrap();
        assert_eq!(parsed.page_files, vec![None]);
        assert_eq!(parsed.glyphs[0].1.page, 0);
        let (path, _) = resolve_bitmap_font_page_path(&resolver, "probe", 0, Some("declared_page.tga")).unwrap();
        assert_eq!(path, "gfx/fonts/probe.tga");
        let pages = load_bitmap_font_pages_from_resolver(&resolver, "probe", &parsed).unwrap();
        assert_eq!(pages.len(), 1);
        assert_eq!(&pages[0].image.pixels[..4], &[10, 20, 30, 255]);
        fs::remove_dir_all(root).unwrap();
    }

    fn admitted_font_resolver(
        root: &std::path::Path,
        required: portable_content_manifest::UsedContentKey,
    ) -> GameFileResolver {
        use portable_content_manifest::*;
        use std::collections::BTreeMap;
        fs::create_dir_all(root).unwrap();
        let mut manifest = ContentManifest {
            schema: SCHEMA.into(),
            product_modset_sha256: "09".repeat(32),
            source_game: SourceGameIdentity {
                version: "1.19.2".into(),
                build: "a729".into(),
                executable_sha256:
                    "870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad".into(),
                build_profile: "release".into(),
            },
            source_profile: SourceProfileIdentity {
                profile_id: "font-atlas-test".into(),
                playset_id: "font-atlas-test".into(),
                selection_sha256: String::new(),
            },
            effective_vfs_manifest_sha256: "11".repeat(32),
            ordered_layers: vec![RecordedContentLayer {
                ordinal: 0,
                layer_kind: ContentLayerKind::Base,
                layer_id: "base".into(),
                display_name: "the game".into(),
                descriptor_sha256: None,
                source_container_sha256: None,
                storage_kind: LayerStorageKind::Directory,
                replace_paths: Vec::new(),
                required_for_playback: true,
            }],
            dlc_selection: Vec::new(),
            localisation_language: "l_english".into(),
            required_settings: BTreeMap::new(),
            recorder_local_settings: BTreeMap::new(),
            required_content: vec![RequiredContent {
                content_key: "font-atlas".into(),
                kind: RequiredContentKind::Other,
                content: required.clone(),
                observation_count: 1,
                consumers: vec!["font-atlas-test".into()],
            }],
            witnessed_absent_content: Vec::new(),
            manifest_digest_sha256: None,
        };
        manifest.source_profile.selection_sha256 = manifest.computed_selection_sha256().unwrap();
        let manifest = manifest.with_computed_digest().unwrap();
        let inventory = PlaybackContentInventory {
            source_game: manifest.source_game.clone(),
            source_profile: manifest.source_profile.clone(),
            layers: vec![PlaybackContentLayer {
                recorded_ordinal: 0,
                layer_kind: ContentLayerKind::Base,
                layer_id: "base".into(),
                descriptor_sha256: None,
                source_container_sha256: None,
                storage_kind: LayerStorageKind::Directory,
                replace_paths: Vec::new(),
            }],
            dlc_selection: Vec::new(),
            localisation_language: "l_english".into(),
            settings: BTreeMap::new(),
            resolved_content: vec![ResolvedPlaybackContent {
                stored_game_relative_path: required.stored_game_relative_path.clone(),
                normalized_game_relative_path: required.normalized_game_relative_path.clone(),
                winning_layer_ordinal: required.winning_layer_ordinal,
                layer_kind: required.layer_kind,
                layer_id: required.layer_id.clone(),
                source_sha256: required.source_sha256.clone(),
                payload_sha256: required.payload_sha256.clone(),
                payload_bytes: required.payload_bytes,
            }],
        };
        let admission = manifest.admit_playback(&inventory).unwrap();
        let resolver = GameFileResolver::vanilla(root)
            .unwrap()
            .with_content_admission(&admission)
            .unwrap();
        resolver
    }

    const GARAMOND_STYLE_FNT: &str = "\
info face=\"Test Face\" size=16 bold=0 italic=0 charset=\"ANSI\" stretchH=100 smooth=0 aa=1 padding=2,2,2,2 spacing=1,1
common lineHeight=16 base=12 scaleW=8 scaleH=8 pages=1
char id=32   x=0     y=0     width=1     height=0     xoffset=0     yoffset=16    xadvance=4     page=0
char id=65   x=0     y=0     width=4     height=6     xoffset=1     yoffset=2     xadvance=5     page=0
char id=86   x=4     y=0     width=3     height=6     xoffset=0     yoffset=2     xadvance=4     page=0
kerning first=65  second=86 amount=-2
";

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

    fn test_pixels() -> [[u8; 4]; 4] {
        [
            [255, 0, 0, 255],
            [0, 255, 0, 128],
            [0, 0, 255, 255],
            [255, 255, 0, 64],
        ]
    }

    fn tga_header(image_type: u8, bpp: u8, descriptor: u8) -> Vec<u8> {
        let mut header = vec![0u8; 18];
        header[2] = image_type;
        header[12] = 2;
        header[14] = 2;
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
            vec![None]
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
        let mut bytes = tga_header(2, 32, 0x08);
        for [r, g, b, a] in [pixels[2], pixels[3], pixels[0], pixels[1]] {
            bytes.extend_from_slice(&[b, g, r, a]);
        }
        let page = decode_tga(&bytes, "test.tga").expect("decodes");
        assert_eq!((page.width, page.height), (2, 2));
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
        bytes.push(0x81);
        bytes.extend_from_slice(&[pixels[0][2], pixels[0][1], pixels[0][0], pixels[0][3]]);
        bytes.push(0x01);
        for [r, g, b, a] in [pixels[2], pixels[3]] {
            bytes.extend_from_slice(&[b, g, r, a]);
        }
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
        bytes.extend_from_slice(&[0u8; 7]);
        let err = decode_tga(&bytes, "short.tga").expect_err("must fail");
        assert!(err.to_string().contains("expected 16"), "{err}");
    }

    #[test]
    fn tga_rejects_truncated_rle_stream() {
        let mut bytes = tga_header(10, 32, 0x20);
        bytes.push(0x83);
        let err = decode_tga(&bytes, "short_rle.tga").expect_err("must fail");
        assert!(err.to_string().contains("truncated"), "{err}");
    }

    fn precolored_page() -> RgbaImage {
        RgbaImage {
            width: 2,
            height: 1,
            pixels: vec![10, 10, 10, 200, 140, 140, 140, 0],
        }
    }

    #[test]
    fn preserved_color_mode_passes_atlas_rgba_through() {
        let page = precolored_page();
        let out = atlas_rgba(&page, Some(0), AtlasMode::PreservedColor);
        assert_eq!(out, vec![10, 10, 10, 200, 140, 140, 140, 0]);
    }

    #[test]
    fn coverage_mode_is_unchanged_for_the_overlay_path() {
        let page = precolored_page();
        let out = atlas_rgba(&page, Some(0), AtlasMode::Coverage);
        assert_eq!(out, vec![0xFF, 0xFF, 0xFF, 245, 0xFF, 0xFF, 0xFF, 115]);
    }

    const CORE_GFX_FIXTURE: &str = r#"
        bitmapfonts = {
            bitmapfont = {
                name = "PathFont"
                path = "gfx/fonts/one"
                color = 0xffffffff
            }
            # Commented-out font declarations must not leak into the registry.
            # bitmapfont = { name = "CommentFont" path = "gfx/fonts/commented" color = 0xffffffff }
            bitmapfont = {
                name = "ListFont"
                fontfiles = {
                    "gfx/fonts/a"
                    "gfx/fonts/b"
                }
                color = 0xff7799ff
            }
            bitmapfont = {
                name = "BadColor"
                path = "gfx/fonts/one"
                color = 0x4ffffffff
            }
            bitmapfont = {
                name = "NoFiles"
                color = 0xffffffff
            }
        }
    "#;

    #[test]
    fn core_gfx_font_spec_resolves_single_path_fonts() {
        let spec = font_spec_from_core_gfx(CORE_GFX_FIXTURE, "PathFont", "core.gfx").expect("spec");
        assert_eq!(spec.files, vec!["gfx/fonts/one".to_string()]);
        assert_eq!(spec.color, 0xFFFF_FFFF);
    }

    #[test]
    fn core_gfx_font_spec_resolves_fontfiles_lists() {
        let spec = font_spec_from_core_gfx(CORE_GFX_FIXTURE, "ListFont", "core.gfx").expect("spec");
        assert_eq!(
            spec.files,
            vec!["gfx/fonts/a".to_string(), "gfx/fonts/b".to_string()]
        );
        assert_eq!(spec.color, 0xFF77_99FF);
    }

    #[test]
    fn core_gfx_language_override_selects_files_and_inherits_base_color() {
        let source = r#"
            bitmapfonts = {
                bitmapfont = {
                    name = "hoi_16mbs"
                    fontfiles = { "gfx/fonts/hoi_16mbs" "gfx/fonts/hoi_16mbs_cryllic" }
                    color = 0xff7799ff
                }
                bitmapfont_override = {
                    name = "hoi_16mbs"
                    fontfiles = { "gfx/fonts/japanese/hoi_16mbs" }
                    languages = { "l_japanese" }
                }
            }
        "#;
        let japanese = font_spec_from_core_gfx_for_language(
            source,
            "hoi_16mbs",
            "japanese",
            "core.gfx",
        )
        .expect("Japanese override");
        assert_eq!(
            japanese.files,
            vec!["gfx/fonts/japanese/hoi_16mbs".to_string()]
        );
        assert_eq!(japanese.color, 0xFF77_99FF);

        let german = font_spec_from_core_gfx_for_language(
            source,
            "hoi_16mbs",
            "l_german",
            "core.gfx",
        )
        .expect("base fallback");
        assert_eq!(
            german.files,
            vec![
                "gfx/fonts/hoi_16mbs".to_string(),
                "gfx/fonts/hoi_16mbs_cryllic".to_string(),
            ]
        );
        assert_eq!(german.color, 0xFF77_99FF);
    }

    #[test]
    fn core_gfx_duplicate_language_overrides_fail_closed() {
        let source = r#"
            bitmapfonts = {
                bitmapfont = { name="f" path="gfx/fonts/base" color=0xffffffff }
                bitmapfont_override = { name="f" path="gfx/fonts/a" languages={ "l_japanese" } }
                bitmapfont_override = { name="f" path="gfx/fonts/b" languages={ "l_japanese" } }
            }
        "#;
        let error = font_spec_from_core_gfx_for_language(
            source,
            "f",
            "l_japanese",
            "core.gfx",
        )
        .expect_err("ambiguous override");
        assert!(error.to_string().contains("ambiguous"), "{error}");
    }

    #[test]
    fn game_font_dependencies_follow_the_core_alias_and_loader_order() {
        let root = crate::bundle::tests::unique_temp_dir("font_dependency_paths");
        let interface = root.join("interface");
        fs::create_dir_all(&interface).expect("create interface dir");
        fs::write(interface.join("core.gfx"), CORE_GFX_FIXTURE).expect("write core.gfx");
        write_font(&root, "a", GARAMOND_STYLE_FNT, [255, 255, 255, 255]);
        write_font(&root, "b", GARAMOND_STYLE_FNT, [255, 255, 255, 255]);
        write_font(
            &root,
            "ListFont",
            GARAMOND_STYLE_FNT,
            [255, 255, 255, 255],
        );
        let resolver = GameFileResolver::vanilla(root.clone()).expect("resolver");

        assert_eq!(
            game_font_dependency_paths(&resolver, "ListFont").expect("dependency closure"),
            vec![
                "interface/core.gfx",
                "gfx/fonts/a.fnt",
                "gfx/fonts/a.tga",
                "gfx/fonts/b.fnt",
                "gfx/fonts/b.tga",
            ]
        );
        fs::remove_dir_all(root).ok();
    }

    #[test]
    fn core_gfx_font_spec_unknown_name_fails_closed() {
        let err = font_spec_from_core_gfx(CORE_GFX_FIXTURE, "Nope", "core.gfx").expect_err("fails");
        assert!(err.to_string().contains("\"Nope\""), "{err}");
    }

    #[test]
    fn core_gfx_font_spec_malformed_color_fails_closed() {
        let err =
            font_spec_from_core_gfx(CORE_GFX_FIXTURE, "BadColor", "core.gfx").expect_err("fails");
        assert!(err.to_string().contains("8 hex digits"), "{err}");
    }

    #[test]
    fn core_gfx_font_spec_without_files_fails_closed() {
        let err =
            font_spec_from_core_gfx(CORE_GFX_FIXTURE, "NoFiles", "core.gfx").expect_err("fails");
        assert!(err.to_string().contains("no font files"), "{err}");
    }

    #[test]
    fn core_gfx_comments_do_not_create_fonts() {
        let err = font_spec_from_core_gfx(CORE_GFX_FIXTURE, "CommentFont", "core.gfx")
            .expect_err("commented font must not parse");
        assert!(err.to_string().contains("CommentFont"), "{err}");
    }

    #[test]
    fn measure_applies_advances_and_kerning() {
        let parsed = parse_fnt_text(GARAMOND_STYLE_FNT.as_bytes(), "test.fnt").expect("parses");
        let font = font_from(parsed, 0xFFFF_FFFF);
        assert_eq!(font.measure("AV"), (7, 16));
        assert_eq!(font.measure("A V"), (5 + 4 + 4, 16));
        assert_eq!(font.measure(""), (0, 16));
    }

    #[test]
    fn layout_skips_and_records_missing_glyphs() {
        let parsed = parse_fnt_text(GARAMOND_STYLE_FNT.as_bytes(), "test.fnt").expect("parses");
        let font = font_from(parsed, 0xFFFF_FFFF);
        let layout = font.layout_text(0.0, 0.0, "AZV");
        assert_eq!(layout.missing, vec!['Z']);
        assert_eq!(layout.quads.len(), 2);
        assert_eq!(layout.advance, 9.0);
    }

    #[test]
    fn cap_height_is_the_modal_capital_glyph_height() {
        let parsed = parse_fnt_text(GARAMOND_STYLE_FNT.as_bytes(), "test.fnt").expect("parses");
        let font = font_from(parsed, 0xFFFF_FFFF);
        assert_eq!(font.cap_height(), 6);
    }

    #[test]
    fn tracked_layout_adds_uniform_advance_per_character_slot() {
        let parsed = parse_fnt_text(GARAMOND_STYLE_FNT.as_bytes(), "test.fnt").expect("parses");
        let font = font_from(parsed, 0xFFFF_FFFF);
        let layout = font.layout_text_tracked(0.0, 0.0, "A AV", 10.0);
        assert_eq!(layout.quads.len(), 3);
        let a2 = &layout.quads[1];
        assert_eq!(a2.x0, 30.0);
        let v = &layout.quads[2];
        assert_eq!(v.x0, 42.0);
        assert_eq!(layout.advance, 5.0 + 4.0 + 5.0 + 4.0 + 4.0 * 10.0 - 2.0);
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
        assert_eq!((v.x0, v.y0), (13.0, 22.0));
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

    fn a8r8g8b8_dds(width: u32, height: u32, declared_mips: u32, levels: u32) -> Vec<u8> {
        let mut data = vec![0u8; 128];
        data[0..4].copy_from_slice(b"DDS ");
        data[4..8].copy_from_slice(&124u32.to_le_bytes());
        data[12..16].copy_from_slice(&height.to_le_bytes());
        data[16..20].copy_from_slice(&width.to_le_bytes());
        data[28..32].copy_from_slice(&declared_mips.to_le_bytes());
        data[76..80].copy_from_slice(&32u32.to_le_bytes());
        data[80..84].copy_from_slice(&0x40u32.to_le_bytes());
        data[88..92].copy_from_slice(&32u32.to_le_bytes());
        data[92..96].copy_from_slice(&0x00ff_0000u32.to_le_bytes());
        data[96..100].copy_from_slice(&0x0000_ff00u32.to_le_bytes());
        data[100..104].copy_from_slice(&0x0000_00ffu32.to_le_bytes());
        data[104..108].copy_from_slice(&0xff00_0000u32.to_le_bytes());
        let (mut w, mut h) = (width, height);
        for _ in 0..levels {
            data.extend(std::iter::repeat(0xFFu8).take((w * h * 4) as usize));
            w = (w / 2).max(1);
            h = (h / 2).max(1);
        }
        data
    }

    #[test]
    fn negative_bmfont_dimensions_are_typed_refusals() {
        // under the workspace's release overflow-checks.
        let head = "info face=\"T\"\n";
        let cases = [
            (
                format!("{head}common lineHeight=-1 base=0 scaleW=1 scaleH=1 pages=1\n"),
                "lineHeight=-1",
            ),
            (
                format!("{head}common lineHeight=1 base=0 scaleW=-1 scaleH=1 pages=1\n"),
                "scaleW=-1",
            ),
        ];
        for (source, expected) in cases {
            let err = parse_fnt_text(source.as_bytes(), "probe.fnt")
                .expect_err("a negative unsigned field must be refused");
            let message = err.to_string();
            assert!(message.contains(expected), "{expected}: {message}");
            assert!(message.contains("outside"), "{expected}: {message}");
        }
    }

    #[test]
    fn negative_glyph_rect_components_are_typed_refusals() {
        for field in ["x", "y", "width", "height"] {
            let row =
                "char id=65 x=2 y=2 width=2 height=2 xoffset=0 yoffset=0 xadvance=1 page=0\n"
                    .replace(&format!("{field}=2 "), &format!("{field}=-1 "));
            let source = format!(
                "info face=\"T\"\n\
                 common lineHeight=1 base=0 scaleW=8 scaleH=8 pages=1\n{row}"
            );
            let err = parse_fnt_text(source.as_bytes(), "probe.fnt")
                .expect_err("a negative glyph rect component must be refused");
            let message = err.to_string();
            assert!(message.contains(&format!("{field}=-1")), "{field}: {message}");
        }
    }

    #[test]
    fn a_truncated_mip_chain_is_an_error_not_a_silent_mip_zero_degrade() {
        let dir = crate::bundle::tests::unique_temp_dir("font_mip_truncated");
        let fonts_dir = dir.join("gfx").join("fonts");
        fs::create_dir_all(&fonts_dir).expect("create fonts dir");
        fs::write(fonts_dir.join("mipfont.fnt"), GARAMOND_STYLE_FNT).expect("write fnt");
        fs::write(
            fonts_dir.join("mipfont.dds"),
            a8r8g8b8_dds(8, 8, 2, 1),
        )
        .expect("write dds");
        let spec = FontSpec {
            name: "mipfont".to_string(),
            files: vec!["gfx/fonts/mipfont".to_string()],
            color: 0xFFFF_FFFF,
        };
        let err = Font::load(&dir, &spec).expect_err("a truncated mip chain must fail closed");
        let message = err.to_string();
        assert!(message.contains("shipped mip chain"), "{message}");
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_complete_mip_chain_is_uploaded_and_a_compressed_page_still_falls_back() {
        let dir = crate::bundle::tests::unique_temp_dir("font_mip_complete");
        let fonts_dir = dir.join("gfx").join("fonts");
        fs::create_dir_all(&fonts_dir).expect("create fonts dir");
        fs::write(fonts_dir.join("mipfont.fnt"), GARAMOND_STYLE_FNT).expect("write fnt");
        fs::write(fonts_dir.join("mipfont.dds"), a8r8g8b8_dds(8, 8, 2, 2)).expect("write dds");
        let spec = FontSpec {
            name: "mipfont".to_string(),
            files: vec!["gfx/fonts/mipfont".to_string()],
            color: 0xFFFF_FFFF,
        };
        let font = Font::load(&dir, &spec).expect("complete chain loads");
        assert_eq!(font.pages[0].mips.len(), 1);
        assert_eq!((font.pages[0].mips[0].0, font.pages[0].mips[0].1), (4, 4));
        fs::remove_dir_all(&dir).ok();

        let dir = crate::bundle::tests::unique_temp_dir("font_mip_dxt1");
        let fonts_dir = dir.join("gfx").join("fonts");
        fs::create_dir_all(&fonts_dir).expect("create fonts dir");
        fs::write(fonts_dir.join("dxtfont.fnt"), GARAMOND_STYLE_FNT).expect("write fnt");
        let mut dxt = vec![0u8; 128];
        dxt[0..4].copy_from_slice(b"DDS ");
        dxt[4..8].copy_from_slice(&124u32.to_le_bytes());
        dxt[12..16].copy_from_slice(&8u32.to_le_bytes());
        dxt[16..20].copy_from_slice(&8u32.to_le_bytes());
        dxt[28..32].copy_from_slice(&1u32.to_le_bytes());
        dxt[76..80].copy_from_slice(&32u32.to_le_bytes());
        dxt[80..84].copy_from_slice(&0x4u32.to_le_bytes());
        dxt[84..88].copy_from_slice(b"DXT1");
        dxt.extend(std::iter::repeat(0xFFu8).take(2 * 2 * 8));
        fs::write(fonts_dir.join("dxtfont.dds"), &dxt).expect("write dds");
        let spec = FontSpec {
            name: "dxtfont".to_string(),
            files: vec!["gfx/fonts/dxtfont".to_string()],
            color: 0xFFFF_FFFF,
        };
        let font = Font::load(&dir, &spec).expect("compressed page still loads from mip 0");
        assert!(font.pages[0].mips.is_empty());
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn load_falls_back_to_stem_tga_when_declared_page_is_missing() {
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
    #[cfg_attr(
        not(gate_hoi4_install),
        ignore = "the game install (set BATTLE_PLAN_GAME_ROOT / BATTLE_PLAN_GAME_DIR / BATTLE_PLAN_VANILLA, or install at the default Steam path)"
    )]
    fn hoi_18mbs_loads_from_real_install_when_present() {
        let root = real_install_root();
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
    #[cfg_attr(
        not(gate_hoi4_install),
        ignore = "the game install (set BATTLE_PLAN_GAME_ROOT / BATTLE_PLAN_GAME_DIR / BATTLE_PLAN_VANILLA, or install at the default Steam path)"
    )]
    fn overlay_font_uses_real_hoi18mbs_when_resolver_exists() {
        let root = real_install_root();
        let resolver = GameFileResolver::vanilla(root).expect("resolver");
        let selection = load_overlay_font(&resolver).expect("real overlay font loads");
        assert_eq!(selection.font_name, OVERLAY_FONT_NAME);
        assert_eq!(selection.font.name, OVERLAY_FONT_NAME);
        assert!(selection.font.glyph_count() > 0);
        assert!(!selection.font.pages.is_empty());
    }

    fn real_install_root() -> PathBuf {
        let root = replay_test_gates::require_vanilla_game_root();
        replay_test_gates::require_existing_file(
            root.join("gfx").join("fonts").join("hoi_18mbs.fnt"),
        );
        root
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
