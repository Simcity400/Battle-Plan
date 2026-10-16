//! Shared GUI resources, viewport and per-occurrence diagnostics.

use super::{
    lazy_icons::LazyFlags,
    native_flags,
    native_gui::{self, Layout, SpriteInfo, Texture},
    native_windows::NativeWindows,
    sprites::SpriteSheet,
};
use installed_content::gui::native;
use std::collections::BTreeMap;

/// What the windows draw from: their resources as the native registry loads them
/// ([`super::native_resources::GuiResources`]), the recording's localisation, and the hour's countries and flag art
/// the flags resolve through ([`native_flags::FlagSource`]).
pub struct GuiAssets<'a> {
    pub gui: &'a crate::map::native_resources::GuiResources,
    pub windows: &'a NativeWindows,
    pub localisation: &'a installed_content::localisation::Localisation,
    pub flags: Option<GuiFlags<'a>>,
    pub physical_viewport: Option<[i32; 2]>,
}

/// Flag resolution and the two atlases used by GUI text and shields.
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

    fn unloaded_sprite(&self, name: &[u8]) -> bool {
        self.gui.registry().sprite(name).is_some()
    }

    fn font(&self, name: &[u8]) -> Option<&crate::map::native_text::Font> {
        self.gui.font(name)
    }

    fn localisation(&self) -> &installed_content::localisation::Localisation {
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

/// Refused for a flag a window draws while its hour's countries are not recorded.
const NO_COUNTRIES: &str =
    "a flag's country is unknown: the hour's countries (CountryAppearance) are not recorded";

/// The GUI viewport for parentless windows, with scaled extents truncated to integers.
/// Reading, construction, layout and projection share this viewport
/// ([`crate::map::native_shader::draw::gui_viewport`]).
pub fn viewport(viewport: [u32; 2], ui_scale: f32) -> [i32; 2] {
    crate::map::native_shader::draw::gui_viewport(viewport, ui_scale).map(|value| value as i32)
}

/// Why `window` is not drawn: the module's `refusal`.
pub fn refused_cause(window: &str, refusal: &native_gui::Refusal) -> String {
    format!("{window} is not drawn: {refusal}")
}

/// A part `window` draws nothing for without failing, and why.
pub fn noted_cause(window: &str, diagnostic: &native_gui::Diagnostic) -> String {
    format!(
        "{window} draws nothing for {} ({}): {}",
        diagnostic.path,
        diagnostic.kind.name(),
        diagnostic.detail
    )
}

/// Counts each refusal of `window` under its reason.
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

/// Counts each part `window` draws nothing for without failing, under its reason.
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
    /// Only a supported shape can dispatch its observer. Other shapes retain
    /// conservative bounds to block input without invoking an unverified callback.
    pub supported: bool,
    pub geometry: Option<native_gui::HitGeometry>,
    pub disabled: bool,
}

/// A presented observer's native hit rectangle and attachment order. Unknown
/// hit forms refuse interaction in their drawn bounds; they never pass input through.
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

/// Which native observer geometry the caller has adopted.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum HitMode {
    DrawBounds,
    NativeGeometry,
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
        // Keep existing capture rectangles. A zero stretch can still have a
        // full-frame native hit, so only an empty draw uses diagnostic bounds.
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
    // Without retained native geometry the drawn crop is only a blocker.
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

/// The one screen-rectangle hit test; the map's counter hits share it.
pub fn contains(rect: [f32; 4], point: [f32; 2]) -> bool {
    point[0] >= rect[0]
        && point[1] >= rect[1]
        && point[0] < rect[0] + rect[2]
        && point[1] < rect[1] + rect[3]
}
