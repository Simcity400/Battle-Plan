
use super::{
    lazy_icons::LazyFlags,
    native_flags,
    native_gui::{self, Layout, SpriteInfo, Texture},
    native_windows::NativeWindows,
    sprites::SpriteSheet,
};
use content_reader_core::gui::native;
use std::collections::BTreeMap;

pub type PortraitTextures<'a> = &'a dyn Fn(&[u8]) -> Option<Texture>;

pub struct GuiAssets<'a> {
    pub gui: &'a crate::map::native_resources::GuiResources,
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

    fn font(&self, name: &[u8]) -> Option<&crate::map::native_text::Font> {
        self.gui.font(name)
    }

    fn localisation(&self) -> &content_reader_localisation::localisation::Localisation {
        self.localisation
    }

    fn text_colors(&self) -> &native::fonts::TextColors {
        self.gui.text_colors()
    }

    fn text_effect(&self) -> Result<crate::map::native_effects::Effect, String> {
        self.gui.text_effect()
    }

    fn effect(&self, file: &str) -> Result<crate::map::native_effects::Effect, String> {
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
    crate::map::native_shader::draw::gui_viewport(viewport, ui_scale).map(|value| value as i32)
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
            .filter_map(|background| {
                observer_hit_rectangle(background, layout, origin, resources, notes, mode)
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
    let region = layout
        .regions
        .iter()
        .enumerate()
        .find(|(_, region)| region.path == control.path);
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
        let mut rect = layout
            .parts
            .iter()
            .find(|part| part.path == control.path)
            .and_then(|part| native_gui::part_rect(&part.kind))
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
    let (order, part) = layout.parts.iter().enumerate().find(|(_, part)| {
        part.path == control.path && native_gui::part_rect(&part.kind).is_some()
    })?;
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

pub fn contains(rect: [f32; 4], point: [f32; 2]) -> bool {
    point[0] >= rect[0]
        && point[1] >= rect[1]
        && point[0] < rect[0] + rect[2]
        && point[1] < rect[1] + rect[3]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::map::{native_gui::lifecycle::Instance, test_support::gui};

    #[test]
    fn separated_container_backgrounds_return_individual_hit_rectangles() {
        content_reader_core::test_game::enter();
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
}
