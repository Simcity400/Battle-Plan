
use std::{
    collections::BTreeMap,
    sync::{Arc, OnceLock},
};

use content_reader_layers::kept::{Input, Table, put_bytes, put_texts};

use crate::{
    ResolvedLayers,
    gui::native::sprites::{self, SpriteFacts, SpriteKind, Sprites},
};

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

#[derive(Clone, Debug, PartialEq)]
pub struct SpriteType {
    pub name: String,
    pub texture: Option<String>,
    pub frames: u32,
    pub facts: SpriteFacts,
}

impl SpriteType {
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

#[derive(Clone, Debug, Default, PartialEq)]
pub struct SpriteSet {
    sprites: Declared,
    refused_names: BTreeMap<String, String>,
    pub refused: Vec<String>,
}

pub type Decode = fn(&str, &[u8]) -> Result<SpriteType, String>;

#[derive(Clone)]
enum Declared {
    Read(BTreeMap<String, SpriteType>),
    Kept(Kept),
}

#[derive(Clone)]
struct Kept {
    table: Table,
    decode: Decode,
    decoded: Arc<[OnceLock<Result<SpriteType, String>>]>,
}

impl Kept {
    fn damaged() -> String {
        "the kept sprites are damaged".to_owned()
    }

    fn decoded(&self, index: usize) -> Result<&SpriteType, String> {
        let cell = self.decoded.get(index).ok_or_else(Self::damaged)?;
        cell.get_or_init(|| {
            let name = self
                .table
                .key(index)
                .and_then(|name| std::str::from_utf8(name).ok())
                .ok_or_else(Self::damaged)?;
            let body = self.table.value(index).ok_or_else(Self::damaged)?;
            (self.decode)(name, body)
        })
        .as_ref()
        .map_err(|why| format!("its kept declaration cannot be read: {why}"))
    }
}

impl Default for Declared {
    fn default() -> Self {
        Self::Read(BTreeMap::new())
    }
}

impl PartialEq for Declared {
    fn eq(&self, other: &Self) -> bool {
        self.len() == other.len() && self.iter().eq(other.iter())
    }
}

impl std::fmt::Debug for Declared {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.debug_map().entries(self.iter()).finish()
    }
}

impl Declared {
    fn len(&self) -> usize {
        match self {
            Self::Read(sprites) => sprites.len(),
            Self::Kept(kept) => kept.table.len(),
        }
    }

    fn contains(&self, name: &str) -> bool {
        match self {
            Self::Read(sprites) => sprites.contains_key(name),
            Self::Kept(kept) => kept.table.find(name.as_bytes()).is_some(),
        }
    }

    fn get(&self, name: &str) -> Option<Result<&SpriteType, String>> {
        match self {
            Self::Read(sprites) => sprites.get(name).map(Ok),
            Self::Kept(kept) => Some(kept.decoded(kept.table.find(name.as_bytes())?)),
        }
    }

    fn iter(&self) -> Box<dyn Iterator<Item = (&String, &SpriteType)> + '_> {
        match self {
            Self::Read(sprites) => Box::new(sprites.iter()),
            Self::Kept(kept) => Box::new(
                (0..kept.table.len())
                    .filter_map(move |index| kept.decoded(index).ok())
                    .map(|sprite| (&sprite.name, sprite)),
            ),
        }
    }
}

impl SpriteSet {
    pub fn new(files: &[(String, Sprites)]) -> Self {
        let mut sprites = BTreeMap::new();
        let mut refused_names = BTreeMap::new();
        let mut refused = Vec::new();
        for (path, file) in files {
            refused.extend(
                file.refusals
                    .iter()
                    .map(|refusal| format!("{path}: {refusal}")),
            );
            for facts in &file.sprites {
                let read = if facts.refusals.is_empty() {
                    SpriteType::new(facts)
                } else {
                    Err(facts
                        .refusals
                        .iter()
                        .map(ToString::to_string)
                        .collect::<Vec<_>>()
                        .join("; "))
                };
                match read {
                    Ok(sprite) => {
                        refused_names.remove(&sprite.name);
                        sprites.insert(sprite.name.clone(), sprite);
                    }
                    Err(reason) => {
                        let name = String::from_utf8_lossy(&facts.name);
                        refused.push(format!("{path}: sprite {name:?} is refused: {reason}"));
                        if let Ok(name) = std::str::from_utf8(&facts.name) {
                            sprites.remove(name);
                            refused_names.insert(name.to_owned(), format!("{path}: {reason}"));
                        }
                    }
                }
            }
        }
        Self {
            sprites: Declared::Read(sprites),
            refused_names,
            refused,
        }
    }

    pub fn keep(
        &self,
        out: &mut Vec<u8>,
        encode: impl Fn(&SpriteType) -> Result<Vec<u8>, String>,
    ) -> Result<(), String> {
        out.extend((self.refused_names.len() as u64).to_le_bytes());
        for (name, reason) in &self.refused_names {
            put_bytes(out, name.as_bytes());
            put_bytes(out, reason.as_bytes());
        }
        put_texts(out, &self.refused);
        let declared = self
            .iter()
            .map(|(name, sprite)| Ok((name.as_bytes(), encode(sprite)?)))
            .collect::<Result<Vec<_>, String>>()?;
        Table::write(out, declared.into_iter());
        Ok(())
    }

    pub fn from_kept(
        bytes: &Arc<Vec<u8>>,
        at: usize,
        decode: Decode,
    ) -> std::io::Result<(Self, usize)> {
        let mut input = Input::new(bytes, at);
        let names = input.u64()?;
        let mut refused_names = BTreeMap::new();
        for _ in 0..names {
            let name = input.text()?;
            refused_names.insert(name, input.text()?);
        }
        let refused = input.texts()?;
        let (table, end) = Table::read(bytes, input.at())?;
        let decoded = (0..table.len()).map(|_| OnceLock::new()).collect();
        Ok((
            Self {
                sprites: Declared::Kept(Kept {
                    table,
                    decode,
                    decoded,
                }),
                refused_names,
                refused,
            },
            end,
        ))
    }

    pub fn lookup(&self, name: &str) -> Result<&SpriteType, String> {
        match self.sprites.get(name) {
            Some(Ok(sprite)) => Ok(sprite),
            Some(Err(why)) => Err(format!("sprite {name} is unavailable: {why}")),
            None => Err(self
                .refusal(name)
                .unwrap_or_else(|| format!("interface/*.gfx declares no sprite {name}"))),
        }
    }

    pub fn find(&self, name: &str) -> Result<Option<&SpriteType>, String> {
        if let Some(why) = self.refusal(name) {
            return Err(why);
        }
        match self.sprites.get(name) {
            Some(Ok(sprite)) => Ok(Some(sprite)),
            Some(Err(why)) => Err(format!("sprite {name} is unavailable: {why}")),
            None => Ok(None),
        }
    }

    pub fn refusal(&self, name: &str) -> Option<String> {
        self.refused_names
            .get(name)
            .map(|reason| format!("sprite {name} is refused: {reason}"))
    }

    pub fn contains(&self, name: &str) -> bool {
        self.sprites.contains(name)
    }

    pub fn refused_names(&self) -> impl Iterator<Item = (&String, &String)> {
        self.refused_names.iter()
    }

    pub fn iter(&self) -> impl Iterator<Item = (&String, &SpriteType)> {
        self.sprites.iter()
    }
}

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
        content_reader_script::test_game::enter();
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
        let a = set.lookup("GFX_a").unwrap();
        assert_eq!(a.texture.as_deref(), Some("gfx/interface/a.dds"));
        assert_eq!(a.frames, 1);
        let b = set.lookup("GFX_b").unwrap();
        assert_eq!(b.frames, 14);
        assert!(matches!(
            &b.facts.kind,
            SpriteKind::Plain(plain) if plain.always_transparent
        ));
        let tile = set.lookup("GFX_tile").unwrap();
        assert_eq!(
            (tile.texture.as_deref(), tile.frames),
            (Some("gfx/t.dds"), 2)
        );
        assert!(matches!(
            &tile.facts.kind,
            SpriteKind::CorneredTile(tile) if tile.border == [2, 2] && tile.size == [8, 8]
        ));
        let bar = set.lookup("GFX_bar").unwrap();
        assert_eq!((bar.texture.as_deref(), bar.frames), (None, 1));
        assert!(matches!(
            &bar.facts.kind,
            SpriteKind::Progress(progress) if progress.texture1 == b"gfx/p.dds"
        ));
        assert_eq!(set.lookup("GFX_anim").unwrap().frames, 3);
        assert!(set.refused.is_empty(), "{:?}", set.refused);
    }

    #[test]
    fn a_file_the_tree_could_not_parse_reads_as_the_game_reads_it() {
        content_reader_script::test_game::enter();
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
            assert!(set.contains(name), "{name}: {set:?}");
        }
        assert!(!set.contains("GFX_unread"));
        assert!(set.refused.is_empty(), "{:?}", set.refused);
    }

    #[test]
    fn a_later_declaration_replaces_an_earlier_one_and_a_refused_one_hides_it() {
        content_reader_script::test_game::enter();
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
        assert_eq!(set.lookup("GFX_x").unwrap().texture.as_deref(), Some("second.dds"));
        assert!(!set.contains("GFX_y"));
        assert!(!set.contains("GFX_z"));
        assert_eq!(set.iter().count(), 1);
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
        content_reader_script::test_game::enter();
        let set = set(&[(
            "interface/a.gfx",
            br#"spriteTypes = {
                corneredTileSpriteType = { name = "GFX_ok" texturefile = "a.dds" }
                corneredTileSpriteType = { name = "GFX_bad" texturefile = "a.dds"
                    size = { x = y y = 1 } } }"#,
        )]);
        assert!(set.contains("GFX_ok"));
        assert!(!set.contains("GFX_bad"));
        assert_eq!(set.refused.len(), 1, "{:?}", set.refused);
        assert!(set.refused[0].starts_with("interface/a.gfx: sprite \"GFX_bad\" is refused"));
    }

}
