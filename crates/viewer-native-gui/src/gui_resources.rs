#[cfg(test)]
use viewer_runtime::geometry::contains;
use super::{
    lazy_icons::LazyFlags,
    native_flags,
    native_gui::{self, Layout, SpriteInfo, Texture},
    native_windows::NativeWindows,
    sprites::SpriteSheet,
};
use content_reader_gui::gui::native;
use std::collections::BTreeMap;

pub type PortraitTextures<'a> = &'a dyn Fn(&[u8]) -> Option<Texture>;

pub struct GuiAssets<'a> {
    pub gui: &'a crate::native_resources::GuiResources,
    pub windows: &'a NativeWindows,
    pub localisation: &'a content_reader_localisation::localisation::Localisation,
    pub flags: Option<GuiFlags<'a>>,
    pub physical_viewport: Option<[i32; 2]>,
    pub portrait_textures: Option<PortraitTextures<'a>>,
}

pub struct GuiFlags<'a> {
    pub source: &'a dyn native_flags::FlagSource,
    pub small: &'a LazyFlags,
    pub large: &'a LazyFlags,
}

impl native_gui::Resources for GuiAssets<'_> {
    fn physical_viewport(&self) -> Option<[i32; 2]> {
        self.physical_viewport
    }
    fn sprite(&self, name: &[u8]) -> Option<SpriteInfo> {
        self.gui.sprite(name)
    }

    fn sprite_info(&self, name: &[u8]) -> Option<std::borrow::Cow<'_, SpriteInfo>> {
        self.gui.sprite_info(name).map(std::borrow::Cow::Borrowed)
    }

    fn portrait_texture(&self, name: &[u8]) -> Option<Texture> {
        self.portrait_textures.and_then(|resolve| resolve(name))
    }

    fn sprite_alpha(&self, name: &[u8]) -> Option<native_gui::AlphaMask> {
        self.gui.sprite_alpha(name)
    }

    fn unloaded_sprite(&self, name: &[u8]) -> bool {
        self.gui.registry().sprite(name).is_some()
    }

    fn font(&self, name: &[u8]) -> Option<&crate::native_text::Font> {
        self.gui.font(name)
    }

    fn localisation(&self) -> &content_reader_localisation::localisation::Localisation {
        self.localisation
    }

    fn text_colors(&self) -> &native::fonts::TextColors {
        self.gui.text_colors()
    }

    fn text_effect(&self) -> Result<crate::native_effects::Effect, String> {
        self.gui.text_effect()
    }

    fn effect(&self, file: &str) -> Result<crate::native_effects::Effect, String> {
        self.gui.effect(file)
    }

    fn shield_flag(&self, small: bool, index: i32) -> Result<Option<(Texture, [f32; 4])>, String> {
        match &self.flags {
            Some(flags) => native_flags::shield_flag(flags.source, small, index),
            None => Err(NO_COUNTRIES.to_owned()),
        }
    }

    fn text_flag(&self, tag: &str) -> Result<Option<Texture>, String> {
        match &self.flags {
            Some(flags) => native_flags::text_flag(flags.source, tag),
            None => Err(NO_COUNTRIES.to_owned()),
        }
    }

    fn resolved_text_flag(&self, key: &[u8]) -> Result<Option<Texture>, String> {
        match &self.flags {
            Some(flags) => native_flags::resolved_text_flag(flags.small, key),
            None => Err(NO_COUNTRIES.to_owned()),
        }
    }

    fn resolved_text_flag_key(&self, text: &[char]) -> Option<usize> {
        self.flags
            .as_ref()
            .and_then(|flags| flags.large.key_prefix(text))
    }

    fn gui_type(&self, name: &[u8]) -> Option<&native::Element> {
        self.windows.gui_type(name)
    }

    fn session_text_sprite(&self, name: &[u8]) -> bool {
        self.windows.text_box_named(name)
    }

    fn texture_file(&self, path: &[u8]) -> Option<native_gui::TextureFile> {
        self.gui.texture_file(path)
    }

    fn sprite_animations(&self, sprite: &[u8]) -> Option<Vec<native_gui::AnimationRecord>> {
        self.gui.sprite_animations(sprite)
    }
}

impl native_gui::Sheets for GuiAssets<'_> {
    fn sheet(&self, sprite: &[u8]) -> Option<&SpriteSheet> {
        self.gui.sheet(sprite)
    }

    fn progress(&self, sprite: &[u8]) -> Option<[&SpriteSheet; 2]> {
        self.gui.progress(sprite)
    }

    fn masking(&self, sprite: &[u8]) -> Option<&SpriteSheet> {
        self.gui.masking(sprite)
    }

    fn mask(&self, sprite: &[u8]) -> Option<&SpriteSheet> {
        self.gui.mask(sprite)
    }
}

const NO_COUNTRIES: &str =
    "a flag's country is unknown: the hour's countries (CountryAppearance) are not recorded";

pub fn viewport(viewport: [u32; 2], ui_scale: f32) -> [i32; 2] {
    crate::native_shader::draw::gui_viewport(viewport, ui_scale).map(|value| value as i32)
}

pub fn refused_cause(window: &str, refusal: &native_gui::Refusal) -> String {
    format!("{window} is not drawn: {refusal}")
}

pub fn noted_cause(window: &str, diagnostic: &native_gui::Diagnostic) -> String {
    format!(
        "{window} draws nothing for {} ({}): {}",
        diagnostic.path,
        diagnostic.kind.name(),
        diagnostic.detail
    )
}

pub fn count_refused<K: From<String> + Ord>(
    gaps: &mut BTreeMap<K, usize>,
    window: &str,
    refusals: &[native_gui::Refusal],
) {
    for refusal in refusals {
        *gaps
            .entry(refused_cause(window, refusal).into())
            .or_default() += 1;
    }
}

pub fn count_noted<K: From<String> + Ord>(
    gaps: &mut BTreeMap<K, usize>,
    window: &str,
    diagnostics: &[native_gui::Diagnostic],
) {
    for diagnostic in diagnostics {
        *gaps
            .entry(noted_cause(window, diagnostic).into())
            .or_default() += 1;
    }
}

pub struct HitRectangle {
    pub rect: [f32; 4],
    pub order: usize,
    pub supported: bool,
    pub geometry: Option<native_gui::HitGeometry>,
    pub disabled: bool,
}

pub fn hit_rectangle(
    control: &native::Element,
    layout: &Layout,
    origin: [i32; 2],
    resources: &dyn native_gui::Resources,
    notes: &mut BTreeMap<String, usize>,
) -> Option<HitRectangle> {
    observer_hit_rectangle(
        control,
        layout,
        origin,
        resources,
        notes,
        HitMode::DrawBounds,
    )
}

pub fn hit_rectangles(
    control: &native::Element,
    layout: &Layout,
    origin: [i32; 2],
    resources: &dyn native_gui::Resources,
    notes: &mut BTreeMap<String, usize>,
) -> Vec<HitRectangle> {
    observer_hit_rectangles(
        control,
        layout,
        origin,
        resources,
        notes,
        HitMode::DrawBounds,
    )
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum HitMode {
    DrawBounds,
    NativeGeometry,
}

pub fn observer_hit_rectangles(
    control: &native::Element,
    layout: &Layout,
    origin: [i32; 2],
    resources: &dyn native_gui::Resources,
    notes: &mut BTreeMap<String, usize>,
    mode: HitMode,
) -> Vec<HitRectangle> {
    if let Some(container) = control.container() {
        if mode == HitMode::NativeGeometry
            && layout
                .regions
                .iter()
                .any(|region| region.path == control.path && region.hit.is_some())
        {
            return observer_hit_rectangle(control, layout, origin, resources, notes, mode)
                .into_iter()
                .collect();
        }
        return container
            .backgrounds
            .iter()
            .enumerate()
            .filter_map(|(index, background)| {
                observer_hit_rectangle_for(
                    background,
                    layout,
                    origin,
                    resources,
                    notes,
                    mode,
                    Some(index),
                )
            })
            .collect();
    }
    observer_hit_rectangle(control, layout, origin, resources, notes, mode)
        .into_iter()
        .collect()
}

pub fn observer_hit_rectangle(
    control: &native::Element,
    layout: &Layout,
    origin: [i32; 2],
    resources: &dyn native_gui::Resources,
    notes: &mut BTreeMap<String, usize>,
    mode: HitMode,
) -> Option<HitRectangle> {
    observer_hit_rectangle_for(control, layout, origin, resources, notes, mode, None)
}

fn observer_hit_rectangle_for(
    control: &native::Element,
    layout: &Layout,
    origin: [i32; 2],
    resources: &dyn native_gui::Resources,
    notes: &mut BTreeMap<String, usize>,
    mode: HitMode,
    background_index: Option<usize>,
) -> Option<HitRectangle> {
    if control
        .button()
        .is_some_and(|button| button.always_transparent)
        || control.icon().is_some_and(|icon| icon.always_transparent)
        || (mode == HitMode::NativeGeometry
            && control
                .text_box()
                .is_some_and(|text| text.always_transparent))
    {
        return None;
    }
    let matches_part = |part: &native_gui::Part| {
        part.path == control.path
            && background_index.is_none_or(|index| part.background_index == Some(index))
            && native_gui::part_rect(&part.kind).is_some()
    };
    let find_part = || {
        let mut matching = layout
            .parts
            .iter()
            .enumerate()
            .filter(|(_, part)| matches_part(part));
        if background_index.is_some() {
            matching.next_back()
        } else {
            matching.next()
        }
    };
    let region = layout.regions.iter().enumerate().find(|(_, region)| {
        region.path == control.path
            && background_index.is_none_or(|index| region.background_index == Some(index))
    });
    if let Some((order, region)) = region.filter(|_| mode == HitMode::NativeGeometry)
        && let Some(geometry) = &region.hit
    {
        if geometry.form == native_gui::Hit::Through {
            return None;
        }
        let geometry = geometry.shifted(origin);
        let supported = geometry.form == native_gui::Hit::Frame;
        if !supported {
            *notes
                .entry(format!(
                    "{} interaction is refused: native sprite hit texels/form unavailable",
                    control.path
                ))
                .or_default() += 1;
        }
        let mut rect = find_part()
            .and_then(|(_, part)| native_gui::part_rect(&part.kind))
            .unwrap_or(region.rect);
        if let Some([left, top, right, bottom]) = region.clip {
            let end = [
                (rect[0] + rect[2]).min(right as f32),
                (rect[1] + rect[3]).min(bottom as f32),
            ];
            rect[0] = rect[0].max(left as f32);
            rect[1] = rect[1].max(top as f32);
            rect[2] = end[0] - rect[0];
            rect[3] = end[1] - rect[1];
        }
        rect[0] += origin[0] as f32;
        rect[1] += origin[1] as f32;
        let rect = if rect[2] > 0.0 && rect[3] > 0.0 {
            rect
        } else {
            geometry.bounds()?
        };
        return Some(HitRectangle {
            rect,
            order,
            supported,
            disabled: region.disabled,
            geometry: Some(geometry),
        });
    }
    let (order, part) = find_part()?;
    let sprite = match &part.kind {
        native_gui::PartKind::Sprite { sprite, .. }
        | native_gui::PartKind::Tiles { sprite, .. }
        | native_gui::PartKind::TextSprite { sprite, .. }
        | native_gui::PartKind::Progress { sprite, .. }
        | native_gui::PartKind::MaskedFlag { sprite, .. } => Some(sprite.as_slice()),
        native_gui::PartKind::Animated(animated) => Some(animated.sprite.as_slice()),
        _ => None,
    };
    let hit = sprite
        .and_then(|sprite| resources.sprite(sprite))
        .map(|sprite| sprite.hit);
    if hit == Some(native_gui::Hit::Through) {
        return None;
    }
    let supported = mode == HitMode::DrawBounds && hit == Some(native_gui::Hit::Frame);
    if !supported {
        let reason = if mode == HitMode::NativeGeometry && resources.physical_viewport().is_none() {
            "native hit physical viewport unavailable"
        } else {
            "native sprite hit texels/form unavailable"
        };
        let why = format!("{} interaction is refused: {reason}", control.path);
        *notes.entry(why).or_default() += 1;
    }
    let mut rect = native_gui::part_rect(&part.kind).expect("checked above");
    if let Some([left, top, right, bottom]) = part.clip {
        let x1 = (rect[0] + rect[2]).min(right as f32);
        let y1 = (rect[1] + rect[3]).min(bottom as f32);
        rect[0] = rect[0].max(left as f32);
        rect[1] = rect[1].max(top as f32);
        rect[2] = x1 - rect[0];
        rect[3] = y1 - rect[1];
    }
    rect[0] += origin[0] as f32;
    rect[1] += origin[1] as f32;
    (rect[2] > 0.0 && rect[3] > 0.0).then_some(HitRectangle {
        rect,
        order,
        supported,
        geometry: None,
        disabled: region.is_some_and(|(_, region)| region.disabled),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{native_gui::lifecycle::Instance, test_support::gui};

    #[test]
    fn separated_container_backgrounds_return_individual_hit_rectangles() {
        content_reader_script::test_game::enter();
        let windows = gui::windows(
            br#"guiTypes = {
            containerWindowType = { name = split_panel size = { width = 80 height = 20 } clipping = no
                background = { name = Left position = { x = 0 y = 0 } spriteType = GFX_even }
                background = { name = Right position = { x = 40 y = 0 } spriteType = GFX_even }
                background = { name = Ghost position = { x = 20 y = 0 } spriteType = GFX_even alwaystransparent = yes } } }"#,
        );
        let resources = gui::fixture();
        let root = windows.gui_type(b"split_panel").unwrap();
        let mut instance = Instance::construct(root, [200, 100], &resources).unwrap();
        instance.gui_show(root, root).unwrap();
        let layout = instance.layout(root, &resources, None).unwrap().unwrap();
        let mut notes = BTreeMap::new();
        let hits = hit_rectangles(root, &layout, [10, 5], &resources, &mut notes);
        assert!(notes.is_empty(), "{notes:?}");
        let rects: Vec<[f32; 4]> = hits.iter().map(|hit| hit.rect).collect();
        assert_eq!(rects, [[10.0, 5.0, 20.0, 10.0], [50.0, 5.0, 20.0, 10.0]]);
        let blocked = |point: [f32; 2]| rects.iter().any(|rect| contains(*rect, point));
        assert!(blocked([20.0, 10.0]));
        assert!(blocked([60.0, 10.0]));
        assert!(!blocked([40.0, 10.0]));
    }

    #[test]
    fn unnamed_separated_container_backgrounds_hit_independently() {
        content_reader_script::test_game::enter();
        let windows = gui::windows(
            br#"guiTypes = {
            containerWindowType = { name = split_unnamed size = { width = 80 height = 20 } clipping = no
                background = { position = { x = 0 y = 0 } spriteType = GFX_even }
                background = { position = { x = 20 y = 0 } spriteType = GFX_even alwaystransparent = yes }
                background = { position = { x = 40 y = 0 } spriteType = GFX_even } }
            containerWindowType = { name = split_missing size = { width = 80 height = 20 } clipping = no
                background = { position = { x = 0 y = 0 } spriteType = GFX_missing }
                background = { position = { x = 40 y = 0 } spriteType = GFX_even } }
            containerWindowType = { name = split_unloaded size = { width = 80 height = 20 } clipping = no
                background = { position = { x = 0 y = 0 } spriteType = GFX_unloaded alwaystransparent = yes }
                background = { position = { x = 40 y = 0 } spriteType = GFX_even } }
            containerWindowType = { name = split_unloaded_opaque size = { width = 80 height = 20 } clipping = no
                background = { position = { x = 0 y = 0 } spriteType = GFX_unloaded }
                background = { position = { x = 40 y = 0 } spriteType = GFX_even } }
            containerWindowType = { name = split_text_sprite size = { width = 80 height = 20 } clipping = no
                background = { position = { x = 0 y = 0 } spriteType = GFX_text_bg alwaystransparent = yes }
                background = { position = { x = 40 y = 0 } spriteType = GFX_even } }
            containerWindowType = { name = split_line_chart size = { width = 80 height = 20 } clipping = no
                background = { position = { x = 0 y = 0 } spriteType = GFX_line_chart }
                background = { position = { x = 40 y = 0 } spriteType = GFX_even alwaystransparent = yes } }
            containerWindowType = { name = split_line_chart_opaque size = { width = 80 height = 20 } clipping = no
                background = { position = { x = 0 y = 0 } spriteType = GFX_line_chart }
                background = { position = { x = 40 y = 0 } spriteType = GFX_even } }
            containerWindowType = { name = split_refused_effect size = { width = 80 height = 20 } clipping = no
                background = { position = { x = 0 y = 0 } spriteType = GFX_bad_effect alwaystransparent = yes }
                background = { position = { x = 40 y = 0 } spriteType = GFX_even } } }"#,
        );
        let resources = gui::fixture();
        let root = windows.gui_type(b"split_unnamed").unwrap();
        let mut instance = Instance::construct(root, [200, 100], &resources).unwrap();
        instance.gui_show(root, root).unwrap();
        let layout = instance.layout(root, &resources, None).unwrap().unwrap();
        let mut notes = BTreeMap::new();
        let hits = hit_rectangles(root, &layout, [10, 5], &resources, &mut notes);
        assert!(notes.is_empty(), "{notes:?}");
        let rects: Vec<[f32; 4]> = hits.iter().map(|hit| hit.rect).collect();
        assert_eq!(rects, [[10.0, 5.0, 20.0, 10.0], [50.0, 5.0, 20.0, 10.0]]);
        let blocked = |point: [f32; 2]| rects.iter().any(|rect| contains(*rect, point));
        assert!(blocked([20.0, 10.0]));
        assert!(blocked([60.0, 10.0]));
        assert!(!blocked([40.0, 10.0]));

        let missing = windows.gui_type(b"split_missing").unwrap();
        let mut instance = Instance::construct(missing, [200, 100], &resources).unwrap();
        instance.gui_show(missing, missing).unwrap();
        let layout = instance.layout(missing, &resources, None).unwrap().unwrap();
        let hits = hit_rectangles(missing, &layout, [10, 5], &resources, &mut notes);
        assert!(notes.is_empty(), "{notes:?}");
        assert_eq!(
            hits.iter().map(|hit| hit.rect).collect::<Vec<_>>(),
            [[10.0, 5.0, 8.0, 8.0], [50.0, 5.0, 20.0, 10.0]]
        );

        let mut no_fallback = gui::fixture();
        no_fallback
            .sprites
            .remove(std::str::from_utf8(native_gui::FALLBACK_SPRITE).unwrap());
        let mut instance = Instance::construct(missing, [200, 100], &no_fallback).unwrap();
        instance.gui_show(missing, missing).unwrap();
        let layout = instance
            .layout(missing, &no_fallback, None)
            .unwrap()
            .unwrap();
        let hits = hit_rectangles(missing, &layout, [10, 5], &no_fallback, &mut notes);
        assert!(notes.is_empty(), "{notes:?}");
        assert_eq!(
            hits.iter().map(|hit| hit.rect).collect::<Vec<_>>(),
            [[50.0, 5.0, 20.0, 10.0]]
        );

        let mut special = gui::fixture();
        let mut unloaded = special.sprites["GFX_even"].clone();
        unloaded.missing = Some("its texture is missing".into());
        special.sprites.insert("GFX_unloaded", unloaded);
        let mut text_sprite = special.sprites["GFX_even"].clone();
        text_sprite.kind = native_gui::SpriteKind::Text { textured: false };
        special.sprites.insert("GFX_text_bg", text_sprite);
        let mut line_chart = special.sprites["GFX_even"].clone();
        line_chart.kind = native_gui::SpriteKind::LineChart { size: [20, 10] };
        special.sprites.insert("GFX_line_chart", line_chart);
        let mut bad_effect = special.sprites["GFX_even"].clone();
        bad_effect.effect = Err("custom effect is refused".into());
        special.sprites.insert("GFX_bad_effect", bad_effect);
        for name in [
            b"split_unloaded".as_slice(),
            b"split_unloaded_opaque".as_slice(),
            b"split_text_sprite".as_slice(),
            b"split_line_chart_opaque".as_slice(),
        ] {
            let panel = windows.gui_type(name).unwrap();
            let mut instance = Instance::construct(panel, [200, 100], &special).unwrap();
            instance.gui_show(panel, panel).unwrap();
            let layout = instance.layout(panel, &special, None).unwrap().unwrap();
            let hits = hit_rectangles(panel, &layout, [10, 5], &special, &mut notes);
            assert!(notes.is_empty(), "{notes:?}");
            assert_eq!(
                hits.iter().map(|hit| hit.rect).collect::<Vec<_>>(),
                [[50.0, 5.0, 20.0, 10.0]],
                "{}",
                String::from_utf8_lossy(name)
            );
        }
        let line_panel = windows.gui_type(b"split_line_chart").unwrap();
        let mut instance = Instance::construct(line_panel, [200, 100], &special).unwrap();
        instance.gui_show(line_panel, line_panel).unwrap();
        let layout = instance
            .layout_with_regions(line_panel, &special, None, true)
            .unwrap()
            .unwrap();
        let hits = hit_rectangles(line_panel, &layout, [10, 5], &special, &mut notes);
        assert!(notes.is_empty(), "{notes:?}");
        assert!(
            hits.is_empty(),
            "line chart must not steal transparent sibling: {:?}",
            hits.iter().map(|hit| hit.rect).collect::<Vec<_>>()
        );

        let refused_panel = windows.gui_type(b"split_refused_effect").unwrap();
        let (layout, refusals) =
            native_gui::partial_layout_for_tests(refused_panel, [200, 100], &special);
        assert!(
            refusals
                .iter()
                .any(|refusal| refusal.kind == native_gui::RefusalKind::CustomEffect),
            "{refusals:?}"
        );
        let hits = hit_rectangles(refused_panel, &layout, [10, 5], &special, &mut notes);
        assert!(notes.is_empty(), "{notes:?}");
        assert_eq!(
            hits.iter().map(|hit| hit.rect).collect::<Vec<_>>(),
            [[50.0, 5.0, 20.0, 10.0]]
        );
    }
}
