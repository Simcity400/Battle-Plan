use std::{collections::BTreeMap, sync::Arc};

use content_reader_layers::ResolvedLayers;
use content_reader_defines::defines::Defines;
use content_reader_gui::gui::native::sprites::{self, AnimatedMapTextDefinition, ObjectKind};
use content_reader_image::native::Decode;
use shader_pipeline::library::Effect;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Style {
    CombatLoss,
    SiegeInfo,
}

impl Style {
    pub fn name(self) -> &'static [u8] {
        match self {
            Self::CombatLoss => b"CombatLoss",
            Self::SiegeInfo => b"siegeinfo",
        }
    }
}

#[derive(Clone)]
pub(super) struct Definition {
    pub order: usize,
    pub scale: f32,
    pub text: AnimatedMapTextDefinition,
}

pub struct Assets {
    pub(super) definitions: BTreeMap<Style, Result<Definition, String>>,
    pub(super) effect: Result<Arc<Effect>, String>,
    pub(super) cutoff: Result<f32, String>,
}

impl Assets {
    pub fn load(
        layers: &ResolvedLayers,
        (defines, graphics): (&Defines, &Defines),
        map: [u32; 2],
        decode: Decode,
        files: &[(String, sprites::Sprites)],
    ) -> Self {
        let registry = sprites::object_registry(files);
        let file_refusals = files
            .iter()
            .flat_map(|(_, read)| &read.refusals)
            .map(ToString::to_string)
            .collect::<Vec<_>>();
        let definitions = [Style::CombatLoss, Style::SiegeInfo]
            .into_iter()
            .map(|style| {
                let definition = (|| -> Result<Definition, String> {
                    if !file_refusals.is_empty() {
                        return Err(file_refusals.join("; "));
                    }
                    let object = registry.get(style.name()).ok_or_else(|| {
                        format!(
                            "object {:?} is not registered",
                            String::from_utf8_lossy(style.name())
                        )
                    })?;
                    if !object.refusals.is_empty() {
                        return Err(object
                            .refusals
                            .iter()
                            .map(ToString::to_string)
                            .collect::<Vec<_>>()
                            .join("; "));
                    }
                    let ObjectKind::AnimatedMapText(text) = &object.kind else {
                        return Err("latest object declaration is not animatedmaptext".into());
                    };
                    let order = files
                        .iter()
                        .flat_map(|(_, read)| &read.objects)
                        .filter(|candidate| {
                            matches!(candidate.kind, ObjectKind::AnimatedMapText(_))
                        })
                        .position(|candidate| std::ptr::eq(candidate, *object))
                        .expect("registered map text belongs to the construction stream");
                    if order > i16::MAX as usize {
                        return Err(
                            "animated-map-text bucket exceeds its native signed-i16 identity"
                                .into(),
                        );
                    }
                    if !object.scale.is_finite() || !text.speed.is_finite() {
                        return Err("animated-map-text scale or speed is not finite".into());
                    }
                    Ok(Definition {
                        order,
                        scale: object.scale,
                        text: text.clone(),
                    })
                })();
                (
                    style,
                    definition.map_err(|reason| {
                        format!(
                            "object {:?}: {reason}",
                            String::from_utf8_lossy(style.name())
                        )
                    }),
                )
            })
            .collect();
        let (mut library, shader_defines) =
            viewer_scene_base::shader_sources::library(layers, map, decode);
        let effect = library
            .load(b"gfx/FX/text.lua", &shader_defines)
            .map_err(|error| format!("gfx/FX/text.lua: {error}"));
        let cutoff = layers
            .graphics_define("floating map text", graphics, defines, |defines| {
                defines.fixed5("NGraphics", "PROVINCE_ANIM_TEXT_DISTANCE_CUTOFF")
            })
            .map(|value| value as f32 / 100_000.0)
            .map_err(|error| format!("NGraphics.PROVINCE_ANIM_TEXT_DISTANCE_CUTOFF: {error}"));
        Self {
            definitions,
            effect,
            cutoff,
        }
    }

    pub fn refusals(&self) -> Vec<String> {
        self.definitions
            .values()
            .filter_map(|definition| definition.as_ref().err().cloned())
            .chain(self.effect.as_ref().err().cloned())
            .chain(self.cutoff.as_ref().err().cloned())
            .collect()
    }

    pub fn gui_assets(&self) -> viewer_native_gui::native_gui::Assets {
        viewer_native_gui::native_gui::Assets {
            fonts: self
                .definitions
                .values()
                .filter_map(|definition| definition.as_ref().ok())
                .map(|definition| definition.text.text_block.font.clone())
                .collect(),
            ..Default::default()
        }
    }
}
