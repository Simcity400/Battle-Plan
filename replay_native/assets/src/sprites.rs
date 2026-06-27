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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FormationKind {
    Army,
    ArmyGroup,
}

impl FormationKind {
    pub fn base_insignia_sprite(self) -> &'static str {
        match self {
            FormationKind::Army => "GFX_theatre_army_shield",
            FormationKind::ArmyGroup => "GFX_theatre_army_group_shield",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FormationInsigniaOrigin {
    ExplicitMetadata,
    IconFrameMetadata,
    Unavailable,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FormationInsigniaSprite {
    pub asset: SpriteAsset,
    pub origin: FormationInsigniaOrigin,
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

pub fn resolve_sprite_fields(
    manifest: &SpriteManifest,
    record: &Value,
    fields: &[&str],
) -> SpriteAsset {
    let mut first_miss = None;
    for field in fields {
        let resolved = resolve_sprite_value(manifest, record.get(*field));
        match resolved {
            SpriteAsset::Resolved(_) => return resolved,
            SpriteAsset::Unavailable(SpriteMiss::NoSpriteName) => {}
            miss => {
                if first_miss.is_none() {
                    first_miss = Some(miss);
                }
            }
        }
    }
    first_miss.unwrap_or(SpriteAsset::Unavailable(SpriteMiss::NoSpriteName))
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
            let mut first_miss = None;
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
                let resolved = match map.get(key) {
                    Some(Value::String(s)) => resolve_sprite_name(manifest, Some(s.as_str())),
                    Some(Value::Object(_)) => resolve_sprite_value(manifest, map.get(key)),
                    _ => continue,
                };
                match resolved {
                    SpriteAsset::Resolved(_) => return resolved,
                    SpriteAsset::Unavailable(SpriteMiss::NoSpriteName) => {}
                    miss => {
                        if first_miss.is_none() {
                            first_miss = Some(miss);
                        }
                    }
                }
            }
            first_miss.unwrap_or(SpriteAsset::Unavailable(SpriteMiss::NoSpriteName))
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

const FORMATION_INSIGNIA_FIELDS: &[&str] = &[
    "insignia_gfx",
    "icon_gfx",
    "emblem_gfx",
    "gfx",
    "sprite",
    "sprite_name",
    "insignia",
    "emblem",
];

pub fn resolve_formation_insignia_sprite(
    manifest: &SpriteManifest,
    formation: &Value,
    kind: FormationKind,
) -> FormationInsigniaSprite {
    resolve_formation_insignia_sprite_from_base(manifest, formation, kind.base_insignia_sprite())
}

pub fn resolve_formation_insignia_sprite_from_base(
    manifest: &SpriteManifest,
    formation: &Value,
    base_sprite: &str,
) -> FormationInsigniaSprite {
    let direct = resolve_sprite_fields(manifest, formation, FORMATION_INSIGNIA_FIELDS);
    if !matches!(direct, SpriteAsset::Unavailable(SpriteMiss::NoSpriteName)) {
        return FormationInsigniaSprite {
            asset: direct,
            origin: FormationInsigniaOrigin::ExplicitMetadata,
        };
    }

    let icon = formation.get("icon").and_then(one_based_frame_value);
    let asset = resolve_one_based_frame_sprite(manifest, base_sprite, icon);
    let origin = match asset {
        SpriteAsset::Unavailable(SpriteMiss::NoSpriteName) => FormationInsigniaOrigin::Unavailable,
        _ => FormationInsigniaOrigin::IconFrameMetadata,
    };
    FormationInsigniaSprite { asset, origin }
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

fn one_based_frame_value(value: &Value) -> Option<u32> {
    match value {
        Value::Number(n) => n.as_u64().and_then(|u| u32::try_from(u).ok()),
        Value::String(s) => s.trim().parse::<u32>().ok(),
        _ => None,
    }
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
        sprites.insert(
            "GFX_exact_formation".to_string(),
            entry(
                "GFX_exact_formation",
                Some("gfx/interface/formation.dds"),
                1,
            ),
        );
        sprites.insert(
            "GFX_theatre_army_shield".to_string(),
            entry(
                "GFX_theatre_army_shield",
                Some("gfx/interface/army_shield.dds"),
                4,
            ),
        );
        sprites.insert(
            "GFX_theatre_army_group_shield".to_string(),
            entry(
                "GFX_theatre_army_group_shield",
                Some("gfx/interface/army_group_shield.dds"),
                3,
            ),
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
    fn nested_generic_unknown_portrait_surfaces_placeholder_miss() {
        let mut manifest = manifest();
        manifest.sprites.insert(
            "GFX_leader_unknown".to_string(),
            entry(
                "GFX_leader_unknown",
                Some("gfx/leaders/leader_unknown.dds"),
                1,
            ),
        );
        let record = serde_json::json!({
            "portraits": {
                "civilian": {
                    "large": "GFX_leader_unknown"
                }
            }
        });
        assert_eq!(
            resolve_portrait_sprite(&manifest, &record),
            SpriteAsset::Unavailable(SpriteMiss::GenericPlaceholder(
                "GFX_leader_unknown".to_string()
            ))
        );
    }

    #[test]
    fn nested_missing_portrait_child_surfaces_missing_sprite_miss() {
        let record = serde_json::json!({
            "portraits": {
                "army": {
                    "large": "GFX_missing_nested"
                }
            }
        });
        assert_eq!(
            resolve_portrait_sprite(&manifest(), &record),
            SpriteAsset::Unavailable(SpriteMiss::MissingSprite("GFX_missing_nested".to_string()))
        );
    }

    #[test]
    fn formation_insignia_direct_sprite_uses_explicit_metadata() {
        let formation = serde_json::json!({
            "name": "First Army",
            "insignia_gfx": "GFX_exact_formation",
            "icon": 3
        });
        assert_eq!(
            resolve_formation_insignia_sprite(&manifest(), &formation, FormationKind::Army),
            FormationInsigniaSprite {
                asset: SpriteAsset::resolved("GFX_exact_formation", 0),
                origin: FormationInsigniaOrigin::ExplicitMetadata,
            }
        );
    }

    #[test]
    fn formation_insignia_later_explicit_field_can_resolve() {
        let formation = serde_json::json!({
            "insignia_gfx": "-",
            "emblem_gfx": "GFX_exact_formation",
            "icon": 2
        });
        assert_eq!(
            resolve_formation_insignia_sprite(&manifest(), &formation, FormationKind::Army),
            FormationInsigniaSprite {
                asset: SpriteAsset::resolved("GFX_exact_formation", 0),
                origin: FormationInsigniaOrigin::ExplicitMetadata,
            }
        );
    }

    #[test]
    fn formation_insignia_uses_one_based_army_and_group_icon_frames() {
        let army = serde_json::json!({"name": "First Army", "icon": 3});
        assert_eq!(
            resolve_formation_insignia_sprite(&manifest(), &army, FormationKind::Army),
            FormationInsigniaSprite {
                asset: SpriteAsset::resolved("GFX_theatre_army_shield", 2),
                origin: FormationInsigniaOrigin::IconFrameMetadata,
            }
        );

        let group = serde_json::json!({"name": "Army Group", "icon": "2"});
        assert_eq!(
            resolve_formation_insignia_sprite(&manifest(), &group, FormationKind::ArmyGroup),
            FormationInsigniaSprite {
                asset: SpriteAsset::resolved("GFX_theatre_army_group_shield", 1),
                origin: FormationInsigniaOrigin::IconFrameMetadata,
            }
        );
    }

    #[test]
    fn formation_insignia_generic_placeholder_is_unavailable_not_exact() {
        let mut manifest = manifest();
        manifest.sprites.insert(
            "GFX_unknown_formation".to_string(),
            entry(
                "GFX_unknown_formation",
                Some("gfx/interface/unknown_formation.dds"),
                1,
            ),
        );
        let formation = serde_json::json!({
            "emblem_gfx": "GFX_unknown_formation",
            "icon": 1
        });
        assert_eq!(
            resolve_formation_insignia_sprite(&manifest, &formation, FormationKind::Army),
            FormationInsigniaSprite {
                asset: SpriteAsset::Unavailable(SpriteMiss::GenericPlaceholder(
                    "GFX_unknown_formation".to_string()
                )),
                origin: FormationInsigniaOrigin::ExplicitMetadata,
            }
        );
    }

    #[test]
    fn formation_insignia_explicit_missing_sprite_does_not_fallback_to_icon() {
        let formation = serde_json::json!({
            "insignia_gfx": "GFX_missing_formation",
            "icon": 2
        });
        assert_eq!(
            resolve_formation_insignia_sprite(&manifest(), &formation, FormationKind::Army),
            FormationInsigniaSprite {
                asset: SpriteAsset::Unavailable(SpriteMiss::MissingSprite(
                    "GFX_missing_formation".to_string()
                )),
                origin: FormationInsigniaOrigin::ExplicitMetadata,
            }
        );
    }

    #[test]
    fn formation_insignia_missing_icon_and_bad_frames_are_explicit_misses() {
        let missing_icon = serde_json::json!({"name": "No Icon"});
        assert_eq!(
            resolve_formation_insignia_sprite(&manifest(), &missing_icon, FormationKind::Army),
            FormationInsigniaSprite {
                asset: SpriteAsset::Unavailable(SpriteMiss::NoSpriteName),
                origin: FormationInsigniaOrigin::Unavailable,
            }
        );

        let zero_icon = serde_json::json!({"icon": 0});
        assert_eq!(
            resolve_formation_insignia_sprite(&manifest(), &zero_icon, FormationKind::Army),
            FormationInsigniaSprite {
                asset: SpriteAsset::Unavailable(SpriteMiss::FrameOutOfRange {
                    name: "GFX_theatre_army_shield".to_string(),
                    frame: 0,
                    frames: 4,
                }),
                origin: FormationInsigniaOrigin::IconFrameMetadata,
            }
        );

        let out_of_range = serde_json::json!({"icon": 5});
        assert_eq!(
            resolve_formation_insignia_sprite(&manifest(), &out_of_range, FormationKind::Army),
            FormationInsigniaSprite {
                asset: SpriteAsset::Unavailable(SpriteMiss::FrameOutOfRange {
                    name: "GFX_theatre_army_shield".to_string(),
                    frame: 4,
                    frames: 4,
                }),
                origin: FormationInsigniaOrigin::IconFrameMetadata,
            }
        );

        let no_base_manifest = SpriteManifest {
            sprites: HashMap::new(),
        };
        let icon_without_base = serde_json::json!({"icon": 1});
        assert_eq!(
            resolve_formation_insignia_sprite(
                &no_base_manifest,
                &icon_without_base,
                FormationKind::Army
            ),
            FormationInsigniaSprite {
                asset: SpriteAsset::Unavailable(SpriteMiss::MissingSprite(
                    "GFX_theatre_army_shield".to_string()
                )),
                origin: FormationInsigniaOrigin::IconFrameMetadata,
            }
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
