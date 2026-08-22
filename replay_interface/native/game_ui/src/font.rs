use crate::assets::{normalize_rel_path, AssetResolver};
use crate::dds::RgbaImage;
use crate::paradox_text::{destructive_event_count, diagnose_text_decode, parse_file};
use crate::{GameUiError, Result};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Component, Path};

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
    pub color_table: Option<crate::markup::TextColorTable>,
    pub authored_base_color_rgba8: Option<[u8; 4]>,
    pub authored_base_border_color_rgba8: Option<[u8; 4]>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FontPage {
    pub id: u32,
    pub file: String,
    pub image: RgbaImage,
}

const MAX_GLYPH_EXTENT: usize = 1 << 26;

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
        let direct_rel = format!("gfx/fonts/{name}.fnt");
        let resolved = match resolver.resolve_file(&direct_rel) {
            Ok(resolved) => resolved,
            Err(direct_error) => {
                let Some(alias_rel) = bitmap_font_alias_path(resolver, name)? else {
                    return Err(direct_error);
                };
                resolver.resolve_file(&alias_rel)?
            }
        };
        let bytes = fs::read(&resolved.path)?;
        let page_stem = std::path::Path::new(&resolved.rel_path)
            .file_stem()
            .and_then(|value| value.to_str())
            .unwrap_or(name);
        reject_descriptor_decode_damage(&resolved.rel_path, &bytes)?;
        let parsed = parse_bmfont_bytes(page_stem, &resolved.rel_path, &bytes)?;
        let mut font = parsed.load_pages(resolver)?;
        font.name = name.to_string();
        Ok(font)
    }

    pub fn load_from_descriptor(
        resolver: &AssetResolver,
        font_identity: &str,
        descriptor_path: &str,
    ) -> Result<Self> {
        let resolved = resolver.resolve_file(descriptor_path)?;
        let bytes = fs::read(&resolved.path)?;
        let page_stem = Path::new(&resolved.rel_path)
            .file_stem()
            .and_then(|value| value.to_str())
            .unwrap_or(font_identity);
        reject_descriptor_decode_damage(&resolved.rel_path, &bytes)?;
        let parsed = parse_bmfont_bytes(page_stem, &resolved.rel_path, &bytes)?;
        let mut font = parsed.load_pages(resolver)?;
        font.name = font_identity.to_string();
        Ok(font)
    }

    pub fn load_from_authored_binding(
        resolver: &AssetResolver,
        binding: &crate::authored_role_index::AuthoredFontRole,
    ) -> Result<Self> {
        let selected = binding
            .variants
            .get(binding.selected_variant_index)
            .ok_or_else(|| GameUiError::missing("authored font selected variant is absent"))?;
        let mut contents = Vec::new();
        for variant in &binding.variants {
            contents.push(&variant.registration_source_content);
            for descriptor in &variant.ordered_descriptors {
                contents.push(&descriptor.descriptor_content);
                contents.extend(descriptor.ordered_pages.iter().map(|page| &page.content));
            }
        }
        contents.extend(
            binding
                .global_textcolor_tables
                .iter()
                .map(|table| &table.source_content),
        );
        contents.extend(
            binding
                .font_local_textcolors
                .iter()
                .map(|table| &table.source_content),
        );
        contents.extend(binding.base_color.iter().map(|color| &color.source_content));
        contents.extend(
            binding
                .base_border_color
                .iter()
                .map(|color| &color.source_content),
        );
        contents.push(&binding.text_effect_content);
        for content in contents {
            let admitted = resolver
                .admitted_content_key(&content.normalized_game_relative_path)
                .ok_or_else(|| {
                    GameUiError::missing(format!(
                        "authored font {} content {:?} is absent from the portable resolver",
                        binding.font_id, content.normalized_game_relative_path
                    ))
                })?;
            if admitted != content {
                return Err(GameUiError::missing(format!(
                    "authored font {} content {:?} differs from its authenticated binding",
                    binding.font_id, content.normalized_game_relative_path
                )));
            }
        }
        let mut merged = None;
        let mut next_page_id = 0u32;
        for descriptor in &selected.ordered_descriptors {
            let font = Self::load_from_descriptor(
                resolver,
                &binding.font_id,
                &descriptor.descriptor_content.normalized_game_relative_path,
            )?;
            verify_authored_descriptor(binding, descriptor, &font)?;
            merge_authored_descriptor(&mut merged, font, &mut next_page_id)?;
        }
        let mut font = merged
            .ok_or_else(|| GameUiError::missing("authored selected font variant is empty"))?;
        let mut textcolor_tables: Vec<Vec<(String, Vec<String>)>> = Vec::new();
        if let Some(table) = &binding.font_local_textcolors {
            textcolor_tables.push(
                table
                    .ordered_entries
                    .iter()
                    .map(|entry| (entry.code.clone(), entry.ordered_components.clone()))
                    .collect(),
            );
        }
        textcolor_tables.extend(binding.global_textcolor_tables.iter().map(|table| {
            table
                .ordered_entries
                .iter()
                .map(|entry| (entry.code.clone(), entry.ordered_components.clone()))
                .collect()
        }));
        font.color_table = if textcolor_tables.is_empty() {
            None
        } else {
            Some(
                crate::markup::TextColorTable::from_authenticated_ordered_components(
                    &textcolor_tables,
                )?,
            )
        };
        font.authored_base_color_rgba8 = binding.base_color.as_ref().map(|color| color.rgba8());
        font.authored_base_border_color_rgba8 = binding
            .base_border_color
            .as_ref()
            .map(|color| color.rgba8());
        font.name = binding.font_id.clone();
        Ok(font)
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

    /// the release profile sets `overflow-checks = true`, so that is a panic
    pub fn layout_text(&self, x: i32, y: i32, text: &str) -> TextLayout {
        let mut layout = TextLayout::default();
        let mut pen_x = x;
        let mut pen_y = y;
        let mut line_start_x = x;
        let mut max_line_width = 0i32;
        let mut line_count = 1i32;
        let mut prev = None;

        for (char_index, ch) in strip_color_codes(text).chars().enumerate() {
            if ch == '\n' || ch == '\r' {
                max_line_width = max_line_width.max(pen_x.saturating_sub(line_start_x));
                pen_x = x;
                pen_y = pen_y.saturating_add(self.line_height.max(0));
                line_start_x = x;
                line_count = line_count.saturating_add(1);
                prev = None;
                continue;
            }
            let id = ch as u32;
            let kerning = prev
                .and_then(|prev_id| self.kernings.get(&(prev_id, id)).copied())
                .unwrap_or(0);
            pen_x = pen_x.saturating_add(kerning);
            let Some(font_char) = self.chars.get(&id) else {
                layout.missing_glyphs.insert(ch);
                prev = Some(id);
                continue;
            };
            layout.quads.push(GlyphQuad {
                ch,
                char_index,
                id,
                pen_x,
                pen_y,
                baseline_y: pen_y.saturating_add(self.base),
                x: pen_x.saturating_add(font_char.xoffset),
                y: pen_y.saturating_add(font_char.yoffset),
                width: font_char.width,
                height: font_char.height,
                xadvance: font_char.xadvance,
                kerning,
                page: font_char.page,
            });
            pen_x = pen_x.saturating_add(font_char.xadvance);
            prev = Some(id);
        }

        max_line_width = max_line_width.max(pen_x.saturating_sub(line_start_x));
        layout.width = max_line_width.max(0);
        layout.height = line_count.saturating_mul(self.line_height.max(0)).max(0);
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
        let rich = self.colorize(text, color);
        let measured = self.measure_text(&rich.text);
        let x = match (max_width, align) {
            (Some(max_width), TextAlign::Center) => {
                x.saturating_add(max_width.saturating_sub(measured) / 2)
            }
            (Some(max_width), TextAlign::Right) => {
                x.saturating_add(max_width).saturating_sub(measured)
            }
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

    pub fn draw_text_a729(
        &self,
        image: &mut RgbaImage,
        x: i32,
        y: i32,
        max_width: Option<i32>,
        align: TextAlign,
        text: &str,
        color: [u8; 4],
        clip: Option<[i32; 4]>,
    ) -> std::result::Result<TextRenderReport, TextShaderRejection> {
        if self.glyph_channel.is_some() {
            return Err(TextShaderRejection::PackedGlyphChannelUnproved);
        }
        let rich = self.colorize(text, color);
        let measured = self.measure_text(&rich.text);
        let x = match (max_width, align) {
            (Some(max_width), TextAlign::Center) => {
                x.saturating_add(max_width.saturating_sub(measured) / 2)
            }
            (Some(max_width), TextAlign::Right) => {
                x.saturating_add(max_width).saturating_sub(measured)
            }
            _ => x,
        };
        let layout = self.layout_text(x, y, &rich.text);
        for quad in &layout.quads {
            if quad.width == 0 || quad.height == 0 {
                continue;
            }
            let Some(font_char) = self.chars.get(&quad.id) else {
                continue;
            };
            let Some(page) = self.pages.iter().find(|page| page.id == font_char.page) else {
                return Err(TextShaderRejection::PageAbsent {
                    page: font_char.page,
                });
            };
            let tint = rich.colors.get(quad.char_index).copied().unwrap_or(color);
            blit_glyph_a729(image, &page.image, quad.x, quad.y, font_char, tint, clip);
        }
        Ok(TextRenderReport {
            missing_glyphs: layout.missing_glyphs,
        })
    }

    pub fn draw_runtime_glyph(
        &self,
        image: &mut RgbaImage,
        glyph_id: u32,
        x: i32,
        y: i32,
        color: [u8; 4],
        clip: Option<[i32; 4]>,
    ) -> RuntimeGlyphDraw {
        let Some(font_char) = self.chars.get(&glyph_id) else {
            return RuntimeGlyphDraw::GlyphAbsent;
        };
        let Some(page) = self.pages.iter().find(|page| page.id == font_char.page) else {
            return RuntimeGlyphDraw::PageAbsent {
                page: font_char.page,
            };
        };
        blit_glyph_clipped(
            image,
            &page.image,
            x,
            y,
            font_char,
            color,
            self.glyph_channel,
            clip,
        );
        RuntimeGlyphDraw::Drawn
    }

    pub fn draw_runtime_glyph_a729(
        &self,
        image: &mut RgbaImage,
        glyph_id: u32,
        x: i32,
        y: i32,
        color: [u8; 4],
        clip: Option<[i32; 4]>,
    ) -> std::result::Result<RuntimeGlyphDraw, TextShaderRejection> {
        if self.glyph_channel.is_some() {
            return Err(TextShaderRejection::PackedGlyphChannelUnproved);
        }
        let Some(font_char) = self.chars.get(&glyph_id) else {
            return Ok(RuntimeGlyphDraw::GlyphAbsent);
        };
        let Some(page) = self.pages.iter().find(|page| page.id == font_char.page) else {
            return Err(TextShaderRejection::PageAbsent {
                page: font_char.page,
            });
        };
        blit_glyph_a729(image, &page.image, x, y, font_char, color, clip);
        Ok(RuntimeGlyphDraw::Drawn)
    }
}

fn verify_authored_descriptor(
    binding: &crate::authored_role_index::AuthoredFontRole,
    descriptor: &crate::authored_role_index::AuthoredFontDescriptorRole,
    font: &BitmapFont,
) -> Result<()> {
    let loaded_descriptor =
        portable_content_manifest::normalize_game_relative_path(&font.source_file)
            .map_err(GameUiError::parse)?;
    if loaded_descriptor != descriptor.descriptor_content.normalized_game_relative_path {
        return Err(GameUiError::missing(format!(
            "authored font {} loaded descriptor {loaded_descriptor:?} instead of authenticated {:?}",
            binding.font_id, descriptor.descriptor_content.normalized_game_relative_path
        )));
    }
    let loaded_pages = font
        .pages
        .iter()
        .map(|page| {
            portable_content_manifest::normalize_game_relative_path(&page.file)
                .map(|path| (page.id, path))
        })
        .collect::<std::result::Result<Vec<_>, _>>()
        .map_err(GameUiError::parse)?;
    let authenticated_pages = descriptor
        .ordered_pages
        .iter()
        .map(|page| {
            (
                page.page_id,
                page.content.normalized_game_relative_path.clone(),
            )
        })
        .collect::<Vec<_>>();
    if loaded_pages != authenticated_pages {
        return Err(GameUiError::missing(format!(
            "authored font {} loaded atlas pages {:?} instead of authenticated {:?}",
            binding.font_id, loaded_pages, authenticated_pages
        )));
    }
    Ok(())
}

fn merge_authored_descriptor(
    merged: &mut Option<BitmapFont>,
    mut incoming: BitmapFont,
    next_page_id: &mut u32,
) -> Result<()> {
    let mut page_ids = BTreeMap::new();
    for page in &mut incoming.pages {
        let assigned = *next_page_id;
        *next_page_id = (*next_page_id)
            .checked_add(1)
            .ok_or_else(|| GameUiError::parse("multi-descriptor font page identity overflowed"))?;
        if page_ids.insert(page.id, assigned).is_some() {
            return Err(GameUiError::parse(
                "bitmap-font descriptor repeats a local page ID",
            ));
        }
        page.id = assigned;
    }
    for glyph in incoming.chars.values_mut() {
        glyph.page = *page_ids.get(&glyph.page).ok_or_else(|| {
            GameUiError::parse("bitmap-font glyph names an unauthenticated local page")
        })?;
    }
    let incoming_kernings = std::mem::take(&mut incoming.kernings);
    let incoming_chars = std::mem::take(&mut incoming.chars);
    let incoming_pages = std::mem::take(&mut incoming.pages);
    if merged.is_none() {
        incoming.kernings.clear();
        incoming.chars = incoming_chars;
        incoming.pages = incoming_pages;
        *merged = Some(incoming);
    } else {
        let existing = merged.as_mut().expect("presence checked above");
        existing.pages.extend(incoming_pages);
        for (glyph_id, glyph) in incoming_chars {
            existing.chars.entry(glyph_id).or_insert(glyph);
        }
    }
    let existing = merged.as_mut().expect("font initialized above");
    for ((first, second), amount) in incoming_kernings {
        if existing.chars.contains_key(&first) {
            existing.kernings.entry((first, second)).or_insert(amount);
        }
    }
    Ok(())
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RuntimeGlyphDraw {
    Drawn,
    GlyphAbsent,
    PageAbsent { page: u32 },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TextShaderRejection {
    PackedGlyphChannelUnproved,
    PageAbsent { page: u32 },
}

impl RuntimeGlyphDraw {
    pub fn is_drawn(self) -> bool {
        matches!(self, Self::Drawn)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BitmapFontDependencyPaths {
    pub alias_source: Option<String>,
    pub descriptor: String,
    pub pages: Vec<String>,
    pub ordered_pages: Vec<BitmapFontPageDependency>,
    pub descriptor_decode_loss_events: usize,
}

fn reject_descriptor_decode_damage(rel_path: &str, bytes: &[u8]) -> Result<()> {
    let events = destructive_event_count(&diagnose_text_decode(bytes));
    if events != 0 {
        return Err(GameUiError::parse(format!(
            "{rel_path}: bitmap-font descriptor decode destroyed {events} byte sequences"
        )));
    }
    Ok(())
}

pub fn authored_bitmap_font_registry_paths_for_language(
    resolver: &map_viewer::GameFileResolver,
    symbolic_name: &str,
    localisation_language: &str,
) -> Result<AuthoredBitmapFontRegistryPaths> {
    if symbolic_name.trim().is_empty() {
        return Err(GameUiError::parse("blank bitmap-font identity"));
    }
    let selected_language =
        portable_content_manifest::authored_ui::canonical_localisation_language(
            localisation_language,
        )
        .map_err(GameUiError::parse)?;
    let mut files = resolver
        .resolve_dir_files_recursive("interface", Some("gfx"))
        .map_err(|error| GameUiError::missing(error.to_string()))?;
    files.sort_by(|left, right| {
        portable_content_manifest::authored_ui::engine_recursive_path_cmp(&left.0, &right.0)
    });

    struct PendingVariant {
        source: String,
        languages: Vec<String>,
        descriptors: Vec<String>,
    }

    let mut variants = Vec::<PendingVariant>::new();
    let mut base_color = None;
    let mut base_border_color = None;
    let mut font_local_textcolors = None;
    let mut global_textcolor_tables = Vec::new();
    for (source, path) in files {
        let bytes = fs::read(&path)?;
        let losses = destructive_event_count(&diagnose_text_decode(&bytes));
        if losses != 0 {
            return Err(GameUiError::parse(format!(
                "{source}: GFX decode destroyed {losses} byte sequences"
            )));
        }
        let document = crate::paradox_text::parse_bytes(&bytes);
        if !document.recoveries.is_empty()
            && !crate::paradox_text::engine_tolerated_outer_eof(&document, &bytes)
        {
            return Err(GameUiError::parse(format!(
                "{source}: bitmap-font registry parse required recovery"
            )));
        }
        for container in document.root.values_ci("bitmapfonts") {
            for entry in container.fields() {
                let Some(kind) = entry.key.as_deref() else {
                    continue;
                };
                if kind.eq_ignore_ascii_case("textcolors") {
                    global_textcolor_tables.push(AuthoredTextColorTablePaths {
                        registration_source: source.clone(),
                        ordered_entries: parse_authored_textcolors(&entry.value, "global")?,
                    });
                    continue;
                }
                let is_base = kind.eq_ignore_ascii_case("bitmapfont");
                let is_override = kind.eq_ignore_ascii_case("bitmapfont_override");
                if !is_base && !is_override {
                    continue;
                }
                let name = exact_single_scalar(&entry.value, "name")?;
                if name != symbolic_name {
                    continue;
                }
                if is_base {
                    if !variants.is_empty() {
                        return Err(GameUiError::missing(format!(
                            "font {symbolic_name:?} repeats its base CBitmapFont registration"
                        )));
                    }
                    let descriptors = parse_base_fontfiles(&entry.value)?;
                    if descriptors.is_empty() {
                        return Err(GameUiError::missing(format!(
                            "font {symbolic_name:?} base registration has no descriptor"
                        )));
                    }
                    base_color = parse_optional_packed_color(&entry.value, "color")?;
                    base_border_color = parse_optional_packed_color(&entry.value, "border_color")?;
                    font_local_textcolors = parse_optional_textcolors(&entry.value, "font-local")?;
                    variants.push(PendingVariant {
                        source: source.clone(),
                        languages: Vec::new(),
                        descriptors,
                    });
                } else {
                    if variants.is_empty() {
                        return Err(GameUiError::missing(format!(
                            "font override {symbolic_name:?} appears before its base registration"
                        )));
                    }
                    if entry.value.values_ci("path").next().is_some()
                        || entry.value.values_ci("color").next().is_some()
                        || entry.value.values_ci("border_color").next().is_some()
                        || entry.value.values_ci("textcolors").next().is_some()
                    {
                        return Err(GameUiError::parse(format!(
                            "font override {symbolic_name:?} carries a field not accepted by SBitmapFontOverride"
                        )));
                    }
                    let languages = exact_scalar_list(
                        exact_single_value(&entry.value, "languages")?,
                        "bitmapfont_override languages",
                    )?;
                    let descriptors = exact_scalar_list(
                        exact_single_value(&entry.value, "fontfiles")?,
                        "bitmapfont_override fontfiles",
                    )?
                    .into_iter()
                    .map(normalize_font_descriptor)
                    .collect::<Vec<_>>();
                    if descriptors.is_empty() {
                        return Err(GameUiError::missing(format!(
                            "font override {symbolic_name:?} has no descriptor"
                        )));
                    }
                    variants.push(PendingVariant {
                        source: source.clone(),
                        languages,
                        descriptors,
                    });
                }
            }
        }
    }
    if variants.is_empty() {
        return Err(GameUiError::missing(format!(
            "font {symbolic_name:?} has no exact-case CBitmapFont registration"
        )));
    }
    let selected_variant_index = variants
        .iter()
        .enumerate()
        .skip(1)
        .find(|(_, variant)| {
            variant
                .languages
                .iter()
                .any(|language| language == &selected_language)
        })
        .map(|(index, _)| index)
        .unwrap_or(0);
    let variants = variants
        .into_iter()
        .map(|variant| {
            let ordered_descriptors = variant
                .descriptors
                .into_iter()
                .map(|descriptor| plan_authored_descriptor(resolver, symbolic_name, &descriptor))
                .collect::<Result<Vec<_>>>()?;
            Ok(AuthoredBitmapFontVariantPaths {
                registration_source: variant.source,
                ordered_languages: variant.languages,
                ordered_descriptors,
            })
        })
        .collect::<Result<Vec<_>>>()?;
    Ok(AuthoredBitmapFontRegistryPaths {
        font_id: symbolic_name.to_string(),
        selected_language,
        selected_variant_index,
        variants,
        base_color,
        base_border_color,
        font_local_textcolors,
        global_textcolor_tables,
    })
}

fn exact_single_value<'a>(
    value: &'a crate::paradox_text::Value,
    key: &str,
) -> Result<&'a crate::paradox_text::Value> {
    // lifetime so the returned authored value borrows solely from `value`.
    let mut values = value.fields().iter().filter_map(|entry| {
        entry
            .key
            .as_deref()
            .filter(|candidate| candidate.eq_ignore_ascii_case(key))
            .map(|_| &entry.value)
    });
    let first = values
        .next()
        .ok_or_else(|| GameUiError::parse(format!("bitmap-font field {key:?} is absent")))?;
    if values.next().is_some() {
        return Err(GameUiError::parse(format!(
            "bitmap-font field {key:?} repeats"
        )));
    }
    Ok(first)
}

fn exact_single_scalar<'a>(value: &'a crate::paradox_text::Value, key: &str) -> Result<&'a str> {
    exact_single_value(value, key)?
        .as_scalar()
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| GameUiError::parse(format!("bitmap-font field {key:?} is not scalar")))
}

fn exact_scalar_list(value: &crate::paradox_text::Value, label: &str) -> Result<Vec<String>> {
    let entries = value
        .as_object()
        .ok_or_else(|| GameUiError::parse(format!("{label} is not an ordered scalar list")))?;
    entries
        .iter()
        .map(|entry| {
            if entry.key.is_some() {
                return Err(GameUiError::parse(format!(
                    "{label} contains a labeled entry"
                )));
            }
            entry
                .value
                .as_scalar()
                .filter(|value| !value.trim().is_empty())
                .map(str::to_string)
                .ok_or_else(|| {
                    GameUiError::parse(format!("{label} contains a non-scalar or blank entry"))
                })
        })
        .collect()
}

fn normalize_font_descriptor(path: String) -> String {
    let mut path = normalize_rel_path(&path);
    if !path.to_ascii_lowercase().ends_with(".fnt") {
        path.push_str(".fnt");
    }
    path
}

fn parse_base_fontfiles(value: &crate::paradox_text::Value) -> Result<Vec<String>> {
    let mut descriptors = Vec::new();
    for field in value.fields() {
        let Some(key) = field.key.as_deref() else {
            continue;
        };
        if key.eq_ignore_ascii_case("path") {
            let path = field
                .value
                .as_scalar()
                .ok_or_else(|| GameUiError::parse("bitmapfont path is not an authored scalar"))?;
            descriptors.push(normalize_font_descriptor(path.to_string()));
        } else if key.eq_ignore_ascii_case("fontfiles") {
            descriptors.clear();
            descriptors.extend(
                exact_scalar_list(&field.value, "bitmapfont fontfiles")?
                    .into_iter()
                    .map(normalize_font_descriptor),
            );
        }
    }
    Ok(descriptors)
}

fn parse_optional_packed_color(
    value: &crate::paradox_text::Value,
    key: &str,
) -> Result<Option<AuthoredPackedColor>> {
    let mut values = value.values_ci(key);
    let Some(value) = values.next() else {
        return Ok(None);
    };
    if values.next().is_some() {
        return Err(GameUiError::parse(format!(
            "bitmapfont field {key:?} repeats"
        )));
    }
    let token = value
        .as_scalar()
        .ok_or_else(|| GameUiError::parse(format!("bitmapfont field {key:?} is not scalar")))?;
    let trimmed = token.trim();
    let argb = if let Some(hex) = trimmed
        .strip_prefix("0x")
        .or_else(|| trimmed.strip_prefix("0X"))
    {
        u32::from_str_radix(hex, 16)
    } else {
        trimmed.parse::<u32>()
    }
    .map_err(|_| GameUiError::parse(format!("bitmapfont field {key:?} is not packed u32")))?;
    Ok(Some(AuthoredPackedColor {
        authored_token: token.to_string(),
        argb,
    }))
}

fn parse_optional_textcolors(
    value: &crate::paradox_text::Value,
    label: &str,
) -> Result<Option<Vec<AuthoredTextColorEntry>>> {
    let mut values = value.values_ci("textcolors");
    let Some(value) = values.next() else {
        return Ok(None);
    };
    if values.next().is_some() {
        return Err(GameUiError::parse(format!(
            "{label} textcolors table repeats"
        )));
    }
    Ok(Some(parse_authored_textcolors(value, label)?))
}

fn parse_authored_textcolors(
    value: &crate::paradox_text::Value,
    label: &str,
) -> Result<Vec<AuthoredTextColorEntry>> {
    value
        .as_object()
        .ok_or_else(|| GameUiError::parse(format!("{label} textcolors is not a table")))?
        .iter()
        .map(|entry| {
            let code = entry.key.as_ref().ok_or_else(|| {
                GameUiError::parse(format!("{label} textcolors has an anonymous entry"))
            })?;
            let ordered_components =
                exact_scalar_list(&entry.value, &format!("{label} textcolors {code:?}"))?;
            Ok(AuthoredTextColorEntry {
                code: code.clone(),
                ordered_components,
            })
        })
        .collect()
}

fn plan_authored_descriptor(
    resolver: &map_viewer::GameFileResolver,
    symbolic_name: &str,
    descriptor: &str,
) -> Result<AuthoredBitmapFontDescriptorPaths> {
    let resolved = resolver
        .resolve_required_with_provenance(descriptor, "bitmap-font descriptor")
        .map_err(|error| GameUiError::missing(error.to_string()))?;
    let bytes = fs::read(&resolved.local_path)?;
    reject_descriptor_decode_damage(&resolved.logical_path, &bytes)?;
    let stem = Path::new(&resolved.logical_path)
        .file_stem()
        .and_then(|value| value.to_str())
        .unwrap_or(symbolic_name);
    let parsed = parse_bmfont_bytes(stem, &resolved.logical_path, &bytes)?;
    if parsed.lossy_page_names != 0 {
        return Err(GameUiError::parse(format!(
            "{}: bitmap-font page-name decode destroyed {} byte sequences",
            resolved.logical_path, parsed.lossy_page_names
        )));
    }
    let parent = Path::new(&resolved.logical_path)
        .parent()
        .and_then(|value| value.to_str())
        .unwrap_or("");
    let first_planned = |candidates: Vec<String>| -> Result<Option<String>> {
        for candidate in candidates {
            if resolver
                .resolve_with_provenance(&candidate)
                .map_err(|error| GameUiError::missing(error.to_string()))?
                .is_some()
            {
                return Ok(Some(candidate));
            }
        }
        Ok(None)
    };
    let mut ordered_pages = Vec::new();
    if parsed.page_files.is_empty() {
        let path =
            first_planned(inferred_page_candidates(Some(parent), stem, 0))?.ok_or_else(|| {
                GameUiError::missing(format!("font {symbolic_name} has no inferred atlas page"))
            })?;
        ordered_pages.push(BitmapFontPageDependency { page_id: 0, path });
    } else {
        for (page_id, page) in &parsed.page_files {
            reject_unsafe_declared_page(stem, *page_id, page)?;
            let declared = normalize_rel_path(page);
            let selected =
                match first_planned(declared_page_candidates(Some(parent), &declared))? {
                    Some(path) => Some(path),
                    None => first_planned(inferred_page_candidates(Some(parent), stem, *page_id))?,
                }
                .ok_or_else(|| {
                    GameUiError::missing(format!(
                        "bitmap-font page {page} and inferred {stem} page {page_id} are absent"
                    ))
                })?;
            ordered_pages.push(BitmapFontPageDependency {
                page_id: *page_id,
                path: selected,
            });
        }
    }
    Ok(AuthoredBitmapFontDescriptorPaths {
        descriptor: resolved.logical_path,
        ordered_pages,
    })
}

pub fn bitmap_font_dependency_paths(
    resolver: &map_viewer::GameFileResolver,
    symbolic_name: &str,
) -> Result<BitmapFontDependencyPaths> {
    bitmap_font_dependency_paths_for_language(resolver, symbolic_name, RESOLVED_LANGUAGE)
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BitmapFontPageDependency {
    pub page_id: u32,
    pub path: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AuthoredBitmapFontRegistryPaths {
    pub font_id: String,
    pub selected_language: String,
    pub selected_variant_index: usize,
    pub variants: Vec<AuthoredBitmapFontVariantPaths>,
    pub base_color: Option<AuthoredPackedColor>,
    pub base_border_color: Option<AuthoredPackedColor>,
    pub font_local_textcolors: Option<Vec<AuthoredTextColorEntry>>,
    pub global_textcolor_tables: Vec<AuthoredTextColorTablePaths>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AuthoredBitmapFontVariantPaths {
    pub registration_source: String,
    pub ordered_languages: Vec<String>,
    pub ordered_descriptors: Vec<AuthoredBitmapFontDescriptorPaths>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AuthoredBitmapFontDescriptorPaths {
    pub descriptor: String,
    pub ordered_pages: Vec<BitmapFontPageDependency>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AuthoredPackedColor {
    pub authored_token: String,
    pub argb: u32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AuthoredTextColorEntry {
    pub code: String,
    pub ordered_components: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AuthoredTextColorTablePaths {
    pub registration_source: String,
    pub ordered_entries: Vec<AuthoredTextColorEntry>,
}

pub fn bitmap_font_dependency_paths_for_language(
    resolver: &map_viewer::GameFileResolver,
    symbolic_name: &str,
    localisation_language: &str,
) -> Result<BitmapFontDependencyPaths> {
    if symbolic_name.trim().is_empty() {
        return Err(GameUiError::parse("blank bitmap-font identity"));
    }
    let core = resolver
        .resolve_with_provenance("interface/core.gfx")
        .map_err(|error| GameUiError::missing(error.to_string()))?;
    let direct = format!("gfx/fonts/{symbolic_name}.fnt");
    let mut used_alias = false;
    let descriptor = if resolver
        .resolve_with_provenance(&direct)
        .map_err(|error| GameUiError::missing(error.to_string()))?
        .is_some()
    {
        direct
    } else {
        used_alias = true;
        let core = core.as_ref().ok_or_else(|| {
            GameUiError::missing(format!(
                "font {symbolic_name} has no descriptor or core.gfx alias"
            ))
        })?;
        let document = parse_file(&core.local_path)?;
        core_gfx_font_alias_for_language(&document, symbolic_name, localisation_language)?
            .ok_or_else(|| {
                GameUiError::missing(format!("font {symbolic_name} has 0 effective aliases"))
            })?
    };
    let resolved = resolver
        .resolve_required_with_provenance(&descriptor, "bitmap-font descriptor")
        .map_err(|error| GameUiError::missing(error.to_string()))?;
    let bytes = fs::read(&resolved.local_path)?;
    let stem = Path::new(&resolved.logical_path)
        .file_stem()
        .and_then(|value| value.to_str())
        .unwrap_or(symbolic_name);
    let parsed = parse_bmfont_bytes(stem, &resolved.logical_path, &bytes)?;
    let parent = Path::new(&resolved.logical_path)
        .parent()
        .and_then(|value| value.to_str())
        .unwrap_or("");
    let alias_source = used_alias.then(|| "interface/core.gfx".to_string());
    let first_planned = |candidates: Vec<String>| -> Result<Option<String>> {
        for candidate in candidates {
            if resolver
                .resolve_with_provenance(&candidate)
                .map_err(|error| GameUiError::missing(error.to_string()))?
                .is_some()
            {
                return Ok(Some(candidate));
            }
        }
        Ok(None)
    };
    let mut pages = Vec::new();
    let mut ordered_pages = Vec::new();
    if parsed.page_files.is_empty() {
        let page =
            first_planned(inferred_page_candidates(Some(parent), stem, 0))?.ok_or_else(|| {
                GameUiError::missing(format!("font {symbolic_name} has no inferred atlas page"))
            })?;
        pages.push(page.clone());
        ordered_pages.push(BitmapFontPageDependency {
            page_id: 0,
            path: page,
        });
    } else {
        for (id, page) in &parsed.page_files {
            reject_unsafe_declared_page(stem, *id, page)?;
            let declared = normalize_rel_path(page);
            let selected = match first_planned(declared_page_candidates(Some(parent), &declared))? {
                Some(path) => Some(path),
                None => first_planned(inferred_page_candidates(Some(parent), stem, *id))?,
            };
            let selected = selected.ok_or_else(|| {
                GameUiError::missing(format!(
                    "bitmap-font page {page} and inferred {stem} page {id} are absent"
                ))
            })?;
            pages.push(selected.clone());
            ordered_pages.push(BitmapFontPageDependency {
                page_id: *id,
                path: selected,
            });
        }
    }
    pages.sort();
    pages.dedup();
    Ok(BitmapFontDependencyPaths {
        alias_source,
        descriptor: resolved.logical_path,
        pages,
        ordered_pages,
        descriptor_decode_loss_events: destructive_event_count(&diagnose_text_decode(&bytes))
            + parsed.lossy_page_names,
    })
}

const RESOLVED_LANGUAGE: &str = "l_english";

fn core_gfx_font_alias(
    document: &crate::paradox_text::Document,
    symbolic_name: &str,
) -> Result<Option<String>> {
    core_gfx_font_alias_for_language(document, symbolic_name, RESOLVED_LANGUAGE)
}

fn core_gfx_font_alias_for_language(
    document: &crate::paradox_text::Document,
    symbolic_name: &str,
    localisation_language: &str,
) -> Result<Option<String>> {
    if !document.recoveries.is_empty() {
        return Err(GameUiError::parse(
            "core.gfx font alias parse required recovery",
        ));
    }
    let resolved_language =
        portable_content_manifest::authored_ui::canonical_localisation_language(
            localisation_language,
        )
        .map_err(GameUiError::parse)?;
    let mut base = Vec::new();
    for container in document.root.values_ci("bitmapfonts") {
        for entry in container.fields() {
            let Some(kind) = entry.key.as_deref() else {
                continue;
            };
            let is_override = kind.eq_ignore_ascii_case("bitmapfont_override");
            if !is_override && !kind.eq_ignore_ascii_case("bitmapfont") {
                continue;
            }
            if !entry
                .value
                .first_scalar_ci("name")
                .is_some_and(|candidate| candidate.eq_ignore_ascii_case(symbolic_name))
            {
                continue;
            }
            if is_override {
                if applies_to_resolved_language(&entry.value, &resolved_language)? {
                    return Err(GameUiError::missing(format!(
                        "font {symbolic_name} has an applicable bitmapfont_override whose registration-population law is unproved"
                    )));
                }
                continue;
            }
            let Some(path) = entry.value.first_scalar_ci("path") else {
                continue;
            };
            let mut rel = normalize_rel_path(path);
            if !rel.to_ascii_lowercase().ends_with(".fnt") {
                rel.push_str(".fnt");
            }
            base.push(rel);
        }
    }
    let mut aliases = base;
    aliases.sort();
    aliases.dedup();
    match aliases.len() {
        0 => Ok(None),
        1 => Ok(Some(aliases.remove(0))),
        count => Err(GameUiError::missing(format!(
            "font {symbolic_name} has {count} unresolved base aliases"
        ))),
    }
}

fn applies_to_resolved_language(
    entry: &crate::paradox_text::Value,
    localisation_language: &str,
) -> Result<bool> {
    let Some(languages) = entry.first_value_ci("languages") else {
        return Ok(true);
    };
    let entries = languages.as_object().ok_or_else(|| {
        GameUiError::parse("bitmapfont_override languages is not an unlabeled scalar list")
    })?;
    if entries.is_empty()
        || entries
            .iter()
            .any(|entry| entry.key.is_some() || entry.value.as_scalar().is_none())
    {
        return Err(GameUiError::parse(
            "bitmapfont_override languages is not an unlabeled scalar list",
        ));
    }
    Ok(entries
        .iter()
        .filter_map(|entry| entry.value.as_scalar())
        .any(|language| language == localisation_language))
}

fn bitmap_font_alias_path(resolver: &AssetResolver, name: &str) -> Result<Option<String>> {
    let Some(core) = resolver.resolve_optional_file("interface/core.gfx")? else {
        return Ok(None);
    };
    let document = parse_file(&core.path)?;
    core_gfx_font_alias(&document, name)
}

impl BitmapFont {
    pub fn colorize(&self, text: &str, base: [u8; 4]) -> ColorizedText {
        colorize_with_table(text, base, self.color_table.as_ref())
    }
}

pub fn strip_color_codes(text: &str) -> String {
    crate::markup::strip_section_codes(text)
}

pub fn colorize_with_table(
    text: &str,
    base: [u8; 4],
    table: Option<&crate::markup::TextColorTable>,
) -> ColorizedText {
    let empty = crate::markup::TextColorTable::default();
    let table = table.unwrap_or(&empty);
    let tokens = crate::markup::parse(text, table);
    let runs = crate::markup::color_runs(&tokens, [base[0], base[1], base[2]]);
    let colors = runs
        .colors
        .iter()
        .map(|rgb| [rgb[0], rgb[1], rgb[2], base[3]])
        .collect();
    ColorizedText {
        text: runs.text,
        colors,
    }
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
    blit_glyph_clipped(dst, page, x, y, font_char, color, glyph_channel, None)
}

fn glyph_axis_span(
    origin: i32,
    glyph_start: usize,
    glyph_extent: usize,
    atlas_extent: usize,
    dst_extent: usize,
) -> std::ops::Range<usize> {
    let source_limit = atlas_extent.saturating_sub(glyph_start).min(glyph_extent) as i64;
    let origin = i64::from(origin);
    let start = (-origin).max(0);
    let end = source_limit.min(dst_extent as i64 - origin);
    if end <= start {
        return 0..0;
    }
    (start as usize)..(end as usize)
}

fn blit_glyph_clipped(
    dst: &mut RgbaImage,
    page: &RgbaImage,
    x: i32,
    y: i32,
    font_char: &FontChar,
    color: [u8; 4],
    glyph_channel: Option<usize>,
    clip: Option<[i32; 4]>,
) {
    let gx_span = glyph_axis_span(x, font_char.x, font_char.width, page.width, dst.width);
    let gy_span = glyph_axis_span(y, font_char.y, font_char.height, page.height, dst.height);
    for gy in gy_span {
        let dy = y + gy as i32;
        debug_assert!(dy >= 0 && dy < dst.height as i32);
        for gx in gx_span.clone() {
            let dx = x + gx as i32;
            debug_assert!(dx >= 0 && dx < dst.width as i32);
            if let Some([clip_x, clip_y, clip_w, clip_h]) = clip {
                if clip_w < 0
                    || clip_h < 0
                    || dx < clip_x
                    || dy < clip_y
                    || dx >= clip_x.saturating_add(clip_w)
                    || dy >= clip_y.saturating_add(clip_h)
                {
                    continue;
                }
            }
            let sx = font_char.x + gx;
            let sy = font_char.y + gy;
            debug_assert!(sx < page.width && sy < page.height);
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

fn blit_glyph_a729(
    dst: &mut RgbaImage,
    page: &RgbaImage,
    x: i32,
    y: i32,
    font_char: &FontChar,
    color: [u8; 4],
    clip: Option<[i32; 4]>,
) {
    let gx_span = glyph_axis_span(x, font_char.x, font_char.width, page.width, dst.width);
    let gy_span = glyph_axis_span(y, font_char.y, font_char.height, page.height, dst.height);
    for gy in gy_span {
        let dy = y + gy as i32;
        for gx in gx_span.clone() {
            let dx = x + gx as i32;
            if let Some([clip_x, clip_y, clip_w, clip_h]) = clip {
                if clip_w < 0
                    || clip_h < 0
                    || dx < clip_x
                    || dy < clip_y
                    || dx >= clip_x.saturating_add(clip_w)
                    || dy >= clip_y.saturating_add(clip_h)
                {
                    continue;
                }
            }
            let sx = font_char.x + gx;
            let sy = font_char.y + gy;
            let src = (sy * page.width + sx) * 4;
            let sampled: [u8; 4] = page.pixels[src..src + 4]
                .try_into()
                .expect("RGBA font atlas slice");
            let shaded = std::array::from_fn(|channel| {
                ((u16::from(sampled[channel]) * u16::from(color[channel]) + 127) / 255) as u8
            });
            blend_text_pixel_a729(dst, dx as usize, dy as usize, shaded);
        }
    }
}

fn blend_text_pixel_a729(dst: &mut RgbaImage, x: usize, y: usize, src: [u8; 4]) {
    let offset = (y * dst.width + x) * 4;
    let dst_px: [u8; 4] = dst.pixels[offset..offset + 4]
        .try_into()
        .expect("RGBA destination slice");
    let sa = u32::from(src[3]);
    let inv_sa = 255 - sa;
    let mut out = [0_u8; 4];
    for channel in 0..3 {
        out[channel] = ((u32::from(src[channel]) * sa + u32::from(dst_px[channel]) * inv_sa + 127)
            / 255)
            .min(255) as u8;
    }
    out[3] = src[3].max(dst_px[3]);
    dst.pixels[offset..offset + 4].copy_from_slice(&out);
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
    lossy_page_names: usize,
    chars: BTreeMap<u32, FontChar>,
    kernings: BTreeMap<(u32, u32), i32>,
}

impl ParsedFont {
    fn load_pages(self, resolver: &AssetResolver) -> Result<BitmapFont> {
        if self.lossy_page_names != 0 {
            return Err(GameUiError::parse(format!(
                "{}: binary BMFont page-name decode destroyed {} byte sequences",
                self.source_file, self.lossy_page_names
            )));
        }
        let mut pages = Vec::new();
        let descriptor_parent = Path::new(&self.source_file)
            .parent()
            .map(|parent| normalize_rel_path(&parent.to_string_lossy()))
            .filter(|parent| !parent.is_empty());
        let page_files = if self.page_files.is_empty() {
            inferred_page_files(descriptor_parent.as_deref(), &self.name, resolver)?
        } else {
            self.page_files
        };
        for (id, file) in page_files {
            reject_unsafe_declared_page(&self.name, id, &file)?;
            let declared = normalize_rel_path(&file);
            let parent = descriptor_parent.as_deref();
            let selected =
                match first_resolvable_page(resolver, declared_page_candidates(parent, &declared))?
                {
                    Some(rel) => Some(rel),
                    None => first_resolvable_page(
                        resolver,
                        inferred_page_candidates(parent, &self.name, id),
                    )?,
                };
            let decoded = match selected {
                Some(rel) => resolver.decode_texture(&rel)?,
                None => {
                    return Err(GameUiError::missing(format!(
                        "font {} page {id} is absent from the effective VFS: \
                         declared {file} does not resolve and no {} atlas was found \
                         beside the descriptor or under {LEGACY_FONT_ROOT}",
                        self.name, self.name
                    )))
                }
            };
            pages.push(FontPage {
                id,
                file: decoded.rel_path,
                image: decoded.image,
            });
        }
        let loaded = pages.iter().map(|page| page.id).collect::<BTreeSet<_>>();
        let dangling = self
            .chars
            .values()
            .filter(|font_char| !loaded.contains(&font_char.page))
            .map(|font_char| format!("glyph {} -> page {}", font_char.id, font_char.page))
            .take(8)
            .collect::<Vec<_>>();
        if !dangling.is_empty() {
            return Err(GameUiError::missing(format!(
                "font {} has glyphs referencing pages that are not loaded ({:?}); \
                 loaded pages are {:?}",
                self.name, dangling, loaded
            )));
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
            color_table: crate::markup::TextColorTable::load_optional(resolver)?,
            authored_base_color_rgba8: None,
            authored_base_border_color_rgba8: None,
        })
    }
}

const LEGACY_FONT_ROOT: &str = "gfx/fonts";

fn reject_unsafe_declared_page(font: &str, id: u32, file: &str) -> Result<()> {
    let declared = normalize_rel_path(file);
    let raw_path = Path::new(file);
    let declared_path = Path::new(&declared);
    let has_unsafe_component = |path: &Path| {
        path.components().any(|component| {
            matches!(
                component,
                Component::ParentDir | Component::RootDir | Component::Prefix(_)
            )
        })
    };
    if raw_path.is_absolute()
        || declared_path.is_absolute()
        || file.starts_with('/')
        || file.starts_with('\\')
        || file.contains(':')
        || has_unsafe_component(raw_path)
        || has_unsafe_component(declared_path)
    {
        return Err(GameUiError::parse(format!(
            "font {font} page {id} has unsafe declared path {file}"
        )));
    }
    Ok(())
}

fn declared_page_candidates(parent: Option<&str>, declared: &str) -> Vec<String> {
    let mut candidates = Vec::new();
    let explicit_vfs_path = declared
        .get(..4)
        .is_some_and(|prefix| prefix.eq_ignore_ascii_case("gfx/"));
    if !explicit_vfs_path {
        if let Some(parent) = parent.filter(|parent| !parent.is_empty()) {
            candidates.push(normalize_rel_path(&format!("{parent}/{declared}")));
        }
    }
    candidates.push(if declared.contains('/') {
        declared.to_string()
    } else {
        normalize_rel_path(&format!("{LEGACY_FONT_ROOT}/{declared}"))
    });
    dedup_preserving_order(&mut candidates);
    candidates
}

fn inferred_page_candidates(parent: Option<&str>, stem: &str, id: u32) -> Vec<String> {
    let mut candidates = Vec::new();
    let mut push_dir = |dir: &str| {
        if id == 0 {
            candidates.push(normalize_rel_path(&format!("{dir}/{stem}.dds")));
            candidates.push(normalize_rel_path(&format!("{dir}/{stem}.tga")));
        }
        candidates.push(normalize_rel_path(&format!("{dir}/{stem}_{id}.dds")));
        candidates.push(normalize_rel_path(&format!("{dir}/{stem}_{id}.tga")));
    };
    if let Some(parent) = parent.filter(|parent| !parent.is_empty()) {
        push_dir(parent);
    }
    push_dir(LEGACY_FONT_ROOT);
    dedup_preserving_order(&mut candidates);
    candidates
}

fn dedup_preserving_order(candidates: &mut Vec<String>) {
    let mut seen = BTreeSet::new();
    candidates.retain(|candidate| seen.insert(candidate.clone()));
}

fn first_resolvable_page(
    resolver: &AssetResolver,
    candidates: Vec<String>,
) -> Result<Option<String>> {
    for candidate in candidates {
        if resolver.resolve_optional_file(&candidate)?.is_some() {
            return Ok(Some(candidate));
        }
    }
    Ok(None)
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
        lossy_page_names: 0,
        chars: BTreeMap::new(),
        kernings: BTreeMap::new(),
    };
    let mut saw_info = false;
    let mut saw_common = false;
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
                saw_info = true;
                parsed.info_size = field_i32(&fields, "size").unwrap_or(0);
                parsed.info_padding = field_i32_array::<4>(&fields, "padding").unwrap_or([0; 4]);
                parsed.info_spacing = field_i32_array::<2>(&fields, "spacing").unwrap_or([0; 2]);
                parsed.outline = field_i32(&fields, "outline").unwrap_or(0);
            }
            "common" => {
                saw_common = true;
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
                let font_char = parse_text_char(&fields, source_file)?;
                parsed.chars.insert(font_char.id, font_char);
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
    require_bmfont_shape(&parsed, source_file, saw_info, saw_common)?;
    Ok(parsed)
}

fn require_bmfont_shape(
    parsed: &ParsedFont,
    source_file: &str,
    saw_info: bool,
    saw_common: bool,
) -> Result<()> {
    if !saw_info {
        return Err(GameUiError::parse(format!(
            "BMFont descriptor {source_file} has no info block; not a BMFont file"
        )));
    }
    if !saw_common {
        return Err(GameUiError::parse(format!(
            "BMFont descriptor {source_file} has no common block; not a BMFont file"
        )));
    }
    if parsed.chars.is_empty() {
        return Err(GameUiError::parse(format!(
            "BMFont descriptor {source_file} declares no glyphs; a zero-glyph font \
             would report every character as ordinarily missing"
        )));
    }
    Ok(())
}

fn glyph_extent(fields: &BTreeMap<String, String>, key: &str, source_file: &str) -> Result<usize> {
    let value = field_usize(fields, key).ok_or_else(|| {
        GameUiError::parse(format!(
            "BMFont char record in {source_file} has no usable {key} field"
        ))
    })?;
    if value > MAX_GLYPH_EXTENT {
        return Err(GameUiError::parse(format!(
            "BMFont char {key}={value} in {source_file} exceeds the \
             {MAX_GLYPH_EXTENT}-texel glyph-extent bound"
        )));
    }
    Ok(value)
}

fn parse_text_char(fields: &BTreeMap<String, String>, source_file: &str) -> Result<FontChar> {
    let required_i32 = |key: &str| -> Result<i32> {
        field_i32(fields, key).ok_or_else(|| {
            GameUiError::parse(format!(
                "BMFont char record in {source_file} has no usable {key} field"
            ))
        })
    };
    Ok(FontChar {
        id: field_u32(fields, "id").ok_or_else(|| {
            GameUiError::parse(format!(
                "BMFont char record in {source_file} has no usable id field"
            ))
        })?,
        x: glyph_extent(fields, "x", source_file)?,
        y: glyph_extent(fields, "y", source_file)?,
        width: glyph_extent(fields, "width", source_file)?,
        height: glyph_extent(fields, "height", source_file)?,
        xoffset: required_i32("xoffset")?,
        yoffset: required_i32("yoffset")?,
        xadvance: required_i32("xadvance")?,
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
    if bytes[3] != 3 {
        return Err(GameUiError::parse(format!(
            "binary BMFont {source_file} declares unsupported version {}",
            bytes[3]
        )));
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
        lossy_page_names: 0,
        chars: BTreeMap::new(),
        kernings: BTreeMap::new(),
    };
    let mut pos = 4usize;
    let mut saw_info = false;
    let mut saw_common = false;
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
        let end = pos.checked_add(size).ok_or_else(|| {
            GameUiError::parse(format!(
                "binary BMFont block {block_type} declares an impossible size {size}"
            ))
        })?;
        let block = bytes.get(pos..end).ok_or_else(|| {
            GameUiError::parse(format!("binary BMFont block {block_type} is truncated"))
        })?;
        match block_type {
            1 => {
                if block.len() < 14 {
                    return Err(GameUiError::parse(format!(
                        "binary BMFont {source_file} info block is {} bytes, need 14",
                        block.len()
                    )));
                }
                saw_info = true;
                parsed.info_size = i16::from_le_bytes([block[0], block[1]]) as i32;
                parsed.info_padding = [
                    i32::from(block[7]),
                    i32::from(block[8]),
                    i32::from(block[9]),
                    i32::from(block[10]),
                ];
                parsed.info_spacing = [i32::from(block[11]), i32::from(block[12])];
                parsed.outline = i32::from(block[13]);
            }
            2 => {
                if block.len() < 15 {
                    return Err(GameUiError::parse(format!(
                        "binary BMFont {source_file} common block is {} bytes, need 15",
                        block.len()
                    )));
                }
                saw_common = true;
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
            3 => {
                parsed.lossy_page_names += parse_binary_pages(block, &mut parsed.page_files);
            }
            4 => parse_binary_chars(block, &mut parsed.chars)?,
            5 => parse_binary_kernings(block, &mut parsed.kernings)?,
            _ => {}
        }
        pos = end;
    }
    require_bmfont_shape(&parsed, source_file, saw_info, saw_common)?;
    Ok(parsed)
}

fn parse_binary_pages(block: &[u8], out: &mut BTreeMap<u32, String>) -> usize {
    let mut lossy = 0usize;
    for (idx, page) in block
        .split(|byte| *byte == 0)
        .filter(|page| !page.is_empty())
        .enumerate()
    {
        if std::str::from_utf8(page).is_err() {
            lossy += 1;
        }
        out.insert(idx as u32, String::from_utf8_lossy(page).into_owned());
    }
    lossy
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

fn inferred_page_files(
    parent: Option<&str>,
    name: &str,
    resolver: &AssetResolver,
) -> Result<BTreeMap<u32, String>> {
    match first_resolvable_page(resolver, inferred_page_candidates(parent, name, 0))? {
        Some(candidate) => Ok(BTreeMap::from([(0, candidate)])),
        None => Err(GameUiError::missing(format!(
            "no texture page found for font {name}"
        ))),
    }
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU64, Ordering};

    static NEXT_FONT_FIXTURE: AtomicU64 = AtomicU64::new(0);

    struct FontFixtureRoot(PathBuf);

    impl FontFixtureRoot {
        fn new(label: &str) -> Self {
            let ordinal = NEXT_FONT_FIXTURE.fetch_add(1, Ordering::Relaxed);
            let path = std::env::temp_dir().join(format!(
                "game_ui_font_{label}_{}_{}",
                std::process::id(),
                ordinal
            ));
            fs::create_dir_all(&path).unwrap();
            Self(path)
        }

        fn write_tga(&self, rel: &str, rgba: [u8; 4]) {
            let path = self
                .0
                .join(rel.replace('/', &std::path::MAIN_SEPARATOR.to_string()));
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            let mut bytes = vec![0u8; 18];
            bytes[2] = 2;
            bytes[12..14].copy_from_slice(&1u16.to_le_bytes());
            bytes[14..16].copy_from_slice(&1u16.to_le_bytes());
            bytes[16] = 32;
            bytes[17] = 0x28;
            bytes.extend_from_slice(&[rgba[2], rgba[1], rgba[0], rgba[3]]);
            fs::write(path, bytes).unwrap();
        }
    }

    impl Drop for FontFixtureRoot {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn parsed_font_with_page(source_file: &str, page_file: &str) -> ParsedFont {
        ParsedFont {
            name: "probe".into(),
            source_file: source_file.into(),
            info_size: 16,
            info_padding: [0; 4],
            info_spacing: [0; 2],
            outline: 0,
            line_height: 16,
            base: 12,
            scale_w: 1,
            scale_h: 1,
            glyph_channel: None,
            page_files: BTreeMap::from([(0, page_file.into())]),
            lossy_page_names: 0,
            chars: BTreeMap::new(),
            kernings: BTreeMap::new(),
        }
    }

    #[test]
    fn declared_page_prefers_descriptor_parent_over_legacy_font_root() {
        let root = FontFixtureRoot::new("descriptor_parent");
        root.write_tga("gfx/fonts/japanese/atlas.tga", [10, 20, 30, 255]);
        root.write_tga("gfx/fonts/atlas.tga", [90, 80, 70, 255]);
        let resolver = AssetResolver::new(vec![root.0.clone()]);

        let font = parsed_font_with_page("gfx/fonts/japanese/probe.fnt", "atlas.tga")
            .load_pages(&resolver)
            .unwrap();

        assert_eq!(font.pages.len(), 1);
        assert_eq!(font.pages[0].file, "gfx/fonts/japanese/atlas.tga");
        assert_eq!(font.pages[0].image.pixels, [10, 20, 30, 255]);
    }

    #[test]
    fn uppercase_explicit_vfs_page_does_not_probe_descriptor_parent() {
        let root = FontFixtureRoot::new("uppercase_vfs");
        root.write_tga("GFX/fonts/shared.tga", [10, 20, 30, 255]);
        root.write_tga("gfx/fonts/japanese/GFX/fonts/shared.tga", [90, 80, 70, 255]);
        let resolver = AssetResolver::new(vec![root.0.clone()]);

        let font = parsed_font_with_page("gfx/fonts/japanese/probe.fnt", "GFX/fonts/shared.tga")
            .load_pages(&resolver)
            .unwrap();

        assert_eq!(font.pages[0].image.pixels, [10, 20, 30, 255]);
    }

    #[test]
    fn missing_declared_page_falls_back_to_the_descriptor_stem_atlas() {
        let root = FontFixtureRoot::new("explicit_missing");
        root.write_tga("gfx/fonts/probe.tga", [10, 20, 30, 255]);
        let resolver = AssetResolver::new(vec![root.0.clone()]);

        let font = parsed_font_with_page("gfx/fonts/nested/probe.fnt", "missing.tga")
            .load_pages(&resolver)
            .expect("a stale declared page name falls back to the descriptor stem");

        assert_eq!(font.pages.len(), 1);
        assert_eq!(font.pages[0].file, "gfx/fonts/probe.tga");
        assert_eq!(font.pages[0].image.pixels, [10, 20, 30, 255]);
    }

    #[test]
    fn declared_page_absent_with_no_stem_atlas_still_refuses() {
        let root = FontFixtureRoot::new("explicit_missing_no_stem");
        root.write_tga("gfx/fonts/unrelated.tga", [10, 20, 30, 255]);
        let resolver = AssetResolver::new(vec![root.0.clone()]);

        let error = parsed_font_with_page("gfx/fonts/nested/probe.fnt", "missing.tga")
            .load_pages(&resolver)
            .expect_err("nothing resolves, so nothing may be bound");

        assert!(
            error.message.contains("is absent from the effective VFS"),
            "{error}"
        );
    }

    #[test]
    fn inferred_stem_atlas_prefers_the_descriptor_directory() {
        let root = FontFixtureRoot::new("stem_parent_wins");
        root.write_tga("gfx/fonts/korean/probe.tga", [10, 20, 30, 255]);
        root.write_tga("gfx/fonts/probe.tga", [90, 80, 70, 255]);
        let resolver = AssetResolver::new(vec![root.0.clone()]);

        let font = parsed_font_with_page("gfx/fonts/korean/probe.fnt", "missing.tga")
            .load_pages(&resolver)
            .unwrap();

        assert_eq!(font.pages[0].file, "gfx/fonts/korean/probe.tga");
        assert_eq!(font.pages[0].image.pixels, [10, 20, 30, 255]);
    }

    #[test]
    fn page_less_nested_descriptor_infers_beside_itself() {
        let root = FontFixtureRoot::new("page_less_nested");
        root.write_tga("gfx/fonts/korean/probe.tga", [10, 20, 30, 255]);
        root.write_tga("gfx/fonts/probe.tga", [90, 80, 70, 255]);
        let resolver = AssetResolver::new(vec![root.0.clone()]);

        let mut parsed = parsed_font_with_page("gfx/fonts/korean/probe.fnt", "unused.tga");
        parsed.page_files = BTreeMap::new();
        let font = parsed.load_pages(&resolver).unwrap();

        assert_eq!(font.pages[0].file, "gfx/fonts/korean/probe.tga");
        assert_eq!(font.pages[0].image.pixels, [10, 20, 30, 255]);
    }

    #[test]
    fn inferred_page_above_zero_does_not_borrow_the_unsuffixed_atlas() {
        let root = FontFixtureRoot::new("stem_page_one");
        root.write_tga("gfx/fonts/probe.tga", [10, 20, 30, 255]);
        let resolver = AssetResolver::new(vec![root.0.clone()]);

        let mut parsed = parsed_font_with_page("gfx/fonts/probe.fnt", "missing.tga");
        parsed.page_files = BTreeMap::from([(1, "missing.tga".into())]);
        let error = parsed
            .load_pages(&resolver)
            .expect_err("page 1 has no atlas of its own");

        assert!(error.message.contains("page 1"), "{error}");
    }

    #[test]
    fn declared_page_rejects_parent_absolute_and_drive_paths() {
        let root = FontFixtureRoot::new("unsafe_paths");
        let resolver = AssetResolver::new(vec![root.0.clone()]);
        let absolute = root.0.join("outside.tga").to_string_lossy().into_owned();
        let cases = [
            ("parent", "../outside.tga"),
            ("backslash-parent", "..\\outside.tga"),
            ("rooted", "/outside.tga"),
            ("backslash-rooted", "\\outside.tga"),
            ("absolute", absolute.as_str()),
            ("drive-colon", "C:\\outside.tga"),
            ("colon", "atlas:alternate.tga"),
        ];

        for (label, page) in cases {
            let error = parsed_font_with_page("gfx/fonts/nested/probe.fnt", page)
                .load_pages(&resolver)
                .expect_err(label);
            assert!(
                error.message.contains("unsafe declared path"),
                "{label}: {error}"
            );
        }
    }

    #[test]
    fn strips_hoi4_color_codes() {
        assert_eq!(
            strip_color_codes("\u{00a7}Y12\u{00a7}! / \u{00a7}R0\u{00a7}!"),
            "12 / 0"
        );
    }

    #[test]
    fn maps_hoi4_color_codes_to_per_glyph_tints() {
        let doc = crate::paradox_text::parse_str(
            "bitmapfonts = { textcolors = { G = { 0 159 3 } Y = { 255 189 0 } R = { 255 50 50 } } }",
        );
        let table = crate::markup::TextColorTable::from_core_gfx_value(&doc.root)
            .expect("shipped-shape textcolors");
        let default = [221, 214, 190, 255];
        let colorized = colorize_with_table(
            "\u{00a7}GFull\u{00a7}! \u{00a7}Y74%\u{00a7}! \u{00a7}R0\u{00a7}!",
            default,
            Some(&table),
        );
        assert_eq!(colorized.text, "Full 74% 0");
        assert_eq!(colorized.colors[0], [0, 159, 3, 255]);
        assert_eq!(colorized.colors[5], [255, 189, 0, 255]);
        assert_eq!(colorized.colors[9], [255, 50, 50, 255]);
    }

    #[test]
    fn parses_text_char_line() {
        let fields = parse_kv_fields(
            r#"id=65 x=1 y=2 width=3 height=4 xoffset=-1 yoffset=6 xadvance=7 page=0 chnl=15"#,
        );
        let ch = parse_text_char(&fields, "synthetic.fnt").unwrap();
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

    #[test]
    fn parses_synthetic_binary_bmfont() {
        let mut bytes = b"BMF\x03".to_vec();
        let mut info = vec![0u8; 14];
        info[0..2].copy_from_slice(&18i16.to_le_bytes());
        info[7..11].copy_from_slice(&[2, 3, 4, 5]);
        info[11..13].copy_from_slice(&[1, 2]);
        info[13] = 1;
        bytes.extend_from_slice(&bmf_block(1, &info));
        let mut common = vec![0u8; 15];
        common[0..2].copy_from_slice(&20u16.to_le_bytes());
        common[2..4].copy_from_slice(&16u16.to_le_bytes());
        common[4..6].copy_from_slice(&256u16.to_le_bytes());
        common[6..8].copy_from_slice(&128u16.to_le_bytes());
        common[8..10].copy_from_slice(&1u16.to_le_bytes());
        common[11] = 0;
        common[12] = 4;
        common[13] = 4;
        common[14] = 4;
        bytes.extend_from_slice(&bmf_block(2, &common));
        bytes.extend_from_slice(&bmf_block(3, b"page_0.dds\0"));
        let mut char_rec = Vec::new();
        char_rec.extend_from_slice(&65u32.to_le_bytes());
        char_rec.extend_from_slice(&1u16.to_le_bytes());
        char_rec.extend_from_slice(&2u16.to_le_bytes());
        char_rec.extend_from_slice(&3u16.to_le_bytes());
        char_rec.extend_from_slice(&4u16.to_le_bytes());
        char_rec.extend_from_slice(&(-1i16).to_le_bytes());
        char_rec.extend_from_slice(&6i16.to_le_bytes());
        char_rec.extend_from_slice(&7i16.to_le_bytes());
        char_rec.push(0);
        char_rec.push(15);
        bytes.extend_from_slice(&bmf_block(4, &char_rec));
        let mut kerning_rec = Vec::new();
        kerning_rec.extend_from_slice(&65u32.to_le_bytes());
        kerning_rec.extend_from_slice(&66u32.to_le_bytes());
        kerning_rec.extend_from_slice(&(-2i16).to_le_bytes());
        bytes.extend_from_slice(&bmf_block(5, &kerning_rec));

        let parsed = parse_binary_bmfont("synthetic", "synthetic.fnt", &bytes).unwrap();
        assert_eq!(parsed.info_size, 18);
        assert_eq!(parsed.info_padding, [2, 3, 4, 5]);
        assert_eq!(parsed.info_spacing, [1, 2]);
        assert_eq!(parsed.outline, 1);
        assert_eq!(parsed.line_height, 20);
        assert_eq!(parsed.base, 16);
        assert_eq!(parsed.scale_w, 256);
        assert_eq!(parsed.scale_h, 128);
        assert_eq!(parsed.glyph_channel, Some(3));
        assert_eq!(
            parsed.page_files,
            BTreeMap::from([(0, "page_0.dds".to_string())])
        );
        let ch = parsed.chars.get(&65).expect("char 65 parsed");
        assert_eq!((ch.x, ch.y, ch.width, ch.height), (1, 2, 3, 4));
        assert_eq!((ch.xoffset, ch.yoffset, ch.xadvance), (-1, 6, 7));
        assert_eq!((ch.page, ch.chnl), (0, 15));
        assert_eq!(parsed.kernings, BTreeMap::from([((65, 66), -2)]));
    }

    #[test]
    fn binary_bmfont_truncated_header_is_rejected() {
        let err = parse_binary_bmfont("f", "f.fnt", b"BMF").expect_err("must reject");
        assert!(err.message.contains("header is truncated"), "{err}");
    }

    #[test]
    fn binary_bmfont_truncated_block_header_is_rejected() {
        let bytes = b"BMF\x03\x01\x04\x00".to_vec();
        let err = parse_binary_bmfont("f", "f.fnt", &bytes).expect_err("must reject");
        assert!(err.message.contains("block header is truncated"), "{err}");
    }

    #[test]
    fn binary_bmfont_truncated_block_body_is_rejected() {
        let mut bytes = b"BMF\x03".to_vec();
        bytes.push(1);
        bytes.extend_from_slice(&10u32.to_le_bytes());
        bytes.extend_from_slice(&[0u8; 5]);
        let err = parse_binary_bmfont("f", "f.fnt", &bytes).expect_err("must reject");
        assert!(err.message.contains("block 1 is truncated"), "{err}");
    }

    #[test]
    fn binary_bmfont_invalid_chars_block_length_is_rejected() {
        let mut bytes = b"BMF\x03".to_vec();
        bytes.extend_from_slice(&bmf_block(4, &[0u8; 19]));
        let err = parse_binary_bmfont("f", "f.fnt", &bytes).expect_err("must reject");
        assert!(
            err.message.contains("chars block has invalid length"),
            "{err}"
        );
    }

    #[test]
    fn binary_bmfont_invalid_kernings_block_length_is_rejected() {
        let mut bytes = b"BMF\x03".to_vec();
        bytes.extend_from_slice(&bmf_block(5, &[0u8; 9]));
        let err = parse_binary_bmfont("f", "f.fnt", &bytes).expect_err("must reject");
        assert!(
            err.message.contains("kernings block has invalid length"),
            "{err}"
        );
    }

    fn text_fnt(chars: &str) -> String {
        format!(
            "info face=\"T\" size=16 padding=0,0,0,0 spacing=0,0 outline=0\n\
             common lineHeight=16 base=12 scaleW=64 scaleH=64 pages=1\n\
             page id=0 file=\"atlas.tga\"\n{chars}"
        )
    }

    #[test]
    fn unbounded_text_glyph_rect_is_a_typed_parse_refusal() {
        for field in ["x", "y", "width", "height"] {
            let mut row =
                "char id=65 x=0 y=0 width=4 height=4 xoffset=0 yoffset=0 xadvance=4 page=0\n"
                    .to_string();
            row = row.replace(&format!("{field}=0 "), &format!("{field}={} ", usize::MAX));
            row = row.replace(&format!("{field}=4 "), &format!("{field}={} ", usize::MAX));
            let bytes = text_fnt(&row);
            let err = parse_bmfont_bytes("probe", "probe.fnt", bytes.as_bytes())
                .unwrap_err_or_else(field);
            assert!(err.message.contains("glyph-extent bound"), "{field}: {err}");
        }
    }

    trait UnwrapErrOrElse {
        fn unwrap_err_or_else(self, label: &str) -> GameUiError;
    }

    impl<T: std::fmt::Debug> UnwrapErrOrElse for Result<T> {
        fn unwrap_err_or_else(self, label: &str) -> GameUiError {
            match self {
                Ok(value) => panic!("{label} must be refused, got {value:?}"),
                Err(err) => err,
            }
        }
    }

    #[test]
    fn oversized_glyph_rect_clamps_to_the_atlas_instead_of_iterating_it() {
        let mut page_pixels = vec![0u8; 4 * 4 * 4];
        for pixel in page_pixels.chunks_exact_mut(4) {
            pixel.copy_from_slice(&[255, 255, 255, 255]);
        }
        let page = RgbaImage {
            width: 4,
            height: 4,
            pixels: page_pixels,
        };
        let huge = FontChar {
            id: 65,
            x: 0,
            y: 0,
            width: 65_535,
            height: 65_535,
            xoffset: 0,
            yoffset: 0,
            xadvance: 4,
            page: 0,
            chnl: 15,
        };
        let exact = FontChar {
            width: 4,
            height: 4,
            ..huge.clone()
        };

        let mut from_huge = blank_image(8, 8);
        blit_glyph(&mut from_huge, &page, 1, 1, &huge, [10, 20, 30, 255], None);
        let mut from_exact = blank_image(8, 8);
        blit_glyph(
            &mut from_exact,
            &page,
            1,
            1,
            &exact,
            [10, 20, 30, 255],
            None,
        );
        assert_eq!(from_huge.pixels, from_exact.pixels);

        assert_eq!(glyph_axis_span(-2, 0, 65_535, 4, 8), 2..4);
        assert_eq!(glyph_axis_span(0, 9, 65_535, 4, 8), 0..0);
        assert_eq!(glyph_axis_span(7, 0, 65_535, 4, 8), 0..1);
        assert_eq!(glyph_axis_span(0, 0, usize::MAX, 4, 8), 0..4);
        assert_eq!(glyph_axis_span(i32::MIN, 0, usize::MAX, 4, 8), 0..0);
        assert_eq!(glyph_axis_span(i32::MAX, 0, usize::MAX, 4, 8), 0..0);
    }

    #[test]
    fn extreme_signed_metrics_saturate_instead_of_overflowing_the_pen() {
        // release profile's overflow-checks.
        let mut font = test_font_for_date();
        for font_char in font.chars.values_mut() {
            font_char.xadvance = i32::MAX;
            font_char.xoffset = i32::MAX;
            font_char.yoffset = i32::MAX;
        }
        font.line_height = i32::MAX;
        font.base = i32::MAX;
        font.kernings.insert(('1' as u32, '9' as u32), i32::MIN);

        let layout = font.layout_text(i32::MAX - 1, i32::MAX - 1, "99\n11");
        assert_eq!(layout.height, i32::MAX);
        assert!(layout.width >= 0, "{}", layout.width);
        assert!(layout.quads.iter().all(|quad| quad.x == i32::MAX));
        assert!(layout.quads.iter().all(|quad| quad.baseline_y == i32::MAX));

        let layout = font.layout_text(i32::MIN, i32::MIN, "99\n11");
        assert!(layout.width >= 0, "{}", layout.width);

        let mut image = blank_image(8, 8);
        for align in [TextAlign::Left, TextAlign::Center, TextAlign::Right] {
            font.draw_text(
                &mut image,
                i32::MIN,
                i32::MIN,
                Some(i32::MAX),
                align,
                "99\n11",
                [255; 4],
            );
        }
    }

    #[test]
    fn text_bmfont_needs_info_common_and_at_least_one_glyph() {
        let no_glyphs = text_fnt("");
        let err = parse_bmfont_bytes("probe", "probe.fnt", no_glyphs.as_bytes())
            .unwrap_err_or_else("zero-glyph font");
        assert!(err.message.contains("declares no glyphs"), "{err}");

        let no_common = "info face=\"T\" size=16\nchar id=65 x=0 y=0 width=1 height=1 xoffset=0 yoffset=0 xadvance=1\n";
        let err = parse_bmfont_bytes("probe", "probe.fnt", no_common.as_bytes())
            .unwrap_err_or_else("no common block");
        assert!(err.message.contains("no common block"), "{err}");

        let no_info = "common lineHeight=16 base=12 scaleW=64 scaleH=64 pages=1\nchar id=65 x=0 y=0 width=1 height=1 xoffset=0 yoffset=0 xadvance=1\n";
        let err = parse_bmfont_bytes("probe", "probe.fnt", no_info.as_bytes())
            .unwrap_err_or_else("no info block");
        assert!(err.message.contains("no info block"), "{err}");

        let err = parse_bmfont_bytes("probe", "probe.fnt", b"hello world\n")
            .unwrap_err_or_else("non-BMFont text");
        assert!(err.message.contains("not a BMFont file"), "{err}");
    }

    #[test]
    fn text_bmfont_malformed_char_row_is_a_refusal_not_a_dropped_glyph() {
        let row = "char id=65 x=0 y=0 width=4 height=4 yoffset=0 xadvance=4 page=0\n";
        let bytes = text_fnt(row);
        let err = parse_bmfont_bytes("probe", "probe.fnt", bytes.as_bytes())
            .unwrap_err_or_else("char row missing xoffset");
        assert!(err.message.contains("no usable xoffset field"), "{err}");
    }

    #[test]
    fn binary_bmfont_header_only_file_is_rejected() {
        let err = parse_binary_bmfont("f", "f.fnt", b"BMF\x03").unwrap_err_or_else("header-only");
        assert!(err.message.contains("no info block"), "{err}");
    }

    #[test]
    fn binary_bmfont_unsupported_version_is_rejected() {
        let err = parse_binary_bmfont("f", "f.fnt", b"BMF\x02").unwrap_err_or_else("version 2");
        assert!(err.message.contains("unsupported version 2"), "{err}");
    }

    #[test]
    fn binary_bmfont_undersized_info_and_common_blocks_are_rejected() {
        let mut bytes = b"BMF\x03".to_vec();
        bytes.extend_from_slice(&bmf_block(1, &[0u8; 13]));
        let err = parse_binary_bmfont("f", "f.fnt", &bytes).unwrap_err_or_else("short info");
        assert!(err.message.contains("info block is 13 bytes"), "{err}");

        let mut bytes = b"BMF\x03".to_vec();
        bytes.extend_from_slice(&bmf_block(1, &[0u8; 14]));
        bytes.extend_from_slice(&bmf_block(2, &[0u8; 14]));
        let err = parse_binary_bmfont("f", "f.fnt", &bytes).unwrap_err_or_else("short common");
        assert!(err.message.contains("common block is 14 bytes"), "{err}");
    }

    #[test]
    fn glyphs_referencing_an_unloaded_page_are_refused_at_load() {
        let root = FontFixtureRoot::new("dangling_page");
        root.write_tga("gfx/fonts/nested/atlas.tga", [10, 20, 30, 255]);
        let resolver = AssetResolver::new(vec![root.0.clone()]);
        let mut parsed = parsed_font_with_page("gfx/fonts/nested/probe.fnt", "atlas.tga");
        parsed.chars.insert(
            65,
            FontChar {
                id: 65,
                x: 0,
                y: 0,
                width: 1,
                height: 1,
                xoffset: 0,
                yoffset: 0,
                xadvance: 1,
                page: 7,
                chnl: 15,
            },
        );
        let err = parsed
            .load_pages(&resolver)
            .unwrap_err_or_else("glyph on an undeclared page");
        assert!(err.message.contains("glyph 65 -> page 7"), "{err}");
    }

    #[test]
    fn runtime_glyph_draw_separates_absent_glyph_from_absent_page() {
        let mut font = test_font_for_date();
        let mut image = blank_image(8, 8);
        assert_eq!(
            font.draw_runtime_glyph(&mut image, '9' as u32, 0, 0, [255; 4], None),
            RuntimeGlyphDraw::Drawn
        );
        assert_eq!(
            font.draw_runtime_glyph(&mut image, 0x4e00, 0, 0, [255; 4], None),
            RuntimeGlyphDraw::GlyphAbsent
        );
        font.pages.clear();
        assert_eq!(
            font.draw_runtime_glyph(&mut image, '9' as u32, 0, 0, [255; 4], None),
            RuntimeGlyphDraw::PageAbsent { page: 0 }
        );
    }

    #[test]
    fn core_gfx_alias_law_is_one_law() {
        let doc = crate::paradox_text::parse_str(
            "bitmapfonts = { bitmapfont = { name = \"Main_14\" path = \"gfx/fonts/garamond_14\" } }",
        );
        assert_eq!(
            core_gfx_font_alias(&doc, "Main_14").unwrap(),
            Some("gfx/fonts/garamond_14.fnt".to_string())
        );
        assert_eq!(core_gfx_font_alias(&doc, "Absent").unwrap(), None);

        let doc = crate::paradox_text::parse_str(
            "bitmapfonts = { \
               bitmapfont = { name = \"Main_14\" path = \"gfx/fonts/garamond_14\" color = 0xffffffff } \
               bitmapfont_override = { name = \"Main_14\" \
                 fontfiles = { \"gfx/fonts/japanese/garamond_14\" } \
                 languages = { \"l_japanese\" } } }",
        );
        assert_eq!(
            core_gfx_font_alias(&doc, "Main_14").unwrap(),
            Some("gfx/fonts/garamond_14.fnt".to_string())
        );

        let doc = crate::paradox_text::parse_str(
            "bitmapfonts = { bitmapfont = { name = \"Main_14\" path = \"gfx/fonts/garamond_14\" } \
             bitmapfont_override = { name = \"Main_14\" path = \"gfx/fonts/mod_14.fnt\" } }",
        );
        assert!(core_gfx_font_alias(&doc, "Main_14")
            .unwrap_err()
            .message
            .contains("registration-population law is unproved"));

        let doc = crate::paradox_text::parse_str(
            "bitmapfonts = { bitmapfont = { name = \"Main_14\" path = \"gfx/fonts/garamond_14\" } \
             bitmapfont_override = { name = \"Main_14\" path = \"gfx/fonts/jp_14.fnt\" \
               languages = { \"l_japanese\" } } }",
        );
        assert_eq!(
            core_gfx_font_alias(&doc, "Main_14").unwrap(),
            Some("gfx/fonts/garamond_14.fnt".to_string())
        );
        assert!(
            core_gfx_font_alias_for_language(&doc, "Main_14", "l_japanese")
                .unwrap_err()
                .message
                .contains("registration-population law is unproved")
        );
        let doc = crate::paradox_text::parse_str(
            "bitmapfonts = { bitmapfont = { name = \"Main_14\" path = \"gfx/fonts/garamond_14\" } \
             bitmapfont_override = { name = \"Main_14\" path = \"gfx/fonts/en_14.fnt\" \
               languages = { \"l_english\" } } }",
        );
        assert!(core_gfx_font_alias(&doc, "Main_14")
            .unwrap_err()
            .message
            .contains("registration-population law is unproved"));

        let doc = crate::paradox_text::parse_str(
            "bitmapfonts = { bitmapfont = { name = \"Main_14\" path = \"gfx/fonts/a\" } \
             bitmapfont = { name = \"Main_14\" path = \"gfx/fonts/b\" } }",
        );
        let err = core_gfx_font_alias(&doc, "Main_14").unwrap_err_or_else("ambiguous alias");
        assert!(err.message.contains("2 unresolved base aliases"), "{err}");

        let doc = crate::paradox_text::parse_str(
            "bitmapfonts = { bitmapfont = { name = \"Main_14\" path = \"gfx/fonts/base\" } \
             bitmapfont_override = { name = \"Main_14\" path = \"gfx/fonts/wrong_case\" \
               languages = { \"l_ENGLISH\" } } }",
        );
        assert_eq!(
            core_gfx_font_alias_for_language(&doc, "Main_14", "l_english").unwrap(),
            Some("gfx/fonts/base.fnt".to_string())
        );

        for malformed in [
            "languages = l_english",
            "languages = { selected = l_english }",
            "languages = { }",
        ] {
            let doc = crate::paradox_text::parse_str(&format!(
                "bitmapfonts = {{ bitmapfont = {{ name = \"Main_14\" path = \"gfx/fonts/base\" }} \
                 bitmapfont_override = {{ name = \"Main_14\" {malformed} }} }}"
            ));
            assert!(
                core_gfx_font_alias_for_language(&doc, "Main_14", "l_english")
                    .unwrap_err()
                    .message
                    .contains("unlabeled scalar list")
            );
        }
    }

    fn bmf_block(block_type: u8, body: &[u8]) -> Vec<u8> {
        let mut out = vec![block_type];
        out.extend_from_slice(&(body.len() as u32).to_le_bytes());
        out.extend_from_slice(body);
        out
    }

    #[test]
    fn layout_text_breaks_on_cr_and_reports_other_unmapped_controls() {
        let font = test_font_for_date();
        assert!(
            !font.chars.contains_key(&('\r' as u32)),
            "the probe font must not map 0x0D, or this pins nothing"
        );
        let layout = font.layout_text(0, 0, "1\r9");
        assert!(
            layout.missing_glyphs.is_empty(),
            "a line-break control is consumed before glyph lookup: {:?}",
            layout.missing_glyphs
        );
        assert_ne!(
            layout.quads[0].pen_y, layout.quads[1].pen_y,
            "0x0D must advance exactly as 0x0A does"
        );

        let unmapped = '\u{0001}';
        assert!(!font.chars.contains_key(&(unmapped as u32)));
        let refused = font.layout_text(0, 0, &format!("1{unmapped}9"));
        assert_eq!(
            refused.missing_glyphs,
            BTreeSet::from([unmapped]),
            "a non-break control the font cannot map must be visible to the \
             product route's missing-glyph refusal"
        );
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
            color_table: None,
            authored_base_color_rgba8: None,
            authored_base_border_color_rgba8: None,
        }
    }
}
