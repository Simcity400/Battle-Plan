use crate::manifest::SpriteManifest;
use serde_json::Value;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedSprite {
    pub name: String,
    pub frame: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SpriteMiss {
    NoSpriteName,
    MissingSprite(String),
    MissingTexture(String),
    AmbiguousTexturePath(String),
    GenericPlaceholder(String),
    FrameOutOfRange {
        name: String,
        frame: u32,
        frames: u32,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SpriteAsset {
    Resolved(ResolvedSprite),
    Unavailable(SpriteMiss),
}

impl SpriteAsset {
    pub fn resolved(name: impl Into<String>, frame: u32) -> SpriteAsset {
        SpriteAsset::Resolved(ResolvedSprite {
            name: name.into(),
            frame,
        })
    }

    pub fn is_resolved(&self) -> bool {
        matches!(self, SpriteAsset::Resolved(_))
    }
}

pub fn resolve_sprite_name(manifest: &SpriteManifest, raw: Option<&str>) -> SpriteAsset {
    let Some(raw) = raw.map(str::trim).filter(|s| !s.is_empty() && *s != "-") else {
        return SpriteAsset::Unavailable(SpriteMiss::NoSpriteName);
    };
    if is_generic_placeholder(raw) {
        return SpriteAsset::Unavailable(SpriteMiss::GenericPlaceholder(raw.to_string()));
    }
    if let Some(entry) = manifest.get(raw) {
        if entry
            .texturefile
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .is_some()
        {
            return SpriteAsset::resolved(entry.name.clone(), 0);
        }
        return SpriteAsset::Unavailable(SpriteMiss::MissingTexture(raw.to_string()));
    }

    if looks_like_texture_path(raw) {
        let needle = normalize_path(raw);
        let mut matches = manifest.sprites.values().filter(|entry| {
            entry
                .texturefile
                .as_deref()
                .map(normalize_path)
                .map(|tex| tex == needle)
                .unwrap_or(false)
        });
        let first = matches.next();
        if matches.next().is_some() {
            return SpriteAsset::Unavailable(SpriteMiss::AmbiguousTexturePath(raw.to_string()));
        }
        if let Some(entry) = first {
            if is_generic_placeholder(&entry.name) {
                return SpriteAsset::Unavailable(SpriteMiss::GenericPlaceholder(
                    entry.name.clone(),
                ));
            }
            return SpriteAsset::resolved(entry.name.clone(), 0);
        }
    }

    SpriteAsset::Unavailable(SpriteMiss::MissingSprite(raw.to_string()))
}

pub fn resolve_sprite_value(manifest: &SpriteManifest, value: Option<&Value>) -> SpriteAsset {
    match value {
        Some(Value::String(s)) => resolve_sprite_name(manifest, Some(s.as_str())),
        Some(Value::Object(map)) => {
            for key in [
                "portrait_gfx",
                "gfx",
                "sprite",
                "sprite_name",
                "large",
                "small",
                "civilian",
                "army",
                "navy",
            ] {
                match map.get(key) {
                    Some(Value::String(s)) => {
                        let resolved = resolve_sprite_name(manifest, Some(s.as_str()));
                        if resolved.is_resolved() {
                            return resolved;
                        }
                    }
                    Some(Value::Object(_)) => {
                        let resolved = resolve_sprite_value(manifest, map.get(key));
                        if resolved.is_resolved() {
                            return resolved;
                        }
                    }
                    _ => {}
                }
            }
            SpriteAsset::Unavailable(SpriteMiss::NoSpriteName)
        }
        _ => SpriteAsset::Unavailable(SpriteMiss::NoSpriteName),
    }
}

pub fn resolve_portrait_sprite(manifest: &SpriteManifest, record: &Value) -> SpriteAsset {
    for key in [
        "portrait_gfx",
        "gfx",
        "portrait",
        "small_portrait",
        "large_portrait",
    ] {
        if let Some(v) = record.get(key) {
            let resolved = resolve_sprite_value(manifest, Some(v));
            if resolved.is_resolved() {
                return resolved;
            }
            if !matches!(resolved, SpriteAsset::Unavailable(SpriteMiss::NoSpriteName)) {
                return resolved;
            }
        }
    }
    resolve_sprite_value(manifest, record.get("portraits"))
}

pub fn resolve_frame_sprite(
    manifest: &SpriteManifest,
    sprite_name: &str,
    frame: u32,
) -> SpriteAsset {
    if is_generic_placeholder(sprite_name) {
        return SpriteAsset::Unavailable(SpriteMiss::GenericPlaceholder(sprite_name.to_string()));
    }
    let Some(entry) = manifest.get(sprite_name) else {
        return SpriteAsset::Unavailable(SpriteMiss::MissingSprite(sprite_name.to_string()));
    };
    if entry
        .texturefile
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .is_none()
    {
        return SpriteAsset::Unavailable(SpriteMiss::MissingTexture(sprite_name.to_string()));
    }
    let frames = entry.no_of_frames.max(1);
    if frame >= frames {
        return SpriteAsset::Unavailable(SpriteMiss::FrameOutOfRange {
            name: sprite_name.to_string(),
            frame,
            frames,
        });
    }
    SpriteAsset::resolved(entry.name.clone(), frame)
}

pub fn resolve_one_based_frame_sprite(
    manifest: &SpriteManifest,
    sprite_name: &str,
    one_based_frame: Option<u32>,
) -> SpriteAsset {
    let Some(frame) = one_based_frame else {
        return SpriteAsset::Unavailable(SpriteMiss::NoSpriteName);
    };
    if frame == 0 {
        if is_generic_placeholder(sprite_name) {
            return SpriteAsset::Unavailable(SpriteMiss::GenericPlaceholder(
                sprite_name.to_string(),
            ));
        }
        return match manifest.get(sprite_name) {
            Some(entry)
                if entry
                    .texturefile
                    .as_deref()
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .is_some() =>
            {
                SpriteAsset::Unavailable(SpriteMiss::FrameOutOfRange {
                    name: sprite_name.to_string(),
                    frame: 0,
                    frames: entry.no_of_frames.max(1),
                })
            }
            Some(_) => {
                SpriteAsset::Unavailable(SpriteMiss::MissingTexture(sprite_name.to_string()))
            }
            None => SpriteAsset::Unavailable(SpriteMiss::MissingSprite(sprite_name.to_string())),
        };
    }
    resolve_frame_sprite(manifest, sprite_name, frame.saturating_sub(1))
}

pub fn catalog_frames(manifest: &SpriteManifest, sprite_name: &str) -> Vec<ResolvedSprite> {
    if is_generic_placeholder(sprite_name) {
        return Vec::new();
    }
    let Some(entry) = manifest.get(sprite_name) else {
        return Vec::new();
    };
    if entry
        .texturefile
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .is_none()
    {
        return Vec::new();
    }
    (0..entry.no_of_frames.max(1))
        .map(|frame| ResolvedSprite {
            name: entry.name.clone(),
            frame,
        })
        .collect()
}

fn looks_like_texture_path(raw: &str) -> bool {
    let lower = raw.to_ascii_lowercase();
    lower.ends_with(".dds") || lower.ends_with(".tga") || lower.contains("gfx/")
}

fn normalize_path(path: &str) -> String {
    path.replace('\\', "/")
        .trim_start_matches("./")
        .trim_start_matches('/')
        .to_ascii_lowercase()
}

fn is_generic_placeholder(raw: &str) -> bool {
    raw.to_ascii_lowercase().contains("unknown")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::manifest::SpriteEntry;
    use std::collections::HashMap;

    fn entry(name: &str, texturefile: Option<&str>, frames: u32) -> SpriteEntry {
        SpriteEntry {
            name: name.to_string(),
            kind: "spriteType".to_string(),
            no_of_frames: frames,
            source_gfx: None,
            texturefile: texturefile.map(str::to_string),
            category: None,
            width: None,
            height: None,
            nine_slice: None,
            size: None,
            border_size: None,
        }
    }

    fn manifest() -> SpriteManifest {
        let mut sprites = HashMap::new();
        sprites.insert(
            "GFX_portrait_exact".to_string(),
            entry("GFX_portrait_exact", Some("gfx/leaders/exact.dds"), 1),
        );
        sprites.insert(
            "GFX_shield".to_string(),
            entry("GFX_shield", Some("gfx/interface/shield.dds"), 3),
        );
        SpriteManifest { sprites }
    }

    #[test]
    fn exact_sprite_name_binds_when_present() {
        assert_eq!(
            resolve_sprite_name(&manifest(), Some("GFX_portrait_exact")),
            SpriteAsset::resolved("GFX_portrait_exact", 0)
        );
    }

    #[test]
    fn texture_path_alias_binds_only_through_manifest() {
        assert_eq!(
            resolve_sprite_name(&manifest(), Some("gfx/leaders/exact.dds")),
            SpriteAsset::resolved("GFX_portrait_exact", 0)
        );
    }

    #[test]
    fn missing_sprite_remains_unavailable() {
        assert_eq!(
            resolve_sprite_name(&manifest(), Some("GFX_missing")),
            SpriteAsset::Unavailable(SpriteMiss::MissingSprite("GFX_missing".to_string()))
        );
    }

    #[test]
    fn missing_character_portrait_does_not_fabricate_default() {
        let record = serde_json::json!({"id": "GER_test", "name": "Known Person"});
        assert_eq!(
            resolve_portrait_sprite(&manifest(), &record),
            SpriteAsset::Unavailable(SpriteMiss::NoSpriteName)
        );
    }

    #[test]
    fn generic_unknown_portrait_is_unavailable_not_exact_art() {
        let mut manifest = manifest();
        manifest.sprites.insert(
            "GFX_leader_unknown".to_string(),
            entry(
                "GFX_leader_unknown",
                Some("gfx/leaders/leader_unknown.dds"),
                1,
            ),
        );
        let record = serde_json::json!({"portrait_gfx": "GFX_leader_unknown"});
        assert_eq!(
            resolve_portrait_sprite(&manifest, &record),
            SpriteAsset::Unavailable(SpriteMiss::GenericPlaceholder(
                "GFX_leader_unknown".to_string()
            ))
        );
    }

    #[test]
    fn generic_unknown_frame_sprite_is_not_catalogued_as_art() {
        let mut manifest = manifest();
        manifest.sprites.insert(
            "GFX_unknown_shield".to_string(),
            entry("GFX_unknown_shield", Some("gfx/interface/unknown.dds"), 4),
        );
        assert_eq!(
            resolve_one_based_frame_sprite(&manifest, "GFX_unknown_shield", Some(1)),
            SpriteAsset::Unavailable(SpriteMiss::GenericPlaceholder(
                "GFX_unknown_shield".to_string()
            ))
        );
        assert!(catalog_frames(&manifest, "GFX_unknown_shield").is_empty());
    }

    #[test]
    fn insignia_frame_respects_manifest_frame_count() {
        assert_eq!(
            resolve_one_based_frame_sprite(&manifest(), "GFX_shield", Some(2)),
            SpriteAsset::resolved("GFX_shield", 1)
        );
        assert_eq!(
            resolve_one_based_frame_sprite(&manifest(), "GFX_shield", Some(4)),
            SpriteAsset::Unavailable(SpriteMiss::FrameOutOfRange {
                name: "GFX_shield".to_string(),
                frame: 3,
                frames: 3,
            })
        );
    }

    #[test]
    fn catalog_order_is_deterministic() {
        let a = catalog_frames(&manifest(), "GFX_shield");
        let b = catalog_frames(&manifest(), "GFX_shield");
        assert_eq!(a, b);
        assert_eq!(a.iter().map(|s| s.frame).collect::<Vec<_>>(), vec![0, 1, 2]);
    }
}
