use content_reader_key_names::key_name_products::KeyNamesExt as _;

use std::collections::BTreeSet;
use std::{collections::BTreeMap, io, sync::Arc};
use viewer_runtime::errors::examples;
#[cfg(test)]
use viewer_runtime::test_support::test_tga_rgba8;
use content_reader_layers::parallel::map;

use content_reader_layers::ResolvedLayers;
use content_reader_key_names::characters::{self, Authored};
use content_reader_gui::gui::native::resources;
use content_reader_gfx::gfx::SpriteSet;
use content_reader_image::{dds, native::Decode};

use viewer_native_gui::{lazy_icons::{LazyImages, LazyTextures, Lookup}, sprites::SpriteSheet};

fn asset_path(path: &str) -> String {
    path.split(['/', '\\'])
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join("/")
}

enum PortraitSource {
    Texture(String, u32),
    NoTexture,
    Undeclared,
    Refused(String),
}

fn portrait_source(sprites: &SpriteSet, name: &str) -> PortraitSource {
    if let Ok(sprite) = sprites.lookup(name) {
        match &sprite.texture {
            Some(path) => PortraitSource::Texture(asset_path(path), sprite.frames),
            None => PortraitSource::NoTexture,
        }
    } else if let Some(why) = sprites.refusal(name) {
        PortraitSource::Refused(why)
    } else if name.starts_with("GFX_") {
        PortraitSource::Undeclared
    } else {
        PortraitSource::Texture(asset_path(name), 1)
    }
}

pub fn identity_key(identity: &str) -> String {
    if identity.starts_with("GFX_") {
        identity.to_owned()
    } else {
        asset_path(identity).to_ascii_lowercase()
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct TokenSprite {
    name: String,
    path: String,
    frames: u32,
}

pub struct Portraits {
    tokens: BTreeMap<String, TokenSprite>,
    aliases: BTreeMap<String, Vec<String>>,
    declared: BTreeMap<String, TokenSprite>,
    images: Arc<LazyImages>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Resolved {
    Sheet(SpriteSheet),
    Pending,
    Unresolved,
}

impl Portraits {
    pub fn sprite(&self, name: &str) -> Resolved {
        self.declared
            .get(name)
            .and_then(|sprite| self.sheet(sprite))
            .unwrap_or(Resolved::Unresolved)
    }

    pub fn resolve(&self, key: &str) -> Resolved {
        if key.starts_with("GFX_") {
            return self
                .declared
                .get(key)
                .and_then(|sprite| self.sheet(sprite))
                .unwrap_or(Resolved::Unresolved);
        }
        for token in self.aliases.get(key).into_iter().flatten() {
            if let Some(resolved) = self.tokens.get(token).and_then(|sprite| self.sheet(sprite)) {
                return resolved;
            }
        }
        Resolved::Unresolved
    }

    fn sheet(&self, sprite: &TokenSprite) -> Option<Resolved> {
        match self.images.get(&sprite.path) {
            Lookup::Pending => Some(Resolved::Pending),
            Lookup::Failed => None,
            Lookup::Loaded { texture, size } => (sprite.frames != 0 && size[0] >= sprite.frames)
                .then(|| {
                    Resolved::Sheet(SpriteSheet {
                        name: sprite.name.clone(),
                        texture,
                        frames: sprite.frames,
                        size,
                    })
                }),
        }
    }

    pub fn sheet_of_texture(&self, texture: usize) -> Option<SpriteSheet> {
        let path = self.images.path_of(texture)?;
        let Lookup::Loaded { size, .. } = self.images.peek(&path) else {
            return None;
        };
        self.tokens
            .values()
            .chain(self.declared.values())
            .find(|sprite| sprite.path == path)
            .map(|sprite| SpriteSheet {
                name: sprite.name.clone(),
                texture,
                frames: sprite.frames,
                size,
            })
    }
}

pub fn load_sprite(
    layers: &ResolvedLayers,
    sprites: &SpriteSet,
    name: &str,
    decode: Decode,
    images: &mut Vec<dds::Texture>,
) -> io::Result<SpriteSheet> {
    let invalid = |message: String| io::Error::new(io::ErrorKind::InvalidData, message);
    let sprite = sprites.lookup(name).map_err(invalid)?;
    let path = asset_path(
        sprite
            .texture
            .as_deref()
            .ok_or_else(|| invalid(format!("sprite {name} names no texturefile")))?,
    );
    let texture = resources::stack_texture(layers, path.as_bytes(), decode)?
        .map_err(|why| invalid(format!("{name}: {why}")))?;
    let texture = texture.image;
    if sprite.frames == 0 || texture.width < sprite.frames {
        return Err(invalid(format!(
            "{path}: not a {}-frame strip",
            sprite.frames
        )));
    }
    let size = [texture.width, texture.height];
    images.push(texture);
    Ok(SpriteSheet {
        name: name.to_owned(),
        texture: images.len() - 1,
        frames: sprite.frames,
        size,
    })
}

pub fn load(
    shared: &Arc<ResolvedLayers>,
    sprites: &SpriteSet,
    decode: Decode,
    lazy: &Arc<LazyTextures>,
) -> io::Result<(Portraits, Vec<String>)> {
    let layers: &ResolvedLayers = shared;
    let paths = characters::paths(layers)?;
    let names = layers.key_names()?;
    type Parsed = (BTreeMap<String, Authored>, Option<String>);
    let parsed: Vec<io::Result<Parsed>> = map(&paths, |path| {
        characters::portraits(&path.read(layers)?, &names, "army", "large")
            .map_err(|error| io::Error::new(error.kind(), format!("{path}: {error}")))
    });
    let mut characters = BTreeMap::<String, Authored>::new();
    let mut coverage = content_reader_parser::coverage::Coverage::default();
    for (path, parsed_path) in paths.iter().zip(parsed) {
        coverage.next_file();
        let (found, stop) = parsed_path?;
        for (token, portrait) in found {
            coverage.add(&token);
            characters
                .entry(token)
                .and_modify(|previous| previous.agree(portrait.clone()))
                .or_insert(portrait);
        }
        if let Some(cause) = stop {
            coverage.gap(&path.text(), cause);
        }
    }
    for (token, portrait) in &mut characters {
        if let Some(gap) = coverage.unknown(token) {
            *portrait = Authored::Unknown(gap.to_string());
        }
    }
    let mut entries: Vec<(String, String)> = Vec::new();
    let mut ambiguous = Vec::new();
    let mut unknown = Vec::new();
    for (token, portrait) in &characters {
        match portrait {
            Authored::Missing => {}
            Authored::Asset(name) => entries.push((token.clone(), name.clone())),
            Authored::Ambiguous(names) => {
                ambiguous.push(token.clone());
                entries.extend(names.iter().map(|name| (name.clone(), name.clone())));
            }
            Authored::Unknown(cause) => unknown.push(format!("{token}: {cause}")),
        }
    }
    let mut tokens = BTreeMap::new();
    let mut aliases: BTreeMap<String, Vec<String>> = BTreeMap::new();
    let mut textures = BTreeSet::new();
    let mut rejected = Vec::new();
    for (token, name) in entries {
        let (path, frames) = match portrait_source(sprites, &name) {
            PortraitSource::Texture(path, frames) => (path, frames),
            PortraitSource::NoTexture => {
                rejected.push(format!("{token}: {name} has no texture"));
                continue;
            }
            PortraitSource::Undeclared => {
                rejected.push(format!("{token}: undeclared sprite {name}"));
                continue;
            }
            PortraitSource::Refused(why) => {
                rejected.push(format!("{token}: {why}"));
                continue;
            }
        };
        let mut alias = |key: String| {
            let tokens = aliases.entry(key).or_default();
            if !tokens.contains(&token) {
                tokens.push(token.clone());
            }
        };
        if !name.starts_with("GFX_") {
            alias(identity_key(&name));
        }
        if frames == 1 {
            alias(path.to_ascii_lowercase());
        }
        textures.insert(path.clone());
        tokens.insert(token, TokenSprite { name, path, frames });
    }
    let declared = sprites
        .iter()
        .filter_map(|(name, sprite)| {
            let path = asset_path(sprite.texture.as_deref()?);
            Some((
                name.clone(),
                TokenSprite {
                    name: name.clone(),
                    path,
                    frames: sprite.frames,
                },
            ))
        })
        .collect();
    let reader = Arc::clone(shared);
    let recorded = reader.without_overlays();
    let images = LazyImages::new(
        "HQ portrait",
        Arc::new(move |path: &str| {
            let texture = resources::stack_texture_or_recorded(
                &reader,
                recorded.as_ref(),
                path.as_bytes(),
                decode,
            )?
            .map_err(|why| io::Error::new(io::ErrorKind::InvalidData, why.to_string()))?;
            Ok(texture.image)
        }),
        lazy,
    );
    let mut warnings = vec![format!(
        "HQ portraits: {} character tokens, {} distinct authored textures, decoded when a counter first draws them and kept while drawn; a texture that cannot be decoded is not drawn. Portrait textures outside the content coverage roots remain unverified. Formats before 23 draw the authored token and do not preserve runtime character portrait overrides or non-character portrait strings; format 23 resolves a recorded sprite identity against every declared sprite and a recorded texture path against this authored set only.",
        tokens.len(),
        textures.len()
    )];
    for gap in coverage.gaps() {
        warnings.push(format!(
            "HQ portraits: {gap}; any character it could define draws no token portrait."
        ));
    }
    if !unknown.is_empty() {
        warnings.push(format!(
            "HQ portraits: {} character tokens are unknown, so no token draws from them ({}).",
            unknown.len(),
            examples(&unknown)
        ));
    }
    if !ambiguous.is_empty() {
        warnings.push(format!(
            "HQ portraits: {} character tokens have ambiguous or incomplete army.large alternatives, so no token draws from them ({}); instance/DLC triggers are not guessed, but a recorded identity still resolves each alternative's own sprite.",
            ambiguous.len(),
            examples(&ambiguous)
        ));
    }
    if !rejected.is_empty() {
        warnings.push(format!(
            "HQ portraits: {} asset declarations could not be loaded ({}).",
            rejected.len(),
            examples(&rejected)
        ));
    }
    Ok((
        Portraits {
            tokens,
            aliases,
            declared,
            images,
        },
        warnings,
    ))
}

#[cfg(test)]
mod tests {

    #[test]
    fn portrait_sprite_uses_the_native_decoder_and_client_setting() {
        content_reader_script::test_game::enter();
        use super::*;
        let root = std::env::temp_dir().join(format!(
            "hoi4_counter_portrait_native_texture_{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("gfx")).unwrap();
        std::fs::write(root.join("gfx/portrait.tga"), test_tga_rgba8(3, 2)).unwrap();
        let layers = ResolvedLayers::from_folders(&[(root.as_path(), &[][..])]).unwrap();
        let sprites = SpriteSet::new(&[(
            "interface/test.gfx".into(),
            content_reader_gui::gui::native::sprites::read_sprites(
                b"spriteTypes = { spriteType = { name = GFX_portrait texturefile = gfx/portrait.tga } }",
            ),
        )]);
        for (decode, expected) in [
            (Decode::default(), [3, 2]),
            (
                Decode {
                    force_pow2_textures: true,
                },
                [4, 2],
            ),
        ] {
            let sheet =
                load_sprite(&layers, &sprites, "GFX_portrait", decode, &mut Vec::new()).unwrap();
            assert_eq!(sheet.size, expected);
        }
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn an_identity_resolves_to_the_first_token_whose_texture_decodes() {
        use super::*;
        let lazy = LazyTextures::new();
        lazy.set_background(false);
        let images = LazyImages::new(
            "test portrait",
            Arc::new(|path: &str| {
                if path.contains("broken") {
                    return Err(io::Error::other("broken"));
                }
                Ok(dds::Texture::from_rgba8(content_reader_image::Rgba8Image {
                    width: 8,
                    height: 4,
                    pixels: vec![0; 128],
                }))
            }),
            &lazy,
        );
        let sprite = |name: &str, path: &str, frames| TokenSprite {
            name: name.into(),
            path: path.into(),
            frames,
        };
        let portraits = Portraits {
            tokens: BTreeMap::from([
                ("a".into(), sprite("GFX_a", "gfx/leaders/broken.dds", 1)),
                ("b".into(), sprite("GFX_a", "gfx/leaders/b.dds", 1)),
                ("wide".into(), sprite("GFX_w", "gfx/leaders/w.dds", 9)),
            ]),
            aliases: BTreeMap::from([
                ("gfx/leaders/a.dds".into(), vec!["a".into(), "b".into()]),
                ("gfx/leaders/w.dds".into(), vec!["wide".into()]),
            ]),
            declared: BTreeMap::new(),
            images,
        };
        assert_eq!(portraits.resolve("gfx/leaders/a.dds"), Resolved::Pending);
        lazy.settle();
        assert_eq!(portraits.resolve("gfx/leaders/a.dds"), Resolved::Pending);
        lazy.settle();
        let Resolved::Sheet(sheet) = portraits.resolve("gfx/leaders/a.dds") else {
            panic!("resolved")
        };
        assert_eq!((sheet.name.as_str(), sheet.size), ("GFX_a", [8, 4]));
        assert_eq!(
            portraits.sheet_of_texture(sheet.texture).map(|s| s.name),
            Some("GFX_a".into())
        );
        portraits.resolve("gfx/leaders/w.dds");
        lazy.settle();
        assert_eq!(portraits.resolve("gfx/leaders/w.dds"), Resolved::Unresolved);
        assert_eq!(
            portraits.resolve("gfx/leaders/none.dds"),
            Resolved::Unresolved
        );
    }

    #[test]
    fn a_recorded_sprite_no_character_names_resolves_through_the_declared_sprites() {
        use super::*;
        let lazy = LazyTextures::new();
        lazy.set_background(false);
        let images = LazyImages::new(
            "test portrait",
            Arc::new(|path: &str| {
                let width = if path.contains("generic") { 6 } else { 8 };
                Ok(dds::Texture::from_rgba8(content_reader_image::Rgba8Image {
                    width,
                    height: 4,
                    pixels: vec![0; width as usize * 16],
                }))
            }),
            &lazy,
        );
        let sprite = |name: &str, path: &str| TokenSprite {
            name: name.into(),
            path: path.into(),
            frames: 1,
        };
        let portraits = Portraits {
            tokens: BTreeMap::from([("a".into(), sprite("GFX_a", "gfx/leaders/a.dds"))]),
            aliases: BTreeMap::from([("gfx/leaders/a.dds".into(), vec!["a".into()])]),
            declared: BTreeMap::from([
                ("GFX_a".into(), sprite("GFX_a", "gfx/leaders/a.dds")),
                (
                    "GFX_random_17".into(),
                    sprite("GFX_random_17", "gfx/leaders/generic_17.dds"),
                ),
            ]),
            images,
        };
        assert_eq!(portraits.resolve("GFX_random_17"), Resolved::Pending);
        lazy.settle();
        let Resolved::Sheet(sheet) = portraits.resolve("GFX_random_17") else {
            panic!("the declared sprite resolves")
        };
        assert_eq!((sheet.name.as_str(), sheet.size), ("GFX_random_17", [6, 4]));
        assert_eq!(
            portraits.sheet_of_texture(sheet.texture).map(|s| s.name),
            Some("GFX_random_17".into())
        );
        portraits.resolve("GFX_a");
        lazy.settle();
        let (Resolved::Sheet(by_name), Resolved::Sheet(by_path)) = (
            portraits.resolve("GFX_a"),
            portraits.resolve("gfx/leaders/a.dds"),
        ) else {
            panic!("both resolve")
        };
        assert_eq!((by_name.texture, by_name.size), (by_path.texture, [8, 4]));
        assert_eq!(portraits.resolve("GFX_undeclared"), Resolved::Unresolved);
        assert_eq!(
            portraits.resolve("gfx/leaders/generic_17.dds"),
            Resolved::Unresolved
        );
    }
}
