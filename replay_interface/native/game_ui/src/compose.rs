use crate::assets::{AssetResolver, DecodedSprite};
use crate::dds::RgbaImage;
use crate::font::{alpha_blend, blank_image, BitmapFont, TextAlign};
use crate::gfx::{Size, SpriteDefinition, SpriteRegistry};
use crate::gui::{GuiElement, Measure, MeasurePair};
use crate::loc::{is_localisation_key, Localisation};
use crate::paradox_text::{scalar_f32, scalar_i32, scalar_u32, Value};
use crate::{GameUiError, Result};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Debug, PartialEq)]
pub struct ComposeOptions {
    pub viewport_width: usize,
    pub viewport_height: usize,
    pub root_width: usize,
    pub root_height: usize,
    pub text_color: [u8; 4],
    pub text_overrides: BTreeMap<String, String>,
    pub localisation: BTreeMap<String, String>,
    pub sprite_overrides: BTreeMap<String, RgbaImage>,
    pub frame_overrides: BTreeMap<String, usize>,
    pub progress_overrides: BTreeMap<String, f32>,
    pub hidden_elements: BTreeSet<String>,
}

impl ComposeOptions {
    pub fn new(viewport_width: usize, viewport_height: usize) -> Self {
        Self {
            viewport_width,
            viewport_height,
            root_width: viewport_width,
            root_height: viewport_height,
            text_color: [221, 214, 190, 255],
            text_overrides: BTreeMap::new(),
            localisation: BTreeMap::new(),
            sprite_overrides: BTreeMap::new(),
            frame_overrides: BTreeMap::new(),
            progress_overrides: BTreeMap::new(),
            hidden_elements: BTreeSet::new(),
        }
    }

    pub fn with_root_size(mut self, width: usize, height: usize) -> Self {
        self.root_width = width.max(1);
        self.root_height = height.max(1);
        self
    }

    pub fn with_localisation(mut self, localisation: &Localisation) -> Self {
        self.localisation = localisation.entries.clone();
        self
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ComposeReport {
    pub drawn_elements: usize,
    pub missing_sprites: BTreeSet<String>,
    pub undecodable_sprites: Vec<String>,
    pub missing_fonts: BTreeSet<String>,
    pub missing_glyphs: BTreeMap<String, BTreeSet<char>>,
    pub unresolved_loc_keys: BTreeSet<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ComposeOutput {
    pub image: RgbaImage,
    pub report: ComposeReport,
    pub debug_rects: Vec<DebugRect>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DebugRect {
    pub name: String,
    pub element_type: String,
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
    pub depth: usize,
}

#[derive(Clone, Debug)]
pub struct Compositor<'a> {
    resolver: &'a AssetResolver,
    registry: &'a SpriteRegistry,
    sprite_cache: BTreeMap<String, Result<DecodedSprite>>,
    texture_cache: BTreeMap<String, Result<RgbaImage>>,
    font_cache: BTreeMap<String, Result<BitmapFont>>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rect {
    x: i32,
    y: i32,
    w: i32,
    h: i32,
}

impl Rect {
    pub fn new(x: i32, y: i32, w: i32, h: i32) -> Self {
        Self { x, y, w, h }
    }

    pub fn x(&self) -> i32 {
        self.x
    }

    pub fn y(&self) -> i32 {
        self.y
    }

    pub fn w(&self) -> i32 {
        self.w
    }

    pub fn h(&self) -> i32 {
        self.h
    }
}

impl<'a> Compositor<'a> {
    pub fn new(resolver: &'a AssetResolver, registry: &'a SpriteRegistry) -> Self {
        Self {
            resolver,
            registry,
            sprite_cache: BTreeMap::new(),
            texture_cache: BTreeMap::new(),
            font_cache: BTreeMap::new(),
        }
    }

    pub fn compose(&mut self, root: &GuiElement, options: &ComposeOptions) -> ComposeOutput {
        let mut image = blank_image(options.viewport_width, options.viewport_height);
        let mut report = ComposeReport::default();
        let mut debug_rects = Vec::new();
        let parent = Rect {
            x: 0,
            y: 0,
            w: options.root_width as i32,
            h: options.root_height as i32,
        };
        self.draw_element(
            root,
            parent,
            0,
            &mut image,
            options,
            &mut report,
            &mut debug_rects,
        );
        ComposeOutput {
            image,
            report,
            debug_rects,
        }
    }

    fn draw_element(
        &mut self,
        element: &GuiElement,
        parent: Rect,
        depth: usize,
        image: &mut RgbaImage,
        options: &ComposeOptions,
        report: &mut ComposeReport,
        debug_rects: &mut Vec<DebugRect>,
    ) {
        if options.hidden_elements.contains(&element.name) || is_hidden(element) {
            return;
        }
        let natural = self.natural_size(element);
        let rect = resolve_element_rect(element, parent, natural, options);
        debug_rects.push(DebugRect {
            name: element.name.clone(),
            element_type: element.element_type.clone(),
            x: rect.x,
            y: rect.y,
            w: rect.w,
            h: rect.h,
            depth,
        });

        if let Some(override_image) = options.sprite_overrides.get(&element.name) {
            draw_image(image, override_image, rect.x, rect.y);
            report.drawn_elements += 1;
        } else if element
            .element_type
            .eq_ignore_ascii_case("instantTextBoxType")
        {
            self.draw_text(element, rect, image, options, report);
        } else if let Some(sprite_name) = primary_sprite_ref(element) {
            self.draw_sprite(element, sprite_name, rect, image, options, report);
        }

        for child in &element.children {
            self.draw_element(child, rect, depth + 1, image, options, report, debug_rects);
        }
    }

    fn draw_text(
        &mut self,
        element: &GuiElement,
        rect: Rect,
        image: &mut RgbaImage,
        options: &ComposeOptions,
        report: &mut ComposeReport,
    ) {
        let has_override = options.text_overrides.contains_key(&element.name);
        let mut text = options
            .text_overrides
            .get(&element.name)
            .cloned()
            .or_else(|| element.text.clone())
            .unwrap_or_default();
        if text.is_empty() {
            return;
        }
        if !has_override && is_localisation_key(&text) {
            let key = text.trim().trim_matches('$');
            if let Some(localised) = options.localisation.get(key) {
                text = localised.clone();
            } else {
                report.unresolved_loc_keys.insert(key.to_string());
                return;
            }
        }
        let Some(font_name) = element.font.as_deref() else {
            return;
        };
        let font = match self.font(font_name) {
            Ok(font) => font.clone(),
            Err(err) => {
                report
                    .missing_fonts
                    .insert(format!("{font_name}: {}", err.message));
                return;
            }
        };
        let max_width = scalar_i32_ci(&element.raw, "maxWidth").or_else(|| {
            if rect.w > 0 {
                Some(rect.w)
            } else {
                None
            }
        });
        let align = TextAlign::parse(element.format.as_deref());
        let text_report = font.draw_text(
            image,
            rect.x,
            rect.y,
            max_width,
            align,
            &text,
            options.text_color,
        );
        if !text_report.missing_glyphs.is_empty() {
            report
                .missing_glyphs
                .entry(element.name.clone())
                .or_default()
                .extend(text_report.missing_glyphs);
        }
        report.drawn_elements += 1;
    }

    fn draw_sprite(
        &mut self,
        element: &GuiElement,
        sprite_name: &str,
        rect: Rect,
        image: &mut RgbaImage,
        options: &ComposeOptions,
        report: &mut ComposeReport,
    ) {
        let Some(sprite) = self.registry.get(sprite_name) else {
            report.missing_sprites.insert(sprite_name.to_string());
            return;
        };
        let draw_result = if is_progress_bar(sprite) {
            self.draw_progress_bar(element, sprite, rect, image, options)
        } else {
            self.draw_plain_sprite(element, sprite, rect, image, options)
        };
        match draw_result {
            Ok(did_draw) => {
                if did_draw {
                    report.drawn_elements += 1;
                }
            }
            Err(err) => report
                .undecodable_sprites
                .push(format!("{sprite_name}: {}", err.message)),
        }
    }

    fn draw_plain_sprite(
        &mut self,
        element: &GuiElement,
        sprite: &SpriteDefinition,
        rect: Rect,
        image: &mut RgbaImage,
        options: &ComposeOptions,
    ) -> Result<bool> {
        let decoded = self.decoded_sprite(&sprite.name)?.clone();
        let frame_index = frame_index(element, options, decoded.frames.len());
        let frame = decoded
            .frames
            .get(frame_index)
            .or_else(|| decoded.frames.first())
            .ok_or_else(|| GameUiError::missing(format!("sprite {} has no frames", sprite.name)))?;
        let target = target_size(rect, frame.width, frame.height);
        let scale = scalar_f32_ci(&element.raw, "scale").unwrap_or(1.0);
        let target_w = ((target.0 as f32) * scale).round().max(1.0) as usize;
        let target_h = ((target.1 as f32) * scale).round().max(1.0) as usize;
        if let Some(border) = &sprite.border_size {
            let (bx, by) = parse_sprite_size(border).unwrap_or((0, 0));
            draw_nine_slice(image, frame, rect.x, rect.y, target_w, target_h, bx, by);
        } else if target_w != frame.width || target_h != frame.height {
            draw_scaled(image, frame, rect.x, rect.y, target_w, target_h);
        } else {
            draw_image(image, frame, rect.x, rect.y);
        }
        Ok(true)
    }

    fn draw_progress_bar(
        &mut self,
        element: &GuiElement,
        sprite: &SpriteDefinition,
        rect: Rect,
        image: &mut RgbaImage,
        options: &ComposeOptions,
    ) -> Result<bool> {
        let Some(fg_rel) = sprite.texture_files.first().map(|slot| slot.path.clone()) else {
            return Err(GameUiError::missing(format!(
                "progress bar {} has no foreground texture",
                sprite.name
            )));
        };
        let bg_rel = sprite
            .texture_files
            .get(1)
            .map(|slot| slot.path.clone())
            .unwrap_or_else(|| fg_rel.clone());
        let fg = self.decoded_texture(&fg_rel)?.clone();
        let bg = self.decoded_texture(&bg_rel)?.clone();
        let size = sprite
            .size
            .as_ref()
            .and_then(parse_sprite_size)
            .unwrap_or((fg.width, fg.height));
        let target = target_size(rect, size.0, size.1);
        draw_scaled(image, &bg, rect.x, rect.y, target.0, target.1);
        let progress = options
            .progress_overrides
            .get(&element.name)
            .copied()
            .unwrap_or_else(|| scalar_u32_ci(&element.raw, "frame").unwrap_or(100) as f32 / 100.0)
            .clamp(0.0, 1.0);
        let fill_w = ((target.0 as f32) * progress).round() as usize;
        if fill_w > 0 {
            draw_scaled_crop(image, &fg, rect.x, rect.y, target.0, target.1, fill_w);
        }
        Ok(true)
    }

    fn natural_size(&mut self, element: &GuiElement) -> (usize, usize) {
        if let Some(size) = &element.size {
            return (
                measure_to_pixels(&size.x, 0).max(0) as usize,
                measure_to_pixels(&size.y, 0).max(0) as usize,
            );
        }
        if let Some(override_image) = self_override_hint(element) {
            return override_image;
        }
        let Some(sprite_name) = primary_sprite_ref(element) else {
            return (0, 0);
        };
        let Some(sprite) = self.registry.get(sprite_name) else {
            return (0, 0);
        };
        if let Some(size) = &sprite.size {
            if let Some(parsed) = parse_sprite_size(size) {
                return parsed;
            }
        }
        match self.decoded_sprite(&sprite.name) {
            Ok(decoded) => decoded
                .frames
                .first()
                .map(|frame| (frame.width, frame.height))
                .unwrap_or((0, 0)),
            Err(_) => (0, 0),
        }
    }

    fn decoded_sprite(&mut self, sprite_name: &str) -> Result<&DecodedSprite> {
        if !self.sprite_cache.contains_key(sprite_name) {
            let value = self
                .registry
                .get(sprite_name)
                .ok_or_else(|| GameUiError::missing(format!("missing sprite {sprite_name}")))
                .and_then(|sprite| self.resolver.decode_sprite(sprite));
            self.sprite_cache.insert(sprite_name.to_string(), value);
        }
        self.sprite_cache
            .get(sprite_name)
            .expect("cache populated")
            .as_ref()
            .map_err(Clone::clone)
    }

    fn decoded_texture(&mut self, rel: &str) -> Result<&RgbaImage> {
        if !self.texture_cache.contains_key(rel) {
            let value = self
                .resolver
                .decode_texture(rel)
                .map(|decoded| decoded.image);
            self.texture_cache.insert(rel.to_string(), value);
        }
        self.texture_cache
            .get(rel)
            .expect("cache populated")
            .as_ref()
            .map_err(Clone::clone)
    }

    fn font(&mut self, font_name: &str) -> Result<&BitmapFont> {
        if !self.font_cache.contains_key(font_name) {
            let value = BitmapFont::load(self.resolver, font_name);
            self.font_cache.insert(font_name.to_string(), value);
        }
        self.font_cache
            .get(font_name)
            .expect("cache populated")
            .as_ref()
            .map_err(Clone::clone)
    }
}

fn self_override_hint(_element: &GuiElement) -> Option<(usize, usize)> {
    None
}

fn is_hidden(element: &GuiElement) -> bool {
    element
        .raw
        .first_scalar_ci("hide")
        .is_some_and(|value| matches!(value.to_ascii_lowercase().as_str(), "yes" | "true"))
}

fn primary_sprite_ref(element: &GuiElement) -> Option<&str> {
    for key in ["quadTextureSprite", "spriteType", "SpriteType"] {
        if let Some(value) = element.raw.first_scalar_ci(key) {
            return Some(value);
        }
    }
    element.sprite_refs.first().map(String::as_str)
}

fn is_progress_bar(sprite: &SpriteDefinition) -> bool {
    sprite.kind.eq_ignore_ascii_case("progressbartype")
        || sprite.kind.eq_ignore_ascii_case("progressBarType")
        || sprite.texture_files.len() >= 2
            && sprite
                .effect_file
                .as_deref()
                .is_some_and(|effect| effect.to_ascii_lowercase().contains("progress_startend"))
}

pub fn resolve_element_rect(
    element: &GuiElement,
    parent: Rect,
    natural: (usize, usize),
    options: &ComposeOptions,
) -> Rect {
    let size = element.size.as_ref().map(|size| {
        (
            size_measure_to_pixels(&size.x, parent.w),
            size_measure_to_pixels(&size.y, parent.h),
        )
    });
    let (w, h) = size.unwrap_or((natural.0 as i32, natural.1 as i32));
    let (x_offset, y_offset) = element
        .position
        .as_ref()
        .map(|pos| {
            (
                measure_to_pixels(&pos.x, parent.w),
                measure_to_pixels(&pos.y, parent.h),
            )
        })
        .unwrap_or((0, 0));
    let orientation = element
        .orientation
        .as_deref()
        .unwrap_or("UPPER_LEFT")
        .to_ascii_uppercase();
    let anchor_x = if orientation.contains("RIGHT") {
        parent.x + parent.w
    } else if orientation.contains("CENTER") {
        parent.x + parent.w / 2
    } else {
        parent.x
    };
    let anchor_y = if orientation.contains("LOWER") || orientation.contains("BOTTOM") {
        parent.y + parent.h
    } else if orientation.contains("CENTER") && !orientation.contains("UPPER") {
        parent.y + parent.h / 2
    } else {
        parent.y
    };

    let mut rect = Rect {
        x: resolve_axis(
            parent.x,
            parent.w,
            anchor_x,
            x_offset,
            element.position.as_ref().map(|pos| &pos.x),
            orientation.contains("RIGHT"),
        ),
        y: resolve_axis(
            parent.y,
            parent.h,
            anchor_y,
            y_offset,
            element.position.as_ref().map(|pos| &pos.y),
            orientation.contains("LOWER") || orientation.contains("BOTTOM"),
        ),
        w,
        h,
    };
    if element.name.eq_ignore_ascii_case("topbar") && element.size.is_none() {
        rect.w = options.viewport_width as i32;
        rect.h = options.viewport_height as i32;
    }
    rect
}

fn resolve_axis(
    parent_start: i32,
    parent_size: i32,
    anchor: i32,
    offset: i32,
    measure: Option<&Measure>,
    from_end: bool,
) -> i32 {
    if from_end && measure.is_some_and(|measure| measure.is_percent) {
        parent_start + parent_size - offset
    } else {
        anchor + offset
    }
}

fn measure_to_pixels(measure: &Measure, parent: i32) -> i32 {
    if measure.is_percent {
        let value = measure
            .value
            .as_deref()
            .and_then(|value| value.parse::<f32>().ok())
            .unwrap_or(0.0);
        ((parent as f32) * value / 100.0).round() as i32
    } else {
        measure
            .raw
            .trim_end_matches('%')
            .parse::<f32>()
            .map(|value| value.round() as i32)
            .unwrap_or(0)
    }
}

fn size_measure_to_pixels(measure: &Measure, parent: i32) -> i32 {
    if measure.is_percent {
        return measure_to_pixels(measure, parent);
    }
    let value = measure
        .raw
        .trim_end_matches('%')
        .parse::<f32>()
        .map(|value| value.round() as i32)
        .unwrap_or(0);
    if value < 0 {
        (parent + value).max(0)
    } else {
        value
    }
}

pub fn measure_to_pixels_for_parent(measure: &Measure, parent: i32) -> i32 {
    measure_to_pixels(measure, parent)
}

fn target_size(rect: Rect, natural_w: usize, natural_h: usize) -> (usize, usize) {
    (
        if rect.w > 0 {
            rect.w as usize
        } else {
            natural_w
        },
        if rect.h > 0 {
            rect.h as usize
        } else {
            natural_h
        },
    )
}

fn frame_index(element: &GuiElement, options: &ComposeOptions, frame_count: usize) -> usize {
    if frame_count == 0 {
        return 0;
    }
    let frame = options
        .frame_overrides
        .get(&element.name)
        .copied()
        .or_else(|| {
            scalar_u32_ci(&element.raw, "frame").map(|frame| frame.saturating_sub(1) as usize)
        })
        .unwrap_or(0);
    frame.min(frame_count - 1)
}

fn parse_sprite_size(size: &Size) -> Option<(usize, usize)> {
    let x = size.x.parse::<f32>().ok()?.round().max(0.0) as usize;
    let y = size.y.parse::<f32>().ok()?.round().max(0.0) as usize;
    Some((x, y))
}

fn scalar_i32_ci(value: &Value, key: &str) -> Option<i32> {
    value.first_value_ci(key).and_then(scalar_i32)
}

fn scalar_u32_ci(value: &Value, key: &str) -> Option<u32> {
    value.first_value_ci(key).and_then(scalar_u32)
}

fn scalar_f32_ci(value: &Value, key: &str) -> Option<f32> {
    value.first_value_ci(key).and_then(scalar_f32)
}

pub fn draw_image(dst: &mut RgbaImage, src: &RgbaImage, x: i32, y: i32) {
    for sy in 0..src.height {
        let dy = y + sy as i32;
        if dy < 0 || dy >= dst.height as i32 {
            continue;
        }
        for sx in 0..src.width {
            let dx = x + sx as i32;
            if dx < 0 || dx >= dst.width as i32 {
                continue;
            }
            let src_offset = (sy * src.width + sx) * 4;
            blend_at(
                dst,
                dx as usize,
                dy as usize,
                src.pixels[src_offset..src_offset + 4]
                    .try_into()
                    .expect("slice length checked"),
            );
        }
    }
}

pub fn draw_rect_outline(dst: &mut RgbaImage, rect: Rect, rgba: [u8; 4]) {
    if rect.w <= 0 || rect.h <= 0 || dst.width == 0 || dst.height == 0 {
        return;
    }
    let x0 = rect.x.max(0);
    let y0 = rect.y.max(0);
    let x1 = (rect.x + rect.w - 1).min(dst.width as i32 - 1);
    let y1 = (rect.y + rect.h - 1).min(dst.height as i32 - 1);
    if x0 > x1 || y0 > y1 {
        return;
    }
    for x in x0..=x1 {
        blend_at(dst, x as usize, y0 as usize, rgba);
        blend_at(dst, x as usize, y1 as usize, rgba);
    }
    for y in y0..=y1 {
        blend_at(dst, x0 as usize, y as usize, rgba);
        blend_at(dst, x1 as usize, y as usize, rgba);
    }
}

pub fn fill_rect(dst: &mut RgbaImage, rect: Rect, rgba: [u8; 4]) {
    if rect.w <= 0 || rect.h <= 0 || dst.width == 0 || dst.height == 0 {
        return;
    }
    let x0 = rect.x.max(0);
    let y0 = rect.y.max(0);
    let x1 = (rect.x + rect.w).min(dst.width as i32);
    let y1 = (rect.y + rect.h).min(dst.height as i32);
    if x0 >= x1 || y0 >= y1 {
        return;
    }
    for y in y0..y1 {
        for x in x0..x1 {
            blend_at(dst, x as usize, y as usize, rgba);
        }
    }
}

pub fn draw_scaled(
    dst: &mut RgbaImage,
    src: &RgbaImage,
    x: i32,
    y: i32,
    target_w: usize,
    target_h: usize,
) {
    draw_scaled_crop(dst, src, x, y, target_w, target_h, target_w);
}

pub fn draw_scaled_crop(
    dst: &mut RgbaImage,
    src: &RgbaImage,
    x: i32,
    y: i32,
    target_w: usize,
    target_h: usize,
    visible_w: usize,
) {
    if target_w == 0 || target_h == 0 || visible_w == 0 || src.width == 0 || src.height == 0 {
        return;
    }
    let visible_w = visible_w.min(target_w);
    for dy_local in 0..target_h {
        let dy = y + dy_local as i32;
        if dy < 0 || dy >= dst.height as i32 {
            continue;
        }
        let sy = dy_local * src.height / target_h;
        for dx_local in 0..visible_w {
            let dx = x + dx_local as i32;
            if dx < 0 || dx >= dst.width as i32 {
                continue;
            }
            let sx = dx_local * src.width / target_w;
            let src_offset = (sy * src.width + sx) * 4;
            blend_at(
                dst,
                dx as usize,
                dy as usize,
                src.pixels[src_offset..src_offset + 4]
                    .try_into()
                    .expect("slice length checked"),
            );
        }
    }
}

pub fn draw_nine_slice(
    dst: &mut RgbaImage,
    src: &RgbaImage,
    x: i32,
    y: i32,
    target_w: usize,
    target_h: usize,
    border_x: usize,
    border_y: usize,
) {
    if border_x == 0 && border_y == 0 {
        draw_scaled(dst, src, x, y, target_w, target_h);
        return;
    }
    let bx = border_x.min(src.width / 2).min(target_w / 2);
    let by = border_y.min(src.height / 2).min(target_h / 2);
    let src_cols = [
        (0, bx),
        (bx, src.width.saturating_sub(2 * bx)),
        (src.width - bx, bx),
    ];
    let src_rows = [
        (0, by),
        (by, src.height.saturating_sub(2 * by)),
        (src.height - by, by),
    ];
    let dst_cols = [
        (0, bx),
        (bx, target_w.saturating_sub(2 * bx)),
        (target_w - bx, bx),
    ];
    let dst_rows = [
        (0, by),
        (by, target_h.saturating_sub(2 * by)),
        (target_h - by, by),
    ];

    for row in 0..3 {
        for col in 0..3 {
            let (sx, sw) = src_cols[col];
            let (sy, sh) = src_rows[row];
            let (dx, dw) = dst_cols[col];
            let (dy, dh) = dst_rows[row];
            if sw == 0 || sh == 0 || dw == 0 || dh == 0 {
                continue;
            }
            draw_region_scaled(
                dst,
                src,
                (sx, sy, sw, sh),
                x + dx as i32,
                y + dy as i32,
                dw,
                dh,
            );
        }
    }
}

fn draw_region_scaled(
    dst: &mut RgbaImage,
    src: &RgbaImage,
    region: (usize, usize, usize, usize),
    x: i32,
    y: i32,
    target_w: usize,
    target_h: usize,
) {
    let (src_x, src_y, src_w, src_h) = region;
    for dy_local in 0..target_h {
        let dy = y + dy_local as i32;
        if dy < 0 || dy >= dst.height as i32 {
            continue;
        }
        let sy = src_y + dy_local * src_h / target_h;
        for dx_local in 0..target_w {
            let dx = x + dx_local as i32;
            if dx < 0 || dx >= dst.width as i32 {
                continue;
            }
            let sx = src_x + dx_local * src_w / target_w;
            let src_offset = (sy * src.width + sx) * 4;
            blend_at(
                dst,
                dx as usize,
                dy as usize,
                src.pixels[src_offset..src_offset + 4]
                    .try_into()
                    .expect("slice length checked"),
            );
        }
    }
}

fn blend_at(dst: &mut RgbaImage, x: usize, y: usize, src: [u8; 4]) {
    let dst_offset = (y * dst.width + x) * 4;
    let dst_px = [
        dst.pixels[dst_offset],
        dst.pixels[dst_offset + 1],
        dst.pixels[dst_offset + 2],
        dst.pixels[dst_offset + 3],
    ];
    let out = alpha_blend(dst_px, src);
    dst.pixels[dst_offset..dst_offset + 4].copy_from_slice(&out);
}

#[allow(dead_code)]
fn _measure_pair_to_tuple(size: &MeasurePair, parent: Rect) -> (i32, i32) {
    (
        measure_to_pixels(&size.x, parent.w),
        measure_to_pixels(&size.y, parent.h),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gui::parse_measure;

    #[test]
    fn percent_measure_uses_parent_dimension() {
        assert_eq!(measure_to_pixels(&parse_measure("50%%"), 200), 100);
    }

    #[test]
    fn upper_right_anchor_offsets_from_parent_right() {
        let element = GuiElement {
            element_type: "iconType".to_string(),
            name: "x".to_string(),
            position: Some(MeasurePair {
                x: parse_measure("-35"),
                y: parse_measure("7"),
            }),
            size: None,
            orientation: Some("UPPER_RIGHT".to_string()),
            sprite_refs: Vec::new(),
            font: None,
            text: None,
            format: None,
            children: Vec::new(),
            raw: Value::Object(Vec::new()),
        };
        let rect = resolve_element_rect(
            &element,
            Rect {
                x: 0,
                y: 0,
                w: 1920,
                h: 108,
            },
            (30, 30),
            &ComposeOptions::new(1920, 108),
        );
        assert_eq!(rect.x, 1885);
        assert_eq!(rect.y, 7);
    }

    #[test]
    fn right_anchor_offsets_scale_with_viewport_widths() {
        let element = GuiElement {
            element_type: "buttonType".to_string(),
            name: "menu_button".to_string(),
            position: Some(MeasurePair {
                x: parse_measure("-35"),
                y: parse_measure("7"),
            }),
            size: None,
            orientation: Some("UPPER_RIGHT".to_string()),
            sprite_refs: Vec::new(),
            font: None,
            text: None,
            format: None,
            children: Vec::new(),
            raw: Value::Object(Vec::new()),
        };
        for width in [1920, 2560] {
            let rect = resolve_element_rect(
                &element,
                Rect {
                    x: 0,
                    y: 0,
                    w: width,
                    h: 1080,
                },
                (30, 30),
                &ComposeOptions::new(width as usize, 1080),
            );
            assert_eq!(rect.x, width - 35);
            assert_eq!(rect.y, 7);
        }
    }

    #[test]
    fn right_anchor_percent_position_is_measured_from_parent_right() {
        let element = GuiElement {
            element_type: "iconType".to_string(),
            name: "percent_right".to_string(),
            position: Some(MeasurePair {
                x: parse_measure("25%%"),
                y: parse_measure("0"),
            }),
            size: None,
            orientation: Some("UPPER_RIGHT".to_string()),
            sprite_refs: Vec::new(),
            font: None,
            text: None,
            format: None,
            children: Vec::new(),
            raw: Value::Object(Vec::new()),
        };
        let rect_1920 = resolve_element_rect(
            &element,
            Rect::new(0, 0, 1920, 1080),
            (30, 30),
            &ComposeOptions::new(1920, 1080),
        );
        let rect_2560 = resolve_element_rect(
            &element,
            Rect::new(0, 0, 2560, 1440),
            (30, 30),
            &ComposeOptions::new(2560, 1440),
        );
        assert_eq!(rect_1920.x, 1440);
        assert_eq!(rect_2560.x, 1920);
    }

    #[test]
    fn negative_size_fills_parent_minus_offset() {
        let element = GuiElement {
            element_type: "containerWindowType".to_string(),
            name: "fill".to_string(),
            position: None,
            size: Some(MeasurePair {
                x: parse_measure("-20"),
                y: parse_measure("-5"),
            }),
            orientation: None,
            sprite_refs: Vec::new(),
            font: None,
            text: None,
            format: None,
            children: Vec::new(),
            raw: Value::Object(Vec::new()),
        };
        let rect = resolve_element_rect(
            &element,
            Rect::new(0, 0, 550, 1000),
            (0, 0),
            &ComposeOptions::new(550, 1000),
        );
        assert_eq!(rect.w, 530);
        assert_eq!(rect.h, 995);
    }
}
