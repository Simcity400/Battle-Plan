//! What a dashboard window draws from: the content stack's GUI registry and resources as the
//! native GUI loads them ([`GuiResources`]), and the localisation in the user's game language.
//! The windows draw no country flags: a text or shield that needs one is refused by name.

use std::borrow::Cow;

use installed_content_core::gui::native;
use installed_content_localisation::localisation::Localisation;
use viewer_foundation::map::{
    native_effects::Effect,
    native_gui::{self, AnimationRecord, SpriteInfo, Texture, TextureFile},
    native_resources::GuiResources,
    native_text::Font,
};

/// Why a window cannot draw a flag.
const NO_FLAGS: &str = "a dashboard game window draws no country flags";

pub struct WindowResources<'a> {
    pub gui: &'a GuiResources,
    pub guis: &'a native::Guis,
    pub localisation: &'a Localisation,
}

impl native_gui::Resources for WindowResources<'_> {
    fn sprite(&self, name: &[u8]) -> Option<SpriteInfo> {
        self.gui.sprite(name)
    }

    fn sprite_info(&self, name: &[u8]) -> Option<Cow<'_, SpriteInfo>> {
        self.gui.sprite_info(name).map(Cow::Borrowed)
    }

    fn sprite_alpha(&self, name: &[u8]) -> Option<native_gui::AlphaMask> {
        self.gui.sprite_alpha(name)
    }

    fn unloaded_sprite(&self, name: &[u8]) -> bool {
        self.gui.registry().sprite(name).is_some()
    }

    fn font(&self, name: &[u8]) -> Option<&Font> {
        self.gui.font(name)
    }

    fn localisation(&self) -> &Localisation {
        self.localisation
    }

    fn text_colors(&self) -> &native::fonts::TextColors {
        self.gui.text_colors()
    }

    fn text_effect(&self) -> Result<Effect, String> {
        self.gui.text_effect()
    }

    fn effect(&self, file: &str) -> Result<Effect, String> {
        self.gui.effect(file)
    }

    fn shield_flag(
        &self,
        _small: bool,
        _index: i32,
    ) -> Result<Option<(Texture, [f32; 4])>, String> {
        Err(NO_FLAGS.to_owned())
    }

    fn text_flag(&self, _tag: &str) -> Result<Option<Texture>, String> {
        Err(NO_FLAGS.to_owned())
    }

    fn gui_type(&self, name: &[u8]) -> Option<&native::Element> {
        self.guis.template(name)
    }

    fn session_text_sprite(&self, name: &[u8]) -> bool {
        self.guis.text_box_named(name)
    }

    fn texture_file(&self, path: &[u8]) -> Option<TextureFile> {
        self.gui.texture_file(path)
    }

    fn sprite_animations(&self, sprite: &[u8]) -> Option<Vec<AnimationRecord>> {
        self.gui.sprite_animations(sprite)
    }
}
