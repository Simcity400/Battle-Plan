//! Sprite artwork: images for exact authored sprite keys that a product
//! returned (doctrine icons, for one), with no inferred suffix, localisation or
//! recorded hour. It shares the catalogue's sprite renderer with focus and
//! technology artwork.

use crate::{
    Source,
    catalogue::{self, Catalogue},
    products::{LoadContext, Product, check_reply_budget},
};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    io,
    sync::atomic::AtomicBool,
};
use ts_rs::TS;

pub struct SpriteArtwork;

/// One frame of an authored sprite, as a product returned it.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, PartialOrd, Ord, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SpriteFrame {
    /// The exact sprite key (doctrine mastery's `icon`, for one).
    pub key: String,
    /// The zero-based native frame; offsets may continue past the authored strip.
    #[serde(default)]
    pub frame: u32,
}

#[derive(Clone, Debug, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SpriteArtworkParams {
    pub sprites: Vec<SpriteFrame>,
}

#[derive(Clone, Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct SpriteImage {
    pub key: String,
    pub frame: u32,
    /// A PNG data URL, or null with a named, counted refusal.
    pub icon: Option<String>,
}

#[derive(Clone, Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct SpriteArtworkReply {
    pub items: Vec<SpriteImage>,
    pub warnings: Vec<String>,
}

crate::reply_warnings!(SpriteArtworkReply);

/// Validate a lazy route key without decoding any image. Absent authored keys,
/// refused declarations and declarations without textures remain distinct errors.
pub fn icon_key(catalogue: &Catalogue, authored: &[u8]) -> Result<String, String> {
    let key = std::str::from_utf8(installed_content_core::parser::c_string(authored))
        .map_err(|_| "authored icon key is not UTF-8".to_owned())?;
    if key.is_empty() {
        return Err("authored icon key is absent".into());
    }
    let sprite = catalogue
        .sprites
        .find(key)?
        .ok_or_else(|| format!("sprite {key} is undeclared"))?;
    if sprite.texture.is_none() {
        return Err(format!("sprite {key} has no texture"));
    }
    Ok(key.to_owned())
}

impl Product for SpriteArtwork {
    const KEY: &'static str = "sprite-artwork";
    const CONTENT_DEPENDENT: bool = true;
    const INTERACTIVE: bool = true;
    type Params = SpriteArtworkParams;
    type Reply = SpriteArtworkReply;

    fn validate(params: &Self::Params) -> Result<(), &'static str> {
        if params.sprites.is_empty()
            || params
                .sprites
                .iter()
                .any(|sprite| sprite.key.is_empty() || sprite.key.contains('\0'))
        {
            return Err("Invalid sprite keys");
        }
        Ok(())
    }

    fn types() -> Vec<String> {
        vec![
            SpriteArtworkParams::decl(),
            SpriteArtworkReply::decl(),
            SpriteFrame::decl(),
            SpriteImage::decl(),
        ]
    }

    fn load(
        source: &Source,
        params: &Self::Params,
        context: &LoadContext,
        cancelled: &AtomicBool,
    ) -> io::Result<Self::Reply> {
        crate::metadata::check_cancel(cancelled)?;
        Self::validate(params).map_err(crate::invalid)?;
        let catalogue = catalogue::catalogue(source, context.content()?)?;
        images(&catalogue, params, cancelled)
    }
}

fn images(
    catalogue: &Catalogue,
    params: &SpriteArtworkParams,
    cancelled: &AtomicBool,
) -> io::Result<SpriteArtworkReply> {
    let mut reply = SpriteArtworkReply {
        items: Vec::new(),
        warnings: catalogue.verification.clone(),
    };
    reply.warnings.extend(catalogue.warnings.clone());
    let mut refusals = BTreeMap::<String, usize>::new();
    for asked in params.sprites.iter().collect::<BTreeSet<_>>() {
        crate::metadata::check_cancel(cancelled)?;
        let key = &asked.key;
        let result = (|| {
            icon_key(catalogue, key.as_bytes()).map_err(crate::invalid)?;
            let sprite = catalogue
                .sprites
                .find(key)
                .map_err(crate::invalid)?
                .expect("validated sprite key");
            catalogue::sprite_frame_png(
                &catalogue.content.layers,
                sprite,
                *catalogue
                    .gui_decode
                    .as_ref()
                    .map_err(|why| crate::invalid(why.clone()))?,
                asked.frame,
                super::ICON_EDGE,
            )
        })();
        let icon = match result {
            Ok(icon) => Some(icon),
            Err(error) if error.kind() == io::ErrorKind::Interrupted => return Err(error),
            Err(error) => {
                *refusals
                    .entry(format!("sprite {key} frame {}: {error}", asked.frame))
                    .or_default() += 1;
                None
            }
        };
        reply.items.push(SpriteImage {
            key: key.clone(),
            frame: asked.frame,
            icon,
        });
        // Refuse the complete ask, never omit later requested sprites to fit it.
        check_reply_budget(&reply)?;
    }
    reply.warnings.extend(
        refusals
            .into_iter()
            .map(|(why, count)| format!("{count} sprite artwork refusals: {why}")),
    );
    reply.warnings.extend(catalogue.refusal_lines());
    reply
        .warnings
        .extend(catalogue.content.layers.shader_warning_lines());
    crate::metadata::check_cancel(cancelled)?;
    check_reply_budget(&reply)?;
    Ok(reply)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::catalogue::{
        tests::{folder_catalogue, layers_catalogue},
        texture_tests::{from_url, png_bytes},
    };
    use installed_content_image::Rgba8Image;

    fn frame(key: &str, frame: u32) -> SpriteFrame {
        SpriteFrame {
            key: key.into(),
            frame,
        }
    }

    #[test]
    fn sprite_artwork_uses_layered_exact_sprite_keys_and_replace_path() {
        engine_executable::executable_tables::initialize_for_tests().unwrap();
        let (_, base) = folder_catalogue(&[(
            "interface/base.gfx",
            "spriteTypes={spriteType={name=GFX_Hidden textureFile=gfx/hidden.png}}",
        )]);
        let (_, addon) = folder_catalogue(&[(
            "interface/mod.gfx",
            "spriteTypes={spriteType={name=GFX_Mixed_Mod textureFile=gfx/mod.png} spriteType={name=GFX_Missing textureFile=gfx/absent.png}}",
        )]);
        let image = Rgba8Image {
            width: 2,
            height: 1,
            pixels: vec![30, 80, 140, 255, 30, 80, 140, 255],
        };
        std::fs::create_dir_all(addon.0.join("gfx")).unwrap();
        std::fs::write(addon.0.join("gfx/mod.png"), png_bytes(&image)).unwrap();
        let layers = installed_content_core::ResolvedLayers::from_folders(&[
            (&base.0, &[]),
            (&addon.0, &["interface"]),
        ])
        .unwrap();
        let catalogue = layers_catalogue(layers);
        assert_eq!(
            icon_key(&catalogue, b"GFX_Mixed_Mod").unwrap(),
            "GFX_Mixed_Mod"
        );
        assert!(icon_key(&catalogue, b"gfx_mixed_mod").is_err());
        assert!(icon_key(&catalogue, b"GFX_Hidden").is_err());
        let params = SpriteArtworkParams {
            sprites: [
                "GFX_Mixed_Mod",
                "GFX_Hidden",
                "GFX_Missing",
                "GFX_Mixed_Mod",
            ]
            .into_iter()
            .map(|key| frame(key, 0))
            .chain([frame("GFX_Mixed_Mod", 1)])
            .collect(),
        };
        let reply = images(&catalogue, &params, &AtomicBool::new(false)).unwrap();
        // Repeats are asked once; past-strip frames use the native sampler.
        assert_eq!(reply.items.len(), 4);
        let art = reply
            .items
            .iter()
            .find(|art| art.key == "GFX_Mixed_Mod" && art.frame == 0)
            .unwrap();
        assert_eq!(from_url(art.icon.as_deref().unwrap()).pixels, image.pixels);
        let past_strip = reply
            .items
            .iter()
            .find(|art| art.key == "GFX_Mixed_Mod" && art.frame == 1)
            .unwrap();
        assert_eq!(
            from_url(past_strip.icon.as_deref().unwrap()).pixels,
            image.pixels
        );
        assert!(
            reply
                .warnings
                .iter()
                .all(|why| !why.contains("sprite GFX_Mixed_Mod frame 1")),
            "{:?}",
            reply.warnings
        );
        for key in ["GFX_Hidden", "GFX_Missing"] {
            assert!(
                reply
                    .items
                    .iter()
                    .find(|art| art.key == key)
                    .unwrap()
                    .icon
                    .is_none()
            );
            assert!(
                reply
                    .warnings
                    .iter()
                    .any(|why| why.contains(key) && why.contains("1 sprite artwork refusals")),
                "{:?}",
                reply.warnings
            );
        }
        let cached = images(&catalogue, &params, &AtomicBool::new(false)).unwrap();
        assert_eq!(
            serde_json::to_value(&reply).unwrap(),
            serde_json::to_value(cached).unwrap()
        );
        assert!(images(&catalogue, &params, &AtomicBool::new(true)).is_err());
    }

    /// A mod's two-frame strip replacing a base declaration of four frames.
    fn shortened_strip() -> (Catalogue, String, Box<dyn std::any::Any>) {
        engine_executable::executable_tables::initialize_for_tests().unwrap();
        let (_, base) = folder_catalogue(&[(
            "interface/base.gfx",
            "spriteTypes={spriteType={name=GFX_Mod textureFile=absent.png noOfFrames=4}}",
        )]);
        let (_, folder) = folder_catalogue(&[(
            "interface/mod.gfx",
            "spriteTypes={spriteType={name=GFX_Mod textureFile=strip.png noOfFrames=2}}",
        )]);
        let image = Rgba8Image {
            width: 8,
            height: 4,
            pixels: (0..4)
                .flat_map(|y| (0..8).flat_map(move |x| [20 + 10 * x, 30 + 10 * y, 140, 255]))
                .collect(),
        };
        std::fs::write(folder.0.join("strip.png"), png_bytes(&image)).unwrap();
        let layers = installed_content_core::ResolvedLayers::from_folders(&[
            (&base.0, &[]),
            (&folder.0, &["interface"]),
        ])
        .unwrap();
        let catalogue = layers_catalogue(layers);
        let expected = catalogue::texture_png(
            &catalogue.content.layers,
            "strip.png",
            *catalogue.gui_decode.as_ref().unwrap(),
            2,
            super::super::ICON_EDGE,
        )
        .unwrap();
        // The folders must outlive the catalogue's lazy reads.
        (catalogue, expected, Box::new((base, folder)))
    }

    #[test]
    fn s4_sprite_artwork_frame_zero_keeps_existing_png_bytes() {
        let (catalogue, expected, _folders) = shortened_strip();
        let params: SpriteArtworkParams = serde_json::from_value(serde_json::json!({
            "sprites": [{"key": "GFX_Mod"}, {"key": "GFX_Mod", "frame": 0}]
        }))
        .unwrap();
        let reply = images(&catalogue, &params, &AtomicBool::new(false)).unwrap();
        assert!(reply.warnings.is_empty(), "{:?}", reply.warnings);
        assert_eq!(reply.items.len(), 1);
        assert_eq!(reply.items[0].frame, 0);
        assert_eq!(reply.items[0].icon.as_deref(), Some(expected.as_str()));
        let cached = images(&catalogue, &params, &AtomicBool::new(false)).unwrap();
        assert_eq!(cached.items[0].icon.as_deref(), Some(expected.as_str()));
    }

    #[test]
    #[ignore = "requires DXC and a GPU; synthetic content only"]
    fn s4_sprite_artwork_past_strip_frame_samples_the_clamped_edge() {
        let (catalogue, expected, _folders) = shortened_strip();
        let params = SpriteArtworkParams {
            sprites: vec![frame("GFX_Mod", 0), frame("GFX_Mod", 2)],
        };
        let reply = images(&catalogue, &params, &AtomicBool::new(false)).unwrap();
        assert!(reply.warnings.is_empty(), "{:?}", reply.warnings);
        let past_strip = from_url(reply.items[1].icon.as_deref().unwrap());
        let expected_edge = Rgba8Image {
            width: 4,
            height: 4,
            pixels: (0..4)
                .flat_map(|y| [90, 30 + 10 * y, 140, 255].repeat(4))
                .collect(),
        };
        assert_eq!(past_strip, expected_edge);
        assert_eq!(reply.items[0].icon.as_deref(), Some(expected.as_str()));
        assert_ne!(reply.items[0].icon, reply.items[1].icon);
    }

    #[test]
    fn s4_sprite_artwork_past_strip_keeps_named_counted_effect_refusals() {
        engine_executable::executable_tables::initialize_for_tests().unwrap();
        let (catalogue, folder) = folder_catalogue(&[
            (
                "interface/mod.gfx",
                "spriteTypes={spriteType={name=GFX_Mod textureFile=strip.png effectFile=gfx/FX/no_up.lua}}",
            ),
            ("gfx/FX/no_up.shader", "Effect Down {}"),
        ]);
        std::fs::write(
            folder.0.join("strip.png"),
            png_bytes(&Rgba8Image {
                width: 2,
                height: 2,
                pixels: [30, 80, 140, 255].repeat(4),
            }),
        )
        .unwrap();
        let params = SpriteArtworkParams {
            sprites: vec![
                frame("GFX_Mod", 1),
                frame("GFX_Mod", 3),
                frame("GFX_Mod", 1),
            ],
        };
        let reply = images(&catalogue, &params, &AtomicBool::new(false)).unwrap();
        assert_eq!(reply.items.len(), 2);
        assert!(reply.items.iter().all(|item| item.icon.is_none()));
        for frame in [1, 3] {
            assert!(
                reply.warnings.iter().any(|why| {
                    why.contains("1 sprite artwork refusals")
                        && why.contains(&format!("sprite GFX_Mod frame {frame}"))
                        && why.contains("gfx/FX/no_up.lua")
                        && why.contains("no Up technique")
                }),
                "{:?}",
                reply.warnings
            );
        }
        let cached = images(&catalogue, &params, &AtomicBool::new(false)).unwrap();
        assert_eq!(
            serde_json::to_value(reply).unwrap(),
            serde_json::to_value(cached).unwrap()
        );
    }

    #[test]
    fn sprite_artwork_keeps_named_refusals_when_decode_settings_are_unknown() {
        engine_executable::executable_tables::initialize_for_tests().unwrap();
        let (mut catalogue, _folder) = folder_catalogue(&[(
            "interface/mod.gfx",
            "spriteTypes={spriteType={name=GFX_Mod textureFile=gfx/mod.png} spriteType={name=GFX_No_Texture}}",
        )]);
        assert!(icon_key(&catalogue, b"\xff").unwrap_err().contains("UTF-8"));
        assert!(
            icon_key(&catalogue, b"GFX_No_Texture")
                .unwrap_err()
                .contains("no texture")
        );
        catalogue.gui_decode = Err("synthetic unavailable texture settings".into());
        let reply = images(
            &catalogue,
            &SpriteArtworkParams {
                sprites: vec![frame("GFX_Mod", 0)],
            },
            &AtomicBool::new(false),
        )
        .unwrap();
        assert!(reply.items[0].icon.is_none());
        assert!(
            reply
                .warnings
                .iter()
                .any(|why| why.contains("1 sprite artwork refusals")
                    && why.contains("texture settings")),
            "{:?}",
            reply.warnings
        );
    }

    #[test]
    fn sprite_artwork_validates_keys_and_exports_its_types() {
        for sprites in [vec![], vec![frame("", 0)], vec![frame("key\0tail", 0)]] {
            assert!(SpriteArtwork::validate(&SpriteArtworkParams { sprites }).is_err());
        }
        assert!(
            SpriteArtwork::types()
                .iter()
                .any(|decl| decl.contains("type SpriteImage"))
        );
    }
}
