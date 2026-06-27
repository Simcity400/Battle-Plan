//! `assets` — the game asset loaders for the native replay UI (M1).
//!
//! * [`dds`]       — DDS header parse + classify, CPU RGBA8 transcode fallback,
//!                   and BCn block extraction for direct GPU upload.
//! * [`bmfont`]    — text-format BMFont `.fnt` parser + glyph table + measure.
//! * [`manifest`]  — `sprites.json` manifest loader.
//! * [`map_artifacts`] — map_pipeline artifact loader (used at M4; compiles now).
//! * [`resolver`]  — runtime game-path resolver (vanilla + mod overlay).
//!
//! No game path is hardcoded into committed source beyond the documented Steam
//! default that `game_assets/extract.py` already uses; runtime callers override
//! it via CLI/env.

pub mod bmfont;
pub mod dds;
pub mod manifest;
pub mod map_artifacts;
pub mod modset;
pub mod resolver;
pub mod sprites;
pub mod tga;

pub use bmfont::{BmFont, Glyph};
pub use dds::{decode, parse_header, AlphaMode, DdsFormat, DecodedImage, GpuBlocks};
pub use manifest::{SpriteEntry, SpriteManifest};
pub use modset::{match_replay, BundleConfig, MatchOutcome, ModDelta, ModEntry, ModError, ModSet};
pub use resolver::GameResolver;
pub use sprites::{
    catalog_frames, resolve_formation_insignia_sprite, resolve_formation_insignia_sprite_from_base,
    resolve_frame_sprite, resolve_one_based_frame_sprite, resolve_portrait_sprite,
    resolve_sprite_fields, resolve_sprite_name, resolve_sprite_value, FormationInsigniaOrigin,
    FormationInsigniaSprite, FormationKind, ResolvedSprite, SpriteAsset, SpriteMiss,
};
pub use tga::{decode as decode_tga, TgaImage};
