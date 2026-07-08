use crate::paradox_text::{first_scalar_any, parse_file, scalar_u32, Document, Value};
use crate::Result;
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Size {
    pub x: String,
    pub y: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TextureSlot {
    pub key: String,
    pub path: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct SpriteDefinition {
    pub kind: String,
    pub name: String,
    pub texture_files: Vec<TextureSlot>,
    pub no_of_frames: u32,
    pub effect_file: Option<String>,
    pub size: Option<Size>,
    pub border_size: Option<Size>,
    pub source_file: String,
    pub raw: Value,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct SpriteRegistry {
    sprites: Vec<SpriteDefinition>,
    by_name: BTreeMap<String, usize>,
    kinds: BTreeMap<String, usize>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct GfxParseStats {
    pub file_count: usize,
    pub sprite_count: usize,
    pub parse_failures: usize,
    pub recovery_count: usize,
    pub kind_histogram: BTreeMap<String, usize>,
    pub files_with_recoveries: Vec<String>,
    pub failures: Vec<String>,
}

impl SpriteRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn parse_file(&mut self, path: &Path, rel: &str) -> Result<usize> {
        let doc = parse_file(path)?;
        Ok(self.add_document(rel, &doc))
    }

    pub fn add_document(&mut self, source_file: &str, doc: &Document) -> usize {
        let mut added = 0usize;
        for sprite_types in doc.root.values_ci("spriteTypes") {
            for entry in sprite_types.fields() {
                let Some(kind) = entry.key.as_deref() else {
                    continue;
                };
                let Some(sprite) = parse_sprite(kind, source_file, &entry.value) else {
                    continue;
                };
                self.insert(sprite);
                added += 1;
            }
        }
        added
    }

    pub fn insert(&mut self, sprite: SpriteDefinition) {
        let lower = sprite.name.to_ascii_lowercase();
        let idx = self.sprites.len();
        *self.kinds.entry(sprite.kind.clone()).or_insert(0) += 1;
        self.sprites.push(sprite);
        self.by_name.insert(lower, idx);
    }

    pub fn get(&self, name: &str) -> Option<&SpriteDefinition> {
        self.by_name
            .get(&name.to_ascii_lowercase())
            .and_then(|idx| self.sprites.get(*idx))
    }

    pub fn sprites(&self) -> &[SpriteDefinition] {
        &self.sprites
    }

    pub fn len(&self) -> usize {
        self.sprites.len()
    }

    pub fn is_empty(&self) -> bool {
        self.sprites.is_empty()
    }

    pub fn kind_histogram(&self) -> BTreeMap<String, usize> {
        self.kinds.clone()
    }
}

impl SpriteDefinition {
    pub fn primary_texture(&self) -> Option<&str> {
        self.texture_files.first().map(|slot| slot.path.as_str())
    }

    pub fn texture_paths(&self) -> impl Iterator<Item = &str> {
        self.texture_files.iter().map(|slot| slot.path.as_str())
    }

    pub fn is_known_minimum_kind(&self) -> bool {
        matches!(
            self.kind.as_str(),
            "spriteType"
                | "corneredTileSpriteType"
                | "frameAnimatedSpriteType"
                | "textSpriteType"
                | "maskedShieldType"
                | "progressbartype"
        )
    }
}

pub fn parse_sprite(kind: &str, source_file: &str, value: &Value) -> Option<SpriteDefinition> {
    let name = value.first_scalar_ci("name")?.to_string();
    let texture_files = texture_slots(value);
    let no_of_frames = value
        .first_value_ci("noOfFrames")
        .and_then(scalar_u32)
        .unwrap_or(1)
        .max(1);
    let effect_file = value.first_scalar_ci("effectFile").map(str::to_string);
    let size = value.first_value_ci("size").and_then(parse_size);
    let border_size = value.first_value_ci("borderSize").and_then(parse_size);
    Some(SpriteDefinition {
        kind: kind.to_string(),
        name,
        texture_files,
        no_of_frames,
        effect_file,
        size,
        border_size,
        source_file: source_file.to_string(),
        raw: value.clone(),
    })
}

pub fn texture_slots(value: &Value) -> Vec<TextureSlot> {
    let mut slots = Vec::new();
    for entry in value.fields() {
        let Some(key) = entry.key.as_deref() else {
            continue;
        };
        let folded = key.to_ascii_lowercase();
        if !folded.starts_with("texturefile") {
            continue;
        }
        let Some(path) = entry.value.as_scalar() else {
            continue;
        };
        if path.trim().is_empty() {
            continue;
        }
        slots.push(TextureSlot {
            key: key.to_string(),
            path: path.to_string(),
        });
    }
    slots
}

pub fn parse_size(value: &Value) -> Option<Size> {
    let x = first_scalar_any(value, &["x", "width"])?;
    let y = first_scalar_any(value, &["y", "height"])?;
    Some(Size {
        x: x.to_string(),
        y: y.to_string(),
    })
}

pub fn sprite_refs_from_values<'a>(values: impl IntoIterator<Item = &'a str>) -> BTreeSet<String> {
    values
        .into_iter()
        .filter(|value| !value.trim().is_empty())
        .map(str::to_string)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::paradox_text::parse_str;

    #[test]
    fn registry_lookup_is_case_insensitive() {
        let doc = parse_str(
            r#"
            spriteTypes = {
              spriteType = {
                name = "GFX_Test"
                textureFile = "gfx/interface/test.dds"
                noOfFrames = 3
              }
            }
            "#,
        );
        let mut registry = SpriteRegistry::new();
        assert_eq!(registry.add_document("test.gfx", &doc), 1);
        let sprite = registry.get("gfx_test").unwrap();
        assert_eq!(sprite.no_of_frames, 3);
        assert_eq!(sprite.primary_texture(), Some("gfx/interface/test.dds"));
    }
}
