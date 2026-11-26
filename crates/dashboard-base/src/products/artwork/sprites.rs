
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

pub fn unit_icon_key(unit: &str) -> String {
    format!("GFX_unit_{unit}_icon_medium")
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, PartialOrd, Ord, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SpriteFrame {
    #[doc = " The exact sprite key (doctrine mastery's `icon`, for one)."]
    pub key: String,
    #[doc = " The zero-based native frame; offsets may continue past the authored strip."]
    #[serde(default)]
    pub frame: u32,
    #[doc = " Where the visual mods' interface chose `key`, the sprite the recording's own interface"]
    #[doc = " chooses in its place: drawn, named, when the selection cannot draw `key`. Equal to `key`"]
    #[doc = " when the selection could not make its own choice, so the recording's is drawn alone."]
    #[doc = " Absent where recorded facts name the sprite (or both choose the same one); a sprite the"]
    #[doc = " selection cannot draw is then the recording's of that name."]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub recorded: Option<String>,
    #[doc = " True for an authored `£` text icon: `key` is its name, and the game's text-icon rule chooses the sprite and its frame (`frame` is 0)."]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub text: Option<bool>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, PartialOrd, Ord, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TextureFrame {
    pub path: String,
    #[serde(default)]
    pub frame: u32,
    pub frames: u32,
}

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SpriteArtworkParams {
    #[serde(default)]
    pub sprites: Vec<SpriteFrame>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    #[ts(as = "Option<Vec<TextureFrame>>", optional)]
    pub textures: Vec<TextureFrame>,
}

#[derive(Clone, Debug, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct SpriteImage {
    pub key: String,
    pub frame: u32,
    #[doc = " The asked frame's `recorded` choice, as it was asked."]
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub recorded: Option<String>,
    #[doc = " The asked frame's `text`, as it was asked."]
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub text: Option<bool>,
    #[doc = " An artwork URL, or null with a named, counted refusal."]
    pub icon: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub resolution: Option<super::ArtResolution>,
}

#[derive(Clone, Debug, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct TextureImage {
    pub path: String,
    pub frame: u32,
    pub frames: u32,
    pub resolution: super::ArtResolution,
}

#[derive(Clone, Debug, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct SpriteArtworkReply {
    pub items: Vec<SpriteImage>,
    pub textures: Vec<TextureImage>,
    pub warnings: Vec<String>,
}

#[derive(Clone, Debug)]
pub struct NotDrawn(pub Box<super::ArtResolution>);

impl std::fmt::Display for NotDrawn {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.0.reason().unwrap_or("artwork unavailable"))
    }
}

pub type DrawnArt = catalogue::Drawn<Result<super::ArtResolution, NotDrawn>>;

pub fn drawn_art(
    what: impl FnOnce() -> String,
    shown: &Catalogue,
    resolve: impl Fn(&Catalogue) -> super::ArtResolution,
) -> DrawnArt {
    let first = std::sync::Mutex::new(None::<super::ArtResolution>);
    catalogue::draw(what, shown, move |drawing| {
        let attempt = resolve(drawing);
        let mut first = first.lock().unwrap_or_else(|error| error.into_inner());
        match (attempt, first.take()) {
            (ready @ super::ArtResolution::Ready { .. }, _) => Ok(ready),
            (attempt, None) => {
                *first = Some(attempt.clone());
                Err(NotDrawn(Box::new(attempt)))
            }
            (attempt, Some(earlier)) => Err(NotDrawn(Box::new(neither_drew(earlier, attempt)))),
        }
    })
}

fn neither_drew(earlier: super::ArtResolution, later: super::ArtResolution) -> super::ArtResolution {
    match earlier {
        super::ArtResolution::ConfirmedMissing { .. } => later,
        super::ArtResolution::NoArtExpected
        | super::ArtResolution::Pending
        | super::ArtResolution::Ready { .. }
        | super::ArtResolution::Unavailable { .. }
        | super::ArtResolution::Refused { .. }
        | super::ArtResolution::Failed { .. } => earlier,
    }
}

pub fn settled_art(drawn: &DrawnArt) -> super::ArtResolution {
    match drawn.get() {
        Ok(resolution) => resolution.clone(),
        Err(NotDrawn(resolution)) => super::ArtResolution::clone(resolution),
    }
}

pub fn keep_settled<K: Ord>(store: &std::sync::Mutex<std::collections::BTreeMap<K, DrawnArt>>, key: K, draw: impl FnOnce() -> DrawnArt) -> super::ArtResolution {
    if let Some(cached) = store.lock().unwrap_or_else(|error| error.into_inner()).get(&key) {
        return settled_art(cached);
    }
    let drawn = draw();
    let settled = settled_art(&drawn);
    if !matches!(settled, super::ArtResolution::Failed { retryable: true, .. }) {
        store.lock().unwrap_or_else(|error| error.into_inner()).insert(key, drawn);
    }
    settled
}

crate::reply_warnings!(SpriteArtworkReply);

pub fn icon_key(catalogue: &Catalogue, authored: &[u8]) -> Result<String, String> {
    let key = std::str::from_utf8(content_reader_parser::parser::c_string(authored))
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

fn asked_resolution(catalogue: &Catalogue, key: &str, asked: &SpriteFrame) -> super::ArtResolution {
    if asked.text == Some(true) {
        return super::resolve_text_icon(catalogue, key, super::ICON_EDGE);
    }
    super::resolve_sprite(catalogue, key, asked.frame, super::ICON_EDGE)
}

impl Product for SpriteArtwork {
    const KEY: &'static str = "sprite-artwork";
    const CONTENT_DEPENDENT: bool = true;
    const INTERACTIVE: bool = true;
    type Params = SpriteArtworkParams;
    const FIELDS: crate::products::Fields = crate::fields![sprites, "sprites[].frame", textures, "textures[].frame", "textures[].frames"];
    type Reply = SpriteArtworkReply;

    fn validate(params: &Self::Params) -> Result<(), &'static str> {
        let invalid = |key: &str| key.is_empty() || key.contains('\0');
        if params.sprites.is_empty() && params.textures.is_empty()
            || params.textures.iter().any(|texture| invalid(&texture.path) || texture.frames == 0)
            || params.sprites.iter().any(|sprite| {
                invalid(&sprite.key) || sprite.recorded.as_deref().is_some_and(invalid) || sprite.text == Some(false) || sprite.text == Some(true) && sprite.frame != 0
            })
        {
            return Err("Invalid sprite keys");
        }
        Ok(())
    }

    #[cfg(feature = "export-types")]
    fn types() -> Vec<String> {
        vec![
            SpriteArtworkParams::decl(),
            SpriteArtworkReply::decl(),
            SpriteFrame::decl(),
            SpriteImage::decl(),
            TextureFrame::decl(),
            TextureImage::decl(),
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
        textures: Vec::new(),
        warnings: catalogue.verification.clone(),
    };
    reply.warnings.extend(catalogue.warnings.clone());
    let mut refusals = BTreeMap::<String, usize>::new();
    for asked in params.sprites.iter().collect::<BTreeSet<_>>() {
        crate::metadata::check_cancel(cancelled)?;
        let key = &asked.key;
        let resolution = match &asked.recorded {
            Some(recorded) if recorded == key => asked_resolution(
                catalogue.recorded().map_or(catalogue, |recorded| &**recorded),
                key,
                asked,
            ),
            recorded => settled_art(&drawn_art(
                || format!("sprite {key}"),
                catalogue,
                |drawing| {
                    let key = match recorded {
                        Some(recorded) if !std::ptr::eq(drawing, catalogue) => recorded,
                        _ => key,
                    };
                    asked_resolution(drawing, key, asked)
                },
            )),
        };
        let icon = match &resolution {
            super::ArtResolution::Ready { url, .. } => Some(url.clone()),
            _ => {
                let error = resolution
                    .reason()
                    .unwrap_or("sprite artwork unavailable");
                *refusals
                    .entry(format!("sprite {key} frame {}: {error}", asked.frame))
                    .or_default() += 1;
                None
            }
        };
        reply.items.push(SpriteImage {
            key: key.clone(),
            frame: asked.frame,
            recorded: asked.recorded.clone(),
            text: asked.text,
            icon,
            resolution: Some(resolution),
        });
        check_reply_budget(&reply)?;
    }
    for asked in params.textures.iter().collect::<BTreeSet<_>>() {
        crate::metadata::check_cancel(cancelled)?;
        let resolution = settled_art(&drawn_art(
            || format!("texture {}", asked.path),
            catalogue,
            |drawing| super::resolve_texture(drawing, &asked.path, asked.frames, asked.frame, super::ICON_EDGE),
        ));
        if let Some(error) = resolution.reason() {
            *refusals
                .entry(format!("texture {} frame {}: {error}", asked.path, asked.frame))
                .or_default() += 1;
        }
        reply.textures.push(TextureImage {
            path: asked.path.clone(),
            frame: asked.frame,
            frames: asked.frames,
            resolution,
        });
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
        .extend(content_reader_layers::content::diagnostics::current().shader_warnings());
    crate::metadata::check_cancel(cancelled)?;
    check_reply_budget(&reply)?;
    Ok(reply)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_retryable_failure_is_never_kept_so_a_recovered_file_is_drawn_on_the_next_ask() {
        let (catalogue, _folder) = crate::catalogue::tests::localisation_catalogue(&[]);
        let store = std::sync::Mutex::new(std::collections::BTreeMap::new());
        let draw = |resolution: super::super::ArtResolution| drawn_art(|| "portrait sample".to_owned(), &catalogue, move |_| resolution.clone());
        let failed = super::super::ArtResolution::Failed { reason: "read failed".into(), retryable: true };
        let ready = super::super::ArtResolution::Ready {
            asset_id: "sample".into(), url: "art://sample".into(), origin: super::super::ArtOrigin::BaseGame, file: "gfx/sample.dds".into(),
            frame: super::super::ArtFrame { frame_index: 0, frame_count: 1, frame_width: 1, frame_height: 1, texture_width: 1, texture_height: 1, output_width: 1, output_height: 1 },
            provenance: super::super::ArtProvenance { from_current_install: true, label: "Current installation".into(), scope_key: "s".into() },
        };
        assert_eq!(keep_settled(&store, 1, || draw(failed.clone())), failed);
        assert_eq!(keep_settled(&store, 1, || draw(ready.clone())), ready, "the file recovered: the next ask draws it");
        assert_eq!(keep_settled(&store, 1, || panic!("a settled answer is kept")), ready);
        let refused = super::super::ArtResolution::Refused { reason: "unsupported".into(), construct: "sprite-render".into(), origin: None, file: None };
        assert_eq!(keep_settled(&store, 2, || draw(refused.clone())), refused);
        assert_eq!(keep_settled(&store, 2, || panic!("a refusal is kept")), refused);
    }

    #[test]
    fn when_neither_attempt_draws_a_refusal_or_failure_outranks_a_confirmed_miss() {
        let missing: super::super::ArtResolution = serde_json::from_str(r#"{"status":"confirmedMissing","reason":"no layer holds it"}"#).unwrap();
        let refused = super::super::ArtResolution::Refused { reason: "visual mod refused".into(), construct: "sprite-render".into(), origin: None, file: None };
        let failed = super::super::ArtResolution::Failed { reason: "read failed".into(), retryable: true };
        let unrecorded = super::super::ArtResolution::Unavailable { reason: "not recorded".into() };
        assert_eq!(neither_drew(refused.clone(), missing.clone()), refused, "the visual mods' refusal survives the recorded content's miss");
        assert_eq!(neither_drew(missing.clone(), refused.clone()), refused, "a miss followed by a refusal is the refusal");
        assert_eq!(neither_drew(failed.clone(), missing.clone()), failed);
        assert_eq!(neither_drew(unrecorded.clone(), missing.clone()), unrecorded);
        assert_eq!(neither_drew(missing.clone(), missing.clone()), missing, "only two completed misses are a confirmed miss");
    }
    use crate::catalogue::{
        tests::{folder_catalogue, layers_catalogue},
        texture_tests::{from_url, png_bytes},
    };
    use content_reader_image::Rgba8Image;

    fn frame(key: &str, frame: u32) -> SpriteFrame {
        SpriteFrame {
            key: key.into(),
            frame,
            recorded: None,
            text: None,
        }
    }

    #[test]
    fn sprite_artwork_uses_layered_exact_sprite_keys_and_replace_path() {
        let (_, base) = folder_catalogue(&[(
            "interface/base.gfx",
            "spriteTypes={spriteType={name=GFX_Hidden textureFile=gfx/hidden.png}}",
        )]);
        let (_, addon) = folder_catalogue(&[(
            "interface/mod.gfx",
            "spriteTypes={spriteType={name=GFX_Mixed_Mod textureFile=gfx/mod.png} spriteType={name=GFX_Missing textureFile=gfx/absent.png} spriteType={name=GFX_Strip textureFile=gfx/strip.png noOfFrames=3}}",
        )]);
        let image = Rgba8Image {
            width: 2,
            height: 1,
            pixels: vec![30, 80, 140, 255, 30, 80, 140, 255],
        };
        std::fs::create_dir_all(addon.0.join("gfx")).unwrap();
        std::fs::write(addon.0.join("gfx/mod.png"), png_bytes(&image)).unwrap();
        let colours = [[200, 20, 20, 255], [20, 200, 20, 255], [20, 20, 200, 255]];
        let strip = Rgba8Image { width: 3, height: 1, pixels: colours.concat() };
        std::fs::write(addon.0.join("gfx/strip.png"), png_bytes(&strip)).unwrap();
        let layers = content_reader_layers::ResolvedLayers::from_folders(&[
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
            .chain(["Mixed_Mod", "Strip", "Strip|3", "Absent_Icon"].map(|key| SpriteFrame { text: Some(true), ..frame(key, 0) }))
            .collect(),
            textures: vec![
                TextureFrame { path: "gfx/strip.png".into(), frame: 1, frames: 3 },
                TextureFrame { path: "gfx/absent.png".into(), frame: 0, frames: 1 },
            ],
        };
        let reply = images(&catalogue, &params, &AtomicBool::new(false)).unwrap();
        assert_eq!(reply.items.len(), 8);
        let texture = |path: &str| &reply.textures.iter().find(|art| art.path == path).unwrap().resolution;
        assert_eq!(from_url(texture("gfx/strip.png").url().unwrap()).pixels, colours[1], "a texture path draws the asked frame of its strip");
        assert!(matches!(texture("gfx/absent.png"), super::super::ArtResolution::ConfirmedMissing { .. }), "{:?}", texture("gfx/absent.png"));
        let text = |key: &str| reply.items.iter().find(|art| art.key == key).unwrap().icon.as_deref();
        assert_eq!(from_url(text("Mixed_Mod").unwrap()).pixels, image.pixels, "a text icon is drawn from the sprite its name declares");
        assert_eq!(text("Absent_Icon"), None, "an undeclared text icon is refused by name");
        assert_eq!(from_url(text("Strip").unwrap()).pixels, colours[0], "a text icon without a suffix draws the first frame");
        assert_eq!(from_url(text("Strip|3").unwrap()).pixels, colours[2], "a frame suffix draws that frame of the sprite its stem declares");
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

    fn shortened_strip() -> (Catalogue, String, Box<dyn std::any::Any>) {
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
        let layers = content_reader_layers::ResolvedLayers::from_folders(&[
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
            textures: Vec::new(),
        };
        let reply = images(&catalogue, &params, &AtomicBool::new(false)).unwrap();
        assert!(reply.warnings.is_empty(), "{:?}", reply.warnings);
        let past_strip = from_url(reply.items[1].icon.as_deref().unwrap());
        let expected_edge: Rgba8Image = Rgba8Image {
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
            textures: Vec::new(),
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
                textures: Vec::new(),
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
    fn sprite_artwork_validates_keys() {
        for sprites in [vec![], vec![frame("", 0)], vec![frame("key\0tail", 0)]] {
            assert!(SpriteArtwork::validate(&SpriteArtworkParams { sprites, textures: Vec::new() }).is_err());
        }
        let with_recorded = |recorded: &str| SpriteFrame {
            recorded: Some(recorded.into()),
            ..frame("GFX_b", 0)
        };
        assert!(
            SpriteArtwork::validate(&SpriteArtworkParams {
                sprites: vec![with_recorded("")],
                textures: Vec::new(),
            })
            .is_err()
        );
        assert!(
            SpriteArtwork::validate(&SpriteArtworkParams {
                sprites: vec![with_recorded("GFX_a")],
                textures: Vec::new(),
            })
            .is_ok()
        );
        let text = |frame_index: u32, text: bool| SpriteFrame { text: Some(text), ..frame("Mixed_Mod", frame_index) };
        assert!(SpriteArtwork::validate(&SpriteArtworkParams { sprites: vec![text(1, true)], textures: Vec::new() }).is_err(), "a text icon's frame is its name's");
        assert!(SpriteArtwork::validate(&SpriteArtworkParams { sprites: vec![text(0, false)], textures: Vec::new() }).is_err(), "text is asked as true or not at all");
        assert!(SpriteArtwork::validate(&SpriteArtworkParams { sprites: vec![text(0, true)], textures: Vec::new() }).is_ok());
    }

    #[test]
    fn a_text_icon_is_drawn_from_its_stems_sprite_at_its_suffixs_frame_and_a_sprite_at_its_asked_frame() {
        use content_reader_gfx::gfx::SpriteSet;
        use content_reader_gui::gui::native::sprites::{Plain, SpriteFacts, SpriteKind, Sprites};
        let facts = Sprites {
            sprites: vec![SpriteFacts {
                name: b"GFX_Mixed_Mod".to_vec(),
                load_type: 0,
                kind: SpriteKind::Plain(Plain { texture: b"gfx/mod.png".to_vec(), frames: 3, ..Plain::default() }),
                refusals: Vec::new(),
            }],
            ..Sprites::default()
        };
        let sprites = SpriteSet::new(&[("interface/mod.gfx".to_owned(), facts)]);
        let routed = |key: &str, asked: &SpriteFrame| -> io::Result<(String, u32)> {
            if asked.text == Some(true) {
                let (sprite, frame) = dashboard_text::catalogue::text_icon(&sprites, key)?;
                Ok((sprite.name.clone(), frame))
            } else {
                Ok((key.to_owned(), asked.frame))
            }
        };
        let text = |key: &str| SpriteFrame { text: Some(true), ..frame(key, 0) };
        assert_eq!(routed("Mixed_Mod|3", &text("Mixed_Mod|3")).unwrap(), ("GFX_Mixed_Mod".to_owned(), 2), "the suffix names the third frame, never the first");
        assert_eq!(routed("Mixed_Mod", &text("Mixed_Mod")).unwrap(), ("GFX_Mixed_Mod".to_owned(), 0));
        assert_eq!(routed("GFX_Mixed_Mod", &frame("GFX_Mixed_Mod", 2)).unwrap(), ("GFX_Mixed_Mod".to_owned(), 2), "a sprite is drawn at the frame asked");
        assert!(routed("Absent_Icon", &text("Absent_Icon")).is_err(), "an undeclared text icon is refused by name");
    }

    #[test]
    fn a_sprite_the_mod_chose_and_cannot_draw_is_the_recordings_own_choice() {
        use crate::test_fixtures::visual_mods::{
            GREEN, Machine, RED, fallback_line, first_pixel, png, select,
        };
        let machine = Machine::new("sprite_route_choice");
        machine.write(
            "interface/art.gfx",
            b"spriteTypes = { spriteType = { name = GFX_a textureFile = gfx/a.png }               spriteType = { name = GFX_b textureFile = gfx/b.png } }",
        );
        machine.write("gfx/a.png", &png(RED));
        machine.write("gfx/b.png", &png(GREEN));
        machine.write_mod("broken", "gfx/b.png", b"not a png");
        let (source, mut options) = machine.open();
        let ask = |options: &crate::products::ContentOptions, asked: SpriteFrame| {
            let catalogue = catalogue::catalogue(&source, options).unwrap();
            let (reply, line) = fallback_line(|| {
                images(
                    &catalogue,
                    &SpriteArtworkParams {
                        sprites: vec![asked],
                        textures: Vec::new(),
                    },
                    &AtomicBool::new(false),
                )
                .unwrap()
            });
            assert_eq!(reply.items.len(), 1);
            (first_pixel(reply.items[0].icon.as_deref().unwrap()), line)
        };
        let chosen = |recorded: &str| SpriteFrame {
            recorded: Some(recorded.into()),
            ..frame("GFX_b", 0)
        };
        select(&source, &mut options, &[]);
        assert_eq!(ask(&options, frame("GFX_b", 0)), (GREEN, None));
        select(&source, &mut options, &["broken"]);
        for _ in 0..2 {
            let (colour, line) = ask(&options, chosen("GFX_a"));
            assert_eq!(colour, RED, "the recording's own choice");
            assert!(line.is_some_and(|line| line.contains("sprite GFX_b")));
        }
        let (colour, line) = ask(&options, frame("GFX_b", 0));
        assert_eq!(colour, GREEN, "the recording's sprite of that name");
        assert!(line.is_some_and(|line| line.contains("sprite GFX_b")));
        let alone = SpriteFrame {
            recorded: Some("GFX_a".into()),
            ..frame("GFX_a", 0)
        };
        assert_eq!(ask(&options, alone), (RED, None));
    }
}
