use super::{Assets, Values, binding::Bindings, gui::CombatResources};
use content_reader_gui::gui::native;
use content_reader_localisation::localisation::Localisation;
use std::collections::{BTreeMap, BTreeSet};
use viewer_native_gui::native_flags::resolved_flag_cell;
use viewer_native_gui::native_flags::resolved_text_flag as native_flags_resolved_text_flag;
use viewer_native_gui::native_windows::NativeWindows;
use viewer_native_gui::{
    lazy_icons::LazyFlags,
    native_effects::Effect,
    native_gui::{self, Resources, Sheets, SpriteInfo, Texture},
    native_text,
    sprites::SpriteSheet,
};

pub struct GuiAssets {
    sprites: BTreeSet<Vec<u8>>,
    portraits: BTreeMap<Vec<u8>, SpriteSheet>,
    portrait_stamp: Option<(u32, u64)>,
    flags: BTreeMap<String, Result<i32, String>>,
    selectors: BTreeMap<i32, String>,
}

pub struct GuiResources<'a> {
    assets: &'a Assets,
    windows: &'a NativeWindows,
    localisation: &'a Localisation,
    small: &'a LazyFlags,
    large: &'a LazyFlags,
    inventory: &'a GuiAssets,
    physical_viewport: Option<[i32; 2]>,
}

impl GuiAssets {
    pub fn new(assets: &Assets, bindings: &Bindings) -> Self {
        let mut sprites = BTreeSet::new();
        let mut keys = BTreeSet::new();
        let values = std::iter::once(&bindings.values)
            .chain(
                bindings
                    .entries
                    .iter()
                    .flat_map(|rows| rows.iter().map(|(_, values)| values)),
            )
            .chain(bindings.naval_entries.iter().map(|row| &row.values))
            .chain(std::iter::once(&bindings.tactics.values))
            .chain(bindings.tactics.entries.iter());
        for values in values {
            active(values, &mut sprites, &mut keys);
        }
        let flags: BTreeMap<_, _> = keys
            .into_iter()
            .map(|key| {
                let index = assets.flag_index(&key);
                (key, index)
            })
            .collect();
        let selectors = flags
            .iter()
            .filter_map(|(key, index)| index.as_ref().ok().map(|&index| (index, key.clone())))
            .collect();
        Self {
            sprites,
            portraits: BTreeMap::new(),
            portrait_stamp: None,
            flags,
            selectors,
        }
    }

    pub fn refresh_portraits(&mut self, assets: &Assets, time: f32, texture_version: u64) {
        let stamp = (time.to_bits(), texture_version);
        if self.portrait_stamp == Some(stamp) {
            return;
        }
        self.portraits = assets.portrait_sheets(
            assets
                .window_sprites()
                .iter()
                .chain(&self.sprites)
                .map(Vec::as_slice),
        );
        self.portrait_stamp = Some(stamp);
    }
}

impl<'a> GuiResources<'a> {
    pub fn new(
        assets: &'a Assets,
        windows: &'a NativeWindows,
        localisation: &'a Localisation,
        small: &'a LazyFlags,
        large: &'a LazyFlags,
        inventory: &'a GuiAssets,
    ) -> Self {
        Self {
            assets,
            windows,
            localisation,
            small,
            large,
            inventory,
            physical_viewport: None,
        }
    }

    pub fn with_physical_viewport(self, physical_viewport: Option<[i32; 2]>) -> Self {
        Self { physical_viewport, ..self }
    }
}

impl CombatResources for GuiResources<'_> {
    fn art_version(&self) -> u64 {
        self.small.version().wrapping_add(self.large.version())
    }

    fn button_sounds(&self, button: &native::Button) -> native::ButtonSounds {
        let sprite_click = self
            .assets
            .gui
            .registry()
            .sprite(&button.sprite)
            .map_or(&[][..], |facts| facts.kind.click_sound());
        button.sounds(sprite_click, &self.windows.guis.default_sounds())
    }

    fn line_chart(
        &self,
        sprite: &[u8],
    ) -> Result<(viewer_native_gui::native_shader::draw::Effect, f32), String> {
        let facts = self
            .assets
            .gui
            .registry()
            .sprite(sprite)
            .ok_or("line chart sprite declaration is unavailable")?;
        let native::sprites::SpriteKind::LineChart(line) = &facts.kind else {
            return Err("air history sprite is not a native line chart".into());
        };
        let info = self
            .assets
            .native_sprite_info(sprite)
            .ok_or("line chart sprite was not loaded")?;
        let effect = info.effect.as_ref().map_err(Clone::clone)?;
        Ok((
            viewer_native_gui::native_shader::draw::Effect {
                asset: effect
                    .compiled
                    .clone()
                    .ok_or("line chart effect has no executable source")?,
                technique: "linechart".into(),
            },
            line.half_width,
        ))
    }

    fn flag_index(&self, key: &str) -> Result<i32, String> {
        self.inventory.flags.get(key).cloned().unwrap_or_else(|| {
            Err(format!(
                "resolved flag {key} was not included in this combat presentation"
            ))
        })
    }

    fn flag_texture(&self, key: &str, small: bool) -> Option<Texture> {
        let atlas = if small { self.small } else { self.large };
        resolved_flag_cell(atlas, key, small)
    }
}

fn active(values: &Values, sprites: &mut BTreeSet<Vec<u8>>, flags: &mut BTreeSet<String>) {
    for widget in values.values() {
        let mut inline = native_gui::Assets::default();
        for text in widget
            .text
            .as_deref()
            .into_iter()
            .chain(widget.tooltip.as_deref())
            .chain(widget.tooltip_description.as_deref())
            .chain(widget.tooltip_shortcut.as_deref())
        {
            inline.add_text(text);
        }
        sprites.extend(inline.sprites);
        if let Some(sprite) = &widget.sprite {
            sprites.insert(sprite.as_bytes().to_vec());
        }
        if let Some(key) = &widget.flag {
            flags.insert(key.clone());
        }
        for entry in &widget.entries {
            active(entry, sprites, flags);
        }
    }
}

impl Resources for GuiResources<'_> {
    fn physical_viewport(&self) -> Option<[i32; 2]> {
        self.physical_viewport
    }

    fn sprite(&self, name: &[u8]) -> Option<SpriteInfo> {
        self.assets.native_sprite(name)
    }

    fn sprite_alpha(&self, name: &[u8]) -> Option<viewer_native_gui::native_gui::AlphaMask> {
        self.assets.gui.sprite_alpha(name)
    }

    fn sprite_info(&self, name: &[u8]) -> Option<std::borrow::Cow<'_, SpriteInfo>> {
        self.assets.native_sprite_info(name)
    }

    fn unloaded_sprite(&self, name: &[u8]) -> bool {
        self.assets.gui.registry().sprite(name).is_some()
    }

    fn font(&self, name: &[u8]) -> Option<&native_text::Font> {
        self.assets.gui.font(name)
    }

    fn localisation(&self) -> &Localisation {
        self.localisation
    }

    fn text_colors(&self) -> &native::fonts::TextColors {
        self.assets.gui.text_colors()
    }

    fn text_effect(&self) -> Result<Effect, String> {
        self.assets.gui.text_effect()
    }

    fn effect(&self, file: &str) -> Result<Effect, String> {
        self.assets.gui.effect(file)
    }

    fn shield_flag(&self, small: bool, index: i32) -> Result<Option<(Texture, [f32; 4])>, String> {
        let key = self.inventory.selectors.get(&index).ok_or_else(|| {
            format!("shield selector {index} has no resolved combat flag binding")
        })?;
        Ok(self.flag_texture(key, small).map(|cell| {
            let [u0, v0, u1, v1] = cell.uv;
            (
                Texture {
                    uv: [0.0, 0.0, 1.0, 1.0],
                    ..cell
                },
                [u0, v0, u1 - u0, v1 - v0],
            )
        }))
    }

    fn text_flag(&self, tag: &str) -> Result<Option<Texture>, String> {
        Err(format!(
            "authored inline flag {tag} requires the native country database, not a resolved combat flag binding"
        ))
    }

    fn resolved_text_flag(&self, key: &[u8]) -> Result<Option<Texture>, String> {
        native_flags_resolved_text_flag(self.small, key)
    }

    fn resolved_text_flag_key(&self, text: &[char]) -> Option<usize> {
        self.large.key_prefix(text)
    }

    fn gui_type(&self, name: &[u8]) -> Option<&native::Element> {
        self.windows.gui_type(name)
    }

    fn session_text_sprite(&self, name: &[u8]) -> bool {
        self.windows.text_box_named(name)
    }

    fn texture_file(&self, path: &[u8]) -> Option<native_gui::TextureFile> {
        self.assets.gui.texture_file(path)
    }

    fn sprite_animations(&self, sprite: &[u8]) -> Option<Vec<native_gui::AnimationRecord>> {
        self.assets.gui.sprite_animations(sprite)
    }
}

impl Sheets for GuiResources<'_> {
    fn sheet(&self, sprite: &[u8]) -> Option<&SpriteSheet> {
        self.inventory
            .portraits
            .get(sprite)
            .or_else(|| self.assets.gui.sheet(sprite))
    }

    fn progress(&self, sprite: &[u8]) -> Option<[&SpriteSheet; 2]> {
        self.assets.gui.progress(sprite)
    }

    fn masking(&self, sprite: &[u8]) -> Option<&SpriteSheet> {
        self.assets.gui.masking(sprite)
    }

    fn mask(&self, sprite: &[u8]) -> Option<&SpriteSheet> {
        self.assets.gui.mask(sprite)
    }
}
