//! Loader for `game_assets/sprites.json` (the 26k-entry sprite manifest emitted
//! by `game_assets/extract.py`) and a thin lookup over it.
//!
//! Only the fields M1+ needs are modelled; unknown fields are ignored (serde
//! default), so the manifest can grow without breaking this loader.

use serde::Deserialize;
use std::collections::HashMap;
use std::path::Path;

#[derive(Debug, Clone, Deserialize)]
pub struct SpriteEntry {
    /// Sprite GFX name. The manifest keys entries by name and omits a per-entry
    /// `name`, so this defaults and is back-filled from the map key on load.
    #[serde(default)]
    pub name: String,
    #[serde(default, rename = "type")]
    pub kind: String,
    #[serde(default)]
    pub no_of_frames: u32,
    #[serde(default)]
    pub source_gfx: Option<String>,
    /// Relative texture path under the game root, e.g. "gfx/interface/foo.dds".
    #[serde(default)]
    pub texturefile: Option<String>,
    #[serde(default)]
    pub category: Option<String>,
    #[serde(default)]
    pub width: Option<u32>,
    #[serde(default)]
    pub height: Option<u32>,
    /// 9-slice metadata (corneredTileSpriteType).
    #[serde(default)]
    pub nine_slice: Option<bool>,
    /// Authoritative sprite size (progressbar / explicit-size sprites). The
    /// manifest writes this as `{x,y}`; older callers used `[i64;2]` — [`Xy`]
    /// indexes like an array (`sz[0]`/`sz[1]`) for source compatibility.
    #[serde(default)]
    pub size: Option<Xy>,
    /// 9-slice corner inset (`{x,y}`).
    #[serde(default)]
    pub border_size: Option<Xy>,
}

/// An `{x, y}` pair as written in `sprites.json`. Indexable (`v[0]`,`v[1]`).
#[derive(Debug, Clone, Copy, Deserialize)]
pub struct Xy {
    #[serde(default)]
    pub x: i64,
    #[serde(default)]
    pub y: i64,
}

impl std::ops::Index<usize> for Xy {
    type Output = i64;
    fn index(&self, i: usize) -> &i64 {
        match i {
            0 => &self.x,
            _ => &self.y,
        }
    }
}

#[derive(Debug, Deserialize)]
struct ManifestFile {
    sprites: HashMap<String, SpriteEntry>,
}

#[derive(Debug, Clone)]
pub struct SpriteManifest {
    pub sprites: HashMap<String, SpriteEntry>,
}

impl SpriteManifest {
    pub fn load(path: &Path) -> std::io::Result<SpriteManifest> {
        let text = std::fs::read_to_string(path)?;
        let mut parsed: ManifestFile = serde_json::from_str(&text)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
        // The manifest keys entries by GFX name without repeating it inside the
        // value; back-fill each entry's `name` from its map key.
        for (key, entry) in parsed.sprites.iter_mut() {
            if entry.name.is_empty() {
                entry.name = key.clone();
            }
        }
        Ok(SpriteManifest {
            sprites: parsed.sprites,
        })
    }

    pub fn get(&self, name: &str) -> Option<&SpriteEntry> {
        self.sprites.get(name)
    }

    /// First sprite whose name contains `needle` and has a `.dds` texture and a
    /// resolvable size — handy for picking a representative real asset.
    pub fn find_dds(&self, needle: &str) -> Option<&SpriteEntry> {
        self.sprites.values().find(|s| {
            s.name.contains(needle)
                && s.texturefile
                    .as_deref()
                    .map(|t| t.to_ascii_lowercase().ends_with(".dds"))
                    .unwrap_or(false)
        })
    }
}
