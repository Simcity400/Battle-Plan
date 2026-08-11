use crate::content_parse::Result;
use crate::normalize_game_relative_path;
use crate::paradox_text::{
    destructive_event_count, diagnose_text_decode, first_scalar_any, parse_bytes, scalar_u32,
    Document, Value,
};
use std::collections::BTreeMap;
use std::path::Path;

/// Exact-a729 default installed when a sprite definition has no authored
/// `effectFile`. The renderer subsequently replaces `.lua` with `.shader`.
pub const A729_DEFAULT_SPRITE_EFFECT: &str = "gfx/FX/buttonstate.lua";

/// Resolve an authored effect token to the effective-VFS shader source opened
/// by exact-a729.
pub fn a729_effect_shader_path(effect_file: Option<&str>) -> std::result::Result<String, String> {
    let token = match effect_file {
        Some(value) if value.trim().is_empty() => {
            return Err("sprite effect token is authored but empty".into())
        }
        Some(value) => value,
        None => A729_DEFAULT_SPRITE_EFFECT,
    };
    let normalized = normalize_game_relative_path(token)?;
    let Some((stem, extension)) = normalized.rsplit_once('.') else {
        return Err("sprite effect token has no extension".into());
    };
    if extension.eq_ignore_ascii_case("lua") {
        Ok(format!("{stem}.shader"))
    } else if extension.eq_ignore_ascii_case("shader") {
        Ok(normalized)
    } else {
        Err(format!(
            "sprite effect token uses unsupported exact-a729 extension .{extension}"
        ))
    }
}

/// Exact include closure for the built-in shader families implemented by the
/// replay compositor. Unknown shaders remain a typed admission refusal.
pub fn xui07_builtin_shader_includes(shader_path: &str) -> Option<&'static [&'static str]> {
    if shader_path.eq_ignore_ascii_case("gfx/FX/buttonstate.shader") {
        Some(&["gfx/FX/buttonstate.fxh", "gfx/FX/sprite_animation.fxh"])
    } else if shader_path.eq_ignore_ascii_case("gfx/FX/buttonstate_nodowneffect.shader")
        || shader_path.eq_ignore_ascii_case("gfx/FX/buttonstate_nodownordisableeffect.shader")
    {
        Some(&[])
    } else if shader_path.eq_ignore_ascii_case("gfx/FX/buttonstate_onlydisable.shader") {
        Some(&["gfx/FX/buttonstate.fxh", "gfx/FX/sprite_animation.fxh"])
    } else if shader_path.eq_ignore_ascii_case("gfx/FX/buttonstate_blendframes.shader") {
        Some(&["gfx/FX/buttonstate.fxh"])
    } else if shader_path.eq_ignore_ascii_case("gfx/FX/static_button.shader")
        || shader_path.eq_ignore_ascii_case("gfx/FX/progress.shader")
        || shader_path.eq_ignore_ascii_case("gfx/FX/progress_startend.shader")
        || shader_path.eq_ignore_ascii_case("gfx/FX/progress_reverse.shader")
        || shader_path.eq_ignore_ascii_case("gfx/FX/progress_minmax.shader")
        || shader_path.eq_ignore_ascii_case("gfx/FX/progress_radial.shader")
        || shader_path.eq_ignore_ascii_case("gfx/FX/circularprogressbar.shader")
    {
        Some(&[])
    } else {
        None
    }
}

/// Parse the sprite `color` token into portable RGBA f32 bits. Absence uses
/// exact-a729's observed white default; callers must separately retain whether
/// that value was authored.
pub fn a729_sprite_color_rgba_f32_bits(raw: &Value) -> std::result::Result<[u32; 4], String> {
    let mut values = raw.values_ci("color");
    let Some(value) = values.next() else {
        return Ok([1.0f32.to_bits(); 4]);
    };
    if values.next().is_some() {
        return Err("sprite definition repeats color".into());
    }
    if let Some(scalar) = value.as_scalar() {
        let hex = scalar
            .trim()
            .strip_prefix("0x")
            .or_else(|| scalar.trim().strip_prefix("0X"))
            .ok_or_else(|| "sprite color scalar is not 0xAARRGGBB".to_string())?;
        if hex.len() != 8 {
            return Err("sprite color scalar is not 0xAARRGGBB".into());
        }
        let argb = u32::from_str_radix(hex, 16)
            .map_err(|_| "sprite color scalar is not 0xAARRGGBB".to_string())?;
        return Ok([
            (f32::from(((argb >> 16) & 0xff) as u8) / 255.0).to_bits(),
            (f32::from(((argb >> 8) & 0xff) as u8) / 255.0).to_bits(),
            (f32::from((argb & 0xff) as u8) / 255.0).to_bits(),
            (f32::from(((argb >> 24) & 0xff) as u8) / 255.0).to_bits(),
        ]);
    }
    let channels = value
        .fields()
        .iter()
        .map(|entry| {
            entry
                .value
                .as_scalar()
                .ok_or(())
                .and_then(|value| value.parse::<f32>().map_err(|_| ()))
        })
        .collect::<std::result::Result<Vec<_>, _>>()
        .map_err(|_| "sprite color vector contains a non-f32 channel".to_string())?;
    if !(channels.len() == 3 || channels.len() == 4)
        || channels
            .iter()
            .any(|channel| !channel.is_finite() || !(0.0..=1.0).contains(channel))
    {
        return Err("sprite color vector must contain three or four finite unit channels".into());
    }
    Ok([
        channels[0].to_bits(),
        channels[1].to_bits(),
        channels[2].to_bits(),
        channels.get(3).copied().unwrap_or(1.0).to_bits(),
    ])
}

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
    /// Files loaded through [`SpriteRegistry::parse_file`] whose bytes were
    /// destroyed at decode: `(rel_path, destructive event count)`. Only that
    /// entry point owns the bytes, so only it can observe this;
    /// [`SpriteRegistry::add_document`] receives an already-decoded document and
    /// contributes nothing here.
    decode_lossy_files: Vec<(String, usize)>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct GfxParseStats {
    pub file_count: usize,
    pub sprite_count: usize,
    pub parse_failures: usize,
    pub recovery_count: usize,
    pub kind_histogram: BTreeMap<String, usize>,
    pub files_with_recoveries: Vec<String>,
    /// `.gfx` files whose BYTES were destroyed at decode, among those observed
    /// at the byte level (i.e. loaded through [`SpriteRegistry::parse_file`]).
    /// A different axis from `files_with_recoveries` — see
    /// `gui::LayoutDocument::decode_losses`.
    pub files_with_decode_loss: Vec<String>,
    /// Destructive decode EVENTS, never entry counts (the per-site detail is
    /// capped, so entry counts would understate).
    pub decode_loss_count: usize,
    pub failures: Vec<String>,
}

impl SpriteRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// Parse one `.gfx` file from disk. The byte-owning entry point, so it is
    /// the only one that can observe decode damage, and it does.
    pub fn parse_file(&mut self, path: &Path, rel: &str) -> Result<usize> {
        let bytes = std::fs::read(path)?;
        let events = destructive_event_count(&diagnose_text_decode(&bytes));
        if events != 0 {
            self.decode_lossy_files.push((rel.to_string(), events));
        }
        let doc = parse_bytes(&bytes);
        Ok(self.add_document(rel, &doc))
    }

    /// Files whose bytes were destroyed at decode, with EVENT counts, in load
    /// order. Reported, never gating. Empty for the shipped game (measured).
    pub fn decode_lossy_files(&self) -> Vec<String> {
        self.decode_lossy_files
            .iter()
            .map(|(file, events)| format!("{file} ({events} destructive decode events)"))
            .collect()
    }

    /// Total destructive decode events across files loaded from disk.
    pub fn decode_loss_events(&self) -> usize {
        self.decode_lossy_files
            .iter()
            .map(|(_, events)| *events)
            .sum()
    }

    pub fn add_document(&mut self, source_file: &str, doc: &Document) -> usize {
        let mut added = 0usize;
        // The pinned a729 `interface/core.gfx` is heterogeneous: after the
        // `bitmapfonts` declarations it also contains sprite definitions such
        // as `SandboxFloaterBackground`. Clausewitz accepts those entries in
        // that container, so the replay registry must not silently discard
        // them merely because they are outside `spriteTypes`.
        for container_name in ["spriteTypes", "bitmapfonts"] {
            for container in doc.root.values_ci(container_name) {
                for entry in container.fields() {
                    let Some(kind) = entry.key.as_deref() else {
                        continue;
                    };
                    if container_name == "bitmapfonts"
                        && !kind.to_ascii_lowercase().contains("spritetype")
                        && !kind.eq_ignore_ascii_case("progressbartype")
                    {
                        continue;
                    }
                    let Some(sprite) = parse_sprite(kind, source_file, &entry.value) else {
                        continue;
                    };
                    self.insert(sprite);
                    added += 1;
                }
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

    /// Effective last-registered sprite winners in deterministic folded-name
    /// order. This is the semantic registry, unlike `sprites()`, which also
    /// retains shadowed declarations for diagnostics.
    pub fn effective_sprites(&self) -> impl Iterator<Item = &SpriteDefinition> {
        self.by_name
            .values()
            .filter_map(|index| self.sprites.get(*index))
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
