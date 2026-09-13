//! Sprite declarations from the `spriteTypes` blocks of `interface/*.gfx`.
//!
//! The engine's own files write `spriteType` and `SpriteType`, and declare
//! frame-animated, cornered-tile, text, progress-bar, arrow and masked-shield
//! kinds in the same block. The kind is kept as authored and every item of the
//! block is retained, so a pass can read a field this reader does not name.

use std::{collections::BTreeMap, io};

use crate::{
    ResolvedLayers,
    fields::{block, integer, invalid, key_str, last, text},
    parser::{self, Document, Item, MAX_DEPTH},
};

/// Collapses an authored asset path onto one the layers can resolve.
///
/// `interface/core.gfx` writes `gfx//interface//warning_11x24.dds` and two
/// sprites use a backslash, so 1125 of the shipped sprite textures would be
/// unreachable without this. The authored text stays in the parsed block.
pub(crate) fn content_path(raw: &str) -> String {
    let mut path = String::with_capacity(raw.len());
    for part in raw.split(['/', '\\']).filter(|part| !part.is_empty()) {
        if !path.is_empty() {
            path.push('/');
        }
        path.push_str(part);
    }
    path
}

/// `interface/*.gfx` paths in layer order, then path order.
pub(crate) fn gfx_paths(layers: &ResolvedLayers) -> Vec<String> {
    let mut paths = layers
        .paths()
        .filter(|path| {
            let lower = path.to_ascii_lowercase();
            lower.starts_with("interface/")
                && lower.ends_with(".gfx")
                && !lower["interface/".len()..].contains('/')
        })
        .map(|path| (layers.layer_index(path).unwrap_or(0), path.to_owned()))
        .collect::<Vec<_>>();
    paths.sort();
    paths.into_iter().map(|(_, path)| path).collect()
}

/// Parses an interface file, closing blocks the file leaves open at its end.
///
/// The shipped `interface/powerbalanceview.gfx` stops inside its `spriteTypes`
/// block with no closing brace, and the game loads it anyway. Returns whether
/// braces had to be supplied so a caller can report the file. A truncated save
/// is still an error: only this reader recovers, never [`crate::parser::parse`].
pub(crate) fn parse_interface(input: &[u8]) -> io::Result<(Document, bool)> {
    let first = match parser::parse(input) {
        Ok(document) => return Ok((document, false)),
        Err(error) => error,
    };
    for missing in 1..=MAX_DEPTH {
        let mut repaired = input.to_vec();
        repaired.extend(std::iter::repeat_n(b'}', missing));
        if let Ok(document) = parser::parse(&repaired) {
            return Ok((document, true));
        }
    }
    Err(first)
}

/// One entry of a `spriteTypes` block.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SpriteType {
    /// `GFX_...` name the interface and the map passes refer to.
    pub name: String,
    /// The authored block key, such as `spriteType` or `corneredTileSpriteType`.
    pub kind: String,
    /// `texturefile` or `textureFile` as a layer-relative path, with the
    /// doubled and back slashes the shipped files write collapsed. The authored
    /// text stays in `items`.
    pub texture: Option<String>,
    /// `noOfFrames`; 1 when the block does not declare it.
    pub frames: u32,
    pub always_transparent: bool,
    /// Every item of the block, in source order.
    pub items: Vec<Item>,
    /// The `interface/*.gfx` file the sprite came from.
    pub source: String,
}

fn optional_text(items: &[Item], key: &str, what: &str) -> io::Result<Option<String>> {
    match last(items, key.as_bytes()) {
        Some(value) => Ok(Some(text(value, what)?.to_owned())),
        None => Ok(None),
    }
}

/// Parses every `spriteTypes` block of one `.gfx` file, in source order.
pub fn parse_sprites(input: &[u8], source: &str) -> io::Result<Vec<SpriteType>> {
    let (document, _) = parse_interface(input)?;
    sprites_of(&document, source)
}

/// Collects the sprites of an already parsed `.gfx` document.
fn sprites_of(document: &Document, source: &str) -> io::Result<Vec<SpriteType>> {
    let mut sprites = Vec::new();
    for root in crate::fields::all(&document.items, b"spriteTypes") {
        for item in block(root, "spriteTypes")? {
            let kind = key_str(item, "spriteTypes")?.to_owned();
            let items = block(&item.value, &kind)?;
            let Some(name) = optional_text(items, "name", "sprite name")? else {
                // A block with no name cannot be referred to, so it carries
                // nothing a pass can use; skipping it keeps a modded file usable.
                continue;
            };
            let texture = match optional_text(items, "texturefile", "sprite texturefile")? {
                Some(path) => Some(path),
                None => optional_text(items, "textureFile", "sprite textureFile")?,
            }
            .map(|path| content_path(&path));
            sprites.push(SpriteType {
                name,
                kind,
                texture,
                frames: match last(items, b"noOfFrames") {
                    Some(value) => integer(value, "noOfFrames")?,
                    None => 1,
                },
                always_transparent: matches!(
                    last(items, b"alwaystransparent").and_then(parser::Value::scalar_str),
                    Some("yes")
                ),
                items: items.to_vec(),
                source: source.to_owned(),
            });
        }
    }
    Ok(sprites)
}

/// Every sprite of the layers, plus the files whose blocks had to be closed.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SpriteSet {
    pub sprites: BTreeMap<String, SpriteType>,
    /// Files that end inside an open block; see [`parse_interface`].
    pub recovered: Vec<String>,
}

/// Reads every sprite declared under `interface/*.gfx` across the layers.
///
/// A name declared twice keeps the later declaration: the layer order first,
/// then the path in byte order, then the order inside the file.
pub fn read(layers: &ResolvedLayers) -> io::Result<SpriteSet> {
    let mut set = SpriteSet::default();
    for path in gfx_paths(layers) {
        let bytes = layers.read(&path)?;
        let (document, recovered) =
            parse_interface(&bytes).map_err(|error| invalid(format!("{path}: {error}")))?;
        if recovered {
            set.recovered.push(path.clone());
        }
        for sprite in sprites_of(&document, &path)? {
            set.sprites.insert(sprite.name.clone(), sprite);
        }
    }
    Ok(set)
}

/// Reads every sprite declared under `interface/*.gfx` across the layers.
pub fn sprites(layers: &ResolvedLayers) -> io::Result<BTreeMap<String, SpriteType>> {
    Ok(read(layers)?.sprites)
}

#[cfg(test)]
mod tests {
    use super::*;

    const GFX: &[u8] = br#"
spriteTypes = {
    spriteType = {
        name = "GFX_strait_canal_onmap_icon"
        texturefile = "gfx/interface/strait_canal_onmap_icon.dds"
    }
    spriteType = {
        name = "GFX_onmap_action_icon"
        texturefile = "gfx/interface/onmap_action_icon.dds"
        noOfFrames = 14
        alwaystransparent = yes
    }
    SpriteType = {
        name = "GFX_capital_mapicon"
        textureFile = "gfx/interface/capital.dds"
        effectFile = "gfx/FX/buttonstate.lua"
    }
    corneredTileSpriteType = {
        name = "GFX_tile"
        textureFile = "gfx/interface/tile.dds"
        size = { x = 8 y = 8 }
        borderSize = { x = 2 y = 2 }
    }
    progressbartype = {
        name = "GFX_bar"
        color = { 1.0 0.0 0.0 }
        size = { x = 10 y = 4 }
    }
}
"#;

    #[test]
    fn authored_asset_paths_collapse_onto_resolvable_ones() {
        assert_eq!(
            content_path("gfx//interface//warning_11x24.dds"),
            "gfx/interface/warning_11x24.dds"
        );
        assert_eq!(content_path("gfx\\interface\\a.dds"), "gfx/interface/a.dds");
        assert_eq!(content_path("gfx/interface/a.dds"), "gfx/interface/a.dds");
        assert_eq!(content_path("/leading/"), "leading");
        assert_eq!(content_path(""), "");
    }

    #[test]
    fn both_spellings_of_the_sprite_block_and_texture_key_are_read() {
        let sprites = parse_sprites(GFX, "interface/mapicons.gfx").unwrap();
        assert_eq!(sprites.len(), 5);
        assert_eq!(sprites[0].kind, "spriteType");
        assert_eq!(sprites[0].frames, 1);
        assert!(!sprites[0].always_transparent);
        assert_eq!(sprites[1].frames, 14);
        assert!(sprites[1].always_transparent);
        assert_eq!(sprites[2].kind, "SpriteType");
        assert_eq!(
            sprites[2].texture.as_deref(),
            Some("gfx/interface/capital.dds")
        );
        assert_eq!(sprites[3].kind, "corneredTileSpriteType");
        // interface/core.gfx writes doubled slashes in its texture paths.
        let doubled = parse_sprites(
            br#"spriteTypes = { spriteType = { name = "GFX_a"
                texturefile = "gfx//interface//warning_11x24.dds" } }"#,
            "a.gfx",
        )
        .unwrap();
        assert_eq!(
            doubled[0].texture.as_deref(),
            Some("gfx/interface/warning_11x24.dds")
        );
        assert_eq!(
            crate::fields::last(&doubled[0].items, b"texturefile")
                .unwrap()
                .scalar_str(),
            Some("gfx//interface//warning_11x24.dds")
        );
        assert_eq!(sprites[3].source, "interface/mapicons.gfx");
        // A progress bar has no texture but is still declared.
        assert_eq!(sprites[4].texture, None);
    }

    #[test]
    fn the_whole_block_is_retained_for_fields_this_reader_does_not_name() {
        let sprites = parse_sprites(GFX, "a.gfx").unwrap();
        let border = crate::fields::last(&sprites[3].items, b"borderSize").unwrap();
        assert_eq!(block(border, "borderSize").unwrap().len(), 2);
        let effect = crate::fields::last(&sprites[2].items, b"effectFile").unwrap();
        assert_eq!(effect.scalar_str(), Some("gfx/FX/buttonstate.lua"));
    }

    #[test]
    fn unnamed_blocks_are_skipped_and_bad_fields_are_named() {
        let sprites = parse_sprites(
            b"spriteTypes = { spriteType = { texturefile = \"x\" } }",
            "a.gfx",
        )
        .unwrap();
        assert!(sprites.is_empty());
        for (input, message) in [
            (
                br#"spriteTypes = { spriteType = { name = "a" noOfFrames = many } }"#.as_slice(),
                "noOfFrames is not an integer",
            ),
            (
                br#"spriteTypes = { spriteType = { name = { } } }"#,
                "sprite name is not a text scalar",
            ),
            (
                br#"spriteTypes = { spriteType = 1 }"#,
                "spriteType is not a block",
            ),
            (
                br#"spriteTypes = { { } }"#,
                "spriteTypes has an unnamed entry",
            ),
        ] {
            let error = parse_sprites(input, "a.gfx").unwrap_err().to_string();
            assert!(error.contains(message), "{error} lacks {message}");
        }
    }

    #[test]
    fn a_file_that_ends_inside_an_open_block_is_closed_and_reported() {
        // interface/powerbalanceview.gfx ships without its final brace.
        let truncated = br#"spriteTypes = {
            spriteType = { name = "GFX_a" texturefile = "a.dds" }
            spriteType = { name = "GFX_b" texturefile = "b.dds" }"#;
        let (document, recovered) = parse_interface(truncated).unwrap();
        assert!(recovered);
        assert_eq!(sprites_of(&document, "a.gfx").unwrap().len(), 2);
        let (_, recovered) = parse_interface(b"spriteTypes = { }").unwrap();
        assert!(!recovered);
        // A closing brace too many is still an error, not a recovery.
        assert!(parse_interface(b"spriteTypes = { } }").is_err());
    }

    #[test]
    fn a_file_with_no_sprite_block_yields_nothing() {
        assert!(
            parse_sprites(b"bitmapfonts = { bitmapfont = { name = \"x\" } }", "a.gfx")
                .unwrap()
                .is_empty()
        );
    }
}
