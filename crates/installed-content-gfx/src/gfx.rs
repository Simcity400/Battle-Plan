//! The sprites of `interface/**/*.gfx` by name for consumers needing a texture path and frame
//! count. Files use [`sprites::read_layers`] in the game's load order; the latest declaration
//! of each exact name wins. [`SpriteType::facts`] retains every stored field.
//! [`SpriteSet::lookup`] distinguishes refused declarations from absent names, preventing
//! silent fallback past a refusal.

use std::collections::BTreeMap;

use crate::{
    ResolvedLayers,
    gui::native::sprites::{self, SpriteFacts, SpriteKind, Sprites},
};

/// Collapse authored asset paths into paths the layers can resolve. Repeated slashes and
/// backslashes normalize to one `/`; the authored text remains in the declaration.
pub fn content_path(raw: &str) -> String {
    let mut path = String::with_capacity(raw.len());
    for part in raw.split(['/', '\\']).filter(|part| !part.is_empty()) {
        if !path.is_empty() {
            path.push('/');
        }
        path.push_str(part);
    }
    path
}

/// One sprite declaration, as a consumer that draws a texture's frames takes it.
#[derive(Clone, Debug, PartialEq)]
pub struct SpriteType {
    /// `GFX_...` name the interface and the map passes refer to.
    pub name: String,
    /// The kind's one texture as a layer-relative path (the plain family's, a cornered tile's,
    /// a text sprite's or a resizeable sprite's `textureFile`), with the doubled and back
    /// slashes the shipped files write collapsed; `None` for a kind without one, or when the
    /// declaration names none.
    pub texture: Option<String>,
    /// The kind's `noOfFrames`; 1 for a kind without one.
    pub frames: u32,
    /// Everything the game keeps of the declaration.
    pub facts: SpriteFacts,
}

impl SpriteType {
    /// The consumer's view of `facts`, or why it cannot hold them.
    fn new(facts: &SpriteFacts) -> Result<Self, String> {
        let name = std::str::from_utf8(&facts.name)
            .map_err(|_| "its name is not UTF-8".to_owned())?
            .to_owned();
        let (texture, frames) = match &facts.kind {
            SpriteKind::Plain(plain)
            | SpriteKind::FrameAnimated { plain, .. }
            | SpriteKind::Tile { plain, .. }
            | SpriteKind::MaskedShield { plain, .. } => (
                Some(&plain.texture),
                u32::try_from(plain.frames)
                    .map_err(|_| format!("its noOfFrames is {}", plain.frames))?,
            ),
            SpriteKind::CorneredTile(tile) => (Some(&tile.texture), tile.frames),
            SpriteKind::Text(text) => (Some(&text.texture), text.frames),
            SpriteKind::Resizeable(resizeable) => (Some(&resizeable.texture), resizeable.frames),
            SpriteKind::Progress(_)
            | SpriteKind::CircularProgress(_)
            | SpriteKind::PieChart(_)
            | SpriteKind::LineChart(_) => (None, 1),
        };
        let texture = match texture.filter(|texture| !texture.is_empty()) {
            Some(texture) => Some(content_path(
                std::str::from_utf8(texture)
                    .map_err(|_| "its textureFile is not UTF-8".to_owned())?,
            )),
            None => None,
        };
        Ok(Self {
            name,
            texture,
            frames,
            facts: facts.clone(),
        })
    }
}

/// Every sprite of the layers by name, and what is refused.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SpriteSet {
    /// By name exactly as declared. Lookup compares hash, length and then bytes without case
    /// folding, so `GFX_Tank` and `GFX_tank` identify separate sprites.
    sprites: BTreeMap<String, SpriteType>,
    /// Each name whose latest declaration is refused, and why. It hides an earlier declaration
    /// of its name, as the game's lookup finds it first.
    refused_names: BTreeMap<String, String>,
    /// What the game does not define (a file's refusals, and each declaration with one), and
    /// each declaration a [`SpriteType`] cannot hold (a name or texture that is not UTF-8, a
    /// negative frame count), by file and name, in load order.
    pub refused: Vec<String>,
}

impl SpriteSet {
    /// The sprites of files read in the game's load order.
    pub fn new(files: &[(String, Sprites)]) -> Self {
        let mut set = Self::default();
        for (path, file) in files {
            set.refused.extend(
                file.refusals
                    .iter()
                    .map(|refusal| format!("{path}: {refusal}")),
            );
            for facts in &file.sprites {
                let refused = if facts.refusals.is_empty() {
                    SpriteType::new(facts)
                } else {
                    Err(facts
                        .refusals
                        .iter()
                        .map(ToString::to_string)
                        .collect::<Vec<_>>()
                        .join("; "))
                };
                match refused {
                    Ok(sprite) => {
                        set.refused_names.remove(&sprite.name);
                        set.sprites.insert(sprite.name.clone(), sprite);
                    }
                    Err(reason) => {
                        let name = String::from_utf8_lossy(&facts.name);
                        set.refused
                            .push(format!("{path}: sprite {name:?} is refused: {reason}"));
                        // A name that is not UTF-8 is no key here, so it hides none.
                        if let Ok(name) = std::str::from_utf8(&facts.name) {
                            set.sprites.remove(name);
                            set.refused_names
                                .insert(name.to_owned(), format!("{path}: {reason}"));
                        }
                    }
                }
            }
        }
        set
    }

    /// The latest declaration of `name`, or why there is none a consumer can draw: no file
    /// declares it, or its latest declaration is refused.
    pub fn lookup(&self, name: &str) -> Result<&SpriteType, String> {
        self.sprites.get(name).ok_or_else(|| {
            self.refusal(name)
                .unwrap_or_else(|| format!("interface/*.gfx declares no sprite {name}"))
        })
    }

    /// `name`'s latest declaration, `None` when no file declares it, or why it is refused.
    pub fn find(&self, name: &str) -> Result<Option<&SpriteType>, String> {
        match self.refusal(name) {
            Some(why) => Err(why),
            None => Ok(self.sprites.get(name)),
        }
    }

    /// Why `name`'s latest declaration is refused, when it is.
    pub fn refusal(&self, name: &str) -> Option<String> {
        self.refused_names
            .get(name)
            .map(|reason| format!("sprite {name} is refused: {reason}"))
    }

    /// Whether `name`'s latest declaration is one a consumer can draw.
    pub fn contains(&self, name: &str) -> bool {
        self.sprites.contains_key(name)
    }

    /// Each name whose latest declaration is refused, with why. The game's registry still holds
    /// such a name, so a lookup that only tests presence finds it.
    pub fn refused_names(&self) -> impl Iterator<Item = (&String, &String)> {
        self.refused_names.iter()
    }

    /// Every drawable declaration by name; the refused ones are [`SpriteSet::refused`].
    pub fn iter(&self) -> impl Iterator<Item = (&String, &SpriteType)> {
        self.sprites.iter()
    }
}

/// Reads every sprite declared under `interface/**/*.gfx` across the layers.
pub fn read(layers: &ResolvedLayers) -> std::io::Result<SpriteSet> {
    Ok(SpriteSet::new(&sprites::read_layers(layers)?))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gui::native::sprites::read_sprites;

    fn set(files: &[(&str, &[u8])]) -> SpriteSet {
        SpriteSet::new(
            &files
                .iter()
                .map(|(path, bytes)| ((*path).to_owned(), read_sprites(bytes)))
                .collect::<Vec<_>>(),
        )
    }

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
    fn each_kind_gives_its_texture_and_frames() {
        engine_executable::executable_tables::initialize_for_tests()
            .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
        let set = set(&[(
            "interface/mapicons.gfx",
            br#"spriteTypes = {
                spriteType = { name = "GFX_a" texturefile = "gfx//interface//a.dds" }
                SpriteType = { name = "GFX_b" textureFile = "gfx/interface/b.dds"
                    NOOFFRAMES = 14 alwaysTransparent = yes }
                corneredTileSpriteType = { name = "GFX_tile" textureFile = "gfx/t.dds"
                    noOfFrames = 2 size = { x = 8 y = 8 } borderSize = { x = 2 y = 2 } }
                progressbartype = { name = "GFX_bar" textureFile1 = "gfx/p.dds"
                    color = { 1.0 0.0 0.0 } size = { x = 10 y = 4 } }
                frameAnimatedSpriteType = { name = "GFX_anim" texturefile = "gfx/f.dds"
                    noOfFrames = 3 }
            }"#,
        )]);
        let a = &set.sprites["GFX_a"];
        assert_eq!(a.texture.as_deref(), Some("gfx/interface/a.dds"));
        assert_eq!(a.frames, 1);
        // Every key is folded, as the lexer folds a keyword.
        let b = &set.sprites["GFX_b"];
        assert_eq!(b.frames, 14);
        assert!(matches!(
            &b.facts.kind,
            SpriteKind::Plain(plain) if plain.always_transparent
        ));
        let tile = &set.sprites["GFX_tile"];
        assert_eq!(
            (tile.texture.as_deref(), tile.frames),
            (Some("gfx/t.dds"), 2)
        );
        assert!(matches!(
            &tile.facts.kind,
            SpriteKind::CorneredTile(tile) if tile.border == [2, 2] && tile.size == [8, 8]
        ));
        // A progress bar has two textures of its own and no one texture.
        let bar = &set.sprites["GFX_bar"];
        assert_eq!((bar.texture.as_deref(), bar.frames), (None, 1));
        assert!(matches!(
            &bar.facts.kind,
            SpriteKind::Progress(progress) if progress.texture1 == b"gfx/p.dds"
        ));
        assert_eq!(set.sprites["GFX_anim"].frames, 3);
        assert!(set.refused.is_empty(), "{:?}", set.refused);
    }

    /// The main pass reads every file regardless of an earlier result. A stray `}` ends the
    /// level it closes, including the file's own level, so later content is unread. EOF inside
    /// a block ends all open levels. `<` is read as a `textcolors` code.
    #[test]
    fn a_file_the_tree_could_not_parse_reads_as_the_game_reads_it() {
        engine_executable::executable_tables::initialize_for_tests()
            .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
        let set = set(&[
            (
                "interface/a.gfx",
                br#"spriteTypes = { spriteType = { name = "GFX_closed" texturefile = "a.dds" } } }
                spriteTypes = { spriteType = { name = "GFX_unread" texturefile = "b.dds" } }"#,
            ),
            (
                "interface/b.gfx",
                br#"spriteTypes = { spriteType = { name = "GFX_partial" texturefile = "o.dds" }"#,
            ),
            (
                "interface/c.gfx",
                br#"bitmapfonts = { textcolors = { < = { 24 72 120 } } }
                spriteTypes = { spriteType = { name = "GFX_color_tail" texturefile = "c.dds" } }"#,
            ),
        ]);
        for name in ["GFX_closed", "GFX_partial", "GFX_color_tail"] {
            assert!(set.sprites.contains_key(name), "{name}: {set:?}");
        }
        assert!(!set.sprites.contains_key("GFX_unread"));
        assert!(set.refused.is_empty(), "{:?}", set.refused);
    }

    #[test]
    fn a_later_declaration_replaces_an_earlier_one_and_a_refused_one_hides_it() {
        engine_executable::executable_tables::initialize_for_tests()
            .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
        let set = set(&[
            (
                "interface/a.gfx",
                br#"spriteTypes = {
                    spriteType = { name = "GFX_x" texturefile = "first.dds" }
                    spriteType = { name = "GFX_y" texturefile = "y.dds" }
                    spriteType = { name = "GFX_z" texturefile = "z.dds" } }"#,
            ),
            (
                "interface/b.gfx",
                b"spriteTypes = { spriteType = { name = \"GFX_x\" texturefile = \"second.dds\" }
                    spriteType = { name = \"GFX_y\" noOfFrames = -2 }
                    spriteType = { name = \"GFX_z\" texturefile = \"\x80.dds\" }
                    spriteType = { name = \"GFX_\x80\" texturefile = \"n.dds\" } }",
            ),
        ]);
        assert_eq!(set.sprites["GFX_x"].texture.as_deref(), Some("second.dds"));
        assert!(!set.sprites.contains_key("GFX_y"));
        assert!(!set.sprites.contains_key("GFX_z"));
        assert_eq!(set.sprites.len(), 1);
        // A consumer asking for a refused name is told why, not that nothing declares it.
        let why = set.lookup("GFX_y").unwrap_err();
        assert!(
            why.contains("GFX_y is refused") && why.contains("noOfFrames is -2"),
            "{why}"
        );
        assert!(
            set.lookup("GFX_none")
                .unwrap_err()
                .contains("declares no sprite GFX_none")
        );
        assert_eq!(set.refused.len(), 3, "{:?}", set.refused);
        assert!(set.refused[0].contains("GFX_y") && set.refused[0].contains("noOfFrames is -2"));
        assert!(set.refused[1].contains("textureFile is not UTF-8"));
        assert!(set.refused[2].contains("name is not UTF-8"));
    }

    #[test]
    fn a_declaration_the_game_does_not_define_is_refused_by_name() {
        engine_executable::executable_tables::initialize_for_tests()
            .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
        // The integer point reader stores an uninitialised local if the first value scans
        // nothing.
        let set = set(&[(
            "interface/a.gfx",
            br#"spriteTypes = {
                corneredTileSpriteType = { name = "GFX_ok" texturefile = "a.dds" }
                corneredTileSpriteType = { name = "GFX_bad" texturefile = "a.dds"
                    size = { x = y y = 1 } } }"#,
        )]);
        assert!(set.sprites.contains_key("GFX_ok"));
        assert!(!set.sprites.contains_key("GFX_bad"));
        assert_eq!(set.refused.len(), 1, "{:?}", set.refused);
        assert!(set.refused[0].starts_with("interface/a.gfx: sprite \"GFX_bad\" is refused"));
    }

    #[test]
    fn a_file_with_no_sprite_block_yields_nothing() {
        engine_executable::executable_tables::initialize_for_tests()
            .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
        assert!(
            set(&[(
                "interface/a.gfx",
                b"bitmapfonts = { bitmapfont = { name = \"x\" } }"
            )])
            .sprites
            .is_empty()
        );
    }
}
