use crate::{
    catalogue::{self, Catalogue},
    products::equipment_icon::Icon,
};
use content_reader_gfx::gfx::{SpriteSet, SpriteType};
use content_reader_gui::{
    flags::{DecodeFailure, FlagSet, decode_checked},
    gui::native::{resources, sprites::SpriteKind},
};
use content_reader_image::{Rgba8Image, native::Decode};
use content_reader_layers::{
    LayerRole, ResolvedLayer, ResolvedLayers,
    content::{ArtFileSource, is_strict},
    textures::{NoTexture, texture_candidates, texture_within},
};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    io,
    sync::{Arc, Mutex},
};
use ts_rs::TS;

pub const CURRENT_INSTALL_ART_LABEL: &str = "Artwork from current installation";

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, TS)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum ArtOrigin {
    BaseGame,
    Dlc { id: String, name: String },
    Mod { id: String, name: String },
    VisualMod { id: String, name: String },
    Builtin { id: String },
}

impl ArtOrigin {
    pub fn from_layer(layer: &ResolvedLayer) -> Self {
        let name = layer
            .display_name()
            .unwrap_or(layer.id.as_str())
            .to_owned();
        match layer.role {
            LayerRole::Vanilla => Self::BaseGame,
            LayerRole::Dlc => Self::Dlc {
                id: layer.id.clone(),
                name: layer
                    .dlc_name()
                    .ok()
                    .flatten()
                    .unwrap_or(name),
            },
            LayerRole::Mod => Self::Mod {
                id: layer.id.clone(),
                name,
            },
            LayerRole::Visual => Self::VisualMod {
                id: layer.id.clone(),
                name,
            },
            LayerRole::Builtin | LayerRole::AboveMods | LayerRole::UserDirectory => Self::Builtin {
                id: layer.id.clone(),
            },
        }
    }

    pub fn display_name(&self) -> String {
        match self {
            Self::BaseGame => "base game".to_owned(),
            Self::Dlc { name, .. } => format!("DLC {name}"),
            Self::Mod { name, .. } => format!("mod {name}"),
            Self::VisualMod { name, .. } => format!("visual mod {name}"),
            Self::Builtin { id } => format!("built-in layer {id}"),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct ArtFrame {
    pub frame_index: u32,
    pub frame_count: u32,
    pub frame_width: u32,
    pub frame_height: u32,
    pub texture_width: u32,
    pub texture_height: u32,
    pub output_width: u32,
    pub output_height: u32,
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct ArtProvenance {
    pub from_current_install: bool,
    pub label: String,
    pub scope_key: String,
}

impl ArtProvenance {
    pub fn current_install(origin: &ArtOrigin, scope_key: &ArtScopeKey) -> Self {
        Self {
            from_current_install: true,
            label: format!("{CURRENT_INSTALL_ART_LABEL} ({})", origin.display_name()),
            scope_key: scope_key.0.clone(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(tag = "status", rename_all = "camelCase")]
pub enum ArtResolution {
    NoArtExpected,
    Pending,
    Ready {
        #[serde(rename = "assetId")]
        asset_id: String,
        url: String,
        origin: ArtOrigin,
        file: String,
        frame: ArtFrame,
        provenance: ArtProvenance,
    },
    ConfirmedMissing {
        reason: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        #[ts(optional)]
        origin: Option<ArtOrigin>,
        #[serde(skip_serializing_if = "Option::is_none")]
        #[ts(optional)]
        file: Option<String>,
        #[serde(skip, default = "Searched::relayed")]
        #[ts(skip)]
        searched: Searched,
    },
    Unavailable {
        reason: String,
    },
    Refused {
        reason: String,
        construct: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        #[ts(optional)]
        origin: Option<ArtOrigin>,
        #[serde(skip_serializing_if = "Option::is_none")]
        #[ts(optional)]
        file: Option<String>,
    },
    Failed {
        reason: String,
        retryable: bool,
    },
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Searched(());

impl Searched {
    fn relayed() -> Self {
        Self(())
    }
}

impl Serialize for Searched {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_bool(true)
    }
}

impl<'de> Deserialize<'de> for Searched {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        if bool::deserialize(deserializer)? {
            Ok(Self::relayed())
        } else {
            Err(serde::de::Error::custom("a completed search is relayed only as true"))
        }
    }
}

pub enum ArtFile {
    Mounted { layer: usize, path: Vec<u8> },
    Unmounted { layer: usize, reason: String, searched: Searched },
    Missing(Searched),
}

pub fn art_file(stack: &ResolvedLayers, vfs: &[u8]) -> io::Result<ArtFile> {
    Ok(match stack.art_file_source(vfs)? {
        ArtFileSource::Mounted { layer, path } => ArtFile::Mounted { layer, path },
        ArtFileSource::Unmounted { layer, reason } => ArtFile::Unmounted { layer, reason, searched: Searched(()) },
        ArtFileSource::Missing => ArtFile::Missing(Searched(())),
    })
}

pub fn sprite_definition<'a>(sprites: &'a SpriteSet, name: &str) -> Result<Result<&'a SpriteType, Searched>, String> {
    Ok(sprites.find(name)?.ok_or(Searched(())))
}

fn named_texture(raw_name: &[u8]) -> Result<&[u8], Searched> {
    let name = content_reader_parser::parser::c_string(raw_name);
    if name.is_empty() { Err(Searched(())) } else { Ok(name) }
}

impl ArtResolution {
    pub fn confirmed_missing(searched: Searched, reason: String, origin: Option<ArtOrigin>, file: Option<String>) -> Self {
        Self::ConfirmedMissing { reason, origin, file, searched }
    }

    pub fn url(&self) -> Option<&str> {
        match self {
            Self::Ready { url, .. } => Some(url.as_str()),
            _ => None,
        }
    }

    pub fn into_url(self) -> Option<String> {
        match self {
            Self::Ready { url, .. } => Some(url),
            _ => None,
        }
    }

    pub fn reason(&self) -> Option<&str> {
        match self {
            Self::ConfirmedMissing { reason, .. }
            | Self::Unavailable { reason }
            | Self::Refused { reason, .. }
            | Self::Failed { reason, .. } => Some(reason.as_str()),
            Self::NoArtExpected | Self::Pending | Self::Ready { .. } => None,
        }
    }

    pub fn origin(&self) -> Option<&ArtOrigin> {
        match self {
            Self::Ready { origin, .. } => Some(origin),
            Self::ConfirmedMissing { origin, .. } | Self::Refused { origin, .. } => origin.as_ref(),
            Self::NoArtExpected | Self::Pending | Self::Unavailable { .. } | Self::Failed { .. } => None,
        }
    }

    pub fn into_result(self) -> Result<Option<String>, String> {
        match self {
            Self::NoArtExpected => Ok(None),
            Self::Ready { url, .. } => Ok(Some(url)),
            Self::Pending => Err("artwork resolution is pending".to_owned()),
            Self::ConfirmedMissing { reason, .. }
            | Self::Unavailable { reason }
            | Self::Refused { reason, .. }
            | Self::Failed { reason, .. } => Err(reason),
        }
    }

    pub fn into_io_result(self) -> io::Result<Option<String>> {
        match self {
            Self::NoArtExpected => Ok(None),
            Self::Ready { url, .. } => Ok(Some(url)),
            Self::Pending => Err(crate::invalid("artwork resolution is pending")),
            Self::ConfirmedMissing { reason, .. } | Self::Unavailable { reason } | Self::Refused { reason, .. } => {
                Err(crate::invalid(reason))
            }
            Self::Failed { reason, .. } => Err(io::Error::other(reason)),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ArtScopeKey(pub String);

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ArtRenderVariant {
    pub max_edge: u32,
    pub decode: Decode,
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ArtTarget {
    NoneExpected,
    Sprite {
        name: String,
        frame: u32,
    },
    TextIcon {
        name: String,
    },
    Texture {
        path: String,
        frames: u32,
        frame: u32,
    },
    Flag {
        candidates: Vec<String>,
        sets: Vec<FlagSetKey>,
        cells: [[u32; 2]; 3],
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum FlagSetKey {
    Normal,
    Medium,
    Small,
}

impl From<FlagSet> for FlagSetKey {
    fn from(set: FlagSet) -> Self {
        match set {
            FlagSet::Normal => Self::Normal,
            FlagSet::Medium => Self::Medium,
            FlagSet::Small => Self::Small,
        }
    }
}

impl From<FlagSetKey> for FlagSet {
    fn from(set: FlagSetKey) -> Self {
        match set {
            FlagSetKey::Normal => Self::Normal,
            FlagSetKey::Medium => Self::Medium,
            FlagSetKey::Small => Self::Small,
        }
    }
}

pub fn art_scope_key(layers: &ResolvedLayers, language: &str) -> ArtScopeKey {
    let mut hash = blake3::Hasher::new();
    hash.update(b"battleplan-art-scope/v1");
    hash.update(&layers.art_stack_key());
    hash.update(&(language.len() as u64).to_le_bytes());
    hash.update(language.as_bytes());
    ArtScopeKey(hash.finalize().to_hex().to_string())
}

#[derive(Default)]
struct ResolvedArtCache {
    entries: Mutex<BTreeMap<(ArtScopeKey, ArtTarget, ArtRenderVariant), ArtResolution>>,
}

impl ResolvedArtCache {
    fn get_or_resolve(
        &self,
        key: (ArtScopeKey, ArtTarget, ArtRenderVariant),
        resolve: impl FnOnce() -> ArtResolution,
    ) -> ArtResolution {
        if let Some(cached) = self
            .entries
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .get(&key)
            .cloned()
        {
            return cached;
        }
        let result = resolve();
        if !matches!(result, ArtResolution::Failed { retryable: true, .. }) {
            self.entries
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .insert(key, result.clone());
        }
        result
    }
}

fn asset_id_from_url(url: &str) -> String {
    url.strip_prefix(dashboard_text::artwork::base())
        .unwrap_or(url)
        .to_owned()
}

fn vfs_path(candidate: &[u8]) -> Vec<u8> {
    candidate
        .split(|byte| *byte == b'/')
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join(&b'/')
}

fn scaled_size(width: u32, height: u32, max_edge: u32) -> (u32, u32) {
    let scale = (f64::from(width.max(height)) / f64::from(max_edge)).max(1.0);
    (
        (f64::from(width) / scale).round().max(1.0) as u32,
        (f64::from(height) / scale).round().max(1.0) as u32,
    )
}

fn sprite_origin(layers: &ResolvedLayers, sprite: &SpriteType) -> Option<ArtOrigin> {
    if sprite.source_file.is_empty() {
        return None;
    }
    layers
        .layer_index(sprite.source_file.as_bytes())
        .map(|index| ArtOrigin::from_layer(&layers.layers[index]))
}

fn unmounted_missing(searched: Searched, layer: &ResolvedLayer, reason: &str, file: String) -> ArtResolution {
    let origin = ArtOrigin::from_layer(layer);
    ArtResolution::confirmed_missing(
        searched,
        format!(
            "{} is not mounted ({reason}); {file} cannot be resolved without risking a substitute",
            origin.display_name()
        ),
        Some(origin),
        Some(file),
    )
}

fn classify_no_texture(
    no_texture: NoTexture,
    origin: Option<ArtOrigin>,
    fallback_file: Option<String>,
) -> ArtResolution {
    match no_texture {
        NoTexture::Empty => ArtResolution::Refused {
            reason: "the found texture has no name to decode".to_owned(),
            construct: "texture-without-name".to_owned(),
            origin,
            file: fallback_file,
        },
        NoTexture::NoExtension(name) => ArtResolution::Refused {
            reason: format!(
                "texture {:?} has no extension",
                String::from_utf8_lossy(&name)
            ),
            construct: "texture-without-extension".to_owned(),
            origin,
            file: Some(String::from_utf8_lossy(&name).into_owned()),
        },
        NoTexture::Missing(tried) => ArtResolution::Failed {
            reason: format!("the found texture could not be read again as any of {tried:?}"),
            retryable: true,
        },
        NoTexture::TooLarge { path, bytes } => ArtResolution::Refused {
            reason: format!("texture {path} is {bytes} bytes, past the 16 MiB texture limit"),
            construct: "oversized-texture-file".to_owned(),
            origin,
            file: Some(path),
        },
        NoTexture::Undecodable { path, reason } => ArtResolution::Refused {
            reason: format!("texture {path} cannot be decoded: {reason}"),
            construct: "undecodable-texture".to_owned(),
            origin,
            file: Some(path),
        },
        NoTexture::Undefined { path, reason } => ArtResolution::Refused {
            reason: format!("texture {path} has undefined format behaviour: {reason}"),
            construct: "undefined-texture-format".to_owned(),
            origin,
            file: Some(path),
        },
        NoTexture::Device { path, reason } => ArtResolution::Refused {
            reason: format!("texture {path} depends on device limits: {reason}"),
            construct: "device-dependent-texture".to_owned(),
            origin,
            file: Some(path),
        },
        NoTexture::OverBudget { path, reason } => ArtResolution::Refused {
            reason: format!("texture {path} exceeds decode budget: {reason}"),
            construct: "over-budget-texture".to_owned(),
            origin,
            file: Some(path),
        },
    }
}

fn resolve_texture_candidate(
    stack: &ResolvedLayers,
    raw_name: &[u8],
    fallback_origin: Option<ArtOrigin>,
) -> Result<(usize, Vec<u8>, String), ArtResolution> {
    let name = match named_texture(raw_name) {
        Ok(name) => name,
        Err(searched) => {
            return Err(ArtResolution::confirmed_missing(searched, "artwork names no texture".to_owned(), fallback_origin, None));
        }
    };
    let candidates = texture_candidates(name);
    if candidates.is_empty() {
        let text = String::from_utf8_lossy(name).into_owned();
        return Err(ArtResolution::Refused {
            reason: format!("texture {text:?} has no extension"),
            construct: "texture-without-extension".to_owned(),
            origin: fallback_origin,
            file: Some(text),
        });
    }
    let mut tried = Vec::with_capacity(candidates.len());
    let mut searched = None;
    for candidate in &candidates {
        let vfs = vfs_path(candidate);
        let display_path = String::from_utf8_lossy(&vfs).into_owned();
        tried.push(display_path.clone());
        match art_file(stack, &vfs) {
            Ok(ArtFile::Mounted { layer, path }) => {
                let winning = String::from_utf8_lossy(&path).into_owned();
                return Ok((layer, path, winning));
            }
            Ok(ArtFile::Unmounted { layer, reason, searched }) => {
                return Err(unmounted_missing(searched, &stack.layers[layer], &reason, display_path));
            }
            Ok(ArtFile::Missing(miss)) => searched = Some(miss),
            Err(error) => {
                return Err(ArtResolution::Failed {
                    reason: format!("{display_path}: {error}"),
                    retryable: true,
                });
            }
        }
    }
    let Some(searched) = searched else {
        unreachable!("texture_candidates gave at least one candidate");
    };
    Err(ArtResolution::confirmed_missing(
        searched,
        format!("no layer holds any of {tried:?}"),
        fallback_origin,
        tried.into_iter().next(),
    ))
}

fn decode_texture_checked(
    stack: &ResolvedLayers,
    raw_name: &[u8],
    decode: Decode,
    origin: ArtOrigin,
    winning_path: &[u8],
    display_file: &str,
) -> Result<content_reader_layers::textures::Texture, ArtResolution> {
    let read = |path: &[u8]| {
        if path.eq_ignore_ascii_case(winning_path) {
            stack.read_uncovered_path(winning_path)
        } else {
            Err(io::Error::from(io::ErrorKind::NotFound))
        }
    };
    let decoded = match texture_within(raw_name, &read, decode, None) {
        Ok(decoded) => decoded,
        Err(error) => {
            return Err(ArtResolution::Failed {
                reason: format!("{display_file}: {error}"),
                retryable: true,
            });
        }
    };
    let texture = match decoded {
        Ok(texture) => texture,
        Err(no_texture) => {
            return Err(classify_no_texture(
                no_texture,
                Some(origin),
                Some(display_file.to_owned()),
            ));
        }
    };
    if texture.image.faces != 1 {
        return Err(ArtResolution::Refused {
            reason: format!("texture {display_file} is a cube map, not a 2D UI texture"),
            construct: "cube-map-ui-texture".to_owned(),
            origin: Some(origin),
            file: Some(display_file.to_owned()),
        });
    }
    Ok(texture)
}

fn resolve_sprite_in_stack(
    layers: &ResolvedLayers,
    stack: &ResolvedLayers,
    sprite: &SpriteType,
    frame: u32,
    variant: ArtRenderVariant,
    scope_key: &ArtScopeKey,
) -> ArtResolution {
    let declared_origin = sprite_origin(stack, sprite);
    let plain = match &sprite.facts.kind {
        SpriteKind::Plain(plain) | SpriteKind::FrameAnimated { plain, .. } => plain,
        other => {
            return ArtResolution::Refused {
                reason: format!(
                    "sprite {}: {} needs native sprite rendering",
                    sprite.name,
                    other.keyword()
                ),
                construct: other.keyword().to_owned(),
                origin: declared_origin,
                file: sprite.texture.clone(),
            };
        }
    };
    if plain.frames <= 0 || variant.max_edge == 0 {
        return ArtResolution::Refused {
            reason: format!(
                "sprite {} has invalid frame count {} or target edge {}",
                sprite.name, plain.frames, variant.max_edge
            ),
            construct: "invalid-sprite-frames".to_owned(),
            origin: declared_origin,
            file: sprite.texture.clone(),
        };
    }
    let (winning_layer, winning_path, display_file) =
        match resolve_texture_candidate(stack, &plain.texture, declared_origin) {
            Ok(found) => found,
            Err(resolution) => return resolution,
        };
    let origin = ArtOrigin::from_layer(&stack.layers[winning_layer]);
    let texture = match decode_texture_checked(
        stack,
        &plain.texture,
        variant.decode,
        origin.clone(),
        &winning_path,
        &display_file,
    ) {
        Ok(texture) => texture,
        Err(resolution) => return resolution,
    };
    let extent = resources::frame_extent(plain, [texture.width, texture.height]);
    if extent.iter().any(|edge| *edge <= 0) {
        return ArtResolution::Refused {
            reason: format!(
                "sprite {} frame {frame} has no pixels ({}x{} across {} frames)",
                sprite.name, texture.width, texture.height, plain.frames
            ),
            construct: "empty-sprite-frame".to_owned(),
            origin: Some(origin),
            file: Some(display_file),
        };
    }
    let frame_width = extent[0] as u32;
    let frame_height = extent[1] as u32;
    let (output_width, output_height) = scaled_size(frame_width, frame_height, variant.max_edge);
    let (rendered, fallbacks) = content_reader_layers::content::observe_overlay_refusals(|| {
        catalogue::sprite_frame_png(layers, sprite, variant.decode, frame, variant.max_edge)
    });
    if let Some(fallback) = fallbacks.into_iter().next() {
        return ArtResolution::Refused {
            reason: format!(
                "sprite {} drew only without the visual mods its texture came from: {fallback}",
                sprite.name
            ),
            construct: "sprite-render".to_owned(),
            origin: Some(origin),
            file: Some(display_file),
        };
    }
    match rendered {
        Ok(url) => {
            let asset_id = asset_id_from_url(&url);
            let provenance = ArtProvenance::current_install(&origin, scope_key);
            ArtResolution::Ready {
                asset_id,
                url,
                origin,
                file: display_file,
                frame: ArtFrame {
                    frame_index: frame,
                    frame_count: plain.frames as u32,
                    frame_width,
                    frame_height,
                    texture_width: texture.width,
                    texture_height: texture.height,
                    output_width,
                    output_height,
                },
                provenance,
            }
        }
        Err(error) if error.kind() == io::ErrorKind::InvalidData => ArtResolution::Refused {
            reason: error.to_string(),
            construct: "sprite-render".to_owned(),
            origin: Some(origin),
            file: Some(display_file),
        },
        Err(error) => ArtResolution::Failed {
            reason: error.to_string(),
            retryable: true,
        },
    }
}

fn resolve_texture_in_stack(
    stack: &ResolvedLayers,
    path: &str,
    frames: u32,
    frame: u32,
    variant: ArtRenderVariant,
    scope_key: &ArtScopeKey,
) -> ArtResolution {
    if frames == 0 || frame >= frames || variant.max_edge == 0 {
        return ArtResolution::Refused {
            reason: format!(
                "texture {path}: invalid frame {frame} for {frames} frames at edge {}",
                variant.max_edge
            ),
            construct: "invalid-texture-frames".to_owned(),
            origin: None,
            file: Some(path.to_owned()),
        };
    }
    let (winning_layer, winning_path, display_file) =
        match resolve_texture_candidate(stack, path.as_bytes(), None) {
            Ok(found) => found,
            Err(resolution) => return resolution,
        };
    let origin = ArtOrigin::from_layer(&stack.layers[winning_layer]);
    let texture = match decode_texture_checked(
        stack,
        path.as_bytes(),
        variant.decode,
        origin.clone(),
        &winning_path,
        &display_file,
    ) {
        Ok(texture) => texture,
        Err(resolution) => return resolution,
    };
    let texture_width = texture.width;
    let texture_height = texture.height;
    let image = match content_reader_image::rgba::dds_base(texture.image) {
        Ok(image) => image,
        Err(error) => {
            return ArtResolution::Refused {
                reason: format!("texture {display_file}: {error}"),
                construct: "texture-rgba-expansion".to_owned(),
                origin: Some(origin),
                file: Some(display_file),
            };
        }
    };
    let frame_width = texture_width / frames;
    let frame_height = texture_height;
    if frame_width == 0 || frame_height == 0 {
        return ArtResolution::Refused {
            reason: format!(
                "texture {display_file} has no pixels per frame ({texture_width}x{texture_height} / {frames})"
            ),
            construct: "empty-texture-frame".to_owned(),
            origin: Some(origin),
            file: Some(display_file),
        };
    }
    let (output_width, output_height) = scaled_size(frame_width, frame_height, variant.max_edge);
    let rendered = catalogue::image_frame_png(&image, frames, frame, variant.max_edge);
    drop(image);
    match rendered {
        Ok(url) => {
            let asset_id = asset_id_from_url(&url);
            let provenance = ArtProvenance::current_install(&origin, scope_key);
            ArtResolution::Ready {
                asset_id,
                url,
                origin,
                file: display_file,
                frame: ArtFrame {
                    frame_index: frame,
                    frame_count: frames,
                    frame_width,
                    frame_height,
                    texture_width,
                    texture_height,
                    output_width,
                    output_height,
                },
                provenance,
            }
        }
        Err(error) if error.kind() == io::ErrorKind::InvalidData => ArtResolution::Refused {
            reason: error.to_string(),
            construct: "texture-render".to_owned(),
            origin: Some(origin),
            file: Some(display_file),
        },
        Err(error) => ArtResolution::Failed {
            reason: error.to_string(),
            retryable: true,
        },
    }
}

fn safe_flag_key(key: &str) -> bool {
    key.split('/').all(|part| {
        !part.is_empty() && part != "." && part != ".." && !part.contains(['\\', '\0', ':'])
    })
}

fn atlas_index(set: FlagSet) -> usize {
    match set {
        FlagSet::Normal => 0,
        FlagSet::Medium => 1,
        FlagSet::Small => 2,
    }
}

fn resize_flag(image: &Rgba8Image, max_edge: u32) -> Rgba8Image {
    let (width, height) = scaled_size(image.width, image.height, max_edge);
    let mut pixels = Vec::with_capacity((width * height * 4) as usize);
    for y in 0..height {
        for x in 0..width {
            let at = (((y * image.height / height) * image.width + x * image.width / width) * 4)
                as usize;
            pixels.extend_from_slice(&image.pixels[at..at + 4]);
        }
    }
    Rgba8Image {
        width,
        height,
        pixels,
    }
}

pub fn resolve_flag_pixels(
    layers: &ResolvedLayers,
    language: &str,
    sets: &[FlagSet],
    cells: &[[u32; 2]; 3],
    candidates: &[String],
    max_edge: u32,
) -> (ArtResolution, Option<Arc<Rgba8Image>>) {
    if candidates.is_empty() || sets.is_empty() {
        return (ArtResolution::NoArtExpected, None);
    }
    if max_edge == 0 {
        return (
            ArtResolution::Refused {
                reason: "flag max_edge must be positive".to_owned(),
                construct: "invalid-flag-edge".to_owned(),
                origin: None,
                file: None,
            },
            None,
        );
    }
    let scope_key = art_scope_key(layers, language);
    let stack = layers.presentation();
    let mut first_refusal: Option<(String, String, Option<ArtOrigin>, String)> = None;
    let mut searched = None;
    for &set in sets {
        let cell = cells[atlas_index(set)];
        for key in candidates {
            if !safe_flag_key(key) {
                return (
                    ArtResolution::Refused {
                        reason: format!(
                            "flag name {key:?} cannot be resolved inside the flag folder"
                        ),
                        construct: "unsafe-flag-key".to_owned(),
                        origin: None,
                        file: Some(key.clone()),
                    },
                    None,
                );
            }
            let asset = format!("{}{key}.tga", set.directory());
            let (winning_layer, winning_path) = match art_file(stack, asset.as_bytes()) {
                Ok(ArtFile::Mounted { layer, path }) => (layer, path),
                Ok(ArtFile::Unmounted { layer, reason, searched }) => {
                    return (
                        unmounted_missing(searched, &stack.layers[layer], &reason, asset),
                        None,
                    );
                }
                Ok(ArtFile::Missing(miss)) => {
                    searched = Some(miss);
                    continue;
                }
                Err(error) => {
                    return (
                        ArtResolution::Failed {
                            reason: format!("flag file {asset}: {error}"),
                            retryable: true,
                        },
                        None,
                    );
                }
            };
            let origin = ArtOrigin::from_layer(&stack.layers[winning_layer]);
            let bytes = match stack.read_uncovered_path(&winning_path) {
                Ok(bytes) => bytes,
                Err(error) => {
                    return (
                        ArtResolution::Failed {
                            reason: format!("flag file {asset} could not be read: {error}"),
                            retryable: true,
                        },
                        None,
                    );
                }
            };
            match decode_checked(&bytes, cell) {
                Ok(image) => {
                    let resized = resize_flag(&image, max_edge);
                    let url = match dashboard_text::artwork::png_url(&resized) {
                        Ok(url) => url,
                        Err(error) => {
                            return (
                                ArtResolution::Failed {
                                    reason: format!("flag artwork could not be encoded: {error}"),
                                    retryable: true,
                                },
                                None,
                            );
                        }
                    };
                    let asset_id = asset_id_from_url(&url);
                    let provenance = ArtProvenance::current_install(&origin, &scope_key);
                    let frame = ArtFrame {
                        frame_index: 0,
                        frame_count: 1,
                        frame_width: image.width,
                        frame_height: image.height,
                        texture_width: image.width,
                        texture_height: image.height,
                        output_width: resized.width,
                        output_height: resized.height,
                    };
                    return (
                        ArtResolution::Ready {
                            asset_id,
                            url,
                            origin,
                            file: String::from_utf8_lossy(&winning_path).into_owned(),
                            frame,
                            provenance,
                        },
                        Some(Arc::new(image)),
                    );
                }
                Err(DecodeFailure::EngineRefused) => {
                    first_refusal.get_or_insert_with(|| {
                        (
                            format!(
                                "flag decoder refused {asset} at cell {}x{}",
                                cell[0], cell[1]
                            ),
                            "flag-tga-header".to_owned(),
                            Some(origin),
                            asset,
                        )
                    });
                }
                Err(DecodeFailure::Truncated) => {
                    first_refusal.get_or_insert_with(|| {
                        (
                            format!(
                                "flag file {asset} is truncated for cell {}x{}",
                                cell[0], cell[1]
                            ),
                            "flag-tga-truncated".to_owned(),
                            Some(origin),
                            asset,
                        )
                    });
                }
            }
        }
    }
    if let Some((reason, construct, origin, file)) = first_refusal {
        return (
            ArtResolution::Refused {
                reason,
                construct,
                origin,
                file: Some(file),
            },
            None,
        );
    }
    let Some(searched) = searched else {
        unreachable!("a flag search over candidates and sets looks up at least one file");
    };
    (
        ArtResolution::confirmed_missing(
            searched,
            format!("no flag artwork found in the mounted content stack for {candidates:?}"),
            None,
            candidates
                .first()
                .map(|key| format!("{}{key}.tga", sets[0].directory())),
        ),
        None,
    )
}

pub fn resolve_flag(
    layers: &ResolvedLayers,
    language: &str,
    sets: &[FlagSet],
    cells: &[[u32; 2]; 3],
    candidates: &[String],
    max_edge: u32,
) -> ArtResolution {
    let scope_key = art_scope_key(layers, language);
    let target = ArtTarget::Flag {
        candidates: candidates.to_vec(),
        sets: sets.iter().copied().map(FlagSetKey::from).collect(),
        cells: *cells,
    };
    let variant = ArtRenderVariant {
        max_edge,
        decode: Decode::default(),
    };
    let cache = layers.presentation().derived::<ResolvedArtCache>();
    cache.get_or_resolve((scope_key, target, variant), || {
        resolve_flag_pixels(layers, language, sets, cells, candidates, max_edge).0
    })
}

pub fn resolve_sprite_type_on_layers(
    layers: &ResolvedLayers,
    language: &str,
    sprite: &SpriteType,
    frame: u32,
    variant: ArtRenderVariant,
) -> ArtResolution {
    let scope_key = art_scope_key(layers, language);
    let stack = layers.presentation();
    let result = resolve_sprite_in_stack(layers, stack, sprite, frame, variant, &scope_key);
    if !matches!(result, ArtResolution::Ready { .. })
        && stack.shows_visual_mods()
        && !is_strict()
    {
        let recorded = layers.recorded_only();
        let recorded_sprites = catalogue::recorded_sprites(&recorded);
        if let Ok(sprites) = recorded_sprites.as_ref()
            && let Ok(Some(own)) = sprites.find(&sprite.name)
        {
            let recorded_scope = art_scope_key(&recorded, language);
            let fallback =
                resolve_sprite_in_stack(&recorded, &recorded, own, frame, variant, &recorded_scope);
            if matches!(fallback, ArtResolution::Ready { .. }) {
                if let Some(reason) = result.reason() {
                    layers.note_overlay_refusal(format!("sprite {} ({reason})", sprite.name));
                }
                return fallback;
            }
        }
    }
    result
}

pub fn resolve_sprite_type(
    catalogue: &Catalogue,
    sprite: &SpriteType,
    frame: u32,
    max_edge: u32,
) -> ArtResolution {
    let decode = match catalogue.gui_decode.as_ref() {
        Ok(decode) => *decode,
        Err(why) => {
            return ArtResolution::Refused {
                reason: why.clone(),
                construct: "texture-decode-settings".to_owned(),
                origin: sprite_origin(catalogue.content.layers.presentation(), sprite),
                file: sprite.texture.clone(),
            };
        }
    };
    resolve_sprite_type_on_layers(
        &catalogue.content.layers,
        &catalogue.recorded_language,
        sprite,
        frame,
        ArtRenderVariant { max_edge, decode },
    )
}

pub fn resolve_sprite(
    catalogue: &Catalogue,
    name: &str,
    frame: u32,
    max_edge: u32,
) -> ArtResolution {
    let decode = match catalogue.gui_decode.as_ref() {
        Ok(decode) => *decode,
        Err(why) => {
            return ArtResolution::Refused {
                reason: why.clone(),
                construct: "texture-decode-settings".to_owned(),
                origin: None,
                file: None,
            };
        }
    };
    resolve(
        catalogue,
        ArtTarget::Sprite {
            name: name.to_owned(),
            frame,
        },
        ArtRenderVariant { max_edge, decode },
    )
}

pub fn resolve_text_icon(catalogue: &Catalogue, name: &str, max_edge: u32) -> ArtResolution {
    let decode = match catalogue.gui_decode.as_ref() {
        Ok(decode) => *decode,
        Err(why) => {
            return ArtResolution::Refused {
                reason: why.clone(),
                construct: "texture-decode-settings".to_owned(),
                origin: None,
                file: None,
            };
        }
    };
    resolve(
        catalogue,
        ArtTarget::TextIcon {
            name: name.to_owned(),
        },
        ArtRenderVariant { max_edge, decode },
    )
}

pub fn resolve_texture(
    catalogue: &Catalogue,
    path: &str,
    frames: u32,
    frame: u32,
    max_edge: u32,
) -> ArtResolution {
    let decode = match catalogue.gui_decode.as_ref() {
        Ok(decode) => *decode,
        Err(why) => {
            return ArtResolution::Refused {
                reason: why.clone(),
                construct: "texture-decode-settings".to_owned(),
                origin: None,
                file: Some(path.to_owned()),
            };
        }
    };
    resolve(
        catalogue,
        ArtTarget::Texture {
            path: path.to_owned(),
            frames,
            frame,
        },
        ArtRenderVariant { max_edge, decode },
    )
}

pub fn resolve_sprite_or_texture(
    catalogue: &Catalogue,
    name: &str,
    max_edge: u32,
) -> ArtResolution {
    if name.is_empty() {
        return ArtResolution::NoArtExpected;
    }
    match sprite_definition(&catalogue.sprites, name) {
        Ok(Ok(_)) => resolve_sprite(catalogue, name, 0, max_edge),
        Err(why) => ArtResolution::Refused {
            reason: why,
            construct: "sprite-definition".to_owned(),
            origin: None,
            file: None,
        },
        Ok(Err(searched)) if name.starts_with("GFX_") => {
            ArtResolution::confirmed_missing(searched, format!("sprite {name} is undeclared"), None, None)
        }
        Ok(Err(_)) => {
            let normalized = name
                .split(['/', '\\'])
                .filter(|part| !part.is_empty())
                .collect::<Vec<_>>()
                .join("/");
            resolve_texture(catalogue, &normalized, 1, 0, max_edge)
        }
    }
}

pub fn resolve_equipment_icon(catalogue: &Catalogue, icon: &Icon, max_edge: u32) -> ArtResolution {
    match icon {
        Icon::Sprite(name) => resolve_sprite(catalogue, name, 0, max_edge),
        Icon::Path(path) => resolve_texture(catalogue, path, 1, 0, max_edge),
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum NativeIconResolution {
    Ready {
        icon: Icon,
        origin: ArtOrigin,
        file: String,
        provenance: ArtProvenance,
    },
    ConfirmedMissing {
        reason: String,
        origin: Option<ArtOrigin>,
        file: Option<String>,
        searched: Searched,
    },
    Refused {
        reason: String,
        construct: String,
        origin: Option<ArtOrigin>,
        file: Option<String>,
    },
    Failed {
        reason: String,
        retryable: bool,
    },
}

fn native_from_error(resolution: ArtResolution) -> NativeIconResolution {
    match resolution {
        ArtResolution::ConfirmedMissing {
            reason,
            origin,
            file,
            searched,
        } => NativeIconResolution::ConfirmedMissing {
            reason,
            origin,
            file,
            searched,
        },
        ArtResolution::Refused {
            reason,
            construct,
            origin,
            file,
        } => NativeIconResolution::Refused {
            reason,
            construct,
            origin,
            file,
        },
        ArtResolution::Failed { reason, retryable } => {
            NativeIconResolution::Failed { reason, retryable }
        }
        ArtResolution::NoArtExpected
        | ArtResolution::Pending
        | ArtResolution::Ready { .. }
        | ArtResolution::Unavailable { .. } => NativeIconResolution::Failed {
            reason: "a texture lookup answered without a texture or a reason it has none".to_owned(),
            retryable: false,
        },
    }
}

fn resolve_native_sprite_in_stack(
    stack: &ResolvedLayers,
    registry: &resources::Registry,
    name: &str,
    decode: Decode,
    scope_key: &ArtScopeKey,
) -> NativeIconResolution {
    let Some(sprite) = registry.sprite(name.as_bytes()) else {
        return NativeIconResolution::ConfirmedMissing {
            reason: format!("interface/*.gfx declares no sprite {name}"),
            origin: None,
            file: None,
            searched: Searched(()),
        };
    };
    let declared_origin = registry
        .sprite_file(name.as_bytes())
        .filter(|file| !file.is_empty())
        .and_then(|file| stack.layer_index(file.as_bytes()))
        .map(|index| ArtOrigin::from_layer(&stack.layers[index]));
    let raw_texture = match &sprite.kind {
        SpriteKind::Plain(plain) | SpriteKind::FrameAnimated { plain, .. } => &plain.texture,
        other => {
            return NativeIconResolution::Refused {
                reason: format!("sprite {name}: {} cannot be bound as a plain icon", other.keyword()),
                construct: other.keyword().to_owned(),
                origin: declared_origin,
                file: None,
            };
        }
    };
    let (winning_layer, winning_path, display_file) =
        match resolve_texture_candidate(stack, raw_texture, declared_origin) {
            Ok(found) => found,
            Err(err) => return native_from_error(err),
        };
    let origin = ArtOrigin::from_layer(&stack.layers[winning_layer]);
    if let Err(err) = decode_texture_checked(
        stack,
        raw_texture,
        decode,
        origin.clone(),
        &winning_path,
        &display_file,
    ) {
        return native_from_error(err);
    }
    let provenance = ArtProvenance::current_install(&origin, scope_key);
    NativeIconResolution::Ready {
        icon: Icon::Sprite(name.to_owned()),
        origin,
        file: display_file,
        provenance,
    }
}

fn resolve_native_texture_in_stack(
    stack: &ResolvedLayers,
    path: &str,
    decode: Decode,
    scope_key: &ArtScopeKey,
) -> NativeIconResolution {
    let (winning_layer, winning_path, display_file) =
        match resolve_texture_candidate(stack, path.as_bytes(), None) {
            Ok(found) => found,
            Err(err) => return native_from_error(err),
        };
    let origin = ArtOrigin::from_layer(&stack.layers[winning_layer]);
    if let Err(err) = decode_texture_checked(
        stack,
        path.as_bytes(),
        decode,
        origin.clone(),
        &winning_path,
        &display_file,
    ) {
        return native_from_error(err);
    }
    let provenance = ArtProvenance::current_install(&origin, scope_key);
    NativeIconResolution::Ready {
        icon: Icon::Path(display_file.clone()),
        origin,
        file: display_file,
        provenance,
    }
}

pub fn resolve_native_equipment_icon(
    layers: &ResolvedLayers,
    registry: &resources::Registry,
    recorded_registry: Option<&resources::Registry>,
    language: &str,
    icon: &Icon,
    decode: Decode,
) -> NativeIconResolution {
    let scope_key = art_scope_key(layers, language);
    let stack = layers.presentation();
    let result = match icon {
        Icon::Sprite(name) => {
            resolve_native_sprite_in_stack(stack, registry, name, decode, &scope_key)
        }
        Icon::Path(path) => resolve_native_texture_in_stack(stack, path, decode, &scope_key),
    };
    if !matches!(result, NativeIconResolution::Ready { .. })
        && stack.shows_visual_mods()
        && !is_strict()
    {
        let recorded = layers.recorded_only();
        let recorded_scope = art_scope_key(&recorded, language);
        let fallback = match icon {
            Icon::Sprite(name) => recorded_registry
                .map(|reg| resolve_native_sprite_in_stack(&recorded, reg, name, decode, &recorded_scope)),
            Icon::Path(path) => Some(resolve_native_texture_in_stack(
                &recorded,
                path,
                decode,
                &recorded_scope,
            )),
        };
        if let Some(ok @ NativeIconResolution::Ready { .. }) = fallback {
            return ok;
        }
    }
    result
}

pub fn resolve_portrait_entry(
    catalogue: &Catalogue,
    kind: u32,
    text: &str,
    max_edge: u32,
) -> ArtResolution {
    match kind {
        0 => match sprite_definition(&catalogue.sprites, text) {
            Ok(Ok(_)) => resolve_sprite(catalogue, text, 0, max_edge),
            Err(why) => ArtResolution::Refused {
                reason: why,
                construct: "portrait-sprite".to_owned(),
                origin: None,
                file: None,
            },
            Ok(Err(searched)) => ArtResolution::confirmed_missing(
                searched,
                format!("Content has no sprite {text}; the game generates this portrait."),
                None,
                None,
            ),
        },
        1 => {
            let normalized = text
                .split(['/', '\\'])
                .filter(|part| !part.is_empty())
                .collect::<Vec<_>>()
                .join("/");
            resolve_texture(catalogue, &normalized, 1, 0, max_edge)
        }
        other => ArtResolution::Refused {
            reason: format!("Portrait kind {other} is not supported."),
            construct: format!("portrait-kind-{other}"),
            origin: None,
            file: None,
        },
    }
}

fn text_icon_in(
    layers: &ResolvedLayers,
    sprites: &SpriteSet,
    language: &str,
    name: &str,
    variant: ArtRenderVariant,
) -> ArtResolution {
    let refused = |reason: String| ArtResolution::Refused {
        reason,
        construct: "text-icon".to_owned(),
        origin: None,
        file: None,
    };
    let stem = name.split_once('|').map_or(name, |(stem, _)| stem);
    let Ok(frame) = u32::try_from(content_reader_localisation::localisation::icon_lookup(name).1.unwrap_or(0)) else {
        return refused(format!("Text icon {name} names no frame"));
    };
    let prefixed = format!("GFX_{stem}");
    let sprite = match sprite_definition(sprites, &prefixed) {
        Ok(Ok(sprite)) => sprite,
        Ok(Err(_)) => match sprite_definition(sprites, stem) {
            Ok(Ok(sprite)) => sprite,
            Ok(Err(searched)) => {
                return ArtResolution::confirmed_missing(
                    searched,
                    format!("interface/*.gfx declares neither {prefixed} nor {stem}"),
                    None,
                    None,
                );
            }
            Err(why) => return refused(why),
        },
        Err(why) => return refused(why),
    };
    resolve_sprite_type_on_layers(layers, language, sprite, frame, variant)
}

pub fn resolve(
    catalogue: &Catalogue,
    target: ArtTarget,
    variant: ArtRenderVariant,
) -> ArtResolution {
    let layers = &catalogue.content.layers;
    let language = &catalogue.recorded_language;
    resolve_with_sprites(layers, &catalogue.sprites, language, target, variant)
}

pub fn resolve_with_sprites(
    layers: &ResolvedLayers,
    sprites: &SpriteSet,
    language: &str,
    target: ArtTarget,
    variant: ArtRenderVariant,
) -> ArtResolution {
    let scope_key = art_scope_key(layers, language);
    let cache = layers.presentation().derived::<ResolvedArtCache>();
    cache.get_or_resolve((scope_key.clone(), target.clone(), variant), || {
        match &target {
            ArtTarget::NoneExpected => ArtResolution::NoArtExpected,
            ArtTarget::Sprite { name, frame } => match sprite_definition(sprites, name) {
                Ok(Ok(sprite)) => {
                    resolve_sprite_type_on_layers(layers, language, sprite, *frame, variant)
                }
                Ok(Err(searched)) => ArtResolution::confirmed_missing(
                    searched,
                    format!("interface/*.gfx declares no sprite {name}"),
                    None,
                    None,
                ),
                Err(why) => ArtResolution::Refused {
                    reason: why,
                    construct: "sprite-definition".to_owned(),
                    origin: None,
                    file: None,
                },
            },
            ArtTarget::TextIcon { name } => text_icon_in(layers, sprites, language, name, variant),
            ArtTarget::Texture {
                path,
                frames,
                frame,
            } => {
                let stack = layers.presentation();
                let result =
                    resolve_texture_in_stack(stack, path, *frames, *frame, variant, &scope_key);
                if !matches!(result, ArtResolution::Ready { .. })
                    && stack.shows_visual_mods()
                    && !is_strict()
                {
                    let recorded = layers.recorded_only();
                    let recorded_scope = art_scope_key(&recorded, language);
                    let fallback = resolve_texture_in_stack(
                        &recorded,
                        path,
                        *frames,
                        *frame,
                        variant,
                        &recorded_scope,
                    );
                    if matches!(fallback, ArtResolution::Ready { .. }) {
                        if let Some(reason) = result.reason() {
                            layers.note_overlay_refusal(format!("texture {path} ({reason})"));
                        }
                        return fallback;
                    }
                }
                result
            }
            ArtTarget::Flag {
                candidates,
                sets,
                cells,
            } => {
                let flag_sets: Vec<FlagSet> = sets.iter().copied().map(FlagSet::from).collect();
                resolve_flag_pixels(
                    layers,
                    language,
                    &flag_sets,
                    cells,
                    candidates,
                    variant.max_edge,
                )
                .0
            }
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::catalogue::{
        tests::{folder_catalogue, layers_catalogue},
        texture_tests::{from_url, png_bytes},
    };
    use content_reader_image::native::tests_support::dx10;

    fn solid(width: u32, height: u32, rgba: [u8; 4]) -> Rgba8Image {
        Rgba8Image {
            width,
            height,
            pixels: rgba.repeat((width * height) as usize),
        }
    }

    fn tga(width: u16, height: u16, bgra: [u8; 4]) -> Vec<u8> {
        let mut bytes = vec![0_u8; 18];
        bytes[2] = 2;
        bytes[12..14].copy_from_slice(&width.to_le_bytes());
        bytes[14..16].copy_from_slice(&height.to_le_bytes());
        bytes[16] = 32;
        for _ in 0..(usize::from(width) * usize::from(height)) {
            bytes.extend_from_slice(&bgra);
        }
        bytes
    }

    fn write_entries(root: &std::path::Path, files: &[(&str, &[u8])]) {
        for (rel, bytes) in files {
            let path = root.join(rel);
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent).unwrap();
            }
            std::fs::write(path, bytes).unwrap();
        }
    }

    fn two_layer_catalogue(
        base_files: &[(&str, &[u8])],
        mod_files: &[(&str, &[u8])],
        replace_paths: &[&str],
        unmounted: Option<&str>,
    ) -> (Catalogue, [crate::catalogue::tests::Folder; 2]) {
        engine_executable::executable_tables::initialize_for_tests().unwrap();
        let (_, base) = folder_catalogue(&[]);
        let (_, addon) = folder_catalogue(&[]);
        if base_files.iter().chain(mod_files).any(|(path, _)| path.ends_with(".gfx")) {
            crate::catalogue::texture_tests::write_sprite_program(&base.0);
        }
        write_entries(&base.0, base_files);
        write_entries(&addon.0, mod_files);
        let mut layers =
            ResolvedLayers::from_folders(&[(&base.0, &[][..]), (&addon.0, replace_paths)]).unwrap();
        layers.layers[0].role = LayerRole::Vanilla;
        layers.layers[0].display_name = Some("base game".to_owned());
        layers.layers[1].role = LayerRole::Mod;
        layers.layers[1].id = "mod_synthetic".to_owned();
        layers.layers[1].display_name = Some("Synthetic Mod".to_owned());
        if let Some(reason) = unmounted {
            let (_, empty) = folder_catalogue(&[]);
            let mut empty_layers =
                ResolvedLayers::from_folders(&[(&base.0, &[][..]), (&empty.0, replace_paths)]).unwrap();
            empty_layers.layers[0].role = LayerRole::Vanilla;
            empty_layers.layers[0].display_name = Some("base game".to_owned());
            empty_layers.layers[1].role = LayerRole::Mod;
            empty_layers.layers[1].id = "mod_synthetic".to_owned();
            empty_layers.layers[1].display_name = Some("Synthetic Mod".to_owned());
            empty_layers.layers[1].art_unmounted = Some(reason.to_owned());
            let layers = ResolvedLayers::from_test_layers(empty_layers.layers).unwrap();
            return (layers_catalogue(layers), [base, empty]);
        }
        let layers = ResolvedLayers::from_test_layers(layers.layers).unwrap();
        let cat = layers_catalogue(layers);
        (cat, [base, addon])
    }

    #[test]
    fn no_silent_substitute_when_mod_unmounted_and_clean_inheritance_when_mounted() {
        let base_shared = png_bytes(&solid(4, 4, [200, 10, 10, 255]));
        let base_only = png_bytes(&solid(4, 4, [10, 10, 200, 255]));
        let mod_shared = png_bytes(&solid(4, 4, [10, 200, 10, 255]));
        let (cat, [_base, _addon]) = two_layer_catalogue(
            &[
                (
                    "interface/icons.gfx",
                    b"spriteTypes={spriteType={name=GFX_Shared textureFile=gfx/shared.png} spriteType={name=GFX_BaseOnly textureFile=gfx/base_only.png}}",
                ),
                ("gfx/shared.png", &base_shared),
                ("gfx/base_only.png", &base_only),
            ],
            &[
                (
                    "interface/mod_icons.gfx",
                    b"spriteTypes={spriteType={name=GFX_ModMissing textureFile=gfx/absent.png}}",
                ),
                ("gfx/shared.png", &mod_shared),
            ],
            &[],
            None,
        );

        let shared = resolve_sprite(&cat, "GFX_Shared", 0, 48);
        match &shared {
            ArtResolution::Ready { origin, file, url, provenance, .. } => {
                assert_eq!(origin, &ArtOrigin::Mod { id: "mod_synthetic".into(), name: "Synthetic Mod".into() });
                assert_eq!(file, "gfx/shared.png");
                assert_eq!(&from_url(url).pixels[..4], &[10, 200, 10, 255]);
                assert!(provenance.from_current_install);
                assert!(provenance.label.contains(CURRENT_INSTALL_ART_LABEL));
                assert!(provenance.label.contains("Synthetic Mod"));
            }
            other => panic!("expected mod override Ready, got {other:?}"),
        }

        let inherited = resolve_sprite(&cat, "GFX_BaseOnly", 0, 48);
        match &inherited {
            ArtResolution::Ready { origin, file, url, provenance, .. } => {
                assert_eq!(origin, &ArtOrigin::BaseGame);
                assert_eq!(file, "gfx/base_only.png");
                assert_eq!(&from_url(url).pixels[..4], &[10, 10, 200, 255]);
                assert!(provenance.from_current_install);
                assert!(provenance.label.contains("base game"));
            }
            other => panic!("expected base-game inheritance Ready, got {other:?}"),
        }

        let missing = resolve_sprite(&cat, "GFX_ModMissing", 0, 48);
        assert!(matches!(missing, ArtResolution::ConfirmedMissing { .. }), "{missing:?}");

        let native = |cat: &Catalogue, name: &str| {
            let layers = &cat.content.layers;
            let registry = resources::Registry::load(layers.presentation()).unwrap();
            resolve_native_equipment_icon(
                layers,
                &registry,
                None,
                "english",
                &crate::products::equipment_icon::Icon::Sprite(name.to_owned()),
                Decode::default(),
            )
        };
        match native(&cat, "GFX_Shared") {
            NativeIconResolution::Ready { origin, file, .. } => {
                assert_eq!(origin, ArtOrigin::Mod { id: "mod_synthetic".into(), name: "Synthetic Mod".into() });
                assert_eq!(file, "gfx/shared.png");
            }
            other => panic!("expected the native icon from the mod override, got {other:?}"),
        }
        assert!(matches!(native(&cat, "GFX_BaseOnly"), NativeIconResolution::Ready { origin: ArtOrigin::BaseGame, .. }));
        assert!(matches!(native(&cat, "GFX_ModMissing"), NativeIconResolution::ConfirmedMissing { .. }));

        let (unmounted_cat, _unmounted_folders) = two_layer_catalogue(
            &[
                (
                    "interface/icons.gfx",
                    b"spriteTypes={spriteType={name=GFX_Shared textureFile=gfx/shared.png} spriteType={name=GFX_BaseOnly textureFile=gfx/base_only.png}}",
                ),
                ("gfx/shared.png", &base_shared),
                ("gfx/base_only.png", &base_only),
            ],
            &[],
            &[],
            Some("not installed"),
        );

        let unmounted = resolve_sprite(&unmounted_cat, "GFX_Shared", 0, 48);
        match unmounted {
            ArtResolution::ConfirmedMissing { reason, origin, file, .. } => {
                assert!(reason.contains("Synthetic Mod"), "{reason}");
                assert!(reason.contains("not installed"), "{reason}");
                assert_eq!(origin, Some(ArtOrigin::Mod { id: "mod_synthetic".into(), name: "Synthetic Mod".into() }));
                assert_eq!(file.as_deref(), Some("gfx/shared.dds"));
            }
            other => panic!("expected ConfirmedMissing when mod unmounted, got {other:?}"),
        }
        match native(&unmounted_cat, "GFX_Shared") {
            NativeIconResolution::ConfirmedMissing { reason, .. } => assert!(reason.contains("not installed"), "{reason}"),
            other => panic!("the native icon must not fall through to base-game art, got {other:?}"),
        }
    }

    #[test]
    fn frame_geometry_comes_from_active_mod_sprite_definition_and_texture() {
        let base_strip = png_bytes(&solid(189, 27, [200, 20, 20, 255]));
        let mod_strip = png_bytes(&solid(216, 28, [20, 200, 80, 255]));
        let (cat, [_base, _addon]) = two_layer_catalogue(
            &[
                (
                    "interface/sample_panel.gfx",
                    b"spriteTypes={spriteType={name=GFX_sample_strip textureFile=gfx/interface/sample_strip.png noOfFrames=7}}",
                ),
                ("gfx/interface/sample_strip.png", &base_strip),
            ],
            &[
                (
                    "interface/sample_panel.gfx",
                    b"spriteTypes={spriteType={name=GFX_sample_strip textureFile=gfx/interface/sample_strip.png noOfFrames=8}}",
                ),
                ("gfx/interface/sample_strip.png", &mod_strip),
            ],
            &[],
            None,
        );

        let resolved = resolve_sprite(&cat, "GFX_sample_strip", 7, 48);
        match resolved {
            ArtResolution::Ready { origin, file, frame, url, .. } => {
                assert_eq!(origin, ArtOrigin::Mod { id: "mod_synthetic".into(), name: "Synthetic Mod".into() });
                assert_eq!(file, "gfx/interface/sample_strip.png");
                assert_eq!(
                    frame,
                    ArtFrame {
                        frame_index: 7,
                        frame_count: 8,
                        frame_width: 27,
                        frame_height: 28,
                        texture_width: 216,
                        texture_height: 28,
                        output_width: 27,
                        output_height: 28,
                    }
                );
                let img = from_url(&url);
                assert_eq!((img.width, img.height), (27, 28));
            }
            other => panic!("expected Ready with 8-frame 216x28 geometry, got {other:?}"),
        }
    }

    #[test]
    fn dx10_formats_resolve_and_unsupported_dxgi_is_refused() {
        let mut bgra = dx10(2, 2, 1, 91);
        bgra.extend([10, 20, 200, 255].repeat(4));
        let mut rgba = dx10(2, 2, 1, 29);
        rgba.extend([200, 20, 10, 255].repeat(4));
        let mut bad = dx10(2, 2, 1, 61);
        bad.extend([0; 16]);
        let (cat, [_base, _addon]) = two_layer_catalogue(
            &[],
            &[
                (
                    "interface/dx10.gfx",
                    b"spriteTypes={spriteType={name=GFX_BgraSrgb textureFile=gfx/bgra_srgb.dds} spriteType={name=GFX_RgbaSrgb textureFile=gfx/rgba_srgb.dds} spriteType={name=GFX_Unsupported textureFile=gfx/bad_dxgi.dds}}",
                ),
                ("gfx/bgra_srgb.dds", &bgra),
                ("gfx/rgba_srgb.dds", &rgba),
                ("gfx/bad_dxgi.dds", &bad),
            ],
            &[],
            None,
        );

        for name in ["GFX_BgraSrgb", "GFX_RgbaSrgb"] {
            match resolve_sprite(&cat, name, 0, 48) {
                ArtResolution::Ready { url, .. } => {
                    assert_eq!(&from_url(&url).pixels[..4], &[200, 20, 10, 255], "{name}");
                }
                other => panic!("expected Ready for {name}, got {other:?}"),
            }
        }
        match resolve_sprite(&cat, "GFX_Unsupported", 0, 48) {
            ArtResolution::Refused { reason, construct, .. } => {
                assert_eq!(construct, "undecodable-texture");
                assert!(reason.contains("DXGI format 61"), "{reason}");
            }
            other => panic!("expected Refused for unsupported DXGI, got {other:?}"),
        }
    }

    #[test]
    fn flags_resolve_mod_cosmetic_ideology_and_block_unmounted_vanilla_fallback() {
        let base_qqa = tga(4, 2, [0, 0, 220, 255]);
        let base_qqb = tga(4, 2, [40, 40, 40, 255]);
        let mod_qqa = tga(4, 2, [220, 40, 20, 255]);
        let mod_qqc = tga(4, 2, [20, 200, 20, 255]);
        let (cat, [_base, _addon]) = two_layer_catalogue(
            &[
                ("gfx/flags/QQA_red.tga", &base_qqa),
                ("gfx/flags/QQB.tga", &base_qqb),
            ],
            &[
                ("gfx/flags/QQA_red.tga", &mod_qqa),
                ("gfx/flags/QQC_AUTH_S_red.tga", &mod_qqc),
            ],
            &[],
            None,
        );
        let cells = [[4, 2]; 3];

        match resolve_flag(&cat.content.layers, "english", &[FlagSet::Normal], &cells, &["QQA_red".into(), "QQA".into()], 64) {
            ArtResolution::Ready { origin, file, url, frame, .. } => {
                assert_eq!(origin, ArtOrigin::Mod { id: "mod_synthetic".into(), name: "Synthetic Mod".into() });
                assert_eq!(file, "gfx/flags/QQA_red.tga");
                assert_eq!(frame.frame_width, 4);
                assert_eq!(&from_url(&url).pixels[..4], &[20, 40, 220, 255]);
            }
            other => panic!("expected mod QQA flag Ready, got {other:?}"),
        }

        match resolve_flag(&cat.content.layers, "english", &[FlagSet::Normal], &cells, &["QQC_AUTH_S_red".into(), "QQC".into()], 64) {
            ArtResolution::Ready { origin, file, .. } => {
                assert_eq!(origin, ArtOrigin::Mod { id: "mod_synthetic".into(), name: "Synthetic Mod".into() });
                assert_eq!(file, "gfx/flags/QQC_AUTH_S_red.tga");
            }
            other => panic!("expected mod cosmetic QQC flag Ready, got {other:?}"),
        }

        match resolve_flag(&cat.content.layers, "english", &[FlagSet::Normal], &cells, &["QQB".into()], 64) {
            ArtResolution::Ready { origin, file, .. } => {
                assert_eq!(origin, ArtOrigin::BaseGame);
                assert_eq!(file, "gfx/flags/QQB.tga");
            }
            other => panic!("expected inherited base QQB flag Ready, got {other:?}"),
        }

        let (unmounted_cat, _unmounted_folders) = two_layer_catalogue(
            &[
                ("gfx/flags/QQA_red.tga", &base_qqa),
                ("gfx/flags/QQB.tga", &base_qqb),
            ],
            &[],
            &[],
            Some("not installed"),
        );

        match resolve_flag(&unmounted_cat.content.layers, "english", &[FlagSet::Normal], &cells, &["QQA_red".into(), "QQA".into()], 64) {
            ArtResolution::ConfirmedMissing { reason, origin, file, .. } => {
                assert!(reason.contains("Synthetic Mod"), "{reason}");
                assert_eq!(origin, Some(ArtOrigin::Mod { id: "mod_synthetic".into(), name: "Synthetic Mod".into() }));
                assert_eq!(file.as_deref(), Some("gfx/flags/QQA_red.tga"));
            }
            other => panic!("expected ConfirmedMissing for unmounted mod flag, got {other:?}"),
        }
    }

    #[test]
    fn a_texture_found_under_another_spelling_decodes_from_the_file_found() {
        let (_, folder) = crate::catalogue::tests::localisation_catalogue(&[]);
        write_entries(&folder.0, &[("gfx/Sample_Icon.png", &png_bytes(&solid(4, 2, [30, 60, 90, 255])))]);
        let layers = ResolvedLayers::from_folders(&[(&folder.0, &[][..])]).unwrap();
        let layers = ResolvedLayers::from_test_layers(layers.layers).unwrap();
        let scope_key = art_scope_key(&layers, "english");
        let variant = ArtRenderVariant { max_edge: 48, decode: Decode::default() };
        match resolve_texture_in_stack(layers.presentation(), "gfx/sample_icon.png", 1, 0, variant, &scope_key) {
            ArtResolution::Ready { url, frame, .. } => {
                assert_eq!((frame.texture_width, frame.texture_height), (4, 2));
                assert_eq!(&from_url(&url).pixels[..4], &[30, 60, 90, 255]);
            }
            other => panic!("a texture the lookup found must decode, got {other:?}"),
        }
        assert!(matches!(
            resolve_native_texture_in_stack(layers.presentation(), "gfx/sample_icon.png", Decode::default(), &scope_key),
            NativeIconResolution::Ready { .. }
        ));
    }

    #[test]
    fn a_visual_mod_sprite_that_cannot_draw_falls_back_whole_never_its_metadata_over_recorded_pixels() {
        engine_executable::executable_tables::initialize_for_tests().unwrap();
        let (_, base) = crate::catalogue::tests::localisation_catalogue(&[]);
        let (_, visual) = crate::catalogue::tests::localisation_catalogue(&[]);
        crate::catalogue::texture_tests::write_sprite_program(&base.0);
        write_entries(&base.0, &[
            ("interface/sample.gfx", b"spriteTypes={spriteType={name=GFX_sample_badge textureFile=gfx/sample_badge.png}}"),
            ("gfx/sample_badge.png", &png_bytes(&solid(4, 4, [200, 10, 10, 255]))),
        ]);
        write_entries(&visual.0, &[
            ("interface/sample.gfx", b"spriteTypes={spriteType={name=GFX_sample_badge textureFile=gfx/sample_badge.png effectFile=gfx/FX/sample_absent.lua}}"),
            ("gfx/sample_badge.png", &png_bytes(&solid(8, 2, [10, 200, 10, 255]))),
        ]);
        let layers = content_reader_layers::content::fixtures::overlay_folders(&base.0, &visual.0, "Sample Art", false);
        let sprites = content_reader_gfx::gfx::read(layers.presentation()).unwrap();
        let sprite = sprites.find("GFX_sample_badge").unwrap().unwrap();
        let variant = ArtRenderVariant { max_edge: 48, decode: Decode::default() };
        match resolve_sprite_type_on_layers(&layers, "english", sprite, 0, variant) {
            ArtResolution::Ready { url, frame, origin, .. } => {
                let pixel: [u8; 4] = from_url(&url).pixels[..4].try_into().unwrap();
                let shown = (frame.texture_width, frame.texture_height, pixel);
                assert_eq!(shown, (4, 4, [200, 10, 10, 255]), "pixels and metadata both from the recorded sprite, origin {origin:?}");
            }
            other => panic!("the recorded sprite draws, got {other:?}"),
        }
    }

    #[test]
    fn scope_keys_separate_stack_order_replace_paths_dlc_and_language() {
        engine_executable::executable_tables::initialize_for_tests().unwrap();
        let (_, a) = folder_catalogue(&[]);
        let (_, b) = folder_catalogue(&[]);
        let ab = ResolvedLayers::from_folders(&[(&a.0, &[][..]), (&b.0, &[][..])]).unwrap();
        let ba = ResolvedLayers::from_folders(&[(&b.0, &[][..]), (&a.0, &[][..])]).unwrap();
        let replaced =
            ResolvedLayers::from_folders(&[(&a.0, &[][..]), (&b.0, &["gfx/flags"][..])]).unwrap();
        let mut with_dlc = ab.clone();
        with_dlc.layers[1].role = LayerRole::Dlc;

        assert_ne!(art_scope_key(&ab, "english"), art_scope_key(&ba, "english"));
        assert_ne!(art_scope_key(&ab, "english"), art_scope_key(&replaced, "english"));
        assert_ne!(art_scope_key(&ab, "english"), art_scope_key(&with_dlc, "english"));
        assert_ne!(art_scope_key(&ab, "english"), art_scope_key(&ab, "french"));
    }
}
