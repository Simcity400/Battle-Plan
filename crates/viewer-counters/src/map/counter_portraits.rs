//! Authored character portraits for headquarters counters.
//!
//! Army categories zero and one select the large army portrait. Recorded runtime
//! portrait holders and non-character portrait strings take precedence where available;
//! older formats use the leader character's authored portrait.
//!
//! A recorded `GFX_` sprite resolves through all declared content sprites
//! ([`Portraits::declared`]). Texture paths resolve through authored character portrait
//! aliases ([`Portraits::aliases`]). Effective files and textures use the resolved
//! content layers. Conditional character alternatives and duplicate effective
//! definitions must agree on the army portrait; DLC and scripted triggers are not
//! evaluated here.

use std::collections::BTreeSet;
use std::{collections::BTreeMap, io, sync::Arc};
use viewer_foundation::errors::examples;
#[cfg(test)]
use viewer_foundation::map::test_support::test_tga_rgba8;
use viewer_foundation::parallel::map;

use installed_content::{
    ResolvedLayers,
    characters::{self, Authored},
    gfx::SpriteSet,
    gui::native::resources,
    image::{dds, native::Decode},
};

use viewer_foundation::map::{
    lazy_icons::{LazyImages, LazyTextures, Lookup},
    sprites::SpriteSheet,
};

fn asset_path(path: &str) -> String {
    path.split(['/', '\\'])
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join("/")
}

/// Where a portrait name resolves: a texture read, or a dead end the merge
/// loop reports with the same message as the serial load.
enum PortraitSource {
    Texture(String, u32),
    NoTexture,
    Undeclared,
    /// Its latest declaration is refused, with why.
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

/// The lookup key of a recorded portrait identity: a `GFX_` sprite name verbatim, a texture path
/// with its separators normalised and ASCII case folded, as the content layers resolve paths.
pub fn identity_key(identity: &str) -> String {
    if identity.starts_with("GFX_") {
        identity.to_owned()
    } else {
        asset_path(identity).to_ascii_lowercase()
    }
}

/// One character token's authored army portrait: its sprite name, texture and frame count.
#[derive(Clone, Debug, PartialEq, Eq)]
struct TokenSprite {
    name: String,
    path: String,
    frames: u32,
}

/// Authored portraits by character token, with the sprite names and texture paths that reach
/// each one. Textures decode on first draw into a bounded LRU ([`LazyImages`]).
pub struct Portraits {
    tokens: BTreeMap<String, TokenSprite>,
    /// A texture path's [`identity_key`] to the tokens it can name, in the order the serial load
    /// aliased them: the first whose texture decodes to a valid strip is the one drawn.
    aliases: BTreeMap<String, Vec<String>>,
    /// Every declared sprite with a texture, by name. the game looks a kind-0 descriptor
    /// up among all sprites, so a recorded sprite no character token names still draws.
    declared: BTreeMap<String, TokenSprite>,
    images: Arc<LazyImages>,
}

/// What a recorded portrait identity resolves to this frame.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Resolved {
    Sheet(SpriteSheet),
    /// A texture it depends on is queued; nothing is drawn until it decodes.
    Pending,
    /// Nothing loaded answers it, as when the serial load rejected its texture.
    Unresolved,
}

impl Portraits {
    /// The sheet an [`identity_key`] resolves to.
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

    /// One sprite's sheet, `Pending` while its texture decodes, or `None` when the texture
    /// cannot be decoded or is narrower than its frame count.
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

    /// The first token's sheet (in token order) drawn from a resident texture, for the census.
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

/// Portrait textures kept resident at once, beyond the ones the current frame draws. A
pub const PORTRAIT_CAPACITY: usize = 1024;

/// Loads one declared portrait sprite, verifying covered texture paths.
/// Fall back to `GFX_leader_unknown`.
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
    // The game applies the client's force_pow2_textures.
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

/// Indexes every authored army portrait, keyed by character token.
///
/// `SpriteSheet` describes a complete horizontal strip, not an arbitrary atlas rectangle.
/// Native decoded extents and mipmaps are retained. Character files are read
/// once per content set; textures are read and decoded the first time a counter draws them,
/// with no per-hour file reads, and at most [`PORTRAIT_CAPACITY`] stay resident beyond the
/// frame's own.
pub fn load(
    shared: &Arc<ResolvedLayers>,
    sprites: &SpriteSet,
    decode: Decode,
    lazy: &Arc<LazyTextures>,
) -> io::Result<(Portraits, Vec<String>)> {
    let layers: &ResolvedLayers = shared;
    // The loader's own list and order.
    let paths = characters::paths(layers);
    let names = layers.key_names()?;
    // Character files read and parse on a bounded pool; the merge below runs
    // in that order, so token agreement matches the serial load.
    type Parsed = (BTreeMap<String, Authored>, Option<String>);
    let parsed: Vec<io::Result<Parsed>> = map(&paths, |path| {
        characters::portraits(&layers.read(path)?, &names, "army", "large")
            .map_err(|error| io::Error::new(error.kind(), format!("{path}: {error}")))
    });
    // A file whose reading stops short leaves every character it could
    // define or redefine unknown, with the file and the cause.
    let mut characters = BTreeMap::<String, Authored>::new();
    let mut coverage = installed_content::coverage::Coverage::default();
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
            coverage.gap(path, cause);
        }
    }
    for (token, portrait) in &mut characters {
        if let Some(gap) = coverage.unknown(token) {
            *portrait = Authored::Unknown(gap.to_string());
        }
    }
    // Each character's own sprite, then the alternatives of the ambiguous ones. A recorded
    // identity names the sprite outright, so an instance set the viewer cannot choose between
    // still resolves; only the pre-format-23 token path stays undrawn.
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
        // A `GFX_` name resolves through `declared`; only texture paths need the tokens.
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
    let images = LazyImages::new(
        "HQ portrait",
        Arc::new(move |path: &str| {
            let texture = resources::stack_texture(&reader, path.as_bytes(), decode)?
                .map_err(|why| io::Error::new(io::ErrorKind::InvalidData, why.to_string()))?;
            Ok(texture.image)
        }),
        PORTRAIT_CAPACITY,
        lazy,
    );
    let mut warnings = vec![format!(
        "HQ portraits: {} character tokens, {} distinct authored textures, decoded when a counter first draws them and at most {PORTRAIT_CAPACITY} kept beyond the frame's own; a texture that cannot be decoded is not drawn. Portrait textures outside the content coverage roots remain unverified. Formats before 23 draw the authored token and do not preserve runtime character portrait overrides or non-character portrait strings; format 23 resolves a recorded sprite identity against every declared sprite and a recorded texture path against this authored set only.",
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
        engine::executable_tables::initialize_for_tests()
            .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
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
            installed_content::gui::native::sprites::read_sprites(
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
                Ok(dds::Texture::from_rgba8(installed_content::image::Rgba8Image {
                    width: 8,
                    height: 4,
                    pixels: vec![0; 128],
                }))
            }),
            4,
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
        // Queued first: nothing is drawn, and nothing falls back yet.
        assert_eq!(portraits.resolve("gfx/leaders/a.dds"), Resolved::Pending);
        lazy.settle();
        // The first token's texture is broken, so the second is queued.
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
        // A strip narrower than its frame count is refused, as at load.
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
                Ok(dds::Texture::from_rgba8(installed_content::image::Rgba8Image {
                    width,
                    height: 4,
                    pixels: vec![0; width as usize * 16],
                }))
            }),
            4,
            &lazy,
        );
        let sprite = |name: &str, path: &str| TokenSprite {
            name: name.into(),
            path: path.into(),
            frames: 1,
        };
        // A generated leader's holder names a random portrait that no character file uses.
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
        // A character's sprite and its texture path reach the same texture.
        portraits.resolve("GFX_a");
        lazy.settle();
        let (Resolved::Sheet(by_name), Resolved::Sheet(by_path)) = (
            portraits.resolve("GFX_a"),
            portraits.resolve("gfx/leaders/a.dds"),
        ) else {
            panic!("both resolve")
        };
        assert_eq!((by_name.texture, by_name.size), (by_path.texture, [8, 4]));
        // A sprite nothing declares stays unresolved, and so does a texture path no character
        // names, even one a declared sprite uses.
        assert_eq!(portraits.resolve("GFX_undeclared"), Resolved::Unresolved);
        assert_eq!(
            portraits.resolve("gfx/leaders/generic_17.dds"),
            Resolved::Unresolved
        );
    }
}
